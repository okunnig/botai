//! Maps the build target to the release asset names published by `.github/workflows/release.yml`.

use super::UpdateError;

/// Archive format of a release asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

/// A release asset for one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformAsset {
    pub name: String,
    pub kind: ArchiveKind,
}

/// Name of the botai executable inside release archives.
pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "botai.exe"
    } else {
        "botai"
    }
}

/// Release asset for `tag` on the given OS / arch (`std::env::consts` spelling).
pub fn asset_for(os: &str, arch: &str, tag: &str) -> Result<PlatformAsset, UpdateError> {
    let (label, kind) = match (os, arch) {
        ("linux", "x86_64") => ("linux-x86_64", ArchiveKind::TarGz),
        ("macos", "aarch64") => ("macos-aarch64", ArchiveKind::TarGz),
        ("windows", "x86_64") => ("windows-x86_64", ArchiveKind::Zip),
        _ => {
            return Err(UpdateError::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            })
        }
    };
    let ext = match kind {
        ArchiveKind::TarGz => "tar.gz",
        ArchiveKind::Zip => "zip",
    };
    Ok(PlatformAsset {
        name: format!("botai-{tag}-{label}.{ext}"),
        kind,
    })
}

/// Release asset for `tag` on the platform this binary was built for.
pub fn current_asset(tag: &str) -> Result<PlatformAsset, UpdateError> {
    asset_for(std::env::consts::OS, std::env::consts::ARCH, tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_platform_asset_names() {
        assert_eq!(
            asset_for("linux", "x86_64", "v0.0.1").unwrap(),
            PlatformAsset {
                name: "botai-v0.0.1-linux-x86_64.tar.gz".into(),
                kind: ArchiveKind::TarGz
            }
        );
        assert_eq!(
            asset_for("macos", "aarch64", "v0.0.1").unwrap().name,
            "botai-v0.0.1-macos-aarch64.tar.gz"
        );
        let win = asset_for("windows", "x86_64", "v0.2.0-rc.1").unwrap();
        assert_eq!(win.name, "botai-v0.2.0-rc.1-windows-x86_64.zip");
        assert_eq!(win.kind, ArchiveKind::Zip);
    }

    #[test]
    fn unsupported_platform_points_to_cargo_install() {
        for (os, arch) in [
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("freebsd", "x86_64"),
        ] {
            let err = asset_for(os, arch, "v0.0.1").unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains(os) && msg.contains(arch), "{msg}");
            assert!(msg.contains("cargo install --git"), "{msg}");
        }
    }

    #[test]
    fn binary_name_matches_target() {
        if cfg!(windows) {
            assert_eq!(binary_name(), "botai.exe");
        } else {
            assert_eq!(binary_name(), "botai");
        }
    }
}
