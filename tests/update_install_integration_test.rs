mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use botai::update::platform::{self, ArchiveKind};
use botai::update::{self, Client, InstallOutcome, ReleaseQuery, UpdateError};
use support::{serve, FixtureRelease, FixtureServer, Routes, REPO_PATH};

fn client(server: &FixtureServer) -> Client {
    Client::new(server.base.clone(), Duration::from_secs(5))
}

/// A stand-in for the installed executable; never the test runner.
fn dummy_target() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(platform::binary_name());
    fs::write(&target, b"old-binary").unwrap();
    (dir, target)
}

fn dir_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn unsafe_archive() -> Vec<u8> {
    let entries: &[(&str, &[u8])] = &[("../botai", b"evil"), ("./botai", b"new-binary")];
    match platform::current_asset("v0.0.0").unwrap().kind {
        ArchiveKind::TarGz => support::tar_gz(entries),
        ArchiveKind::Zip => support::zip(&[("../botai.exe", b"evil"), ("botai.exe", b"new")]),
    }
}

#[test]
fn check_latest_reports_latest_tag_with_github_headers() {
    let server = serve(FixtureRelease::new("v0.2.0", b"new-binary"), vec![]);
    let client = client(&server).with_token(Some("secret".into()));

    let status = update::check_latest(&client, &ReleaseQuery::Latest, "v0.1.0").unwrap();
    assert_eq!(status.latest, "v0.2.0");
    assert!(status.update_available());

    let requests = server.requests();
    let head = requests[0].to_ascii_lowercase();
    assert!(
        head.starts_with("get /repos/okunnig/botai/releases/latest "),
        "{head}"
    );
    assert!(head.contains("user-agent: botai/"), "{head}");
    assert!(
        head.contains("accept: application/vnd.github+json"),
        "{head}"
    );
    assert!(head.contains("authorization: bearer secret"), "{head}");
}

#[test]
fn check_latest_not_newer_when_current_matches() {
    let server = serve(FixtureRelease::new("v0.2.0", b"x"), vec![]);
    let status = update::check_latest(&client(&server), &ReleaseQuery::Latest, "v0.2.0").unwrap();
    assert!(!status.update_available());
}

#[test]
fn prerelease_query_picks_highest_version_including_prereleases() {
    let mut rc = FixtureRelease::new("v0.3.0-rc.1", b"rc");
    rc.prerelease = true;
    let server = serve(
        FixtureRelease::new("v0.2.0", b"stable"),
        vec![rc, FixtureRelease::new("v0.1.0", b"old")],
    );
    let status = update::check_latest(
        &client(&server),
        &ReleaseQuery::LatestIncludingPrerelease,
        "v0.2.0",
    )
    .unwrap();
    assert_eq!(status.latest, "v0.3.0-rc.1");
}

#[test]
fn unknown_tag_is_reported() {
    let server = serve(FixtureRelease::new("v0.2.0", b"x"), vec![]);
    let err = update::check_latest(
        &client(&server),
        &ReleaseQuery::Tag("9.9.9".into()),
        "v0.1.0",
    )
    .unwrap_err();
    assert!(
        matches!(err, UpdateError::ReleaseNotFound(ref t) if t == "v9.9.9"),
        "{err}"
    );
}

#[test]
fn rate_limit_is_reported_with_token_hint() {
    let server = FixtureServer::start(|_| {
        let mut routes = Routes::new();
        routes.insert(
            format!("{REPO_PATH}/releases/latest"),
            (403, b"{}".to_vec()),
        );
        routes
    });
    let err = update::check_latest(&client(&server), &ReleaseQuery::Latest, "v0.1.0").unwrap_err();
    assert!(matches!(err, UpdateError::RateLimited));
    assert!(err.to_string().contains("GITHUB_TOKEN"));
}

#[test]
fn install_replaces_target_on_happy_path() {
    let server = serve(FixtureRelease::new("v0.2.0", b"new-binary"), vec![]);
    let (dir, target) = dummy_target();

    let outcome =
        update::install(&client(&server), &ReleaseQuery::Latest, "v0.1.0", &target).unwrap();

    assert_eq!(
        outcome,
        InstallOutcome::Installed {
            from: "v0.1.0".into(),
            to: "v0.2.0".into()
        }
    );
    assert_eq!(fs::read(&target).unwrap(), b"new-binary");
    assert_eq!(
        dir_entries(dir.path()),
        vec![platform::binary_name().to_string()]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}

#[test]
fn install_skips_when_already_up_to_date() {
    let server = serve(FixtureRelease::new("v0.2.0", b"new-binary"), vec![]);
    let (_dir, target) = dummy_target();

    let outcome =
        update::install(&client(&server), &ReleaseQuery::Latest, "v0.2.0", &target).unwrap();

    assert_eq!(
        outcome,
        InstallOutcome::AlreadyUpToDate {
            version: "v0.2.0".into()
        }
    );
    assert_eq!(fs::read(&target).unwrap(), b"old-binary");
}

#[test]
fn install_specific_tag_allows_downgrade() {
    let server = serve(
        FixtureRelease::new("v0.2.0", b"new-binary"),
        vec![FixtureRelease::new("v0.1.0", b"older-binary")],
    );
    let (_dir, target) = dummy_target();

    let outcome = update::install(
        &client(&server),
        &ReleaseQuery::Tag("v0.1.0".into()),
        "v0.2.0",
        &target,
    )
    .unwrap();

    assert_eq!(
        outcome,
        InstallOutcome::Installed {
            from: "v0.2.0".into(),
            to: "v0.1.0".into()
        }
    );
    assert_eq!(fs::read(&target).unwrap(), b"older-binary");
}

fn assert_install_fails(release: FixtureRelease, check: impl Fn(&UpdateError) -> bool) {
    let server = serve(release, vec![]);
    let (dir, target) = dummy_target();

    let err =
        update::install(&client(&server), &ReleaseQuery::Latest, "v0.1.0", &target).unwrap_err();

    assert!(check(&err), "unexpected error: {err:?}");
    assert_eq!(fs::read(&target).unwrap(), b"old-binary");
    assert_eq!(
        dir_entries(dir.path()),
        vec![platform::binary_name().to_string()],
        "staging directory must be cleaned up"
    );
}

#[test]
fn install_rejects_checksum_mismatch() {
    let mut release = FixtureRelease::new("v0.2.0", b"new-binary");
    release.sums = Some(format!(
        "{}  {}\n",
        "0".repeat(64),
        support::asset_name("v0.2.0")
    ));
    assert_install_fails(release, |e| {
        matches!(e, UpdateError::ChecksumMismatch { .. })
    });
}

#[test]
fn install_rejects_missing_checksum_line() {
    let mut release = FixtureRelease::new("v0.2.0", b"new-binary");
    release.sums = Some(format!("{}  botai-v0.2.0-other.tar.gz\n", "0".repeat(64)));
    assert_install_fails(release, |e| matches!(e, UpdateError::ChecksumMissing(_)));
}

#[test]
fn install_rejects_missing_platform_asset() {
    let mut release = FixtureRelease::new("v0.2.0", b"new-binary");
    release.archive = None;
    assert_install_fails(release, |e| matches!(e, UpdateError::AssetMissing { .. }));
}

#[test]
fn install_rejects_traversal_entry() {
    let mut release = FixtureRelease::new("v0.2.0", b"new-binary");
    release.archive = Some(unsafe_archive());
    assert_install_fails(release, |e| matches!(e, UpdateError::UnsafeArchiveEntry(_)));
}

#[test]
fn install_rejects_archive_without_binary() {
    let mut release = FixtureRelease::new("v0.2.0", b"new-binary");
    release.archive = Some(support::platform_archive(&[("README.md", b"readme")]));
    assert_install_fails(release, |e| matches!(e, UpdateError::BinaryNotInArchive(_)));
}

mod startup_check {
    use super::*;
    use botai::update::cache::{self, CacheEntry, CHECK_INTERVAL};
    use botai::update::spawn_check_with;

    const NOW: u64 = 1_000_000;

    fn recv(rx: std::sync::mpsc::Receiver<update::UpdateStatus>) -> Option<update::UpdateStatus> {
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }

    #[test]
    fn stale_cache_queries_github_and_rewrites_cache() {
        let server = serve(FixtureRelease::new("v0.3.0", b"x"), vec![]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        cache::write(
            &path,
            &CacheEntry {
                last_checked: NOW - CHECK_INTERVAL.as_secs(),
                latest_tag: "v0.1.0".into(),
            },
        )
        .unwrap();

        let status = recv(spawn_check_with(
            "v0.2.0".into(),
            path.clone(),
            client(&server),
            NOW,
        ));

        assert_eq!(status.unwrap().latest, "v0.3.0");
        assert_eq!(
            cache::read(&path),
            Some(CacheEntry {
                last_checked: NOW,
                latest_tag: "v0.3.0".into()
            })
        );
    }

    #[test]
    fn fresh_cache_answers_without_network() {
        let server = FixtureServer::start(|_| Routes::new());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        cache::write(
            &path,
            &CacheEntry {
                last_checked: NOW - 60,
                latest_tag: "v0.3.0".into(),
            },
        )
        .unwrap();

        let status = recv(spawn_check_with(
            "v0.2.0".into(),
            path,
            client(&server),
            NOW,
        ));

        assert_eq!(status.unwrap().latest, "v0.3.0");
        assert!(server.requests().is_empty());
    }

    #[test]
    fn no_message_when_up_to_date() {
        let server = serve(FixtureRelease::new("v0.2.0", b"x"), vec![]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");

        let status = recv(spawn_check_with(
            "v0.2.0".into(),
            path.clone(),
            client(&server),
            NOW,
        ));

        assert_eq!(status, None);
        assert_eq!(cache::read(&path).unwrap().latest_tag, "v0.2.0");
    }

    #[test]
    fn network_failure_falls_back_to_stale_cache() {
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        let stale = CacheEntry {
            last_checked: 0,
            latest_tag: "v0.3.0".into(),
        };
        cache::write(&path, &stale).unwrap();

        let client = Client::new(base, Duration::from_secs(2));
        let status = recv(spawn_check_with("v0.2.0".into(), path.clone(), client, NOW));

        assert_eq!(status.unwrap().latest, "v0.3.0");
        assert_eq!(cache::read(&path), Some(stale));
    }
}
