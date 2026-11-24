// SPDX-License-Identifier: MPL-2.0
//! Google Drive against a mock of the Drive API v3 and Google's token
//! endpoint: the `pa-vfs` contract, paging, Google Docs left out of reads,
//! and a refused access token refreshed once.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pa_cloud::gdrive::GoogleDrive;
use pa_cloud::{CloudFs, Provider, TokenSource};
use pa_vfs::{EntryKind, ErrorKind, Vfs, VfsPath};
use serde_json::json;
use support::{BASE_NAME, MODIFIED, Req, Resp, Server, serve, tree};

const PAGE: usize = 3;

struct Mock {
    _server: Server,
    base: String,
    /// Access tokens the token endpoint gave out.
    issued: Arc<AtomicUsize>,
    /// `pageToken`s seen.
    pages: Arc<Mutex<Vec<String>>>,
}

fn mock() -> Mock {
    let mut items = tree("root");
    // A Google Docs document: no bytes to read.
    items.push(support::Item {
        id: "doc00001".into(),
        parent: support::BASE_ID.into(),
        name: "Notes".into(),
        data: None,
    });
    let issued = Arc::new(AtomicUsize::new(0));
    let pages = Arc::new(Mutex::new(Vec::new()));
    let (iss, pg) = (Arc::clone(&issued), Arc::clone(&pages));
    let server = serve(move |r: &Req| handle(r, &items, &iss, &pg));
    let base = server.base.clone();
    Mock {
        _server: server,
        base,
        issued,
        pages,
    }
}

fn handle(
    r: &Req,
    items: &[support::Item],
    issued: &AtomicUsize,
    pages: &Mutex<Vec<String>>,
) -> Resp {
    if r.path == "/token" {
        let f = r.form();
        assert_eq!(
            f.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(f.get("client_id").map(String::as_str), Some("test-client"));
        if f.get("refresh_token").map(String::as_str) != Some("refresh-0") {
            return Resp {
                status: 400,
                ..Resp::json(&json!({"error": "invalid_grant"}))
            };
        }
        let n = issued.fetch_add(1, Ordering::SeqCst) + 1;
        return Resp::json(&json!({"access_token": format!("access-{n}"), "expires_in": 3600}));
    }
    // The first token is refused, as if it had expired early.
    if r.header("authorization") == Some("Bearer access-1") || r.header("authorization").is_none() {
        return Resp::status(401);
    }
    if r.path == "/about" {
        return Resp::json(
            &json!({"user": {"emailAddress": "anna@example.org", "displayName": "Anna"}}),
        );
    }
    if r.path == "/files" {
        let q = &r.query["q"];
        let parent = q.split('\'').nth(1).unwrap();
        assert!(q.contains("trashed = false"));
        let children: Vec<_> = items.iter().filter(|i| i.parent == parent).collect();
        let start: usize = r.query.get("pageToken").map_or(0, |p| {
            pages.lock().unwrap().push(p.clone());
            p.parse().unwrap()
        });
        let page: Vec<_> = children
            .iter()
            .skip(start)
            .take(PAGE)
            .map(|i| {
                let mime = match (&i.data, i.id.as_str()) {
                    (_, "doc00001") => "application/vnd.google-apps.document",
                    (None, _) => "application/vnd.google-apps.folder",
                    (Some(_), _) => "application/octet-stream",
                };
                let mut f =
                    json!({"id": i.id, "name": i.name, "mimeType": mime, "modifiedTime": MODIFIED});
                if let Some(d) = &i.data {
                    f["size"] = json!(d.len().to_string());
                }
                f
            })
            .collect();
        let mut out = json!({"files": page});
        if start + PAGE < children.len() {
            out["nextPageToken"] = json!((start + PAGE).to_string());
        }
        return Resp::json(&out);
    }
    if let Some(id) = r.path.strip_prefix("/files/") {
        assert_eq!(r.query.get("alt").map(String::as_str), Some("media"));
        return match items
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.data.as_ref())
        {
            Some(d) => Resp::ranged(r, d),
            None => Resp::status(404),
        };
    }
    Resp::status(404)
}

fn drive(m: &Mock) -> GoogleDrive {
    let p = Provider::google()
        .with_client("test-client", None)
        .at(&m.base);
    GoogleDrive::with_api(Arc::new(TokenSource::new(p, "refresh-0", None)), &m.base)
}

pa_vfs::vfs_contract_tests!(|| {
    let m = mock();
    let fs = CloudFs::new(drive(&m));
    pa_vfs::contract::Harness::new(fs, VfsPath::parse(BASE_NAME).unwrap()).with_guard(m)
});

#[test]
fn pages_docs_and_token_refresh() {
    let m = mock();
    let d = drive(&m);
    assert_eq!(d.account().unwrap(), "anna@example.org");
    // access-1 was refused once, access-2 is used from then on.
    assert_eq!(m.issued.load(Ordering::SeqCst), 2);

    let fs = CloudFs::new(d);
    let base = VfsPath::parse(BASE_NAME).unwrap();
    let entries = fs.list(&base).unwrap();
    assert!(!m.pages.lock().unwrap().is_empty(), "listing was not paged");
    let notes = entries.iter().find(|e| e.name == "Notes").unwrap();
    assert_eq!(notes.metadata.kind, EntryKind::Other);
    assert_eq!(
        fs.open(&notes.path).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(m.issued.load(Ordering::SeqCst), 2);

    // A source rooted at the test folder.
    let rooted = CloudFs::with_root(drive(&m), base);
    assert!(
        rooted
            .stat(&VfsPath::parse("dir/nested.txt").unwrap())
            .unwrap()
            .is_file()
    );
}

#[test]
fn revoked_refresh_token_asks_to_sign_in_again() {
    let m = mock();
    let p = Provider::google()
        .with_client("test-client", None)
        .at(&m.base);
    let d = GoogleDrive::with_api(Arc::new(TokenSource::new(p, "revoked", None)), &m.base);
    let err = CloudFs::new(d).list(&VfsPath::root()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AuthenticationFailed);
    assert!(!err.message().contains("revoked"));
}
