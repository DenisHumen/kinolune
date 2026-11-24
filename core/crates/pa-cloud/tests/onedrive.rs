// SPDX-License-Identifier: MPL-2.0
//! OneDrive against a mock of Microsoft Graph and the identity platform:
//! the `pa-vfs` contract, `@odata.nextLink` paging, reads from
//! pre-authenticated download links (no token sent there, an expired link
//! asked for again) and a rotated refresh token handed to the sink.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pa_cloud::onedrive::OneDrive;
use pa_cloud::{CloudFs, Provider, TokenSink, TokenSource};
use pa_vfs::{ByteSource, Vfs, VfsPath};
use serde_json::json;
use support::{BASE_NAME, MODIFIED, Req, Resp, Server, serve, tree};

struct Mock {
    _server: Server,
    base: String,
    /// Download requests that carried an `Authorization` header.
    leaked: Arc<AtomicUsize>,
    /// Download links handed out per file.
    links: Arc<AtomicUsize>,
}

fn mock() -> Mock {
    let items = tree("root");
    let leaked = Arc::new(AtomicUsize::new(0));
    let links = Arc::new(AtomicUsize::new(0));
    let (lk, ln) = (Arc::clone(&leaked), Arc::clone(&links));
    let server = serve(move |r: &Req| handle(r, &items, &lk, &ln));
    let base = server.base.clone();
    Mock {
        _server: server,
        base,
        leaked,
        links,
    }
}

fn item_json(i: &support::Item, base: &str, link: usize) -> serde_json::Value {
    let mut v = json!({"id": i.id, "name": i.name, "lastModifiedDateTime": MODIFIED});
    match &i.data {
        Some(d) => {
            v["size"] = json!(d.len());
            v["file"] = json!({"mimeType": "application/octet-stream"});
            v["@microsoft.graph.downloadUrl"] = json!(format!("{base}/dl/{link}/{}", i.id));
        }
        None => {
            v["size"] = json!(0);
            v["folder"] = json!({"childCount": 1});
        }
    }
    v
}

fn handle(r: &Req, items: &[support::Item], leaked: &AtomicUsize, links: &AtomicUsize) -> Resp {
    let host = format!("http://{}", r.header("host").unwrap_or(""));
    if r.path == "/token" {
        let f = r.form();
        assert!(
            !f.contains_key("client_secret"),
            "a public client sends no secret"
        );
        // Microsoft rotates refresh tokens.
        return match f.get("refresh_token").map(String::as_str) {
            Some("refresh-0") => Resp::json(&json!({
                "access_token": "access", "refresh_token": "refresh-1", "expires_in": 3600
            })),
            _ => Resp {
                status: 400,
                ..Resp::json(&json!({"error": "invalid_grant"}))
            },
        };
    }
    if let Some(rest) = r.path.strip_prefix("/dl/") {
        if r.header("authorization").is_some() {
            leaked.fetch_add(1, Ordering::SeqCst);
        }
        let (link, id) = rest.split_once('/').unwrap();
        // Links of generation 0 have expired.
        if link == "0" {
            return Resp::status(401);
        }
        let d = items
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.data.as_ref());
        return d.map_or(Resp::status(404), |d| Resp::ranged(r, d));
    }
    if r.header("authorization") != Some("Bearer access") {
        return Resp::status(401);
    }
    if r.path == "/me/drive" {
        return Resp::json(
            &json!({"owner": {"user": {"displayName": "Anna", "email": "anna@example.org"}}}),
        );
    }
    let Some(rest) = r.path.strip_prefix("/me/drive/items/") else {
        return Resp::status(404);
    };
    if let Some(id) = rest.strip_suffix("/children") {
        let children: Vec<_> = items.iter().filter(|i| i.parent == id).collect();
        if !items.iter().any(|i| i.id == id) && id != "root" {
            return Resp::status(404);
        }
        let skip: usize = r.query.get("$skiptoken").map_or(0, |s| s.parse().unwrap());
        let page: Vec<_> = children
            .iter()
            .skip(skip)
            .take(2)
            .map(|i| item_json(i, &host, 1))
            .collect();
        let mut out = json!({"value": page});
        if skip + 2 < children.len() {
            out["@odata.nextLink"] = json!(format!(
                "{host}/me/drive/items/{id}/children?$top=1000&$skiptoken={}",
                skip + 2
            ));
        }
        return Resp::json(&out);
    }
    match items.iter().find(|i| i.id == rest) {
        Some(i) => {
            let generation = links.fetch_add(1, Ordering::SeqCst);
            Resp::json(&item_json(i, &host, generation))
        }
        None => Resp::status(404),
    }
}

fn drive(m: &Mock, sink: Option<TokenSink>) -> OneDrive {
    let p = Provider::microsoft()
        .with_client("test-client", None)
        .at(&m.base);
    OneDrive::with_api(Arc::new(TokenSource::new(p, "refresh-0", sink)), &m.base)
}

pa_vfs::vfs_contract_tests!(|| {
    let m = mock();
    // Generation 0 links expire: every first open asks again.
    m.links.store(1, Ordering::SeqCst);
    let fs = CloudFs::new(drive(&m, None));
    pa_vfs::contract::Harness::new(fs, VfsPath::parse(BASE_NAME).unwrap()).with_guard(m)
});

#[test]
fn rotated_token_expired_link_and_no_token_leak() {
    let m = mock();
    let rotated = Arc::new(Mutex::new(Vec::<String>::new()));
    let r2 = Arc::clone(&rotated);
    let sink: TokenSink = Arc::new(move |t: &str| r2.lock().unwrap().push(t.to_owned()));
    let d = drive(&m, Some(sink));
    assert_eq!(d.account().unwrap(), "anna@example.org");
    assert_eq!(*rotated.lock().unwrap(), ["refresh-1"]);

    let fs = CloudFs::new(d);
    let path = VfsPath::parse(&format!("{BASE_NAME}/{}", pa_vfs::contract::SAMPLE)).unwrap();
    // The first link handed out (generation 0) is expired.
    let src = fs.open(&path).unwrap();
    let mut buf = [0u8; 16];
    src.read_exact_at(1000, &mut buf).unwrap();
    assert_eq!(buf[..], pa_vfs::contract::sample_bytes()[1000..1016]);
    assert_eq!(m.links.load(Ordering::SeqCst), 2);
    assert_eq!(m.leaked.load(Ordering::SeqCst), 0);
}
