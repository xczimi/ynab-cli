use std::io::Read;
use std::process::{Command, Stdio};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Piping into `head` closes stdout early. The CLI must exit quietly
/// instead of panicking with "failed printing to stdout: Broken pipe".
#[tokio::test(flavor = "multi_thread")]
async fn closed_stdout_pipe_exits_quietly() {
    let server = MockServer::start().await;
    // Big enough to outrun the 64KiB pipe buffer, so the child is still
    // writing when we close the read end.
    let budgets: Vec<serde_json::Value> = (0..4000)
        .map(|i| {
            serde_json::json!({
                "id": format!("b-{i}"),
                "name": format!("Budget number {i} with a padded name"),
                "first_month": "2025-01-01",
                "last_month": "2026-08-01"
            })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/budgets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "budgets": budgets, "default_budget": null }
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(assert_cmd::cargo::cargo_bin("ynab"))
            .args(["budgets", "list", "--json"])
            .env("YNAB_CLI_CONFIG_DIR", dir.path())
            .env("YNAB_CLI_API_BASE_URL", &uri)
            .env("YNAB_PAT", "e2e-token")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Read a little, then hang up — this is what `| head` does.
        let mut out = child.stdout.take().unwrap();
        let mut buf = [0u8; 256];
        out.read_exact(&mut buf).unwrap();
        drop(out);

        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("panicked"),
            "panicked on broken pipe; stderr was:\n{stderr}"
        );
        assert!(
            output.status.success(),
            "expected clean exit, got {:?}; stderr:\n{stderr}",
            output.status
        );
    })
    .await
    .unwrap();
}
