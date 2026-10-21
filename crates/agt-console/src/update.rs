//! Whether a newer agt release exists, for the CLI's notice, `agt upgrade` and
//! the console's new-version button. AGT Studio checks with Tauri's updater
//! instead, because it installs itself.
//!
//! The latest release is read from the redirect of GitHub's `releases/latest`
//! page, which needs no API token and has no rate limit. The answer is cached
//! for a day in the system cache directory, not in an agt home, because one
//! machine can have several homes but has one installed `agt`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The version of this build.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const LATEST_URL: &str = "https://github.com/blacsheep/agt/releases/latest";
/// How long a check stays fresh, and how often the CLI shows its notice.
pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// A published release.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Release {
    pub version: String,
    /// The release page.
    pub url: String,
}

impl Release {
    /// Where this release's files are downloaded from.
    pub fn download_url(&self, file: &str) -> String {
        let base = latest_url();
        let releases = base.strip_suffix("/latest").unwrap_or(&base);
        format!("{releases}/download/v{}/{file}", self.version)
    }

    /// Whether this release is newer than the running build.
    pub fn is_newer(&self) -> bool {
        newer(&self.version, CURRENT)
    }
}

/// `AGT_UPDATE_URL` points the check at another `releases/latest` address
/// (tests use a local server).
fn latest_url() -> String {
    std::env::var("AGT_UPDATE_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| LATEST_URL.to_owned())
}

/// `AGT_NO_UPDATE_CHECK=1` turns off the automatic checks: the CLI's notice
/// and the console's button. `agt upgrade` still checks when asked.
pub fn disabled() -> bool {
    std::env::var("AGT_NO_UPDATE_CHECK").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Ask GitHub for the latest release and remember the answer.
pub async fn latest() -> Result<Release> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("agt/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let url = latest_url();
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let Some(release) = location.as_deref().and_then(parse_location) else {
        bail!(
            "无法从 {url} 读出最新版本（HTTP {}）",
            response.status().as_u16()
        );
    };
    let mut cache = Cache::load();
    cache.checked_at = now();
    cache.latest = Some(release.clone());
    cache.save();
    Ok(release)
}

/// The release a `…/releases/tag/vX.Y.Z` redirect points to.
fn parse_location(location: &str) -> Option<Release> {
    let (_, tag) = location.rsplit_once("/tag/")?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    semver::Version::parse(version).ok()?;
    Some(Release {
        version: version.to_owned(),
        url: location.to_owned(),
    })
}

/// Whether `candidate` is a newer stable version than `current`.
fn newer(candidate: &str, current: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(current),
    ) {
        (Ok(candidate), Ok(current)) => candidate.pre.is_empty() && candidate > current,
        _ => false,
    }
}

/// The newer release from the last check, checking again when that is a day
/// old. A failed check counts as a check, so an offline machine is not asked
/// again on every call.
pub async fn newer_release() -> Option<Release> {
    let cache = Cache::load();
    let release = if cache.fresh() {
        cache.latest
    } else {
        match latest().await {
            Ok(release) => Some(release),
            Err(_) => {
                let mut cache = cache;
                cache.checked_at = now();
                cache.save();
                cache.latest
            }
        }
    };
    release.filter(Release::is_newer)
}

/// The CLI's one-line notice, at most once a day: the newer release from the
/// cache, after `newer_release` had its chance to check.
pub fn take_notice() -> Option<String> {
    let mut cache = Cache::load();
    let release = cache.latest.clone().filter(Release::is_newer)?;
    if now().saturating_sub(cache.noticed_at) < DAY.as_secs() {
        return None;
    }
    cache.noticed_at = now();
    cache.save();
    Some(say(
        &format!(
            "agt {} 已发布（当前 {CURRENT}），运行 agt upgrade 升级",
            release.version
        ),
        &format!(
            "agt {} is available (you have {CURRENT}); run agt upgrade to update",
            release.version
        ),
    ))
}

#[derive(Default, Deserialize, Serialize)]
struct Cache {
    /// Unix seconds of the last check, successful or not.
    #[serde(default)]
    checked_at: u64,
    #[serde(default)]
    latest: Option<Release>,
    /// Unix seconds of the last CLI notice.
    #[serde(default)]
    noticed_at: u64,
}

impl Cache {
    /// `AGT_UPDATE_CACHE` names the file instead (tests).
    fn path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("AGT_UPDATE_CACHE").filter(|v| !v.is_empty()) {
            return Some(path.into());
        }
        Some(dirs::cache_dir()?.join("agt").join("update-check.json"))
    }

    fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Best effort: a cache that cannot be written only means checking again.
    fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = std::fs::write(path, bytes);
        }
    }

    fn fresh(&self) -> bool {
        now().saturating_sub(self.checked_at) < DAY.as_secs()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Terminal text follows the system language, as `agt web`'s output does.
pub fn say(zh: &str, en: &str) -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let chinese = locale
        .get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case("zh"));
    if chinese { zh } else { en }.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_release_from_the_redirect() {
        let release =
            parse_location("https://github.com/blacsheep/agt/releases/tag/v0.6.1").unwrap();
        assert_eq!(release.version, "0.6.1");
        assert_eq!(
            release.url,
            "https://github.com/blacsheep/agt/releases/tag/v0.6.1"
        );
        assert!(parse_location("https://github.com/blacsheep/agt/releases").is_none());
        assert!(parse_location("https://github.com/blacsheep/agt/releases/tag/nightly").is_none());
    }

    #[test]
    fn only_a_later_stable_release_is_newer() {
        assert!(newer("0.6.2", "0.6.1"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.6.1", "0.6.1"));
        assert!(!newer("0.6.0", "0.6.1"));
        assert!(!newer("0.7.0-rc.1", "0.6.1"));
        assert!(!newer("latest", "0.6.1"));
    }
}
