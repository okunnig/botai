use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_cli_version_flag_binary() {
    let mut cmd = Command::cargo_bin("botai").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("botai "))
        .stdout(predicate::str::contains(env!("BOTAI_VERSION")));
}

#[test]
fn test_cli_short_version_flag_binary() {
    let mut cmd = Command::cargo_bin("botai").unwrap();
    cmd.arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("botai "))
        .stdout(predicate::str::contains(env!("BOTAI_VERSION")));
}
