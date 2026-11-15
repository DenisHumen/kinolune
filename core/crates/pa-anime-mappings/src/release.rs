// SPDX-License-Identifier: MPL-2.0
//! The latest anime-offline-database release on GitHub (META-030).
//!
//! The app asks the GitHub REST API for the latest release of the upstream
//! repository once a day ([`crate::update::Updater::refresh_offline_db`]),
//! and downloads the zstd-compressed JSONL asset only when the release tag
//! is newer than the database the device has (bundled or downloaded).
//! Weekly releases are tagged `<year>-<week>` (`2026-46`).
//!
//! Upstream: `manami-project/anime-offline-database` was archived on
//! 2026-07-04 (last release `2026-27`); the continuation with the same
//! licence is `cedya77/anime-offline-database` (`docs/legal/anime-mappings.md`
//! §2). Both URLs are fixed here and point at upstream, never at a mirror.
//!
//! What a release must look like before anything is downloaded: a tag of
//! the `YYYY-WW` form, the asset [`ASSET`] with a download URL under
//! [`DOWNLOAD_PREFIX`] (a tampered API answer cannot send the device
//! elsewhere) and a plausible size. GitHub publishes the SHA-256 of every
//! asset (`digest`); the download is checked against it and its size.

use serde::Deserialize;

use crate::{Error, Result};

/// GitHub REST endpoint of the latest release.
pub const LATEST_URL: &str =
    "https://api.github.com/repos/cedya77/anime-offline-database/releases/latest";
/// Every asset URL must start with this.
pub const DOWNLOAD_PREFIX: &str =
    "https://github.com/cedya77/anime-offline-database/releases/download/";
/// The asset the device downloads: the JSONL compressed with zstd (~6 MB
/// instead of ~60 MB).
pub const ASSET: &str = "anime-offline-database.jsonl.zst";
/// Larger assets are rejected before downloading (a truncated or empty one
/// fails the entry-count check after it).
pub const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;

/// The asset of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    /// Release tag, `2026-46`.
    pub tag: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex SHA-256 published by GitHub, if any.
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// `(year, week)` of a `YYYY-WW` tag.
pub fn tag_key(tag: &str) -> Option<(u16, u8)> {
    let (y, w) = tag.split_once('-')?;
    if y.len() != 4 || w.is_empty() || w.len() > 2 {
        return None;
    }
    if !y.bytes().chain(w.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let week: u8 = w.parse().ok()?;
    (1..=53).contains(&week).then_some(())?;
    Some((y.parse().ok()?, week))
}

/// `true` if release tag `a` is newer than `b` (an unparsable `b` is
/// older than any valid tag).
pub fn newer(a: &str, b: Option<&str>) -> bool {
    match (tag_key(a), b.and_then(tag_key)) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// The [`ASSET`] of a GitHub "latest release" response, checked.
pub fn parse_latest(json: &[u8]) -> Result<ReleaseAsset> {
    let r: ApiRelease = serde_json::from_slice(json)?;
    if r.draft || r.prerelease {
        return Err(Error::Invalid(format!(
            "release {} is a draft or pre-release",
            r.tag_name
        )));
    }
    if tag_key(&r.tag_name).is_none() {
        return Err(Error::Invalid(format!(
            "unexpected release tag {:?}",
            r.tag_name
        )));
    }
    let a = r
        .assets
        .into_iter()
        .find(|a| a.name == ASSET)
        .ok_or_else(|| Error::Invalid(format!("release {} has no {ASSET}", r.tag_name)))?;
    let expected = format!("{DOWNLOAD_PREFIX}{}/{ASSET}", r.tag_name);
    if a.browser_download_url != expected {
        return Err(Error::Invalid(format!(
            "asset URL {:?} is not {expected:?}",
            a.browser_download_url
        )));
    }
    if a.size == 0 || a.size > MAX_ASSET_BYTES {
        return Err(Error::Invalid(format!("asset size {} out of range", a.size)));
    }
    let sha256 = match a.digest {
        None => None,
        Some(d) => {
            let hex = d
                .strip_prefix("sha256:")
                .ok_or_else(|| Error::Invalid(format!("unsupported digest {d:?}")))?;
            if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::Invalid(format!("malformed digest {d:?}")));
            }
            Some(hex.to_ascii_lowercase())
        }
    };
    Ok(ReleaseAsset {
        tag: r.tag_name,
        url: a.browser_download_url,
        size: a.size,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, url: &str, size: u64, digest: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[
              {{"name":"anime-offline-database.jsonl","browser_download_url":"x","size":1}},
              {{"name":"{ASSET}","browser_download_url":"{url}","size":{size},"digest":"{digest}"}}]}}"#
        )
    }

    #[test]
    fn tags() {
        assert_eq!(tag_key("2026-46"), Some((2026, 46)));
        assert_eq!(tag_key("2026-1"), Some((2026, 1)));
        for bad in ["latest", "2026-54", "2026-00", "26-46", "2026-4a", "2026-046", ""] {
            assert_eq!(tag_key(bad), None, "{bad}");
        }
        assert!(newer("2026-46", Some("2026-45")));
        assert!(newer("2027-1", Some("2026-52")));
        assert!(!newer("2026-45", Some("2026-45")));
        assert!(!newer("2026-44", Some("2026-45")));
        assert!(newer("2026-44", None));
        assert!(!newer("latest", None));
    }

    #[test]
    fn latest_release_is_checked() {
        let sha = "AB".repeat(32);
        let url = format!("{DOWNLOAD_PREFIX}2026-46/{ASSET}");
        let ok = parse_latest(release("2026-46", &url, 6_000_000, &format!("sha256:{sha}")).as_bytes())
            .unwrap();
        assert_eq!(ok.tag, "2026-46");
        assert_eq!(ok.size, 6_000_000);
        assert_eq!(ok.sha256.as_deref(), Some("ab".repeat(32).as_str()));

        // Another host, another repository, another tag in the path.
        for bad in [
            format!("https://example.org/2026-46/{ASSET}"),
            format!("https://github.com/someone/anime-offline-database/releases/download/2026-46/{ASSET}"),
            format!("{DOWNLOAD_PREFIX}2026-45/{ASSET}"),
        ] {
            let body = release("2026-46", &bad, 6_000_000, &format!("sha256:{sha}"));
            assert!(parse_latest(body.as_bytes()).is_err(), "{bad}");
        }
        // Implausible size, malformed digest, bad tag.
        assert!(parse_latest(release("2026-46", &url, 0, &format!("sha256:{sha}")).as_bytes()).is_err());
        assert!(parse_latest(release("2026-46", &url, 6_000_000, "md5:00").as_bytes()).is_err());
        assert!(parse_latest(release("2026-46", &url, 6_000_000, "sha256:zz").as_bytes()).is_err());
        assert!(parse_latest(release("latest", &url, 6_000_000, &format!("sha256:{sha}")).as_bytes()).is_err());
        assert!(parse_latest(b"<html>rate limited</html>").is_err());
    }
}
