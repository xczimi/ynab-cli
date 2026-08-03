use assert_cmd::Command;
use predicates::prelude::*;

fn ynab(config_dir: &std::path::Path, data_dir: &std::path::Path, base_url: &str) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", config_dir);
    cmd.env("YNAB_CLI_DATA_DIR", data_dir);
    cmd.env("YNAB_CLI_API_BASE_URL", base_url);
    cmd.env("YNAB_PAT", "e2e-token");
    cmd
}

#[test]
fn cache_status_empty_and_clear_missing() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path(), data.path());

    // empty status shows cache is empty
    ynab(cfg, dat, "http://localhost:9999")
        .args(["cache", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));

    // clear on missing cache file is fine
    ynab(cfg, dat, "http://localhost:9999")
        .args(["cache", "clear"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cache cleared."));

    // clear again is still fine
    ynab(cfg, dat, "http://localhost:9999")
        .args(["cache", "clear"])
        .assert()
        .success();
}
