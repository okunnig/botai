//! Parsing and comparison of `BOTAI_VERSION` strings produced by `build.rs`.

use semver::Version;

/// The version this binary was built as, injected by `build.rs` from `git describe`.
pub fn current() -> &'static str {
    env!("BOTAI_VERSION")
}

/// A `BOTAI_VERSION` string split into its semver base and the `git describe` decorations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildVersion {
    pub base: Version,
    pub commits_ahead: bool,
    pub dirty: bool,
    pub dev_fallback: bool,
}

impl BuildVersion {
    /// Parses `v0.1.0`, `v0.1.0-4-gabc1234`, `v0.1.0-dirty`, `v0.0.0-dev-<sha>` and `v0.2.0-rc.1`.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        let without_v = trimmed.strip_prefix('v').unwrap_or(trimmed);

        let (rest, dirty) = match without_v.strip_suffix("-dirty") {
            Some(rest) => (rest, true),
            None => (without_v, false),
        };

        if let Some(sha) = rest.strip_prefix("0.0.0-dev-") {
            if !sha.is_empty() {
                return Some(Self {
                    base: Version::new(0, 0, 0),
                    commits_ahead: false,
                    dirty,
                    dev_fallback: true,
                });
            }
        }

        // Stripped before semver parsing so `v0.0.1-4-gabc` is "after v0.0.1", not a prerelease of it.
        let (rest, commits_ahead) = match strip_describe_suffix(rest) {
            Some(base) => (base, true),
            None => (rest, false),
        };

        let base = Version::parse(rest).ok()?;
        Some(Self {
            base,
            commits_ahead,
            dirty,
            dev_fallback: false,
        })
    }

    /// Whether the build corresponds exactly to a published tag.
    pub fn is_release(&self) -> bool {
        !self.commits_ahead
            && !self.dirty
            && !self.dev_fallback
            && self.base != Version::new(0, 0, 0)
    }
}

/// Strips a trailing `-<N>-g<hex>` from `git describe`, returning the tag part.
fn strip_describe_suffix(s: &str) -> Option<&str> {
    let (head, sha_part) = s.rsplit_once('-')?;
    let sha = sha_part.strip_prefix('g')?;
    if sha.is_empty() || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (tag, count) = head.rsplit_once('-')?;
    if count.is_empty() || !count.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(tag)
}

/// Parses a release tag such as `v0.2.0` or `v0.2.0-rc.1`.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let trimmed = tag.trim();
    Version::parse(trimmed.strip_prefix('v').unwrap_or(trimmed)).ok()
}

/// Whether `raw` is a build of an exact release tag (prerelease tags count).
pub fn is_release_build(raw: &str) -> bool {
    BuildVersion::parse(raw).is_some_and(|v| v.is_release())
}

/// Whether the release tag `latest` is newer than the running build `current`.
///
/// Unparseable current versions count as `0.0.0`; an unparseable `latest` is never newer.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let Some(latest) = parse_tag(latest) else {
        return false;
    };
    let current = BuildVersion::parse(current)
        .map(|v| v.base)
        .unwrap_or_else(|| Version::new(0, 0, 0));
    latest > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_tag() {
        let v = BuildVersion::parse("v0.1.0").unwrap();
        assert_eq!(v.base, Version::new(0, 1, 0));
        assert!(!v.commits_ahead && !v.dirty && !v.dev_fallback);
    }

    #[test]
    fn parses_commits_ahead_as_base_tag() {
        let v = BuildVersion::parse("v0.0.1-4-gabc1234").unwrap();
        assert_eq!(v.base, Version::new(0, 0, 1));
        assert!(v.commits_ahead);
        assert!(!v.dirty);
    }

    #[test]
    fn parses_dirty_and_ahead() {
        let v = BuildVersion::parse("v0.0.1-4-gabc1234-dirty").unwrap();
        assert_eq!(v.base, Version::new(0, 0, 1));
        assert!(v.commits_ahead && v.dirty);
    }

    #[test]
    fn parses_prerelease_tag_with_describe_suffix() {
        let v = BuildVersion::parse("v0.2.0-rc.1-3-gdeadbee").unwrap();
        assert_eq!(v.base, Version::parse("0.2.0-rc.1").unwrap());
        assert!(v.commits_ahead);
    }

    #[test]
    fn parses_dev_fallback() {
        let v = BuildVersion::parse("v0.0.0-dev-a490519").unwrap();
        assert!(v.dev_fallback);
        assert_eq!(v.base, Version::new(0, 0, 0));
        assert!(
            BuildVersion::parse("v0.0.0-dev-a490519-dirty")
                .unwrap()
                .dirty
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(BuildVersion::parse("not-a-version").is_none());
        assert!(BuildVersion::parse("").is_none());
    }

    #[test]
    fn release_build_detection() {
        assert!(is_release_build("v0.0.1"));
        assert!(is_release_build("v0.2.0-rc.1"));
        assert!(is_release_build("v0.0.0-test.1"));
        assert!(!is_release_build("v0.0.0"));
        assert!(!is_release_build("v0.0.0-dev-a490519"));
        assert!(!is_release_build("v0.0.1-dirty"));
        assert!(!is_release_build("v0.0.1-4-gabc1234"));
        assert!(!is_release_build("garbage"));
    }

    #[test]
    fn newer_comparisons() {
        assert!(is_newer("v0.0.1", "v0.0.2"));
        assert!(!is_newer("v0.0.2", "v0.0.1"));
        assert!(!is_newer("v0.0.1", "v0.0.1"));
        assert!(!is_newer("v0.0.1-4-gabc1234", "v0.0.1"));
        assert!(is_newer("v0.0.1-4-gabc1234", "v0.0.2"));
        assert!(is_newer("v0.0.0-dev-a490519", "v0.0.1"));
        assert!(is_newer("v0.0.0-test.1", "v0.0.1"));
        assert!(is_newer("v0.2.0-rc.1", "v0.2.0"));
        assert!(!is_newer("v0.2.0", "v0.2.0-rc.1"));
        assert!(is_newer("garbage", "v0.0.1"));
        assert!(!is_newer("v0.0.1", "garbage"));
    }

    #[test]
    fn parse_tag_accepts_with_and_without_prefix() {
        assert_eq!(parse_tag("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_tag("1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_tag("latest"), None);
    }

    #[test]
    fn current_is_parseable() {
        assert!(BuildVersion::parse(current()).is_some(), "{}", current());
    }
}
