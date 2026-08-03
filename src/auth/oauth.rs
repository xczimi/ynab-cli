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

use crate::auth::{listener, unix_now};
use crate::error::{Error, Result};
use crate::secrets::{SecretKind, SecretStore};

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

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredAccessToken {
    access_token: String,
    expires_at: i64,
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
    let raw = if std::io::stdin().is_terminal() {
        print!("YNAB OAuth client ID: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line
    } else {
        let mut line = String::new();
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
    let raw = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("YNAB OAuth client secret: ")?
    } else {
        let mut line = String::new();
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
    if !reset
        && let (Some(id), Some(secret)) = (
            store.get(SecretKind::OauthClientId)?,
            store.get(SecretKind::OauthClientSecret)?,
        )
    {
        return Ok(AppCredentials {
            client_id: id.expose_secret().to_string(),
            client_secret: secret,
        });
    }

    let client_id = read_client_id()?;
    let client_secret = read_client_secret()?;

    store.set(
        SecretKind::OauthClientId,
        SecretString::from(client_id.clone()),
    )?;
    store.set(
        SecretKind::OauthClientSecret,
        SecretString::from(client_secret.expose_secret().to_string()),
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
    let stored = StoredAccessToken {
        access_token: access.to_string(),
        expires_at: unix_now() + expires_in_secs - EXPIRY_SAFETY_MARGIN_SECS,
    };
    let payload = serde_json::to_string(&stored)
        .map_err(|e| Error::Config(format!("failed to serialize OAuth access token: {e}")))?;

    store.set(SecretKind::OauthAccessToken, SecretString::from(payload))?;
    store.set(
        SecretKind::OauthRefreshToken,
        SecretString::from(refresh.to_string()),
    )?;
    Ok(())
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

    println!("Open this URL to authorize ynab-cli:\n{auth_url}");

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

    println!(
        "Logged in via OAuth. Access token valid for about 2 hours; it will refresh automatically."
    );
    Ok(())
}

/// `None` if there's no OAuth material at all (never logged in via OAuth).
/// Returns the current access token, refreshing it first if it's within
/// `REFRESH_LEEWAY_SECS` of expiry (or already expired). A refresh that
/// fails — including having no refresh token to use — surfaces as
/// `Error::NotAuthenticated`, which tells the user to run
/// `ynab auth login --oauth`.
pub async fn current_access_token(store: &SecretStore) -> Result<Option<SecretString>> {
    let Some(access_json) = store.get(SecretKind::OauthAccessToken)? else {
        return Ok(None);
    };

    let stored: StoredAccessToken =
        serde_json::from_str(access_json.expose_secret()).map_err(|_| Error::NotAuthenticated)?;

    if stored.expires_at > unix_now() + REFRESH_LEEWAY_SECS {
        return Ok(Some(SecretString::from(stored.access_token)));
    }

    let Some(refresh) = store.get(SecretKind::OauthRefreshToken)? else {
        return Err(Error::NotAuthenticated);
    };

    refresh_access_token(store, refresh.expose_secret()).await
}

async fn refresh_access_token(
    store: &SecretStore,
    refresh_token: &str,
) -> Result<Option<SecretString>> {
    let (Some(client_id), Some(client_secret)) = (
        store.get(SecretKind::OauthClientId)?,
        store.get(SecretKind::OauthClientSecret)?,
    ) else {
        return Err(Error::NotAuthenticated);
    };
    let creds = AppCredentials {
        client_id: client_id.expose_secret().to_string(),
        client_secret,
    };
    let client = build_client(&creds)?;
    let http = http_client()?;

    let token_result = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
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

    store_tokens(store, &new_access, expires_in, &new_refresh)?;

    Ok(Some(SecretString::from(new_access)))
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

        // expires_in negative enough to be already-expired even past the
        // safety margin, and no refresh token stored.
        let stored = StoredAccessToken {
            access_token: "stale".to_string(),
            expires_at: unix_now() - 1000,
        };
        store
            .set(
                SecretKind::OauthAccessToken,
                SecretString::from(serde_json::to_string(&stored).unwrap()),
            )
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(current_access_token(&store)).unwrap_err();
        assert!(matches!(err, Error::NotAuthenticated));
    }

    #[test]
    fn expired_token_refreshes_and_rotates_refresh_token() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();

        store
            .set(SecretKind::OauthClientId, SecretString::from("client-1"))
            .unwrap();
        store
            .set(
                SecretKind::OauthClientSecret,
                SecretString::from("secret-1"),
            )
            .unwrap();

        let stored = StoredAccessToken {
            access_token: "old-access".to_string(),
            expires_at: unix_now() - 10,
        };
        store
            .set(
                SecretKind::OauthAccessToken,
                SecretString::from(serde_json::to_string(&stored).unwrap()),
            )
            .unwrap();
        store
            .set(
                SecretKind::OauthRefreshToken,
                SecretString::from("old-refresh"),
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

        assert_eq!(
            store
                .get(SecretKind::OauthRefreshToken)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "new-refresh"
        );
    }
}
