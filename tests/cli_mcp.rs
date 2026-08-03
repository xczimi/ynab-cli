use std::time::Duration;

use assert_cmd::Command;

/// `ynab mcp serve` starts and exits cleanly (0) when stdin closes without a
/// client ever completing the MCP handshake — the shape a bare health check
/// or `echo -n | ynab mcp serve` produces. Verified against rmcp 3.1.0:
/// `.serve(stdio())` returns `Err(ServerInitializeError::ConnectionClosed)`
/// in this case (it does not hang, and does not return `Ok` on its own);
/// `src/mcp/mod.rs::serve` maps that specific variant to a clean exit.
#[test]
fn mcp_serve_exits_cleanly_on_stdin_close() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", dir.path());
    cmd.args(["mcp", "serve"]);
    cmd.write_stdin(""); // pipe stdin, then close it immediately (no bytes written)
    cmd.timeout(Duration::from_secs(10));
    cmd.assert().success();
}
