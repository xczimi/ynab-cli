use serde_json::Value;

use crate::api::client::{Client, ListResult};
use crate::api::types::{
    AccountsWrapper, CategoryGroupsWrapper, PayeesWrapper, TransactionsWrapper,
};
use crate::cache::Cache;
use crate::error::{Error, Result};

/// (id, raw entity) pairs from an envelope array; entities without a string
/// id are skipped (defensive — the API always sends ids).
fn entity_pairs(raw: &Value, key: &str) -> Vec<(String, Value)> {
    raw.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let id = v.get("id")?.as_str()?.to_string();
                    Some((id, v.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn envelope<T: serde::de::DeserializeOwned>(
    cache: &Cache,
    budget: &str,
    resource: &str,
    key: &str,
    order: Option<&str>,
) -> Result<ListResult<T>> {
    let all = cache.load_entities(budget, resource, order)?;
    let sk = cache.server_knowledge(budget, resource)?.unwrap_or(0);
    let raw = serde_json::json!({ key: all, "server_knowledge": sk });
    let parsed = serde_json::from_value(raw.clone()).map_err(|e| Error::Decode(e.to_string()))?;
    Ok(ListResult { raw, parsed })
}

fn store_knowledge(cache: &Cache, budget: &str, resource: &str, raw: &Value) -> Result<()> {
    if let Some(sk) = raw.get("server_knowledge").and_then(Value::as_i64) {
        cache.set_server_knowledge(budget, resource, sk)?;
    }
    Ok(())
}

pub async fn accounts(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<AccountsWrapper>> {
    let known = cache.server_knowledge(budget, "accounts")?;
    let fetched = client.get_accounts(budget, known).await?;
    cache.upsert_entities(budget, "accounts", &entity_pairs(&fetched.raw, "accounts"))?;
    store_knowledge(cache, budget, "accounts", &fetched.raw)?;
    envelope(cache, budget, "accounts", "accounts", Some("$.name"))
}

pub async fn payees(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<PayeesWrapper>> {
    let known = cache.server_knowledge(budget, "payees")?;
    let fetched = client.get_payees(budget, known).await?;
    cache.upsert_entities(budget, "payees", &entity_pairs(&fetched.raw, "payees"))?;
    store_knowledge(cache, budget, "payees", &fetched.raw)?;
    envelope(cache, budget, "payees", "payees", Some("$.name"))
}

/// Categories: always a FULL fetch, replaced wholesale (locked decision —
/// nested-group delta merge isn't worth it for a small payload).
pub async fn categories(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<CategoryGroupsWrapper>> {
    let fetched = client.get_categories(budget, None).await?;
    cache.replace_entities(
        budget,
        "category_groups",
        &entity_pairs(&fetched.raw, "category_groups"),
    )?;
    store_knowledge(cache, budget, "category_groups", &fetched.raw)?;
    envelope(cache, budget, "category_groups", "category_groups", None)
}

/// Transactions: delta sync, NEVER since_date — the cache must stay a
/// complete record; `--since` is applied locally by the filter layer.
pub async fn transactions(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<TransactionsWrapper>> {
    let known = cache.server_knowledge(budget, "transactions")?;
    let fetched = client.get_transactions(budget, None, known).await?;
    cache.upsert_entities(
        budget,
        "transactions",
        &entity_pairs(&fetched.raw, "transactions"),
    )?;
    store_knowledge(cache, budget, "transactions", &fetched.raw)?;
    envelope(
        cache,
        budget,
        "transactions",
        "transactions",
        Some("$.date"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::Client;
    use crate::cache::Cache;
    use crate::secrets::SecretStore;
    use secrecy::SecretString;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    fn cache_in(dir: &tempfile::TempDir) -> Cache {
        Cache::open_at(&mock_store(), &dir.path().join("cache.db")).unwrap()
    }

    fn client(server: &MockServer) -> Client {
        Client::with_base_url(SecretString::from("t"), server.uri())
    }

    #[tokio::test]
    async fn accounts_first_sync_then_delta_merge() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache_in(&dir);

        // first sync: no knowledge yet → no last_knowledge_of_server param
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing", "type": "checking", "on_budget": true,
                      "closed": false, "balance": 1000, "deleted": false, "note": "keepme" }
                ], "server_knowledge": 10 }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let first = accounts(&client(&server), &mut cache, "b-1").await.unwrap();
        assert_eq!(first.parsed.accounts.len(), 1);
        assert_eq!(first.raw["server_knowledge"], 10);
        assert_eq!(first.raw["accounts"][0]["note"], "keepme"); // unknown field survives
        server.reset().await;

        // second sync: sends knowledge 10, gets a delta (one changed + one new)
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .and(query_param("last_knowledge_of_server", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing RENAMED", "type": "checking",
                      "on_budget": true, "closed": false, "balance": 2000, "deleted": false },
                    { "id": "a-2", "name": "Savings", "type": "savings", "on_budget": true,
                      "closed": false, "balance": 500, "deleted": false }
                ], "server_knowledge": 11 }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let second = accounts(&client(&server), &mut cache, "b-1").await.unwrap();
        assert_eq!(second.parsed.accounts.len(), 2);
        assert_eq!(second.raw["server_knowledge"], 11);
        let names: Vec<&str> = second
            .parsed
            .accounts
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert!(names.contains(&"Chequing RENAMED"));
        assert!(names.contains(&"Savings"));
    }

    #[tokio::test]
    async fn categories_replace_wholesale() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache_in(&dir);
        let body = |groups: serde_json::Value, sk: i64| serde_json::json!({ "data": { "category_groups": groups, "server_knowledge": sk } });
        let group = |id: &str, name: &str| {
            serde_json::json!({ "id": id, "name": name, "hidden": false, "deleted": false,
                                "categories": [] })
        };

        Mock::given(method("GET"))
            .and(path("/budgets/b-1/categories"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(body(serde_json::json!([group("g-1", "Bills")]), 5)),
            )
            .expect(1)
            .mount(&server)
            .await;
        categories(&client(&server), &mut cache, "b-1")
            .await
            .unwrap();
        server.reset().await;

        // full refetch replaces — g-1 gone, g-2 present; NO last_knowledge param sent
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/categories"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(body(serde_json::json!([group("g-2", "Fun")]), 6)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let second = categories(&client(&server), &mut cache, "b-1")
            .await
            .unwrap();
        assert_eq!(second.parsed.category_groups.len(), 1);
        assert_eq!(second.parsed.category_groups[0].id, "g-2");
    }

    #[tokio::test]
    async fn transactions_sync_never_sends_since_date() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache_in(&dir);
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/transactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "transactions": [
                    { "id": "t-1", "date": "2026-07-15", "amount": -100, "memo": null,
                      "approved": true, "account_id": "a-1", "account_name": "Chq",
                      "payee_id": null, "payee_name": null, "category_id": null,
                      "category_name": null, "deleted": false }
                ], "server_knowledge": 3 }
            })))
            .mount(&server)
            .await;
        let result = transactions(&client(&server), &mut cache, "b-1")
            .await
            .unwrap();
        assert_eq!(result.parsed.transactions.len(), 1);
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].url.query().unwrap_or("").contains("since_date"));
    }
}
