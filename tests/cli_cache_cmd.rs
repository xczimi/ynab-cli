use assert_cmd::Command;
use predicates::prelude::*;

fn ynab(config_dir: &std::path::Path, data_dir: &std::path::Path, base_url: &str) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", config_dir);
    cmd.env("YNAB_CLI_DATA_DIR", data_dir);
    cmd.env("YNAB_CLI_API_BASE_URL", base_url);
    cmd.env("YNAB_PAT", "e2e-token");
    cmd.env("YNAB_CLI_CACHE_KEY", "ab".repeat(32));
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

#[test]
fn cache_status_empty_tips_default_budget_when_unset() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path(), data.path());

    ynab(cfg, dat, "http://localhost:9999")
        .args(["cache", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Tip: set a default budget (ynab config set default_budget <id>) to enable delta caching.",
        ));
}

#[test]
fn cache_status_empty_omits_tip_when_default_budget_set() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path(), data.path());

    ynab(cfg, dat, "http://localhost:9999")
        .args([
            "config",
            "set",
            "default_budget",
            "11111111-1111-1111-1111-111111111111",
        ])
        .assert()
        .success();

    ynab(cfg, dat, "http://localhost:9999")
        .args(["cache", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tip:").not());
}
