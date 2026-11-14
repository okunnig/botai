//! Persisted result of the last update check (`update-check.toml` in the user cache dir).

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::parent_or_cwd;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a cached check result stays fresh.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Contents of the cache file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Unix seconds of the last check attempt, successful or not.
    pub last_checked: u64,
    /// Latest release tag known; kept across failed checks, `None` until one succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_tag: Option<String>,
}

/// Default cache file location, e.g. `~/Library/Caches/botai/update-check.toml`.
pub fn default_path() -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.cache_dir().join("botai").join("update-check.toml"))
}

/// Current time in unix seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads the cache, treating a missing or corrupt file as absent.
pub fn read(path: &Path) -> Option<CacheEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

/// Writes the cache via a temp file and rename, since two botai instances may write at once.
pub fn write(path: &Path, entry: &CacheEntry) -> std::io::Result<()> {
    let parent = parent_or_cwd(path);
    std::fs::create_dir_all(parent)?;
    let content = toml::to_string(entry).map_err(std::io::Error::other)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(content.as_bytes())?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Whether a new network check is due at `now` (unix seconds).
pub(crate) fn is_stale(entry: Option<&CacheEntry>, now: u64) -> bool {
    match entry {
        None => true,
        // A clock that moved backwards also forces a fresh check.
        Some(e) => e.last_checked > now || now - e.last_checked >= CHECK_INTERVAL.as_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(last_checked: u64) -> CacheEntry {
        CacheEntry {
            last_checked,
            latest_tag: Some("v0.0.2".into()),
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("update-check.toml");
        write(&path, &entry(42)).unwrap();
        assert_eq!(read(&path), Some(entry(42)));

        write(&path, &entry(43)).unwrap();
        assert_eq!(read(&path), Some(entry(43)));

        let unknown = CacheEntry {
            last_checked: 44,
            latest_tag: None,
        };
        write(&path, &unknown).unwrap();
        assert_eq!(read(&path), Some(unknown));
        write(&path, &entry(43)).unwrap();
        assert_eq!(read(&path), Some(entry(43)));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn reads_cache_written_by_earlier_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        std::fs::write(&path, "last_checked = 42\nlatest_tag = \"v0.0.2\"\n").unwrap();
        assert_eq!(read(&path), Some(entry(42)));
    }

    #[test]
    fn missing_or_corrupt_cache_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        assert_eq!(read(&path), None);
        std::fs::write(&path, "this is = = not toml").unwrap();
        assert_eq!(read(&path), None);
        std::fs::write(&path, "last_checked = \"soon\"").unwrap();
        assert_eq!(read(&path), None);
    }

    #[test]
    fn staleness() {
        let day = CHECK_INTERVAL.as_secs();
        assert!(is_stale(None, 1_000));
        assert!(!is_stale(Some(&entry(1_000)), 1_000));
        assert!(!is_stale(Some(&entry(1_000)), 1_000 + day - 1));
        assert!(is_stale(Some(&entry(1_000)), 1_000 + day));
        assert!(is_stale(Some(&entry(5_000)), 1_000));
    }

    #[test]
    fn default_path_is_in_botai_cache_dir() {
        if let Some(path) = default_path() {
            assert_eq!(path.file_name().unwrap(), "update-check.toml");
            assert!(path.to_string_lossy().to_lowercase().contains("botai"));
        }
    }
}
