// SPDX-License-Identifier: MPL-2.0
//! A local stand-in for plex.tv and a Plex Media Server (NET-012): serves
//! the answers in `pa-plex/tests/fixtures/`, checks the tokens and the
//! `X-Plex-*` headers and honours byte ranges. Shared by the tests of
//! `pa-plex` and `pa-sources` (`#[path]`).
//!
//! NET-013: it also keeps the user's state per item (position, view
//! count, last view) the way a server does — changed by the timeline,
//! scrobble, unscrobble, progress and Continue Watching calls, or by a
//! test standing in for Plex Web ([`Mock::set_user_state`]) — and shows
//! it in the listings and `/library/metadata/{id}`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// The user's state of an item at the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UserState {
    /// Resume position, ms.
    pub view_offset: Option<i64>,
    /// Times watched.
    pub view_count: i64,
    /// Last view, Unix seconds.
    pub last_viewed_at: Option<i64>,
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
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
    /// The user's state per item, over the fixtures' (NET-013).
    pub state: Mutex<HashMap<String, UserState>>,
}

impl Mock {
    /// Sets the user's state of item `key` as Plex Web would.
    pub fn set_user_state(&self, key: &str, state: UserState) {
        self.state.lock().unwrap().insert(key.to_owned(), state);
    }

    /// The user's state of item `key` (the fixtures' until changed).
    pub fn user_state(&self, key: &str) -> UserState {
        if let Some(s) = self.state.lock().unwrap().get(key) {
            return *s;
        }
        for name in ["movies.json", "episodes.json"] {
            let v: serde_json::Value = serde_json::from_str(&fixture(name)).unwrap();
            let items = v
                .get("MediaContainer")
                .and_then(|c| c.get("Metadata"))
                .unwrap_or(&v)
                .as_array()
                .cloned()
                .unwrap_or_default();
            for i in items {
                if i["ratingKey"] == key {
                    return UserState {
                        view_offset: i["viewOffset"].as_i64(),
                        view_count: i["viewCount"].as_i64().unwrap_or(0),
                        last_viewed_at: i["lastViewedAt"].as_i64(),
                    };
                }
            }
        }
        UserState::default()
    }

    /// Puts the user's state into the items of a listing.
    fn patch(&self, items: &mut serde_json::Value) {
        let Some(list) = items.as_array_mut() else {
            return;
        };
        let state = self.state.lock().unwrap();
        for i in list {
            let Some(s) = i["ratingKey"].as_str().and_then(|k| state.get(k)) else {
                continue;
            };
            let o = i.as_object_mut().unwrap();
            o.remove("viewOffset");
            o.remove("viewCount");
            o.remove("lastViewedAt");
            if let Some(v) = s.view_offset {
                o.insert("viewOffset".into(), v.into());
            }
            if s.view_count > 0 {
                o.insert("viewCount".into(), s.view_count.into());
            }
            if let Some(v) = s.last_viewed_at {
                o.insert("lastViewedAt".into(), v.into());
            }
        }
    }

    /// The playback API (NET-013): changes the user's state like a server.
    fn playback(&self, r: &Request, path: &str) -> Option<Reply> {
        let q = |k: &str| r.query.get(k).cloned();
        let ident = q("identifier");
        let state = |key: &str, f: &dyn Fn(&mut UserState)| {
            let mut s = self.user_state(key);
            f(&mut s);
            self.set_user_state(key, s);
        };
        match (r.method.as_str(), path) {
            ("GET", "/:/timeline") => {
                if ident.as_deref() != Some("com.plexapp.plugins.library") {
                    return Some(status(400));
                }
                let key = q("ratingKey")?;
                let time: i64 = q("time")?.parse().ok()?;
                let duration: Option<i64> = q("duration").and_then(|d| d.parse().ok());
                let stopped = q("state").as_deref() == Some("stopped");
                state(&key, &|s| {
                    s.last_viewed_at = Some(now_s());
                    // Past 90 %: watched, like the server's default.
                    if stopped && duration.is_some_and(|d| d > 0 && time * 10 >= d * 9) {
                        s.view_count += 1;
                        s.view_offset = None;
                    } else {
                        s.view_offset = (time > 0).then_some(time);
                    }
                });
                Some(status(200))
            }
            ("GET", "/:/scrobble") => {
                let key = q("key")?;
                state(&key, &|s| {
                    s.view_count += 1;
                    s.view_offset = None;
                    s.last_viewed_at = Some(now_s());
                });
                Some(status(200))
            }
            ("GET", "/:/unscrobble") => {
                let key = q("key")?;
                state(&key, &|s| {
                    s.view_count = 0;
                    s.view_offset = None;
                });
                Some(status(200))
            }
            ("GET", "/:/progress") => {
                let key = q("key")?;
                let time: i64 = q("time")?.parse().ok()?;
                state(&key, &|s| s.view_offset = (time > 0).then_some(time));
                Some(status(200))
            }
            ("PUT", "/actions/removeFromContinueWatching") => {
                let key = q("ratingKey")?;
                state(&key, &|s| s.view_offset = None);
                Some(status(200))
            }
            ("GET", "/video/:/transcode/universal/stop") => Some(status(200)),
            _ => None,
        }
    }

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
                    Some("1234") => {
                        json(201, r#"{"uuid":"kid-uuid","authToken":"kid-token"}"#.into())
                    }
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
            (_, path)
                if path.starts_with("/:/")
                    || path.starts_with("/actions/")
                    || path.starts_with("/video/") =>
            {
                if !token.is_some_and(|t| SERVER_TOKENS.contains(&t)) {
                    return status(401);
                }
                self.playback(r, path).unwrap_or_else(|| status(400))
            }
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
                let mut page =
                    serde_json::Value::Array(all.iter().skip(start).take(1).cloned().collect());
                self.patch(&mut page);
                json(
                    200,
                    serde_json::json!({"MediaContainer": {
                        "size": page.as_array().map_or(0, Vec::len), "totalSize": all.len(), "offset": start,
                        "Metadata": page}})
                    .to_string(),
                )
            }
            "/library/sections/2/all" if q("type") == Some("2") => json(200, fixture("shows.json")),
            "/library/sections/2/all" if q("type") == Some("4") => {
                let mut v: serde_json::Value =
                    serde_json::from_str(&fixture("episodes.json")).unwrap();
                self.patch(&mut v["MediaContainer"]["Metadata"]);
                json(200, v.to_string())
            }
            "/library/sections/3/all" if q("type") == Some("10") => {
                json(200, fixture("tracks.json"))
            }
            "/playlists" if q("playlistType") == Some("audio") => {
                json(200, fixture("playlists.json"))
            }
            "/playlists/30/items" => json(200, fixture("tracks.json")),
            "/playlists" if q("playlistType") == Some("video") => {
                json(200, fixture("video_playlists.json"))
            }
            "/playlists/40/items" => json(200, fixture("video_playlist_items.json")),
            p if p.starts_with("/library/metadata/")
                && p[18..].chars().all(|c| c.is_ascii_digit()) =>
            {
                let key = &p[18..];
                let movies: serde_json::Value =
                    serde_json::from_str(&fixture("movies.json")).unwrap();
                let episodes: serde_json::Value =
                    serde_json::from_str(&fixture("episodes.json")).unwrap();
                let found = movies
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(
                        episodes["MediaContainer"]["Metadata"]
                            .as_array()
                            .into_iter()
                            .flatten(),
                    )
                    .find(|i| i["ratingKey"] == key)
                    .cloned();
                match found {
                    Some(item) => {
                        let mut list = serde_json::Value::Array(vec![item]);
                        self.patch(&mut list);
                        json(
                            200,
                            serde_json::json!({"MediaContainer": {"size": 1, "Metadata": list}})
                                .to_string(),
                        )
                    }
                    None => status(404),
                }
            }
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
        if let Some(n) = headers
            .get("content-length")
            .and_then(|n| n.parse::<usize>().ok())
        {
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
        state: Mutex::new(HashMap::new()),
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
