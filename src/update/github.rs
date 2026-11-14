//! Minimal GitHub Releases API client.

use serde::Deserialize;
use std::io::Read;
use std::time::Duration;

use super::{version, UpdateError};

/// Repository whose releases botai updates from.
const REPO: &str = "okunnig/botai";
/// Public GitHub API endpoint.
const DEFAULT_API_BASE: &str = "https://api.github.com";
/// Hidden override of the API base URL for tests. Debug builds only, so a release binary
/// cannot be pointed at another download source.
#[cfg(debug_assertions)]
const BASE_URL_ENV: &str = "BOTAI_UPDATE_BASE_URL";

/// Upper bound on a downloaded asset; release archives are a few MB.
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

/// A downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

impl Release {
    /// Download URL of the attached asset with exactly this name.
    pub fn asset_url(&self, name: &str) -> Result<&str, UpdateError> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.as_str())
            .ok_or_else(|| UpdateError::AssetMissing {
                tag: self.tag_name.clone(),
                asset: name.to_string(),
            })
    }
}

/// Which release to resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseQuery {
    /// Newest stable release (`/releases/latest` excludes prereleases and drafts).
    Latest,
    /// Newest release including prereleases.
    LatestIncludingPrerelease,
    /// A specific tag.
    Tag(String),
}

/// Normalizes a user supplied tag so `0.2.0` and `v0.2.0` both work.
fn normalize_tag(tag: &str) -> String {
    let bare = version::strip_v(tag);
    if bare.starts_with(|c: char| c.is_ascii_digit()) {
        format!("v{bare}")
    } else {
        tag.trim().to_string()
    }
}

/// Rejects tags that could alter the request path (`/`, `?`, `%`, `..`, ...).
fn validate_tag(tag: &str) -> Result<(), UpdateError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-');
    if tag.is_empty() || tag == "." || tag.contains("..") || !tag.chars().all(allowed) {
        return Err(UpdateError::InvalidTag(tag.to_string()));
    }
    Ok(())
}

/// The `message` field of a GitHub JSON error body, if any.
fn github_message(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct ErrorBody {
        message: String,
    }
    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .map(|b| b.message)
        .filter(|m| !m.trim().is_empty())
}

/// API base URL, honouring `BOTAI_UPDATE_BASE_URL` in debug builds.
fn api_base_from_env() -> String {
    #[cfg(debug_assertions)]
    if let Some(base) = std::env::var(BASE_URL_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        return base;
    }
    DEFAULT_API_BASE.to_string()
}

/// HTTP client for release lookups and asset downloads.
#[derive(Debug, Clone)]
pub struct Client {
    agent: ureq::Agent,
    api_base: String,
    token: Option<String>,
    user_agent: String,
}

impl Client {
    /// Creates a client against `api_base` with a per-request timeout.
    pub fn new(api_base: impl Into<String>, timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            api_base: api_base.into().trim_end_matches('/').to_string(),
            token: None,
            user_agent: format!("botai/{}", version::current()),
        }
    }

    /// Creates a client from the environment: `GITHUB_TOKEN`, plus `BOTAI_UPDATE_BASE_URL` in debug builds.
    pub fn from_env(timeout: Duration) -> Self {
        let token = std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty());
        Self::new(api_base_from_env(), timeout).with_token(token)
    }

    /// Sets the bearer token sent to the API (raises the 60 req/h anonymous limit).
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    /// GETs an API path; `None` means 404.
    fn api_get(&self, path: &str) -> Result<Option<String>, UpdateError> {
        let url = format!("{}/repos/{REPO}{path}", self.api_base);
        let mut req = self
            .agent
            .get(&url)
            .header("User-Agent", &self.user_agent)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &self.token {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        let mut resp = req.call().map_err(|e| UpdateError::Http {
            url: url.clone(),
            message: e.to_string(),
        })?;
        let status = resp.status().as_u16();
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok());
        // GitHub signals primary limits with `x-ratelimit-remaining: 0`, secondary ones with
        // `retry-after`; any other 403 is a permission problem.
        let rate_limited = status == 429
            || (status == 403
                && (header("x-ratelimit-remaining").is_some_and(|v| v.trim() == "0")
                    || header("retry-after").is_some()));
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| UpdateError::Http {
                url: url.clone(),
                message: e.to_string(),
            })?;
        match status {
            200..=299 => Ok(Some(body)),
            404 => Ok(None),
            _ if rate_limited => Err(UpdateError::RateLimited),
            _ => Err(UpdateError::Http {
                url,
                message: match github_message(&body) {
                    Some(message) => format!("HTTP {status}: {message}"),
                    None => format!("HTTP {status}"),
                },
            }),
        }
    }

    /// Resolves a release.
    pub fn fetch_release(&self, query: &ReleaseQuery) -> Result<Release, UpdateError> {
        match query {
            ReleaseQuery::Latest => {
                let body = self
                    .api_get("/releases/latest")?
                    .ok_or(UpdateError::NoRelease)?;
                parse_json(&body)
            }
            ReleaseQuery::Tag(tag) => {
                let tag = normalize_tag(tag);
                validate_tag(&tag)?;
                let Some(body) = self.api_get(&format!("/releases/tags/{tag}"))? else {
                    return Err(UpdateError::ReleaseNotFound(tag));
                };
                let release: Release = parse_json(&body)?;
                if release.draft {
                    return Err(UpdateError::ReleaseNotFound(tag));
                }
                Ok(release)
            }
            ReleaseQuery::LatestIncludingPrerelease => {
                let body = self
                    .api_get("/releases?per_page=30")?
                    .ok_or(UpdateError::NoRelease)?;
                let releases: Vec<Release> = parse_json(&body)?;
                releases
                    .into_iter()
                    .filter(|r| !r.draft)
                    .filter_map(|r| version::parse_tag(&r.tag_name).map(|v| (v, r)))
                    .max_by(|a, b| a.0.cmp(&b.0))
                    .map(|(_, r)| r)
                    .ok_or(UpdateError::NoRelease)
            }
        }
    }

    /// Downloads a release asset into memory. The token is not sent: asset URLs redirect to a CDN.
    pub fn download(&self, url: &str) -> Result<Vec<u8>, UpdateError> {
        let http_err = |message: String| UpdateError::Http {
            url: url.to_string(),
            message,
        };
        let resp = self
            .agent
            .get(url)
            .header("User-Agent", &self.user_agent)
            .header("Accept", "application/octet-stream")
            .call()
            .map_err(|e| http_err(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(http_err(format!("HTTP {status}")));
        }
        let mut bytes = Vec::new();
        resp.into_body()
            .into_with_config()
            .limit(MAX_DOWNLOAD_BYTES)
            .reader()
            .read_to_end(&mut bytes)
            .map_err(|e| http_err(e.to_string()))?;
        Ok(bytes)
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, UpdateError> {
    serde_json::from_str(body).map_err(|e| UpdateError::InvalidResponse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize_tag("0.2.0"), "v0.2.0");
        assert_eq!(normalize_tag("v0.2.0"), "v0.2.0");
        assert_eq!(normalize_tag(" v0.2.0-rc.1 "), "v0.2.0-rc.1");
    }

    #[test]
    fn validates_tags() {
        for ok in [
            "v0.2.0",
            "0.2.0",
            "v0.2.0-rc.1",
            "v1.0.0+build.5",
            "nightly_2",
        ] {
            assert!(validate_tag(&normalize_tag(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "", " ", ".", "..", "v1..2", "../x", "v1/2", "v1?x", "v1#x", "v1%2F", "a b", "v1\\x",
        ] {
            assert!(
                matches!(
                    validate_tag(&normalize_tag(bad)),
                    Err(UpdateError::InvalidTag(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn github_message_is_extracted_from_json_body() {
        assert_eq!(
            github_message(r#"{"message":"Bad credentials","documentation_url":"x"}"#),
            Some("Bad credentials".to_string())
        );
        assert_eq!(github_message("<html>"), None);
        assert_eq!(github_message(r#"{"message":""}"#), None);
    }

    #[test]
    fn parses_release_payload_ignoring_unknown_fields() {
        let json = r#"{"tag_name":"v0.0.1","draft":false,"prerelease":false,"id":1,
            "assets":[{"name":"SHA256SUMS","browser_download_url":"https://x/SHA256SUMS","size":3}]}"#;
        let release: Release = parse_json(json).unwrap();
        assert_eq!(release.tag_name, "v0.0.1");
        assert_eq!(
            release.asset_url("SHA256SUMS").unwrap(),
            "https://x/SHA256SUMS"
        );
        assert!(matches!(
            release.asset_url("missing"),
            Err(UpdateError::AssetMissing { ref tag, ref asset }) if tag == "v0.0.1" && asset == "missing"
        ));
    }

    #[test]
    fn invalid_json_is_reported() {
        let err = parse_json::<Release>("<html>").unwrap_err();
        assert!(matches!(err, UpdateError::InvalidResponse(_)));
    }
}
