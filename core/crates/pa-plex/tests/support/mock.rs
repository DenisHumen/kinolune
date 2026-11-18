// SPDX-License-Identifier: MPL-2.0
//! A local stand-in for plex.tv and a Plex Media Server (NET-012): serves
//! the answers in `pa-plex/tests/fixtures/`, checks the tokens and the
//! `X-Plex-*` headers and honours byte ranges. Shared by the tests of
//! `pa-plex` and `pa-sources` (`#[path]`).

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn fixture(name: &str) -> String {
    let path = fixtures().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `pa-plex/tests/fixtures`, from this file.
fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("pa-plex")
        .join("tests")
        .join("fixtures")
}

/// Size of the part file at `path` (`/library/parts/<id>/…`), as the
/// fixtures give it.
pub fn part_size(path: &str) -> Option<u64> {
    let id = path.strip_prefix("/library/parts/")?.split('/').next()?;
    Some(match id {
        "601" => 300_000,
        "602" => 1000,
        "801" | "802" => 5000,
        "911" | "912" => 4000,
        _ => return None,
    })
}

/// Where the bytes of part `path` start in the [`byte`] sequence: 0 for
/// the movie of the tests (`601`), apart for the others, so that no two
/// files have the same content (the library keys files by a hash).
fn seed(path: &str) -> u64 {
    let id: u64 = path
        .strip_prefix("/library/parts/")
        .and_then(|p| p.split('/').next())
        .and_then(|id| id.parse().ok())
        .unwrap_or(0);
    if id == 601 { 0 } else { id * 1_000_003 }
}

/// Byte `i` of a part file.
pub fn byte(i: u64) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 8)) as u8
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn json(status: u16, body: String) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: body.into_bytes(),
    }
}

fn status(code: u16) -> Reply {
    json(code, String::new())
}

/// Server tokens plex.tv hands out: the account's and the kid's.
const SERVER_TOKENS: [&str; 2] = ["server-token", "kid-server-token"];

pub struct Mock {
    port: u16,
    pin_checks: AtomicUsize,
    /// `X-Plex-Client-Identifier` of every request.
    pub clients: Mutex<Vec<String>>,
    /// Paths and queries seen (for the paging check).
    pub seen: Mutex<Vec<String>>,
}

impl Mock {
    fn route(&self, r: &Request) -> Reply {
        let token = r.headers.get("x-plex-token").map(String::as_str);
        let resources = |server_token: &str| {
            json(
                200,
                fixture("resources.json")
                    .replace("{PORT}", &self.port.to_string())
                    .replace("{SERVER_TOKEN}", server_token),
            )
        };
        match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/api/v2/pins") => {
                if r.query.get("strong").map(String::as_str) != Some("false") {
                    return status(400);
                }
                json(201, fixture("pin_new.json"))
            }
            ("GET", "/api/v2/pins/4242") => {
                // Linked on the second check.
                if self.pin_checks.fetch_add(1, Ordering::SeqCst) == 0 {
                    json(200, fixture("pin_new.json"))
                } else {
                    json(200, fixture("pin_linked.json"))
                }
            }
            ("GET", "/api/v2/pins/1") => status(404),
            ("GET", "/api/v2/user") => match token {
                Some("account-token") => json(200, fixture("user.json")),
                _ => status(401),
            },
            ("GET", "/api/v2/home/users") => match token {
                Some("account-token") => json(200, fixture("home_users.json")),
                _ => status(401),
            },
            ("POST", "/api/v2/home/users/kid-uuid/switch") => {
                if token != Some("account-token") {
                    return status(401);
                }
                match r.query.get("pin").map(String::as_str) {
                    Some("1234") => json(201, r#"{"uuid":"kid-uuid","authToken":"kid-token"}"#.into()),
                    _ => status(403),
                }
            }
            ("GET", "/api/v2/resources") => {
                if r.query.get("includeRelay").map(String::as_str) != Some("1") {
                    return status(400);
                }
                match token {
                    Some("account-token") => resources("server-token"),
                    Some("kid-token") => resources("kid-server-token"),
                    _ => status(401),
                }
            }
            ("GET", "/identity") => json(200, fixture("identity.json")),
            ("GET", path) => {
                if !token.is_some_and(|t| SERVER_TOKENS.contains(&t)) {
                    return status(401);
                }
                self.library(r, path, token == Some("kid-server-token"))
            }
            _ => status(405),
        }
    }

    fn library(&self, r: &Request, path: &str, kid: bool) -> Reply {
        let q = |k: &str| r.query.get(k).map(String::as_str);
        match path {
            "/library/sections" => {
                let mut v: serde_json::Value =
                    serde_json::from_str(&fixture("sections.json")).unwrap();
                // The kid's server token sees only TV shows.
                if kid {
                    let dirs = v["MediaContainer"]["Directory"].as_array_mut().unwrap();
                    dirs.retain(|d| d["type"] == "show");
                }
                json(200, v.to_string())
            }
            "/library/sections/1/all" if q("type") == Some("1") => {
                assert_eq!(q("includeGuids"), Some("1"));
                // One movie per page.
                let all: Vec<serde_json::Value> =
                    serde_json::from_str(&fixture("movies.json")).unwrap();
                let start: usize = q("X-Plex-Container-Start").unwrap().parse().unwrap();
                let page: Vec<_> = all.iter().skip(start).take(1).cloned().collect();
                json(
                    200,
                    serde_json::json!({"MediaContainer": {
                        "size": page.len(), "totalSize": all.len(), "offset": start,
                        "Metadata": page}})
                    .to_string(),
                )
            }
            "/library/sections/2/all" if q("type") == Some("2") => {
                json(200, fixture("shows.json"))
            }
            "/library/sections/2/all" if q("type") == Some("4") => {
                json(200, fixture("episodes.json"))
            }
            "/library/sections/3/all" if q("type") == Some("10") => {
                json(200, fixture("tracks.json"))
            }
            "/playlists" if q("playlistType") == Some("audio") => {
                json(200, fixture("playlists.json"))
            }
            "/playlists/30/items" => json(200, fixture("tracks.json")),
            p if p.starts_with("/library/metadata/")
                && (p.contains("/thumb/") || p.contains("/art/")) =>
            {
                Reply {
                status: 200,
                headers: vec![("Content-Type".into(), "image/jpeg".into())],
                body: b"\xff\xd8\xff\xe0poster".to_vec(),
            }
            }
            p if p.starts_with("/library/parts/") => {
                let Some(size) = part_size(p) else {
                    return status(404);
                };
                let (start, end) = r
                    .headers
                    .get("range")
                    .and_then(|v| v.strip_prefix("bytes="))
                    .and_then(|v| v.split_once('-'))
                    .map_or((0, size - 1), |(a, b)| {
                        (a.parse().unwrap(), b.parse::<u64>().unwrap().min(size - 1))
                    });
                Reply {
                    status: 206,
                    headers: vec![(
                        "Content-Range".into(),
                        format!("bytes {start}-{end}/{size}"),
                    )],
                    body: (start..=end).map(|i| byte(i + seed(p))).collect(),
                }
            }
            _ => status(404),
        }
    }

    fn handle(&self, stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_owned();
        let target = parts.next().unwrap_or("").to_owned();
        let mut headers = HashMap::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
            }
        }
        if let Some(n) = headers.get("content-length").and_then(|n| n.parse::<usize>().ok()) {
            let mut body = vec![0; n];
            let _ = reader.read_exact(&mut body);
        }
        let (path, query) = target.split_once('?').unwrap_or((&target, ""));
        let query = query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        self.clients.lock().unwrap().push(
            headers
                .get("x-plex-client-identifier")
                .cloned()
                .unwrap_or_default(),
        );
        self.seen.lock().unwrap().push(target.clone());
        let reply = self.route(&Request {
            method,
            path: path.to_owned(),
            query,
            headers,
        });
        let mut out = stream;
        let mut head = format!(
            "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
            reply.status,
            reply.body.len()
        );
        for (k, v) in &reply.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let _ = out.write_all(head.as_bytes());
        let _ = out.write_all(&reply.body);
    }
}

/// Starts the stand-in; returns it and its base URL.
pub fn start() -> (Arc<Mock>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Arc::new(Mock {
        port,
        pin_checks: AtomicUsize::new(0),
        clients: Mutex::new(Vec::new()),
        seen: Mutex::new(Vec::new()),
    });
    let m = Arc::clone(&mock);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let m = Arc::clone(&m);
            std::thread::spawn(move || m.handle(stream));
        }
    });
    (mock, format!("http://127.0.0.1:{port}"))
}

