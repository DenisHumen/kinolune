// SPDX-License-Identifier: MPL-2.0
//! BUG-010: the anime-offline-database update goes through the real
//! `UreqTransport` and installs the zstd release. A local HTTP server plays
//! GitHub: the API answer (JSON), a redirect of the release asset to a
//! "CDN" path, and the binary `.zst` asset.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;

use pa_catalog::pa_anime_mappings::offline_db;
use pa_catalog::pa_anime_mappings::release::{ASSET, DOWNLOAD_PREFIX, LATEST_URL};
use pa_catalog::pa_anime_mappings::update::{Outcome, sha256_hex};
use pa_catalog::pa_metadata::http::{
    BytesResponse, HttpRequest, HttpResponse, HttpTransport, TransportError, UreqTransport,
};
use pa_catalog::{AnimeDb, AnimeDbOrigin};

const TAG: &str = "2026-47";
const ENTRIES: usize = 40;

/// An upstream JSONL release with [`ENTRIES`] entries, zstd-compressed.
fn release_asset() -> Vec<u8> {
    let mut s = String::from(
        r#"{"license":{"name":"ODbL"},"repository":"https://github.com/cedya77/anime-offline-database","lastUpdate":"2026-11-22"}"#,
    );
    s.push('\n');
    for i in 0..ENTRIES {
        s.push_str(&format!(
            "{{\"sources\":[\"https://anidb.net/anime/{}\"],\"title\":\"Тест {i} テスト\",\"type\":\"TV\",\"episodes\":12}}\n",
            70_000 + i
        ));
    }
    offline_db::compress(s.as_bytes())
}

/// One canned answer per path.
struct Route {
    path: String,
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// A minimal HTTP/1.1 server on 127.0.0.1, one request per connection.
/// Returns its base URL (`http://127.0.0.1:port`).
fn serve(routes: Vec<Route>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes = Arc::new(routes);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                // Skip the request headers.
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let (status, headers, body) = match routes.iter().find(|r| r.path == path) {
                    Some(r) => (r.status, r.headers.clone(), r.body.clone()),
                    None => (404, Vec::new(), b"not found".to_vec()),
                };
                let mut head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            });
        }
    });
    base
}

/// The live `UreqTransport`, with GitHub's hosts pointed at the local
/// server. Everything else (HTTP, redirects, reading the body) is the
/// real transport.
struct LocalGithub {
    inner: UreqTransport,
    base: String,
}

impl LocalGithub {
    fn local(&self, request: &HttpRequest) -> HttpRequest {
        let mut r = request.clone();
        for (from, to) in [
            ("https://api.github.com", "/api"),
            ("https://github.com", "/gh"),
        ] {
            if let Some(rest) = r.base.strip_prefix(from) {
                r.base = format!("{}{to}{rest}", self.base);
                break;
            }
        }
        r
    }
}

impl HttpTransport for LocalGithub {
    fn get(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.inner.get(&self.local(request))
    }

    fn get_bytes(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
        self.inner.get_bytes(&self.local(request))
    }
}

fn path_of(url: &str, host: &str, prefix: &str) -> String {
    format!("{prefix}{}", url.strip_prefix(host).unwrap())
}

fn github(asset: &[u8]) -> LocalGithub {
    let download = format!("{DOWNLOAD_PREFIX}{TAG}/{ASSET}");
    let latest = format!(
        r#"{{"tag_name":"{TAG}","draft":false,"prerelease":false,"assets":[{{"name":"{ASSET}","browser_download_url":"{download}","size":{},"digest":"sha256:{}"}}]}}"#,
        asset.len(),
        sha256_hex(asset)
    );
    let base = serve(vec![
        Route {
            path: path_of(LATEST_URL, "https://api.github.com", "/api"),
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: latest.into_bytes(),
        },
        // GitHub answers a release asset with a redirect to its CDN.
        Route {
            path: path_of(&download, "https://github.com", "/gh"),
            status: 302,
            headers: vec![("location".into(), "/cdn/release-asset".into())],
            body: Vec::new(),
        },
        Route {
            path: "/cdn/release-asset".into(),
            status: 200,
            headers: vec![("content-type".into(), "application/octet-stream".into())],
            body: asset.to_vec(),
        },
    ]);
    LocalGithub {
        inner: UreqTransport::new("kinolune-test"),
        base,
    }
}

#[test]
fn check_now_downloads_and_installs_the_zstd_release_through_ureq() {
    let asset = release_asset();
    // The point of the bug: the asset is not UTF-8.
    assert!(std::str::from_utf8(&asset).is_err());
    let transport = github(&asset);
    let dir = tempfile::tempdir().unwrap();
    // No bundled file: a development build, or a device before the first
    // update.
    let db = AnimeDb::new(dir.path(), None).with_min_entries(ENTRIES);
    assert_eq!(db.info(), None);

    let outcome = db.refresh(&transport, 1_000, true).unwrap();
    assert_eq!(outcome, Outcome::Updated { entries: ENTRIES });

    let info = db.info().unwrap();
    assert_eq!(info.origin, AnimeDbOrigin::Downloaded);
    assert_eq!(info.tag, TAG);
    assert_eq!(info.last_update, "2026-11-22");
    assert_eq!(info.entries, Some(ENTRIES));
    assert_eq!(info.checked_at, Some(1_000));
    let loaded = db.load().unwrap().unwrap();
    assert_eq!(loaded.entries.len(), ENTRIES);

    // Asked again: the same release is not downloaded twice.
    assert_eq!(
        db.refresh(&transport, 2_000, true).unwrap(),
        Outcome::NotModified
    );
}

#[test]
fn text_get_of_the_release_fails_and_bytes_get_returns_it_whole() {
    let asset = release_asset();
    let transport = github(&asset);
    let request = HttpRequest {
        base: format!("{DOWNLOAD_PREFIX}{TAG}/{ASSET}"),
        path: String::new(),
        query: Vec::new(),
        headers: Vec::new(),
    };
    // The text path cannot carry zstd (what the update used before
    // BUG-010)...
    assert!(transport.get(&request).is_err());
    // ...the bytes path follows the redirect and keeps every byte.
    let r = transport.get_bytes(&request).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.header("Content-Type"), Some("application/octet-stream"));
    assert_eq!(r.body, asset);
}
