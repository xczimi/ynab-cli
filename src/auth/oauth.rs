//! OAuth Authorization Code flow: app-credential storage, the interactive
//! login flow, and transparent access-token refresh.
//!
//! `oauth2` v5 performs the token-endpoint HTTP POST internally via
//! [`oauth2::TokenResponse`]/`request_async` — this module never issues an
//! HTTP write itself (no `reqwest::Client::post` call anywhere below).

use std::io::{BufRead, IsTerminal, Write};

use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::auth::{listener, unix_now};
use crate::error::{Error, Result};
use crate::secrets::{LEGACY_OAUTH_KINDS, SecretKind, SecretStore};

const DEFAULT_OAUTH_BASE_URL: &str = "https://app.ynab.com";
const DEFAULT_PORT: u16 = 53682;
/// Subtracted from `expires_in` when computing `expires_at`, so a token
/// considered "unexpired" always has a little headroom left.
const EXPIRY_SAFETY_MARGIN_SECS: i64 = 60;
/// `current_access_token` treats a token as needing refresh once fewer than
/// this many seconds remain, so callers never hand out a token that expires
/// mid-request.
const REFRESH_LEEWAY_SECS: i64 = 300;
/// YNAB access tokens are documented as valid for about 2 hours; used only
/// as a fallback if a token response omits `expires_in`.
const FALLBACK_EXPIRES_IN_SECS: i64 = 7200;

pub struct AppCredentials {
    pub client_id: String,
    pub client_secret: SecretString,
}

type ConfiguredClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

/// Every piece of OAuth state lives in ONE keychain entry.
///
/// macOS keychain ACLs are per-item and are pinned to the calling binary's
/// code signature, so each additional entry costs the user another "Always
/// Allow" prompt — and an unsigned local build earns a fresh code hash on
/// every rebuild, which brings all of those prompts back. Folding the four
/// former entries into one means one grant, and one keychain read per
/// command instead of four.
///
/// Every field is optional because the state is built up in stages: app
/// credentials are stored before the browser flow returns any token.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct OauthState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

/// The legacy `oauth-access-token` entry's payload, parsed only by
/// [`migrate_legacy_state`].
#[derive(serde::Deserialize)]
struct LegacyAccessToken {
    access_token: String,
    expires_at: i64,
}

/// Reads the whole OAuth state in a single keychain access, migrating a
/// pre-consolidation install on the way if it finds one. An unparseable
/// entry is `NotAuthenticated` rather than a hard error: the remedy is
/// always `ynab auth login --oauth`.
fn load_state(store: &SecretStore) -> Result<OauthState> {
    match store.get(SecretKind::Oauth)? {
        Some(raw) => serde_json::from_str(raw.expose_secret()).map_err(|_| Error::NotAuthenticated),
        None => migrate_legacy_state(store),
    }
}

fn save_state(store: &SecretStore, state: &OauthState) -> Result<()> {
    let payload = Zeroizing::new(
        serde_json::to_string(state)
            .map_err(|e| Error::Config(format!("failed to serialize OAuth state: {e}")))?,
    );
    store.set(SecretKind::Oauth, SecretString::from(payload.as_str()))
}

/// Installs predating the single-entry layout kept OAuth state in four
/// separate keychain entries. Fold whatever is there into one entry and
/// delete the originals, so the user re-grants keychain access once instead
/// of re-running the whole OAuth login. Returns the default (empty) state
/// when there is nothing to migrate, which is the common case.
///
/// Deletes are best-effort: a consolidated entry that was written
/// successfully is the thing that matters, and a leftover legacy entry is
/// inert once `load_state` stops reading it.
fn migrate_legacy_state(store: &SecretStore) -> Result<OauthState> {
    let legacy_client_id = store.get(SecretKind::LegacyOauthClientId)?;
    let legacy_access = store.get(SecretKind::LegacyOauthAccessToken)?;
    if legacy_client_id.is_none() && legacy_access.is_none() {
        return Ok(OauthState::default());
    }

    // A legacy access-token entry too corrupt to parse is dropped rather
    // than failing the migration: the refresh token below can still rescue
    // the session, and if it can't, the user re-logs in either way.
    let (access_token, expires_at) = legacy_access
        .and_then(|raw| serde_json::from_str::<LegacyAccessToken>(raw.expose_secret()).ok())
        .map(|t| (Some(t.access_token), Some(t.expires_at)))
        .unwrap_or((None, None));

    let state = OauthState {
        client_id: legacy_client_id.map(|s| s.expose_secret().to_string()),
        client_secret: store
            .get(SecretKind::LegacyOauthClientSecret)?
            .map(|s| s.expose_secret().to_string()),
        access_token,
        expires_at,
        refresh_token: store
            .get(SecretKind::LegacyOauthRefreshToken)?
            .map(|s| s.expose_secret().to_string()),
    };

    save_state(store, &state)?;
    for kind in LEGACY_OAUTH_KINDS {
        let _ = store.delete(kind);
    }
    Ok(state)
}

/// Returns `state` with a freshly issued token pair folded in, leaving the
/// app credentials untouched.
fn with_tokens(state: OauthState, access: &str, expires_in_secs: i64, refresh: &str) -> OauthState {
    OauthState {
        access_token: Some(access.to_string()),
        expires_at: Some(unix_now() + expires_in_secs - EXPIRY_SAFETY_MARGIN_SECS),
        refresh_token: Some(refresh.to_string()),
        ..state
    }
}

fn oauth_base_url() -> String {
    std::env::var("YNAB_CLI_OAUTH_BASE_URL").unwrap_or_else(|_| DEFAULT_OAUTH_BASE_URL.to_string())
}

fn authorize_url_str() -> String {
    format!("{}/oauth/authorize", oauth_base_url())
}

fn token_url_str() -> String {
    format!("{}/oauth/token", oauth_base_url())
}

fn oauth_port() -> u16 {
    std::env::var("YNAB_CLI_OAUTH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn redirect_uri_str(port: u16) -> String {
    format!("http://localhost:{port}/callback")
}

fn build_client(creds: &AppCredentials) -> Result<ConfiguredClient> {
    let auth_url = AuthUrl::new(authorize_url_str())
        .map_err(|e| Error::Config(format!("invalid OAuth authorize URL: {e}")))?;
    let token_url = TokenUrl::new(token_url_str())
        .map_err(|e| Error::Config(format!("invalid OAuth token URL: {e}")))?;

    Ok(BasicClient::new(ClientId::new(creds.client_id.clone()))
        .set_client_secret(ClientSecret::new(
            creds.client_secret.expose_secret().to_string(),
        ))
        .set_auth_uri(auth_url)
        .set_token_uri(token_url))
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(Error::from)
}

fn read_client_id() -> Result<String> {
    // `client_id` isn't secret, but the read buffer is wrapped in
    // `Zeroizing` anyway: it keeps this function's shape identical to
    // `read_client_secret` and `read_token`, and costs nothing.
    let raw: Zeroizing<String> = if std::io::stdin().is_terminal() {
        print!("YNAB OAuth client ID: ");
        std::io::stdout().flush()?;
        let mut line = Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        line
    } else {
        let mut line = Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        line
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("no OAuth client ID provided".into()));
    }
    Ok(trimmed.to_string())
}

fn read_client_secret() -> Result<SecretString> {
    let raw: Zeroizing<String> = if std::io::stdin().is_terminal() {
        Zeroizing::new(rpassword::prompt_password("YNAB OAuth client secret: ")?)
    } else {
        let mut line = Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        line
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("no OAuth client secret provided".into()));
    }
    Ok(SecretString::from(trimmed.to_string()))
}

/// Keychain first; prompts for and stores fresh credentials when missing or
/// `reset` is set. Prompting mirrors PAT login's TTY/pipe handling: visible
/// input for the client id, hidden for the secret when interactive.
pub fn get_or_prompt_app_credentials(store: &SecretStore, reset: bool) -> Result<AppCredentials> {
    let state = load_state(store)?;

    if !reset
        && let (Some(id), Some(secret)) =
            (state.client_id.as_deref(), state.client_secret.as_deref())
    {
        return Ok(AppCredentials {
            client_id: id.to_string(),
            client_secret: SecretString::from(secret),
        });
    }

    let client_id = read_client_id()?;
    let client_secret = read_client_secret()?;

    // Keeps any existing token pair: re-entering app credentials shouldn't
    // silently log the user out of a session that's still valid.
    save_state(
        store,
        &OauthState {
            client_id: Some(client_id.clone()),
            client_secret: Some(client_secret.expose_secret().to_string()),
            ..state
        },
    )?;

    Ok(AppCredentials {
        client_id,
        client_secret,
    })
}

/// Stores the access token (as JSON carrying its computed expiry) and the
/// raw refresh token. Callers doing a refresh MUST call this before handing
/// the new access token back to anyone — refresh tokens are single-use and
/// rotate on every refresh.
pub fn store_tokens(
    store: &SecretStore,
    access: &str,
    expires_in_secs: i64,
    refresh: &str,
) -> Result<()> {
    let state = load_state(store)?;
    save_state(store, &with_tokens(state, access, expires_in_secs, refresh))
}

/// Full interactive OAuth login: obtains app credentials, builds the
/// authorize URL (always printed; browser opened best-effort unless
/// `YNAB_CLI_NO_BROWSER` is set), waits for the localhost redirect, exchanges
/// the code, and stores the resulting token pair.
pub async fn login(store: &SecretStore, reset_app: bool) -> Result<()> {
    let creds = get_or_prompt_app_credentials(store, reset_app)?;
    let port = oauth_port();

    let client = build_client(&creds)?.set_redirect_uri(
        RedirectUrl::new(redirect_uri_str(port))
            .map_err(|e| Error::Config(format!("invalid OAuth redirect URL: {e}")))?,
    );

    let (auth_url, csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("read-only".to_string()))
        .url();

    crate::output::print_line(&format!("Open this URL to authorize ynab-cli:\n{auth_url}"))?;

    if std::env::var("YNAB_CLI_NO_BROWSER").is_err() {
        let _ = webbrowser::open(auth_url.as_str());
    }

    let code = listener::wait_for_code(port, csrf_token.secret())?;

    let http = http_client()?;
    let token_result = client
        .exchange_code(AuthorizationCode::new(code))
        .request_async(&http)
        .await
        .map_err(|e| Error::Config(format!("OAuth token exchange failed: {e}")))?;

    let access = token_result.access_token().secret().to_string();
    let expires_in = token_result
        .expires_in()
        .map(|d| d.as_secs() as i64)
        .unwrap_or(FALLBACK_EXPIRES_IN_SECS);
    let refresh = token_result
        .refresh_token()
        .ok_or_else(|| Error::Config("OAuth server did not return a refresh token".into()))?
        .secret()
        .to_string();

    store_tokens(store, &access, expires_in, &refresh)?;

    crate::output::print_line(
        "Logged in via OAuth. Access token valid for about 2 hours; it will refresh automatically.",
    )?;
    Ok(())
}

/// `None` if there's no OAuth material at all (never logged in via OAuth).
/// Returns the current access token, refreshing it first if it's within
/// `REFRESH_LEEWAY_SECS` of expiry (or already expired). A refresh that
/// fails — including having no refresh token to use — surfaces as
/// `Error::NotAuthenticated`, which tells the user to run
/// `ynab auth login --oauth`.
pub async fn current_access_token(store: &SecretStore) -> Result<Option<SecretString>> {
    let state = load_state(store)?;

    let (Some(access_token), Some(expires_at)) = (state.access_token.as_deref(), state.expires_at)
    else {
        return Ok(None);
    };

    if expires_at > unix_now() + REFRESH_LEEWAY_SECS {
        return Ok(Some(SecretString::from(access_token)));
    }

    refresh_access_token(store, state).await
}

/// Takes the already-loaded `state` rather than re-reading it, so a refresh
/// costs one keychain read and one write in total.
async fn refresh_access_token(
    store: &SecretStore,
    state: OauthState,
) -> Result<Option<SecretString>> {
    let (Some(client_id), Some(client_secret), Some(refresh_token)) = (
        state.client_id.as_deref(),
        state.client_secret.as_deref(),
        state.refresh_token.as_deref(),
    ) else {
        return Err(Error::NotAuthenticated);
    };
    let creds = AppCredentials {
        client_id: client_id.to_string(),
        client_secret: SecretString::from(client_secret),
    };
    let refresh_token = refresh_token.to_string();
    let client = build_client(&creds)?;
    let http = http_client()?;

    let token_result = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token))
        .request_async(&http)
        .await
        .map_err(|_| Error::NotAuthenticated)?;

    let new_access = token_result.access_token().secret().to_string();
    let expires_in = token_result
        .expires_in()
        .map(|d| d.as_secs() as i64)
        .unwrap_or(FALLBACK_EXPIRES_IN_SECS);
    // Refresh tokens rotate on every use; a response without one would mean
    // reusing the just-spent token, which YNAB's server will reject next
    // time — treat it as a refresh failure rather than silently keeping the
    // stale token around.
    let new_refresh = token_result
        .refresh_token()
        .ok_or(Error::NotAuthenticated)?
        .secret()
        .to_string();

    save_state(
        store,
        &with_tokens(state, &new_access, expires_in, &new_refresh),
    )?;

    Ok(Some(SecretString::from(new_access.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn store_and_current_roundtrip_unexpired_token() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        store_tokens(&store, "access-abc", 3600, "refresh-xyz").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let got = rt.block_on(current_access_token(&store)).unwrap();
        assert_eq!(got.unwrap().expose_secret(), "access-abc");
    }

    #[test]
    fn no_oauth_material_returns_none() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let got = rt.block_on(current_access_token(&store)).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn expired_without_refresh_token_is_not_authenticated() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        // Already expired even past the safety margin, and no refresh token.
        save_state(
            &store,
            &OauthState {
                access_token: Some("stale".to_string()),
                expires_at: Some(unix_now() - 1000),
                ..OauthState::default()
            },
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(current_access_token(&store)).unwrap_err();
        assert!(matches!(err, Error::NotAuthenticated));
    }

    #[test]
    fn app_credentials_and_tokens_share_one_entry() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        save_state(
            &store,
            &OauthState {
                client_id: Some("client-1".to_string()),
                client_secret: Some("secret-1".to_string()),
                ..OauthState::default()
            },
        )
        .unwrap();
        store_tokens(&store, "access-abc", 3600, "refresh-xyz").unwrap();

        // Storing tokens must not drop the app credentials stored earlier,
        // and everything must land in the single `Oauth` entry.
        let state = load_state(&store).unwrap();
        assert_eq!(state.client_id.as_deref(), Some("client-1"));
        assert_eq!(state.client_secret.as_deref(), Some("secret-1"));
        assert_eq!(state.access_token.as_deref(), Some("access-abc"));
        assert_eq!(state.refresh_token.as_deref(), Some("refresh-xyz"));

        for kind in LEGACY_OAUTH_KINDS {
            assert!(store.get(kind).unwrap().is_none());
        }
    }

    #[test]
    fn migrates_legacy_split_entries_into_one() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        // Exactly what a pre-consolidation install left behind.
        store
            .set(
                SecretKind::LegacyOauthClientId,
                SecretString::from("client-1"),
            )
            .unwrap();
        store
            .set(
                SecretKind::LegacyOauthClientSecret,
                SecretString::from("secret-1"),
            )
            .unwrap();
        store
            .set(
                SecretKind::LegacyOauthAccessToken,
                SecretString::from(
                    serde_json::json!({
                        "access_token": "legacy-access",
                        "expires_at": unix_now() + 3600,
                    })
                    .to_string(),
                ),
            )
            .unwrap();
        store
            .set(
                SecretKind::LegacyOauthRefreshToken,
                SecretString::from("legacy-refresh"),
            )
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let got = rt.block_on(current_access_token(&store)).unwrap();
        assert_eq!(got.unwrap().expose_secret(), "legacy-access");

        // Every field carried over into the single entry...
        let state = load_state(&store).unwrap();
        assert_eq!(state.client_id.as_deref(), Some("client-1"));
        assert_eq!(state.client_secret.as_deref(), Some("secret-1"));
        assert_eq!(state.access_token.as_deref(), Some("legacy-access"));
        assert_eq!(state.refresh_token.as_deref(), Some("legacy-refresh"));

        // ...and the old entries are gone, so they're never prompted for again.
        for kind in LEGACY_OAUTH_KINDS {
            assert!(store.get(kind).unwrap().is_none());
        }
    }

    #[test]
    fn nothing_stored_needs_no_migration() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        // A fresh install must not leave an empty `Oauth` entry behind just
        // by being asked for a token.
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(rt.block_on(current_access_token(&store)).unwrap().is_none());
        assert!(store.get(SecretKind::Oauth).unwrap().is_none());
    }

    #[test]
    fn expired_token_refreshes_and_rotates_refresh_token() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        save_state(
            &store,
            &OauthState {
                client_id: Some("client-1".to_string()),
                client_secret: Some("secret-1".to_string()),
                access_token: Some("old-access".to_string()),
                expires_at: Some(unix_now() - 10),
                refresh_token: Some("old-refresh".to_string()),
            },
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .and(body_string_contains("grant_type=refresh_token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "access_token": "new-access",
                    "token_type": "bearer",
                    "expires_in": 7200,
                    "refresh_token": "new-refresh",
                })))
                .mount(&server)
                .await;
            server
        });

        let got = temp_env::with_var("YNAB_CLI_OAUTH_BASE_URL", Some(server.uri()), || {
            rt.block_on(current_access_token(&store))
        })
        .unwrap();
        assert_eq!(got.unwrap().expose_secret(), "new-access");

        // The rotated refresh token is persisted, and the app credentials
        // survive the refresh.
        let state = load_state(&store).unwrap();
        assert_eq!(state.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(state.client_id.as_deref(), Some("client-1"));
    }
}
