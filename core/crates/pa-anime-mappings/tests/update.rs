// SPDX-License-Identifier: MPL-2.0
//! Updater tests with a mocked HTTP client (no network).

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use pa_anime_mappings::update::{HttpClient, HttpResponse, Outcome, Source, Updater, sha256_hex};

const XML: &str = include_str!("fixtures/anime-list-master.xml");
const DAY: u64 = 24 * 60 * 60;

/// Serves queued responses and records requests.
#[derive(Default)]
struct Mock {
    responses: RefCell<Vec<HttpResponse>>,
    requests: RefCell<Vec<(String, Option<String>)>>,
}

impl Mock {
    fn push(&self, status: u16, etag: Option<&str>, body: &[u8]) {
        self.responses.borrow_mut().push(HttpResponse {
            status,
            etag: etag.map(str::to_owned),
            body: body.to_vec(),
        });
    }
}

impl HttpClient for Mock {
    fn get(&self, url: &str, if_none_match: Option<&str>) -> Result<HttpResponse, String> {
        self.requests
            .borrow_mut()
            .push((url.to_owned(), if_none_match.map(str::to_owned)));
        let mut r = self.responses.borrow_mut();
        if r.is_empty() {
            return Err("offline".into());
        }
        Ok(r.remove(0))
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("pa-anime-mappings-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn updater(dir: &TempDir) -> Updater {
    let mut u = Updater::new(&dir.0);
    u.min_entries_override = Some(1);
    u
}

fn data_files(dir: &TempDir) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != "manifest.json")
        .collect();
    v.sort();
    v
}

#[test]
fn download_cache_etag_and_atomic_replace() {
    let dir = TempDir::new("flow");
    let u = updater(&dir);
    let http = Mock::default();
    let src = Source::AnimeLists;

    assert_eq!(u.load(src).unwrap(), None);

    http.push(200, Some("\"v1\""), XML.as_bytes());
    assert_eq!(
        u.refresh(&http, src, 1_000, false).unwrap(),
        Outcome::Updated { entries: 6 }
    );
    assert_eq!(u.load(src).unwrap().unwrap(), XML.as_bytes());
    let st = u.status(src).unwrap().unwrap();
    assert_eq!(st.sha256, sha256_hex(XML.as_bytes()));
    assert_eq!(st.url, src.spec().url);

    // Within 24 h: no request at all.
    assert_eq!(
        u.refresh(&http, src, 1_000 + DAY - 1, false).unwrap(),
        Outcome::Skipped
    );
    assert_eq!(http.requests.borrow().len(), 1);

    // After 24 h: conditional request, 304.
    http.push(304, None, b"");
    assert_eq!(
        u.refresh(&http, src, 1_000 + DAY, false).unwrap(),
        Outcome::NotModified
    );
    assert_eq!(http.requests.borrow()[1].1.as_deref(), Some("\"v1\""));
    assert_eq!(u.status(src).unwrap().unwrap().checked_at, 1_000 + DAY);

    // Broken download: rejected, old version kept.
    http.push(200, Some("\"v2\""), b"<anime-list><anime tvdbid=");
    assert!(u.refresh(&http, src, 5 * DAY, true).is_err());
    assert_eq!(u.load(src).unwrap().unwrap(), XML.as_bytes());

    // Network error: old version kept.
    assert!(u.refresh(&http, src, 5 * DAY, true).is_err());
    assert_eq!(u.load(src).unwrap().unwrap(), XML.as_bytes());

    // New version replaces the old file; only one data file remains.
    let v2 = XML.replace("Example Saga</name>", "Example Saga (v2)</name>");
    http.push(200, Some("\"v2\""), v2.as_bytes());
    assert_eq!(
        u.refresh(&http, src, 6 * DAY, true).unwrap(),
        Outcome::Updated { entries: 6 }
    );
    assert_eq!(u.load(src).unwrap().unwrap(), v2.as_bytes());
    let files = data_files(&dir);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].starts_with("anime-lists.") && files[0].ends_with(".xml"));
}

#[test]
fn corrupted_cache_is_detected_and_refetched_without_etag() {
    let dir = TempDir::new("corrupt");
    let u = updater(&dir);
    let http = Mock::default();
    let src = Source::AnimeLists;
    http.push(200, Some("\"v1\""), XML.as_bytes());
    u.refresh(&http, src, 0, false).unwrap();

    let file = u.status(src).unwrap().unwrap().file;
    fs::write(dir.0.join(file), b"tampered").unwrap();
    assert!(u.load(src).is_err());

    // Not skipped despite the interval, and no ETag sent.
    http.push(200, Some("\"v1\""), XML.as_bytes());
    assert_eq!(
        u.refresh(&http, src, 10, false).unwrap(),
        Outcome::Updated { entries: 6 }
    );
    assert_eq!(http.requests.borrow()[1].1, None);
    assert_eq!(u.load(src).unwrap().unwrap(), XML.as_bytes());
}

#[test]
fn rejects_too_small_and_shrunken_lists() {
    let dir = TempDir::new("small");
    let mut u = Updater::new(&dir.0);
    let http = Mock::default();
    // Default minimum for Anime-Lists is thousands of entries.
    http.push(200, None, XML.as_bytes());
    assert!(u.refresh(&http, Source::AnimeLists, 0, false).is_err());
    assert_eq!(u.load(Source::AnimeLists).unwrap(), None);

    u.min_entries_override = Some(1);
    http.push(200, None, XML.as_bytes());
    u.refresh(&http, Source::AnimeLists, 0, false).unwrap();
    let tiny = "<anime-list><anime anidbid=\"1\" tvdbid=\"2\"/></anime-list>";
    http.push(200, None, tiny.as_bytes());
    assert!(u.refresh(&http, Source::AnimeLists, 0, true).is_err());
    assert_eq!(u.load(Source::AnimeLists).unwrap().unwrap(), XML.as_bytes());

    http.push(500, None, b"");
    assert!(u.refresh(&http, Source::AnimeLists, 0, true).is_err());
}

#[test]
fn unlicensed_sources_come_from_upstream_only() {
    for src in Source::ALL {
        let spec = src.spec();
        assert!(spec.url.starts_with("https://"), "{}", spec.url);
        assert!(
            !spec.url.to_lowercase().contains("project_alpha"),
            "{}",
            spec.url
        );
        assert!(
            !spec.url.to_lowercase().contains("denishumen"),
            "{}",
            spec.url
        );
    }
    let al = Source::AnimeLists.spec();
    assert!(!al.redistributable);
    assert!(
        al.url
            .starts_with("https://raw.githubusercontent.com/Anime-Lists/anime-lists/")
    );
    let fr = Source::Fribb.spec();
    assert!(!fr.redistributable);
    assert!(
        fr.url
            .starts_with("https://raw.githubusercontent.com/Fribb/anime-lists/")
    );
    let od = Source::OfflineDatabase.spec();
    assert!(od.redistributable);
    assert!(
        od.url
            .starts_with("https://github.com/cedya77/anime-offline-database/")
    );
    assert!(
        pa_anime_mappings::release::LATEST_URL
            .starts_with("https://api.github.com/repos/cedya77/anime-offline-database/")
    );
}

// --- anime-offline-database releases (META-030) -----------------------------

mod offline_db_releases {
    use super::*;
    use pa_anime_mappings::offline_db;
    use pa_anime_mappings::release::{ASSET, DOWNLOAD_PREFIX, LATEST_URL};
    use pa_anime_mappings::update::OFFLINE_DB_KEY;

    const AOD: &str = include_str!("fixtures/anime-offline-database.jsonl");

    /// Entries of the fixture with a recognised provider id.
    fn base() -> usize {
        offline_db::parse_upstream_jsonl(AOD.as_bytes())
            .unwrap()
            .1
            .len()
    }

    /// The fixture JSONL with `extra` more entries.
    fn upstream(extra: usize) -> String {
        let mut s = AOD.to_owned();
        if !s.ends_with('\n') {
            s.push('\n');
        }
        for i in 0..extra {
            s.push_str(&format!(
                "{{\"sources\":[\"https://anidb.net/anime/{}\"],\"title\":\"Extra {i}\",\"type\":\"TV\",\"episodes\":12}}\n",
                50_000 + i
            ));
        }
        s
    }

    /// A GitHub "latest release" answer listing `body` as the zstd asset.
    fn latest(tag: &str, body: &[u8], digest: Option<&str>) -> Vec<u8> {
        let digest = digest.map_or_else(|| format!("sha256:{}", sha256_hex(body)), str::to_owned);
        format!(
            r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[{{"name":"{ASSET}","browser_download_url":"{DOWNLOAD_PREFIX}{tag}/{ASSET}","size":{},"digest":"{digest}"}}]}}"#,
            body.len()
        )
        .into_bytes()
    }

    fn installed(u: &Updater) -> (String, usize) {
        let (h, e) = u.load_offline_db().unwrap().unwrap();
        (h.upstream_tag, e.len())
    }

    #[test]
    fn newer_release_replaces_the_database_atomically() {
        let dir = TempDir::new("aod-flow");
        let u = updater(&dir);
        let http = Mock::default();
        assert_eq!(u.load_offline_db().unwrap(), None);

        // Same tag as the bundled database: only the API is asked.
        let v1 = offline_db::compress(upstream(10).as_bytes());
        http.push(200, None, &latest("2026-45", &v1, None));
        assert_eq!(
            u.refresh_offline_db(&http, 1_000, false, Some("2026-45"))
                .unwrap(),
            Outcome::NotModified
        );
        assert_eq!(http.requests.borrow().len(), 1);
        assert_eq!(http.requests.borrow()[0].0, LATEST_URL);
        assert_eq!(u.load_offline_db().unwrap(), None);

        // Within 24 h: no request at all.
        assert_eq!(
            u.refresh_offline_db(&http, 1_000 + DAY - 1, false, Some("2026-45"))
                .unwrap(),
            Outcome::Skipped
        );
        assert_eq!(http.requests.borrow().len(), 1);

        // A newer release: downloaded, verified, installed.
        http.push(200, None, &latest("2026-46", &v1, None));
        http.push(200, None, &v1);
        assert_eq!(
            u.refresh_offline_db(&http, 1_000 + DAY, false, Some("2026-45"))
                .unwrap(),
            Outcome::Updated {
                entries: base() + 10
            }
        );
        assert_eq!(
            http.requests.borrow()[2].0,
            format!("{DOWNLOAD_PREFIX}2026-46/{ASSET}")
        );
        assert_eq!(installed(&u), ("2026-46".to_owned(), base() + 10));
        let st = u.offline_db_status().unwrap().unwrap();
        assert_eq!(st.tag.as_deref(), Some("2026-46"));
        assert_eq!(u.offline_db_checked_at().unwrap(), Some(1_000 + DAY));

        // The next release replaces it; one data file remains.
        let v2 = offline_db::compress(upstream(20).as_bytes());
        http.push(200, None, &latest("2026-47", &v2, None));
        http.push(200, None, &v2);
        assert_eq!(
            u.refresh_offline_db(&http, 1_000 + 2 * DAY, false, Some("2026-45"))
                .unwrap(),
            Outcome::Updated {
                entries: base() + 20
            }
        );
        assert_eq!(installed(&u), ("2026-47".to_owned(), base() + 20));
        let files = data_files(&dir);
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].starts_with(OFFLINE_DB_KEY) && files[0].ends_with(".jsonl.zst"));

        // An older release is not installed over a newer one.
        http.push(200, None, &latest("2026-46", &v1, None));
        assert_eq!(
            u.refresh_offline_db(&http, 1_000 + 3 * DAY, false, None)
                .unwrap(),
            Outcome::NotModified
        );
        assert_eq!(installed(&u), ("2026-47".to_owned(), base() + 20));
    }

    /// Done when META-030: a replaced (tampered) release and corrupt
    /// downloads never replace the working database.
    #[test]
    fn tampered_or_corrupt_downloads_keep_the_working_file() {
        let dir = TempDir::new("aod-bad");
        let mut u = updater(&dir);
        u.min_entries_override = Some(5);
        let http = Mock::default();
        let good = offline_db::compress(upstream(10).as_bytes());
        http.push(200, None, &latest("2026-46", &good, None));
        http.push(200, None, &good);
        u.refresh_offline_db(&http, 0, false, Some("2026-45"))
            .unwrap();
        let working = data_files(&dir);
        let working_bytes = fs::read(dir.0.join(&working[0])).unwrap();

        let evil =
            offline_db::compress(upstream(10).replace("Example Saga", "Evil Saga").as_bytes());
        let truncated = good[..good.len() / 2].to_vec();
        let not_odbl = offline_db::compress(
            upstream(10)
                .replace(
                    "Open Data Commons Open Database License (ODbL)",
                    "All rights reserved",
                )
                .as_bytes(),
        );
        let tiny = offline_db::compress(
            AOD.lines()
                .take(2)
                .collect::<Vec<_>>()
                .join("\n")
                .as_bytes(),
        );
        let html = b"<html>oops</html>".to_vec();
        let zeros = format!("sha256:{}", "0".repeat(64));
        let cases: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
            // The asset was swapped after the release was published: its
            // SHA-256 no longer matches the one GitHub lists.
            (
                "replaced asset",
                latest("2026-47", &good, None),
                evil.clone(),
            ),
            // The release answer itself carries another checksum.
            ("wrong digest", latest("2026-47", &evil, Some(&zeros)), evil),
            // Cut-off download: size and checksum differ.
            (
                "truncated",
                latest("2026-47", &good, None),
                truncated.clone(),
            ),
            // Corrupt zstd that matches its (equally corrupt) digest.
            (
                "corrupt zstd",
                latest("2026-47", &truncated, None),
                truncated,
            ),
            ("not zstd", latest("2026-47", &html, None), html),
            (
                "licence changed",
                latest("2026-47", &not_odbl, None),
                not_odbl,
            ),
            ("too few entries", latest("2026-47", &tiny, None), tiny),
        ];
        for (i, (name, api, body)) in cases.into_iter().enumerate() {
            http.responses.borrow_mut().clear();
            http.push(200, None, &api);
            http.push(200, None, &body);
            let now = (i as u64 + 1) * DAY;
            assert!(
                u.refresh_offline_db(&http, now, true, None).is_err(),
                "{name} was accepted"
            );
            assert_eq!(data_files(&dir), working, "{name}");
            assert_eq!(
                fs::read(dir.0.join(&working[0])).unwrap(),
                working_bytes,
                "{name}"
            );
            assert_eq!(installed(&u), ("2026-46".to_owned(), base() + 10), "{name}");
        }
        http.responses.borrow_mut().clear();

        // A release pointing somewhere else is refused before downloading.
        let elsewhere = String::from_utf8(latest("2026-48", &good, None))
            .unwrap()
            .replace(DOWNLOAD_PREFIX, "https://example.org/");
        let before = http.requests.borrow().len();
        http.push(200, None, elsewhere.as_bytes());
        assert!(u.refresh_offline_db(&http, 20 * DAY, true, None).is_err());
        assert_eq!(http.requests.borrow().len(), before + 1);

        // GitHub rate limit: kept, and the check still counts for the day.
        http.push(403, None, b"{\"message\":\"API rate limit exceeded\"}");
        assert!(u.refresh_offline_db(&http, 30 * DAY, false, None).is_err());
        assert_eq!(
            u.refresh_offline_db(&http, 30 * DAY + 1, false, None)
                .unwrap(),
            Outcome::Skipped
        );
        // Offline: kept, and the next start asks again.
        assert!(u.refresh_offline_db(&http, 40 * DAY, false, None).is_err());
        assert_eq!(u.offline_db_checked_at().unwrap(), Some(30 * DAY));
        assert_eq!(installed(&u), ("2026-46".to_owned(), base() + 10));
    }

    #[test]
    fn damaged_installed_file_is_not_trusted() {
        let dir = TempDir::new("aod-damaged");
        let u = updater(&dir);
        let http = Mock::default();
        let good = offline_db::compress(upstream(10).as_bytes());
        http.push(200, None, &latest("2026-46", &good, None));
        http.push(200, None, &good);
        u.refresh_offline_db(&http, 0, false, None).unwrap();
        let file = u.offline_db_status().unwrap().unwrap().file;
        fs::write(dir.0.join(&file), b"tampered").unwrap();
        // Loading fails (the app falls back to the bundled database)…
        assert!(u.load_offline_db().is_err());
        // …and the same release counts as not installed: downloaded again.
        http.push(200, None, &latest("2026-46", &good, None));
        http.push(200, None, &good);
        assert_eq!(
            u.refresh_offline_db(&http, DAY, false, Some("2026-45"))
                .unwrap(),
            Outcome::Updated {
                entries: base() + 10
            }
        );
        assert_eq!(installed(&u), ("2026-46".to_owned(), base() + 10));
    }
}
