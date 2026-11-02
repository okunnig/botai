//! Download verification, archive extraction and executable replacement.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};

use super::github::{Client, Release};
use super::platform::{self, ArchiveKind};
use super::UpdateError;

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Looks up the checksum for `asset_name` in `sha256sum` output (`<hex>  <name>` or `<hex> *<name>`).
pub fn expected_checksum(sums: &str, asset_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        let name = name.strip_prefix('*').unwrap_or(name);
        (name == asset_name).then(|| hash.to_ascii_lowercase())
    })
}

/// Fails unless `bytes` hash to the value listed for `asset_name` in `sums`.
pub fn verify_checksum(bytes: &[u8], sums: &str, asset_name: &str) -> Result<(), UpdateError> {
    let expected = expected_checksum(sums, asset_name)
        .ok_or_else(|| UpdateError::ChecksumMissing(asset_name.to_string()))?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(UpdateError::ChecksumMismatch {
            asset: asset_name.to_string(),
            expected,
            actual,
        });
    }
    Ok(())
}

/// Whether an archive entry is `bin_name` at the archive root; rejects traversal and absolute paths.
fn is_binary_entry(raw: &str, bin_name: &str) -> Result<bool, UpdateError> {
    let unsafe_entry = || UpdateError::UnsafeArchiveEntry(raw.to_string());
    // Backslashes are separators on Windows only, but must not smuggle `..` past a unix check.
    if raw.split(['/', '\\']).any(|part| part == "..") {
        return Err(unsafe_entry());
    }
    let mut normal = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normal.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_entry())
            }
        }
    }
    Ok(normal.len() == 1 && normal[0] == bin_name)
}

/// Extracts only `bin_name` from a release archive into `dest`.
pub fn extract_binary(
    archive: &[u8],
    kind: ArchiveKind,
    bin_name: &str,
    dest: &Path,
) -> Result<(), UpdateError> {
    let found = match kind {
        ArchiveKind::TarGz => extract_from_tar_gz(archive, bin_name, dest)?,
        ArchiveKind::Zip => extract_from_zip(archive, bin_name, dest)?,
    };
    if !found {
        return Err(UpdateError::BinaryNotInArchive(bin_name.to_string()));
    }
    Ok(())
}

fn archive_err(e: impl std::fmt::Display) -> UpdateError {
    UpdateError::InvalidArchive(e.to_string())
}

fn extract_from_tar_gz(archive: &[u8], bin_name: &str, dest: &Path) -> Result<bool, UpdateError> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut found = false;
    for entry in tar.entries().map_err(archive_err)? {
        let mut entry = entry.map_err(archive_err)?;
        let raw = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        if is_binary_entry(&raw, bin_name)? && entry.header().entry_type().is_file() && !found {
            write_file(&mut entry, dest)?;
            found = true;
        }
    }
    Ok(found)
}

fn extract_from_zip(archive: &[u8], bin_name: &str, dest: &Path) -> Result<bool, UpdateError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).map_err(archive_err)?;
    let mut found = false;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(archive_err)?;
        let raw = file.name().map_err(archive_err)?.into_owned();
        if file.enclosed_name().is_none() {
            return Err(UpdateError::UnsafeArchiveEntry(raw));
        }
        if is_binary_entry(&raw, bin_name)? && file.is_file() && !found {
            write_file(&mut file, dest)?;
            found = true;
        }
    }
    Ok(found)
}

fn write_file(reader: &mut impl Read, dest: &Path) -> Result<(), UpdateError> {
    let mut out = fs::File::create(dest).map_err(|e| io_err(e, dest))?;
    io::copy(reader, &mut out).map_err(|e| io_err(e, dest))?;
    out.sync_all().map_err(|e| io_err(e, dest))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
            .map_err(|e| io_err(e, dest))?;
    }
    Ok(())
}

fn io_err(e: io::Error, path: &Path) -> UpdateError {
    if e.kind() == io::ErrorKind::PermissionDenied {
        UpdateError::PermissionDenied {
            path: path.to_path_buf(),
        }
    } else {
        UpdateError::Io {
            path: path.to_path_buf(),
            source: e,
        }
    }
}

/// The executable that is currently running, with symlinks resolved.
pub fn current_exe() -> Result<PathBuf, UpdateError> {
    let exe = std::env::current_exe().map_err(|e| io_err(e, Path::new("<current exe>")))?;
    Ok(dunce::canonicalize(&exe).unwrap_or(exe))
}

/// Downloads `release`'s asset for this platform, verifies it and replaces `target` with it.
///
/// When `target` is the running executable the swap goes through `self_replace`, which renames
/// instead of overwriting: Windows locks a running exe, and macOS kills a signed binary that is
/// overwritten in place.
pub fn install_release(
    client: &Client,
    release: &Release,
    target: &Path,
) -> Result<(), UpdateError> {
    let asset = platform::current_asset(&release.tag_name)?;
    let archive_url = release
        .asset(&asset.name)
        .ok_or_else(|| UpdateError::AssetMissing {
            tag: release.tag_name.clone(),
            asset: asset.name.clone(),
        })?
        .browser_download_url
        .clone();
    let sums_url = release
        .asset("SHA256SUMS")
        .ok_or_else(|| UpdateError::AssetMissing {
            tag: release.tag_name.clone(),
            asset: "SHA256SUMS".to_string(),
        })?
        .browser_download_url
        .clone();

    let sums = client.download(&sums_url)?;
    let sums = String::from_utf8_lossy(&sums);
    let archive = client.download(&archive_url)?;
    verify_checksum(&archive, &sums, &asset.name)?;

    let target = dunce::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let target_dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // Same directory as the target so the final rename stays on one filesystem.
    let staging = tempfile::Builder::new()
        .prefix(".botai-update-")
        .tempdir_in(target_dir)
        .map_err(|e| io_err(e, target_dir))?;
    let new_binary = staging.path().join(platform::binary_name());
    extract_binary(&archive, asset.kind, platform::binary_name(), &new_binary)?;

    let is_running_exe = current_exe().is_ok_and(|exe| exe == target);
    if is_running_exe {
        self_replace::self_replace(&new_binary).map_err(|e| io_err(e, &target))?;
    } else {
        fs::rename(&new_binary, &target).map_err(|e| io_err(e, &target))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_known_input() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn finds_checksum_lines() {
        let sums = "AAAA  botai-v1-linux-x86_64.tar.gz\nbbbb *botai-v1-windows-x86_64.zip\n";
        assert_eq!(
            expected_checksum(sums, "botai-v1-linux-x86_64.tar.gz").as_deref(),
            Some("aaaa")
        );
        assert_eq!(
            expected_checksum(sums, "botai-v1-windows-x86_64.zip").as_deref(),
            Some("bbbb")
        );
        assert_eq!(expected_checksum(sums, "botai-v1-linux"), None);
    }

    #[test]
    fn verify_checksum_outcomes() {
        let good = format!("{}  a.tar.gz\n", sha256_hex(b"data"));
        assert!(verify_checksum(b"data", &good, "a.tar.gz").is_ok());
        assert!(matches!(
            verify_checksum(b"other", &good, "a.tar.gz"),
            Err(UpdateError::ChecksumMismatch { .. })
        ));
        assert!(matches!(
            verify_checksum(b"data", &good, "b.tar.gz"),
            Err(UpdateError::ChecksumMissing(_))
        ));
    }

    #[test]
    fn binary_entry_matching() {
        assert!(is_binary_entry("botai", "botai").unwrap());
        assert!(is_binary_entry("./botai", "botai").unwrap());
        assert!(!is_binary_entry("./README.md", "botai").unwrap());
        assert!(!is_binary_entry("sub/botai", "botai").unwrap());
        assert!(!is_binary_entry("./", "botai").unwrap());
        assert!(is_binary_entry("../botai", "botai").is_err());
        assert!(is_binary_entry("a/../../botai", "botai").is_err());
        assert!(is_binary_entry("..\\botai", "botai").is_err());
        assert!(is_binary_entry("/usr/local/bin/botai", "botai").is_err());
    }
}
