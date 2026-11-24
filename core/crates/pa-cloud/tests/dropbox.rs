// SPDX-License-Identifier: MPL-2.0
//! Dropbox against a mock of API v2: the `pa-vfs` contract,
//! `list_folder/continue` paging, downloads by id with `Range` and an
//! ASCII-only `Dropbox-API-Arg`, `path/not_found` as `NotFound`.

mod support;

use std::sync::Arc;

use pa_cloud::dropbox::Dropbox;
use pa_cloud::{CloudFs, Provider, TokenSource};
use pa_vfs::{ErrorKind, Vfs, VfsPath};
use serde_json::json;
use support::{BASE_NAME, MODIFIED, Req, Resp, Server, serve, tree};

struct Mock {
    _server: Server,
    base: String,
}

fn mock() -> Mock {
    // Dropbox ids look like `id:…`; the root is "".
    let items: Vec<_> = tree("")
        .into_iter()
        .map(|mut i| {
            i.id = format!("id:{}", i.id);
            if !i.parent.is_empty() {
                i.parent = format!("id:{}", i.parent);
            }
            i
        })
        .collect();
    let server = serve(move |r: &Req| handle(r, &items));
    let base = server.base.clone();
    Mock {
        _server: server,
        base,
    }
}

fn not_found() -> Resp {
    Resp {
        status: 409,
        ..Resp::json(&json!({"error_summary": "path/not_found/..", "error": {".tag": "path"}}))
    }
}

fn entries(items: &[support::Item], parent: &str, from: usize) -> serde_json::Value {
    let children: Vec<_> = items.iter().filter(|i| i.parent == parent).collect();
    let page: Vec<_> = children
        .iter()
        .skip(from)
        .take(2)
        .map(|i| match &i.data {
            Some(d) => json!({".tag": "file", "name": i.name, "id": i.id, "size": d.len(), "server_modified": MODIFIED}),
            None => json!({".tag": "folder", "name": i.name, "id": i.id}),
        })
        .collect();
    json!({
        "entries": page,
        "cursor": format!("{parent}|{}", from + 2),
        "has_more": from + 2 < children.len(),
    })
}

fn handle(r: &Req, items: &[support::Item]) -> Resp {
    if r.path == "/token" {
        return Resp::json(&json!({"access_token": "access", "expires_in": 14400}));
    }
    if r.header("authorization") != Some("Bearer access") {
        return Resp::status(401);
    }
    assert_eq!(r.method, "POST");
    match r.path.as_str() {
        "/2/users/get_current_account" => Resp::json(&json!({"email": "anna@example.org"})),
        "/2/files/list_folder" => {
            let path = r.json()["path"].as_str().unwrap().to_owned();
            if !path.is_empty() && !items.iter().any(|i| i.id == path && i.data.is_none()) {
                return not_found();
            }
            Resp::json(&entries(items, &path, 0))
        }
        "/2/files/list_folder/continue" => {
            let cursor = r.json()["cursor"].as_str().unwrap().to_owned();
            let (parent, from) = cursor.rsplit_once('|').unwrap();
            Resp::json(&entries(items, parent, from.parse().unwrap()))
        }
        "/content/2/files/download" => {
            let arg = r.header("dropbox-api-arg").unwrap();
            assert!(arg.is_ascii());
            let path: serde_json::Value = serde_json::from_str(arg).unwrap();
            let id = path["path"].as_str().unwrap();
            match items
                .iter()
                .find(|i| i.id == id)
                .and_then(|i| i.data.as_ref())
            {
                Some(d) => Resp::ranged(r, d),
                None => not_found(),
            }
        }
        _ => Resp::status(404),
    }
}

fn drive(m: &Mock) -> Dropbox {
    let p = Provider::dropbox()
        .with_client("test-client", None)
        .at(&m.base);
    Dropbox::with_api(
        Arc::new(TokenSource::new(p, "refresh-0", None)),
        &format!("{}/2", m.base),
        &format!("{}/content/2", m.base),
    )
}

pa_vfs::vfs_contract_tests!(|| {
    let m = mock();
    let fs = CloudFs::new(drive(&m));
    pa_vfs::contract::Harness::new(fs, VfsPath::parse(BASE_NAME).unwrap()).with_guard(m)
});

#[test]
fn account_and_missing_file() {
    let m = mock();
    let d = drive(&m);
    assert_eq!(d.account().unwrap(), "anna@example.org");
    let fs = CloudFs::new(d);
    assert_eq!(
        fs.stat(&VfsPath::parse("nope/x.mkv").unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    // The root: the test folder and a stray file.
    let names: Vec<_> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}
