// SPDX-License-Identifier: MPL-2.0
//! Mock cloud APIs for the contract tests: a tiny threaded HTTP/1.1
//! server on 127.0.0.1 and the `pa-vfs` contract fixture as a tree of
//! nodes with ids.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pa_vfs::contract::{FixtureEntry, fixture};

/// A request as the mock sees it.
#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    /// Path without the query.
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// `(offset, len)` of a `Range: bytes=a-b` header.
    pub fn range(&self) -> Option<(usize, usize)> {
        let r = self.header("range")?.strip_prefix("bytes=")?;
        let (a, b) = r.split_once('-')?;
        let a: usize = a.parse().ok()?;
        let b: usize = b.parse().ok()?;
        Some((a, b + 1 - a))
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    pub fn form(&self) -> HashMap<String, String> {
        parse_query(std::str::from_utf8(&self.body).unwrap_or(""))
    }
}

/// An answer.
#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(v: &serde_json::Value) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: v.to_string().into_bytes(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
        }
    }

    /// A byte-range answer of `data` for `req` (206 with `Content-Range`,
    /// or 200 with everything when there is no range).
    pub fn ranged(req: &Req, data: &[u8]) -> Self {
        match req.range() {
            Some((off, len)) if off < data.len() => {
                let end = (off + len).min(data.len());
                Self {
                    status: 206,
                    headers: vec![(
                        "content-range".into(),
                        format!("bytes {off}-{}/{}", end - 1, data.len()),
                    )],
                    body: data[off..end].to_vec(),
                }
            }
            Some(_) => Self::status(416),
            None => Self::bytes(200, data.to_vec()),
        }
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(h, 16).unwrap_or(0));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// A running mock; stops when dropped.
pub struct Server {
    pub base: String,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
    }
}

fn read_request(stream: &TcpStream) -> Option<Req> {
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    Some(Req {
        method,
        path: percent_decode(path),
        query: parse_query(query),
        headers,
        body,
    })
}

/// Serves `handler` on a free port.
pub fn serve(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let handler = Arc::new(handler);
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if flag.load(Ordering::Relaxed) {
                break;
            }
            let Ok(mut stream) = stream else { continue };
            let handler = Arc::clone(&handler);
            std::thread::spawn(move || {
                let Some(req) = read_request(&stream) else {
                    return;
                };
                let resp = handler(&req);
                let reason = match resp.status {
                    200 => "OK",
                    206 => "Partial Content",
                    _ => "X",
                };
                let mut head = format!(
                    "HTTP/1.1 {} {reason}\r\ncontent-length: {}\r\nconnection: close\r\n",
                    resp.status,
                    resp.body.len()
                );
                for (k, v) in &resp.headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&resp.body);
                let _ = stream.flush();
            });
        }
    });
    Server { base, stop }
}

/// One node of the fixture tree.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub parent: String,
    pub name: String,
    /// `None` for a folder.
    pub data: Option<Vec<u8>>,
}

/// The id of the folder the fixture lives in.
pub const BASE_ID: &str = "base0001";
/// Its name.
pub const BASE_NAME: &str = "Kinolune test";

/// The contract fixture under folder [`BASE_NAME`] in the drive root
/// `root_id`, plus a stray file at the root, with ids `n0001`, `n0002`, …
pub fn tree(root_id: &str) -> Vec<Item> {
    let mut items = vec![
        Item {
            id: BASE_ID.into(),
            parent: root_id.into(),
            name: BASE_NAME.into(),
            data: None,
        },
        Item {
            id: "stray001".into(),
            parent: root_id.into(),
            name: "readme.txt".into(),
            data: Some(b"hi".to_vec()),
        },
    ];
    let mut ids: HashMap<String, String> = HashMap::new();
    for (i, e) in fixture().into_iter().enumerate() {
        let (rel, data) = match e {
            FixtureEntry::Dir(rel) => (rel.to_owned(), None),
            FixtureEntry::File(rel, bytes) => (rel, Some(bytes)),
        };
        let (parent_rel, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        let parent = if parent_rel.is_empty() {
            BASE_ID.to_owned()
        } else {
            ids[parent_rel].clone()
        };
        let id = format!("n{:04}", i + 1);
        ids.insert(rel.clone(), id.clone());
        items.push(Item {
            id,
            parent,
            name: name.to_owned(),
            data,
        });
    }
    items
}

/// Modification time the mocks report (2024-05-01T10:20:30Z).
pub const MODIFIED: &str = "2024-05-01T10:20:30Z";
pub const MODIFIED_SECS: i64 = 1_714_558_830;
