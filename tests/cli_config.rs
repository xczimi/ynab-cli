use assert_cmd::Command;
use predicates::prelude::*;

fn ynab(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", dir);
    cmd
}

#[test]
fn set_then_get_roundtrip() {
    let dir = tempfile::tempdir().unwrap();

    ynab(dir.path())
        .args(["config", "set", "default_budget", "b-42"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default_budget = b-42"));

    ynab(dir.path())
        .args(["config", "get", "default_budget"])
        .assert()
        .success()
        .stdout(predicate::str::contains("b-42"));
}

#[test]
fn get_unset_key_prints_unset() {
    let dir = tempfile::tempdir().unwrap();
    ynab(dir.path())
        .args(["config", "get", "cache"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<unset>"));
}

#[test]
fn unknown_key_errors() {
    let dir = tempfile::tempdir().unwrap();
    ynab(dir.path())
        .args(["config", "set", "nope", "x"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown key"));
}

#[test]
fn config_file_never_contains_secrets_section() {
    let dir = tempfile::tempdir().unwrap();
    ynab(dir.path())
        .args(["config", "set", "cache", "false"])
        .assert()
        .success();
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(!text.to_lowercase().contains("token"));
    assert!(!text.to_lowercase().contains("secret"));
}
