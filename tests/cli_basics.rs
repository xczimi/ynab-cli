use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_commands() {
    Command::cargo_bin("ynab")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("config"));
}

#[test]
fn unknown_command_fails() {
    Command::cargo_bin("ynab")
        .unwrap()
        .arg("frobnicate")
        .assert()
        .failure();
}

#[test]
fn errors_go_to_stderr_with_prefix() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("ynab")
        .unwrap()
        .env("YNAB_CLI_CONFIG_DIR", dir.path())
        .args(["config", "get", "definitely-not-a-key"])
        .assert()
        .failure()
        .stderr(predicate::str::starts_with("error: "));
}
