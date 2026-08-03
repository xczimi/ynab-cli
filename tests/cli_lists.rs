use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ynab(config_dir: &std::path::Path, base_url: &str) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", config_dir);
    cmd.env("YNAB_CLI_API_BASE_URL", base_url);
    cmd.env("YNAB_PAT", "e2e-token");
    cmd
}

#[tokio::test(flavor = "multi_thread")]
async fn budgets_list_renders_table_and_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/budgets"))
        .and(header("Authorization", "Bearer e2e-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "budgets": [
                { "id": "b-1", "name": "Family", "first_month": "2025-01-01",
                  "last_month": "2026-08-01", "extra_api_field": 7 }
            ], "default_budget": null }
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        let dir = tempfile::tempdir().unwrap();
        // human table
        ynab(dir.path(), &uri)
            .args(["budgets", "list"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Family"))
            .stdout(predicate::str::contains("b-1"));
        // raw json mirrors schema including unknown fields
        ynab(dir.path(), &uri)
            .args(["budgets", "list", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"extra_api_field\": 7"));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn transactions_list_filters_and_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/budgets/last-used/transactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "transactions": [
                { "id": "t-1", "date": "2026-07-15", "amount": -12340,
                  "memo": "weekly shop", "approved": true,
                  "account_id": "a-1", "account_name": "Chequing",
                  "payee_id": "p-1", "payee_name": "Corner Grocer",
                  "category_id": "c-1", "category_name": "Groceries",
                  "deleted": false, "subtransactions": [] },
                { "id": "t-2", "date": "2026-07-20", "amount": -5000,
                  "memo": null, "approved": false,
                  "account_id": "a-1", "account_name": "Chequing",
                  "payee_id": null, "payee_name": "Mystery",
                  "category_id": null, "category_name": null,
                  "deleted": false, "subtransactions": [] }
            ], "server_knowledge": 9 }
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        let dir = tempfile::tempdir().unwrap();
        // human: uncategorized filter keeps only t-2, shows currency
        ynab(dir.path(), &uri)
            .args(["transactions", "list", "--uncategorized"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Mystery"))
            .stdout(predicate::str::contains("-5.00"))
            .stdout(predicate::str::contains("Corner Grocer").not());
        // json: filtered raw array preserves unknown fields (subtransactions)
        // and the envelope (server_knowledge) sibling to "transactions"
        ynab(dir.path(), &uri)
            .args(["transactions", "list", "--uncategorized", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"t-2\""))
            .stdout(predicate::str::contains("subtransactions"))
            .stdout(predicate::str::contains("\"t-1\"").not())
            .stdout(predicate::str::contains("server_knowledge"));
        // bad date errors cleanly
        ynab(dir.path(), &uri)
            .args(["transactions", "list", "--since", "yesterday"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("ISO date"));
    })
    .await
    .unwrap();
}
