//! Self update: release discovery on GitHub, startup checks and binary replacement.

pub mod cache;
pub mod github;
pub mod install;
pub mod platform;
pub mod version;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use thiserror::Error;

use crate::config::UpdateConfig;

pub use github::{Client, Release, ReleaseQuery};

/// Errors raised while checking for or installing an update.
#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("No prebuilt botai release for {os}/{arch}. Install from source instead: cargo install --git https://github.com/okunnig/botai")]
    UnsupportedPlatform { os: String, arch: String },

    #[error("Request to {url} failed: {message}")]
    Http { url: String, message: String },

    #[error(
        "GitHub API rate limit reached or access denied. Try again later or set GITHUB_TOKEN."
    )]
    RateLimited,

    #[error("Unexpected response from GitHub: {0}")]
    InvalidResponse(String),

    #[error("No published botai release found")]
    NoRelease,

    #[error("Release {0} not found")]
    ReleaseNotFound(String),

    #[error("Release {tag} has no asset named {asset}")]
    AssetMissing { tag: String, asset: String },

    #[error("SHA256SUMS has no entry for {0}; refusing to install an unverified download")]
    ChecksumMissing(String),

    #[error("Checksum mismatch for {asset}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        asset: String,
        expected: String,
        actual: String,
    },

    #[error("Invalid release archive: {0}")]
    InvalidArchive(String),

    #[error("Release archive contains an unsafe path: {0}")]
    UnsafeArchiveEntry(String),

    #[error("Release archive does not contain {0}")]
    BinaryNotInArchive(String),

    #[error("Permission denied writing {}. Re-run with sudo, or reinstall botai to a user-writable directory.", path.display())]
    PermissionDenied { path: PathBuf },

    #[error("I/O error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Result of comparing the running build against a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    pub current: String,
    pub latest: String,
}

impl UpdateStatus {
    /// Whether `latest` is newer than `current`.
    pub fn update_available(&self) -> bool {
        version::is_newer(&self.current, &self.latest)
    }
}

/// Resolves `query` and compares it with `current`.
pub fn check_latest(
    client: &Client,
    query: &ReleaseQuery,
    current: &str,
) -> Result<UpdateStatus, UpdateError> {
    let release = client.fetch_release(query)?;
    Ok(UpdateStatus {
        current: current.to_string(),
        latest: release.tag_name,
    })
}

/// What `install` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    AlreadyUpToDate { version: String },
    Installed { from: String, to: String },
}

/// Installs the release selected by `query` over `target`.
///
/// Without an explicit tag nothing happens unless the release is newer than `current`; an
/// explicit tag installs unless it is exactly `current` (downgrades allowed).
pub fn install(
    client: &Client,
    query: &ReleaseQuery,
    current: &str,
    target: &Path,
) -> Result<InstallOutcome, UpdateError> {
    let release = client.fetch_release(query)?;
    let skip = match query {
        ReleaseQuery::Tag(_) => release.tag_name == current,
        _ => !version::is_newer(current, &release.tag_name),
    };
    if skip {
        return Ok(InstallOutcome::AlreadyUpToDate {
            version: current.to_string(),
        });
    }
    install::install_release(client, &release, target)?;
    Ok(InstallOutcome::Installed {
        from: current.to_string(),
        to: release.tag_name,
    })
}

/// Disables the startup check when set to anything but empty or `0`.
pub const NO_CHECK_ENV: &str = "BOTAI_NO_UPDATE_CHECK";
/// Timeout for the background startup check.
pub const STARTUP_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether update notices and the startup check apply to this process.
///
/// `env` is injected so tests do not depend on the real environment (CI sets `CI`).
pub fn startup_check_enabled(
    current: &str,
    config: &UpdateConfig,
    env: impl Fn(&str) -> Option<String>,
) -> bool {
    let opted_out = env(NO_CHECK_ENV).is_some_and(|v| !v.trim().is_empty() && v.trim() != "0");
    config.check_on_startup
        && !opted_out
        && env("CI").is_none()
        && version::is_release_build(current)
}

/// The newer release recorded in the cache, if any.
pub fn cached_update(current: &str, entry: Option<&cache::CacheEntry>) -> Option<UpdateStatus> {
    let entry = entry?;
    let status = UpdateStatus {
        current: current.to_string(),
        latest: entry.latest_tag.clone(),
    };
    status.update_available().then_some(status)
}

/// Line printed to stderr after short CLI commands.
pub fn cli_notice_text(status: &UpdateStatus) -> String {
    format!(
        "botai {} is available (current {}). Run `botai update` to install it.",
        status.latest, status.current
    )
}

/// Text shown in the TUI bars.
pub fn notice_text(latest: &str) -> String {
    format!("update {latest} available: botai update")
}

/// Starts the TUI startup check. The receiver yields once, and only when an update exists.
///
/// A fresh cache answers immediately; otherwise a background thread queries GitHub and
/// rewrites the cache. Errors are swallowed: the check must never affect the launch.
pub fn spawn_startup_check(config: &UpdateConfig) -> Option<Receiver<UpdateStatus>> {
    let current = version::current();
    if !startup_check_enabled(current, config, |k| std::env::var(k).ok()) {
        return None;
    }
    let cache_path = cache::default_path()?;
    Some(spawn_check_with(
        current.to_string(),
        cache_path,
        Client::from_env(STARTUP_CHECK_TIMEOUT),
        cache::now_secs(),
    ))
}

/// Testable core of [`spawn_startup_check`].
pub fn spawn_check_with(
    current: String,
    cache_path: PathBuf,
    client: Client,
    now: u64,
) -> Receiver<UpdateStatus> {
    let (tx, rx) = mpsc::channel();
    let entry = cache::read(&cache_path);
    if !cache::is_stale(entry.as_ref(), now) {
        if let Some(status) = cached_update(&current, entry.as_ref()) {
            let _ = tx.send(status);
        }
        return rx;
    }
    std::thread::spawn(move || {
        let latest = match check_latest(&client, &ReleaseQuery::Latest, &current) {
            Ok(status) => {
                let _ = cache::write(
                    &cache_path,
                    &cache::CacheEntry {
                        last_checked: now,
                        latest_tag: status.latest.clone(),
                    },
                );
                Some(status)
            }
            Err(_) => cached_update(&current, entry.as_ref()),
        };
        if let Some(status) = latest.filter(UpdateStatus::update_available) {
            let _ = tx.send(status);
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn startup_check_policy() {
        let on = UpdateConfig::default();
        let off = UpdateConfig {
            check_on_startup: false,
        };
        assert!(startup_check_enabled("v0.0.1", &on, env_from(&[])));
        assert!(startup_check_enabled("v0.2.0-rc.1", &on, env_from(&[])));
        assert!(!startup_check_enabled("v0.0.1", &off, env_from(&[])));
        assert!(!startup_check_enabled(
            "v0.0.1",
            &on,
            env_from(&[("CI", "true")])
        ));
        assert!(!startup_check_enabled(
            "v0.0.1",
            &on,
            env_from(&[(NO_CHECK_ENV, "1")])
        ));
        assert!(startup_check_enabled(
            "v0.0.1",
            &on,
            env_from(&[(NO_CHECK_ENV, "0")])
        ));
        assert!(startup_check_enabled(
            "v0.0.1",
            &on,
            env_from(&[(NO_CHECK_ENV, "")])
        ));
        for dev in [
            "v0.0.0",
            "v0.0.0-dev-abc1234",
            "v0.0.1-dirty",
            "v0.0.1-3-gabc1234",
        ] {
            assert!(!startup_check_enabled(dev, &on, env_from(&[])), "{dev}");
        }
    }

    #[test]
    fn cached_update_only_when_newer() {
        let entry = |tag: &str| cache::CacheEntry {
            last_checked: 0,
            latest_tag: tag.into(),
        };
        assert_eq!(cached_update("v0.0.1", None), None);
        assert_eq!(cached_update("v0.0.2", Some(&entry("v0.0.2"))), None);
        assert_eq!(
            cached_update("v0.0.1", Some(&entry("v0.0.2"))),
            Some(UpdateStatus {
                current: "v0.0.1".into(),
                latest: "v0.0.2".into()
            })
        );
    }

    #[test]
    fn cli_notice_mentions_both_versions_and_command() {
        let text = cli_notice_text(&UpdateStatus {
            current: "v0.1.0".into(),
            latest: "v0.2.0".into(),
        });
        assert!(text.contains("v0.1.0") && text.contains("v0.2.0"), "{text}");
        assert!(text.contains("botai update"), "{text}");
    }

    #[test]
    fn notice_text_mentions_command() {
        assert_eq!(
            notice_text("v0.2.0"),
            "update v0.2.0 available: botai update"
        );
    }
}
