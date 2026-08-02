use std::io::{BufRead, IsTerminal};

use secrecy::{ExposeSecret, SecretString};

use crate::api::client::Client;
use crate::error::{Error, Result};
use crate::secrets::{SecretKind, SecretStore};

fn make_client(token: SecretString, base_url: Option<String>) -> Client {
    match base_url {
        Some(url) => Client::with_base_url(token, url),
        None => Client::new(token),
    }
}

fn read_token() -> Result<SecretString> {
    let raw = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Paste your YNAB Personal Access Token: ")?
    } else {
        let mut line = String::new();
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
    let Some(token) = store.get(SecretKind::Pat)? else {
        println!("Not logged in. Run `ynab auth login`.");
        return Err(Error::NotAuthenticated);
    };
    let client = make_client(token, api_base_url);
    let user = client.get_user().await?;
    println!("Logged in (PAT). YNAB user id: {}", user.id);
    Ok(())
}

pub fn logout(store: &SecretStore) -> Result<()> {
    for kind in [
        SecretKind::Pat,
        SecretKind::OauthClientId,
        SecretKind::OauthClientSecret,
        SecretKind::OauthAccessToken,
        SecretKind::OauthRefreshToken,
    ] {
        store.delete(kind)?;
    }
    println!("Logged out. Credentials removed from the OS keychain.");
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

    #[tokio::test]
    async fn status_without_token_is_not_authenticated() {
        let store = mock_store();
        let err = status(&store, None).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }

    #[tokio::test]
    async fn status_with_valid_token_succeeds() {
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("tok"))
            .unwrap();
        let server = user_ok_server().await;
        status(&store, Some(server.uri())).await.unwrap();
    }

    #[tokio::test]
    async fn login_with_token_validates_and_stores() {
        let store = mock_store();
        let server = user_ok_server().await;
        login_with_token(&store, SecretString::from("tok-new"), Some(server.uri()))
            .await
            .unwrap();
        assert!(store.get(SecretKind::Pat).unwrap().is_some());
    }

    #[tokio::test]
    async fn login_with_bad_token_stores_nothing() {
        let store = mock_store();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = login_with_token(&store, SecretString::from("bad"), Some(server.uri()))
            .await
            .unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
    }

    #[tokio::test]
    async fn logout_removes_token() {
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("tok"))
            .unwrap();
        logout(&store).unwrap();
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
        // logout when already logged out is fine
        logout(&store).unwrap();
    }
}
