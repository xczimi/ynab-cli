use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};

use crate::api::types::{DataEnvelope, ErrorEnvelope, User, UserWrapper};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.ynab.com/v1";

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    token: SecretString,
}

impl Client {
    pub fn new(token: SecretString) -> Self {
        Self::with_base_url(token, BASE_URL.to_string())
    }

    pub fn with_base_url(token: SecretString, base_url: String) -> Self {
        Client {
            http: reqwest::Client::new(),
            base_url,
            token,
        }
    }

    /// The ONLY http verb in this codebase is GET. Read-only is structural.
    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(self.token.expose_secret())
            .send()
            .await?;

        let status = resp.status();
        match status {
            s if s.is_success() => Ok(resp.json::<T>().await?),
            StatusCode::UNAUTHORIZED => Err(Error::NotAuthenticated),
            StatusCode::TOO_MANY_REQUESTS => Err(Error::RateLimited),
            s => {
                let message = match resp.json::<ErrorEnvelope>().await {
                    Ok(env) => env.error.detail,
                    Err(_) => s.canonical_reason().unwrap_or("unknown error").to_string(),
                };
                Err(Error::Api {
                    status: s.as_u16(),
                    message,
                })
            }
        }
    }

    pub async fn get_user(&self) -> Result<User> {
        let env: DataEnvelope<UserWrapper> = self.get_json("/user").await?;
        Ok(env.data.user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> Client {
        Client::with_base_url(SecretString::from("tok-1"), server.uri())
    }

    #[tokio::test]
    async fn get_user_success_sends_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("Authorization", "Bearer tok-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "user": { "id": "u-123" } }
            })))
            .mount(&server)
            .await;

        let user = client(&server).get_user().await.unwrap();
        assert_eq!(user.id, "u-123");
    }

    #[tokio::test]
    async fn unauthorized_maps_to_not_authenticated() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": { "id": "401", "name": "unauthorized", "detail": "Unauthorized" }
            })))
            .mount(&server)
            .await;

        let err = client(&server).get_user().await.unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }

    #[tokio::test]
    async fn rate_limit_maps_to_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let err = client(&server).get_user().await.unwrap_err();
        assert!(matches!(err, crate::error::Error::RateLimited));
    }

    #[tokio::test]
    async fn other_errors_carry_detail() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "error": { "id": "500", "name": "internal", "detail": "server exploded" }
            })))
            .mount(&server)
            .await;

        let err = client(&server).get_user().await.unwrap_err();
        assert_eq!(err.to_string(), "YNAB API error (500): server exploded");
    }
}
