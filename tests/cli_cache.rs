use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ynab(config: &std::path::Path, data: &std::path::Path, base: &str) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", config);
    cmd.env("YNAB_CLI_DATA_DIR", data);
    cmd.env("YNAB_CLI_API_BASE_URL", base);
    cmd.env("YNAB_PAT", "e2e-token");
    cmd.env("YNAB_CLI_CACHE_KEY", "ab".repeat(32));
    cmd
}

fn tx_body(rows: serde_json::Value, sk: i64) -> serde_json::Value {
    serde_json::json!({ "data": { "transactions": rows, "server_knowledge": sk } })
}

fn tx(id: &str, date: &str, payee: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "date": date, "amount": -1000, "memo": null,
        "approved": true, "account_id": "a-1", "account_name": "Chq",
        "payee_id": null, "payee_name": payee, "category_id": null,
        "category_name": null, "deleted": false })
}

#[tokio::test(flavor = "multi_thread")]
async fn transactions_delta_cache_roundtrip() {
    let server = MockServer::start().await;
    // first call: full fetch (no last_knowledge_of_server)
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tx_body(
            serde_json::json!([tx("t-1", "2026-07-01", "Grocer")]),
            10,
        )))
        .expect(1)
        .mount(&server)
        .await;

    let uri = server.uri();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path().to_path_buf(), data.path().to_path_buf());
    let u = uri.clone();
    tokio::task::spawn_blocking(move || {
        ynab(&cfg, &dat, &u)
            .args(["transactions", "list", "--budget", "b-1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Grocer"));
    })
    .await
    .unwrap();
    server.reset().await;

    // second call: MUST send last_knowledge_of_server=10; delta adds t-2;
    // output contains BOTH rows (t-1 from cache, t-2 from delta)
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .and(query_param("last_knowledge_of_server", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tx_body(
            serde_json::json!([tx("t-2", "2026-07-20", "Landlord")]),
            11,
        )))
        .expect(1)
        .mount(&server)
        .await;
    // third invocation (--since) syncs again with knowledge 11 → empty delta
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .and(query_param("last_knowledge_of_server", "11"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tx_body(serde_json::json!([]), 11)))
        .expect(1)
        .mount(&server)
        .await;
    let (cfg, dat) = (config.path().to_path_buf(), data.path().to_path_buf());
    let u = uri.clone();
    tokio::task::spawn_blocking(move || {
        ynab(&cfg, &dat, &u)
            .args(["transactions", "list", "--budget", "b-1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Grocer"))
            .stdout(predicate::str::contains("Landlord"));
        // --since filters locally over the cached set (no new since_date request)
        ynab(&cfg, &dat, &u)
            .args([
                "transactions",
                "list",
                "--budget",
                "b-1",
                "--since",
                "2026-07-10",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Landlord"))
            .stdout(predicate::str::contains("Grocer").not());
    })
    .await
    .unwrap();

    // exactly 3 requests total, none with since_date
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| !r.url.query().unwrap_or("").contains("since_date"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_cache_flag_bypasses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tx_body(
            serde_json::json!([tx("t-1", "2026-07-01", "Grocer")]),
            10,
        )))
        .expect(2)
        .mount(&server)
        .await;
    let uri = server.uri();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path().to_path_buf(), data.path().to_path_buf());
    tokio::task::spawn_blocking(move || {
        for _ in 0..2 {
            ynab(&cfg, &dat, &uri)
                .args(["transactions", "list", "--budget", "b-1", "--no-cache"])
                .assert()
                .success();
        }
    })
    .await
    .unwrap();
    // no cache DB was created
    assert!(!data.path().join("cache.db").exists());
    // and neither request carried last_knowledge_of_server
    let requests = server.received_requests().await.unwrap();
    assert!(requests.iter().all(|r| {
        !r.url
            .query()
            .unwrap_or("")
            .contains("last_knowledge_of_server")
    }));
}
