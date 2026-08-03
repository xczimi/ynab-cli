use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};

use crate::api::types::{
    AccountsWrapper, BudgetsWrapper, CategoryGroupsWrapper, DataEnvelope, ErrorEnvelope,
    PayeesWrapper, TransactionsWrapper, User, UserWrapper,
};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.ynab.com/v1";

#[derive(Debug)]
pub struct ListResult<T> {
    pub raw: serde_json::Value,
    pub parsed: T,
}

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

    fn append_param(path: String, param: &str) -> String {
        let sep = if path.contains('?') { '&' } else { '?' };
        format!("{path}{sep}{param}")
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

    /// GET `path`, keep the raw `data` object AND a typed parse of it.
    /// Raw is what `--json` prints — the API schema, mirrored exactly.
    pub(crate) async fn get_data<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<ListResult<T>> {
        let value: serde_json::Value = self.get_json(path).await?;
        let raw = value
            .get("data")
            .cloned()
            .ok_or_else(|| Error::Decode("missing data envelope".into()))?;
        let parsed =
            serde_json::from_value(raw.clone()).map_err(|e| Error::Decode(e.to_string()))?;
        Ok(ListResult { raw, parsed })
    }

    pub async fn get_budgets(&self) -> Result<ListResult<BudgetsWrapper>> {
        self.get_data("/budgets").await
    }

    pub async fn get_accounts(
        &self,
        budget: &str,
        last_knowledge: Option<i64>,
    ) -> Result<ListResult<AccountsWrapper>> {
        let mut path = format!("/budgets/{budget}/accounts");
        if let Some(k) = last_knowledge {
            path = Self::append_param(path, &format!("last_knowledge_of_server={k}"));
        }
        self.get_data(&path).await
    }

    pub async fn get_categories(
        &self,
        budget: &str,
        last_knowledge: Option<i64>,
    ) -> Result<ListResult<CategoryGroupsWrapper>> {
        let mut path = format!("/budgets/{budget}/categories");
        if let Some(k) = last_knowledge {
            path = Self::append_param(path, &format!("last_knowledge_of_server={k}"));
        }
        self.get_data(&path).await
    }

    pub async fn get_payees(
        &self,
        budget: &str,
        last_knowledge: Option<i64>,
    ) -> Result<ListResult<PayeesWrapper>> {
        let mut path = format!("/budgets/{budget}/payees");
        if let Some(k) = last_knowledge {
            path = Self::append_param(path, &format!("last_knowledge_of_server={k}"));
        }
        self.get_data(&path).await
    }

    pub async fn get_transactions(
        &self,
        budget: &str,
        since_date: Option<&str>,
        last_knowledge: Option<i64>,
    ) -> Result<ListResult<TransactionsWrapper>> {
        let mut path = match since_date {
            Some(d) => format!("/budgets/{budget}/transactions?since_date={d}"),
            None => format!("/budgets/{budget}/transactions"),
        };
        if let Some(k) = last_knowledge {
            path = Self::append_param(path, &format!("last_knowledge_of_server={k}"));
        }
        self.get_data(&path).await
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

    #[tokio::test]
    async fn get_budgets_parses_and_keeps_raw() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets"))
            .and(header("Authorization", "Bearer tok-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "budgets": [
                        { "id": "b-1", "name": "Family", "first_month": "2025-01-01",
                          "last_month": "2026-08-01",
                          "some_future_field": { "kept": true } }
                    ],
                    "default_budget": null
                }
            })))
            .mount(&server)
            .await;

        let result = client(&server).get_budgets().await.unwrap();
        assert_eq!(result.parsed.budgets.len(), 1);
        assert_eq!(result.parsed.budgets[0].name, "Family");
        // raw keeps fields the typed parse ignores
        assert_eq!(result.raw["budgets"][0]["some_future_field"]["kept"], true);
        assert!(result.raw.get("default_budget").is_some());
    }

    #[tokio::test]
    async fn get_data_missing_envelope_is_decode_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"nope": 1})))
            .mount(&server)
            .await;

        let err = client(&server).get_budgets().await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Decode(_)));
    }

    #[tokio::test]
    async fn get_accounts_hits_budget_scoped_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing", "type": "checking",
                      "on_budget": true, "closed": false, "balance": 100500,
                      "deleted": false }
                ], "server_knowledge": 42 }
            })))
            .mount(&server)
            .await;

        let r = client(&server).get_accounts("b-1", None).await.unwrap();
        assert_eq!(r.parsed.accounts[0].kind, "checking");
        assert_eq!(r.raw["server_knowledge"], 42);
    }

    #[tokio::test]
    async fn get_accounts_passes_last_knowledge() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .and(wiremock::matchers::query_param(
                "last_knowledge_of_server",
                "42",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [], "server_knowledge": 43 }
            })))
            .mount(&server)
            .await;
        client(&server).get_accounts("b-1", Some(42)).await.unwrap();
    }

    #[tokio::test]
    async fn get_transactions_passes_since_date() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/last-used/transactions"))
            .and(wiremock::matchers::query_param("since_date", "2026-07-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "transactions": [], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        let r = client(&server)
            .get_transactions("last-used", Some("2026-07-01"), None)
            .await
            .unwrap();
        assert!(r.parsed.transactions.is_empty());
    }

    #[tokio::test]
    async fn get_transactions_combines_since_and_knowledge() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/transactions"))
            .and(wiremock::matchers::query_param("since_date", "2026-07-01"))
            .and(wiremock::matchers::query_param(
                "last_knowledge_of_server",
                "7",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "transactions": [], "server_knowledge": 8 }
            })))
            .mount(&server)
            .await;
        client(&server)
            .get_transactions("b-1", Some("2026-07-01"), Some(7))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_categories_and_payees_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/categories"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "category_groups": [], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/payees"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "payees": [
                    { "id": "p-1", "name": "Grocer", "deleted": false }
                ], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        let c = client(&server).get_categories("b-1", None).await.unwrap();
        assert!(c.parsed.category_groups.is_empty());
        let p = client(&server).get_payees("b-1", None).await.unwrap();
        assert_eq!(p.parsed.payees[0].name, "Grocer");
    }
}
