mod support;

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use botai::update::{platform, version};
use predicates::prelude::*;
use support::{serve, FixtureRelease, FixtureServer};

fn botai(server: &FixtureServer) -> Command {
    let mut cmd = Command::cargo_bin("botai").unwrap();
    cmd.env("BOTAI_UPDATE_BASE_URL", &server.base)
        .env_remove("GITHUB_TOKEN");
    cmd
}

/// A private copy of the built binary, so self-replacement never touches target/debug/botai.
fn copied_botai() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join(platform::binary_name());
    fs::copy(env!("CARGO_BIN_EXE_botai"), &exe).unwrap();
    (dir, exe)
}

fn copied_command(server: &FixtureServer, exe: &std::path::Path) -> Command {
    let mut cmd = Command::new(exe);
    cmd.env("BOTAI_UPDATE_BASE_URL", &server.base)
        .env_remove("GITHUB_TOKEN");
    cmd
}

#[test]
fn update_check_prints_current_and_latest() {
    let server = serve(FixtureRelease::new("v99.0.0", b"new"), vec![]);
    botai(&server)
        .args(["update", "--check"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Current version: {}",
            version::current()
        )))
        .stdout(predicate::str::contains("Latest version:  v99.0.0"))
        .stdout(predicate::str::contains("Update available"));
}

#[test]
fn update_check_with_pre_includes_prereleases() {
    let rc = FixtureRelease::new("v99.1.0-rc.1", b"rc");
    let server = serve(FixtureRelease::new("v99.0.0", b"new"), vec![rc]);
    botai(&server)
        .args(["update", "--check", "--pre"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Latest version:  v99.1.0-rc.1"));
}

#[test]
fn update_check_exits_zero_when_up_to_date() {
    let server = serve(FixtureRelease::new("v0.0.0", b"old"), vec![]);
    botai(&server)
        .args(["update", "--check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("botai is up to date."));
}

#[test]
fn update_check_conflicts_with_version() {
    let server = serve(FixtureRelease::new("v99.0.0", b"new"), vec![]);
    botai(&server)
        .args(["update", "--check", "--version", "v99.0.0"])
        .assert()
        .failure();
}

#[test]
fn update_reports_unknown_tag() {
    let server = serve(FixtureRelease::new("v99.0.0", b"new"), vec![]);
    let (_dir, exe) = copied_botai();
    copied_command(&server, &exe)
        .args(["update", "--version", "v42.0.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Release v42.0.0 not found"));
}

#[test]
fn update_works_with_a_broken_config() {
    let server = serve(FixtureRelease::new("v0.0.0", b"old"), vec![]);
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    fs::write(&config, "this is = = not toml").unwrap();
    botai(&server)
        .arg("--config")
        .arg(&config)
        .args(["update", "--check"])
        .assert()
        .success();
}

#[test]
fn update_version_replaces_the_running_copy() {
    let server = serve(
        FixtureRelease::new("v99.0.0", b"latest-binary"),
        vec![FixtureRelease::new("v98.0.0", b"pinned-binary")],
    );
    let (_dir, exe) = copied_botai();

    let assert = copied_command(&server, &exe)
        .args(["update", "--version", "v98.0.0"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Updated botai {} -> v98.0.0",
            version::current()
        )));

    let dev_warning = predicate::str::contains("is a source build");
    if version::is_release_build(version::current()) {
        assert.stderr(dev_warning.not());
    } else {
        assert.stderr(dev_warning);
    }
    assert_eq!(fs::read(&exe).unwrap(), b"pinned-binary");
}

#[test]
fn update_without_newer_release_leaves_binary_alone() {
    let server = serve(FixtureRelease::new("v0.0.0", b"old"), vec![]);
    let (_dir, exe) = copied_botai();
    let before = fs::read(&exe).unwrap();

    copied_command(&server, &exe)
        .arg("update")
        .assert()
        .success()
        .stdout(predicate::str::contains("already up to date"))
        .stderr(predicate::str::contains("is a source build").not());

    assert_eq!(fs::read(&exe).unwrap(), before);
}

#[test]
fn update_rejects_invalid_tag_without_a_request() {
    let server = serve(FixtureRelease::new("v99.0.0", b"new"), vec![]);
    let (_dir, exe) = copied_botai();
    copied_command(&server, &exe)
        .args(["update", "--version", "../latest"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid release tag"));
    assert!(server.requests().is_empty());
}
