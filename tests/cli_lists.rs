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
