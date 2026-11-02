//! Self update: release discovery on GitHub, startup checks and binary replacement.

pub mod cache;
pub mod github;
pub mod install;
pub mod platform;
pub mod version;

use std::path::{Path, PathBuf};
use thiserror::Error;

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
