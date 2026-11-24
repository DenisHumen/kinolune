// SPDX-License-Identifier: MPL-2.0
//! Cloud sources (NET-017) in the source manager against mocks: a MEGA
//! account added with its 2FA code keeps the session, not the password; a
//! Dropbox sign-in through the app redirect keeps the refresh token and
//! the user's own client id; both connect and list.

#[path = "../../pa-cloud/tests/support/mod.rs"]
mod support;

use std::sync::Arc;

use pa_cloud::mega::crypto::{b64_encode, ecb_encrypt, v2_keys};
use pa_secrets::{MemoryStore, SecretKey, SecretStore, SecretString};
use pa_sources::{CloudEndpoints, CloudSecret, CloudSignIn, SourceKind, SourceManager};
use serde_json::{Value, json};
use support::{Req, Resp, serve};

const EMAIL: &str = "anna@example.org";
const PASSWORD: &str = "correct horse";
const MK: [u8; 16] = *b"master-key-16byt";

fn tsid() -> String {
    let r = *b"random-block-16b";
    let mut raw = r.to_vec();
    raw.extend(ecb_encrypt(&MK, &r).unwrap());
    b64_encode(&raw)
}

fn mega(r: &Req) -> Resp {
    let cmd = r.json()[0].clone();
    let answer = |v: Value| Resp::json(&json!([v]));
    match cmd["a"].as_str().unwrap_or("") {
        "us0" => answer(json!({"v": 2, "s": b64_encode(&[1; 32])})),
        "us" => {
            let (pw, uh) = v2_keys(PASSWORD, &[1; 32]);
            if cmd["uh"] != uh.as_str() {
                return Resp::json(&json!([-9]));
            }
            if cmd["mfa"] != "654321" {
                return Resp::json(&json!([-26]));
            }
            answer(
                json!({"k": b64_encode(&ecb_encrypt(&pw, &MK).unwrap()), "tsid": tsid(), "u": "USER0001"}),
            )
        }
        "f" if r.query.get("sid") == Some(&tsid()) => {
            answer(json!({"f": [{"h": "ROOT0001", "p": "", "t": 2}]}))
        }
        _ => Resp::json(&json!([-15])),
    }
}

fn dropbox(r: &Req) -> Resp {
    match r.path.as_str() {
        "/oauth/token" => {
            let f = r.form();
            assert_eq!(f["client_id"], "my-own-app");
            match f["grant_type"].as_str() {
                "authorization_code" => {
                    assert_eq!(f["code"], "the-code");
                    assert_eq!(f["redirect_uri"], "kinolune://oauth");
                    assert!(f["code_verifier"].len() >= 43);
                    Resp::json(
                        &json!({"access_token": "access", "refresh_token": "refresh-1", "expires_in": 14400}),
                    )
                }
                _ => Resp::json(&json!({"access_token": "access", "expires_in": 14400})),
            }
        }
        "/2/users/get_current_account" => Resp::json(&json!({"email": EMAIL})),
        "/2/files/list_folder" => Resp::json(&json!({
            "entries": [{".tag": "folder", "name": "Movies", "id": "id:m"}],
            "cursor": "", "has_more": false
        })),
        _ => Resp::status(404),
    }
}

fn manager(endpoints: CloudEndpoints) -> (SourceManager, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    let m = SourceManager::new(pa_library::open_in_memory().unwrap(), store.clone());
    m.set_cloud_endpoints(endpoints);
    (m, store)
}

/// The one secret in the store (each test adds one source).
fn stored_secret(store: &MemoryStore) -> String {
    let keys: Vec<SecretKey> = store.keys();
    assert_eq!(keys.len(), 1);
    store.get(&keys[0]).unwrap().unwrap().expose().to_owned()
}

#[test]
fn mega_account_keeps_the_session() {
    let server = serve(mega);
    let (m, store) = manager(CloudEndpoints {
        mega: server.base.clone(),
        ..CloudEndpoints::default()
    });
    let password = SecretString::new(PASSWORD);
    let err = m
        .create_mega(EMAIL, &password, None, None, None)
        .unwrap_err();
    assert!(
        err.to_string().contains(pa_cloud::mega::MFA_REQUIRED_TEXT),
        "{err}"
    );
    let s = m
        .create_mega(EMAIL, &password, Some("654321"), None, Some("Movies/4K"))
        .unwrap();
    assert_eq!(s.kind, SourceKind::Mega);
    assert_eq!(s.uri, "mega:/Movies/4K");
    assert_eq!(s.username.as_deref(), Some(EMAIL));
    assert_eq!(s.name, "MEGA · anna@example.org");
    let secret = stored_secret(&store);
    assert!(!secret.contains(PASSWORD));
    assert!(secret.contains(&tsid()));

    // The Cloud Drive is empty, so the 4K folder is missing.
    let err = m.list_folder(s.id, "").unwrap_err();
    assert_eq!(err.source_kind(), Some(pa_vfs::ErrorKind::NotFound));
}

#[test]
fn dropbox_sign_in_through_the_app_link() {
    let server = serve(dropbox);
    let (m, store) = manager(CloudEndpoints {
        dropbox_api: format!("{}/2", server.base),
        dropbox_content: format!("{}/content/2", server.base),
        oauth: Some(format!("{}/oauth", server.base)),
        ..CloudEndpoints::default()
    });
    let mut sign_in = CloudSignIn::start(
        pa_cloud::Cloud::Dropbox,
        Some("my-own-app"),
        None,
        true,
        m.cloud_endpoints(),
    )
    .unwrap();
    let url = sign_in.url();
    assert!(url.contains("redirect_uri=kinolune%3A%2F%2Foauth"));
    let state = url
        .split('&')
        .find_map(|p| p.strip_prefix("state="))
        .unwrap()
        .to_owned();
    assert!(
        sign_in
            .complete("kinolune://oauth?code=x&state=wrong")
            .is_err()
    );
    let account = sign_in
        .complete(&format!("kinolune://oauth?code=the-code&state={state}"))
        .unwrap();
    assert_eq!(account, EMAIL);
    let s = m.create_cloud(sign_in, None, None).unwrap();
    assert_eq!(s.kind, SourceKind::Dropbox);
    assert_eq!(s.uri, "dropbox:/");
    assert_eq!(s.options.cloud.as_ref().unwrap().client_id, "my-own-app");
    let secret = CloudSecret::parse(&stored_secret(&store));
    assert_eq!(secret.refresh_token, "refresh-1");

    let names: Vec<_> = m
        .list_folder(s.id, "")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["Movies"]);
}

#[test]
fn cloud_addresses() {
    for (kind, given, stored) in [
        (SourceKind::GoogleDrive, "gdrive:/", "gdrive:/"),
        (SourceKind::OneDrive, "Movies//4K/", "onedrive:/Movies/4K"),
        (SourceKind::Dropbox, "dropbox:/Фильмы", "dropbox:/Фильмы"),
        (
            SourceKind::Mega,
            "https://mega.nz/folder/AbCd1234#AAAAAAAAAAAAAAAAAAAAAA",
            "https://mega.nz/folder/AbCd1234",
        ),
        (
            SourceKind::Mega,
            "https://mega.co.nz/#F!AbCd1234!AAAAAAAAAAAAAAAAAAAAAA",
            "https://mega.nz/folder/AbCd1234",
        ),
    ] {
        assert_eq!(pa_sources::normalize_address(kind, given).unwrap(), stored);
    }
    assert!(pa_sources::normalize_address(SourceKind::Dropbox, "dropbox:/a/../b").is_err());
    assert!(pa_sources::normalize_address(SourceKind::GoogleDrive, "https://x@y/").is_err());
}
