//! Self update: release discovery on GitHub, startup checks and binary replacement.

pub mod cache;
pub mod platform;
pub mod version;

use thiserror::Error;

/// Errors raised while checking for or installing an update.
#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("No prebuilt botai release for {os}/{arch}. Install from source instead: cargo install --git https://github.com/okunnig/botai")]
    UnsupportedPlatform { os: String, arch: String },
}
