use std::io::{BufRead, IsTerminal};

use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::api::client::Client;
use crate::cache::Cache;
use crate::error::{Error, Result};
use crate::secrets::{SecretKind, SecretStore};

fn make_client(token: SecretString, base_url: Option<String>) -> Client {
    match base_url {
        Some(url) => Client::with_base_url(token, url),
        None => Client::new(token),
    }
}

fn read_token() -> Result<SecretString> {
    let raw: Zeroizing<String> = if std::io::stdin().is_terminal() {
        Zeroizing::new(rpassword::prompt_password(
            "Paste your YNAB Personal Access Token: ",
        )?)
    } else {
        let mut line = Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        line
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("no token provided".into()));
    }
    Ok(SecretString::from(trimmed.to_string()))
}

pub async fn login(store: &SecretStore, api_base_url: Option<String>) -> Result<()> {
    let token = read_token()?;
    login_with_token(store, token, api_base_url).await
}

pub async fn login_with_token(
    store: &SecretStore,
    token: SecretString,
    api_base_url: Option<String>,
) -> Result<()> {
    let client_token = SecretString::from(token.expose_secret().to_string());
    let client = make_client(client_token, api_base_url);
    let user = client.get_user().await?;
    store.set(SecretKind::Pat, token)?;
    println!("Logged in. YNAB user id: {}", user.id);
    Ok(())
}

pub async fn status(store: &SecretStore, api_base_url: Option<String>) -> Result<()> {
    if let Some(token) = store.get(SecretKind::Pat)? {
        let client = make_client(token, api_base_url);
        let user = client.get_user().await?;
        println!("Logged in (PAT). YNAB user id: {}", user.id);
        return Ok(());
    }

    match crate::auth::oauth::current_access_token(store).await {
        Ok(Some(token)) => {
            let client = make_client(token, api_base_url);
            let user = client.get_user().await?;
            println!("Logged in (OAuth). YNAB user id: {}", user.id);
            Ok(())
        }
        Ok(None) | Err(Error::NotAuthenticated) => {
            println!("Not logged in. Run `ynab auth login`.");
            Err(Error::NotAuthenticated)
        }
        Err(e) => Err(e),
    }
}

/// Removes every credential kind (PAT, OAuth app + token material, cache
/// key) and the cache DB itself (with its SQLite/SQLCipher siblings) — a
/// full-scope logout, per the 2026-08-02 user ruling.
pub fn logout(store: &SecretStore) -> Result<()> {
    // Legacy kinds included so a logout on an install that never ran the
    // OAuth-state migration still leaves nothing behind.
    for kind in [SecretKind::Pat, SecretKind::Oauth, SecretKind::CacheKey]
        .into_iter()
        .chain(crate::secrets::LEGACY_OAUTH_KINDS)
    {
        store.delete(kind)?;
    }
    if let Ok(path) = Cache::db_path() {
        crate::cache::remove_db_and_siblings(&path);
    }
    println!("Logged out. Credentials and cached data removed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{SecretKind, SecretStore};
    use secrecy::SecretString;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    async fn user_ok_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "user": { "id": "u-1" } }
            })))
            .mount(&server)
            .await;
        server
    }

    // Plain `#[test]` + `Runtime::block_on` (not `#[tokio::test]`) throughout
    // this module, matching `auth::oauth`'s tests: holding the std
    // `TEST_LOCK` `MutexGuard` across an `.await` inside an `async fn` trips
    // clippy's `await_holding_lock`; a sync test driving its own runtime
    // avoids that entirely.

    #[test]
    fn status_without_token_is_not_authenticated() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(status(&store, None)).unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }

    #[test]
    fn status_with_valid_token_succeeds() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("tok"))
            .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = user_ok_server().await;
            status(&store, Some(server.uri())).await.unwrap();
        });
    }

    #[test]
    fn status_with_oauth_token_succeeds() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let stored_oauth = serde_json::json!({
            "access_token": "oauth-access",
            "expires_at": crate::auth::unix_now() + 3600,
        });
        store
            .set(
                SecretKind::Oauth,
                SecretString::from(stored_oauth.to_string()),
            )
            .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = user_ok_server().await;
            status(&store, Some(server.uri())).await.unwrap();
        });
    }

    #[test]
    fn login_with_token_validates_and_stores() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = user_ok_server().await;
            login_with_token(&store, SecretString::from("tok-new"), Some(server.uri()))
                .await
                .unwrap();
        });
        assert!(store.get(SecretKind::Pat).unwrap().is_some());
    }

    #[test]
    fn login_with_bad_token_stores_nothing() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(async {
                let server = MockServer::start().await;
                Mock::given(method("GET"))
                    .and(path("/user"))
                    .respond_with(ResponseTemplate::new(401))
                    .mount(&server)
                    .await;
                login_with_token(&store, SecretString::from("bad"), Some(server.uri())).await
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
    }

    #[test]
    fn logout_removes_token() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("tok"))
            .unwrap();
        logout(&store).unwrap();
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
        // logout when already logged out is fine
        logout(&store).unwrap();
    }

    #[test]
    fn logout_clears_cache_file_and_cache_key() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("tok"))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        temp_env::with_var(
            "YNAB_CLI_DATA_DIR",
            Some(data_dir.to_string_lossy().to_string()),
            || {
                {
                    let _cache = Cache::open_at(&store, &data_dir.join("cache.db")).unwrap();
                }
                assert!(data_dir.join("cache.db").exists());
                assert!(store.get(SecretKind::CacheKey).unwrap().is_some());

                logout(&store).unwrap();

                assert!(!data_dir.join("cache.db").exists());
            },
        );
        assert!(store.get(SecretKind::CacheKey).unwrap().is_none());
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
    }
}
