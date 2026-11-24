// SPDX-License-Identifier: MPL-2.0
//! MEGA against a mock of its JSON API that encrypts like MEGA does: node
//! keys under the master key (or a folder link's key), attributes in
//! AES-CBC, contents in AES-CTR. The `pa-vfs` contract on an account and
//! on a public folder link, unaligned range decryption, the sign-in (v2
//! key derivation, 2FA, temporary session check) and a public file link.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pa_cloud::CloudFs;
use pa_cloud::mega::crypto::{
    b64_encode, ctr_xor, ecb_encrypt, encrypt_attributes, file_key_parts, v2_keys,
};
use pa_cloud::mega::{MegaDrive, PublicLink, Session, login};
use pa_vfs::{ByteSource, ErrorKind, Vfs, VfsPath};
use serde_json::{Value, json};
use support::{BASE_ID, BASE_NAME, MODIFIED_SECS, Req, Resp, Server, serve, tree};

const USER: &str = "USER0001";
const ROOT: &str = "ROOT0001";
const EMAIL: &str = "anna@example.org";
const PASSWORD: &str = "correct horse";
const SALT: [u8; 32] = [7; 32];
const MK: [u8; 16] = *b"master-key-16byt";
const FOLDER_KEY: [u8; 16] = *b"folder-link-key!";
const MFA: &str = "123456";

/// Deterministic per-node key bytes.
fn key_of(id: &str, len: usize) -> Vec<u8> {
    let mut x: u32 = id.bytes().fold(2_166_136_261, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    });
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

struct Node {
    id: String,
    parent: String,
    kind: i64,
    /// Key encrypted for the account and for the folder link.
    k_account: String,
    k_link: String,
    attrs: String,
    size: usize,
    /// Encrypted contents.
    data: Option<Vec<u8>>,
}

fn nodes() -> Vec<Node> {
    let mut out = vec![Node {
        id: ROOT.into(),
        parent: String::new(),
        kind: 2,
        k_account: String::new(),
        k_link: String::new(),
        attrs: String::new(),
        size: 0,
        data: None,
    }];
    for item in tree(ROOT) {
        let is_file = item.data.is_some();
        let key = key_of(&item.id, if is_file { 32 } else { 16 });
        let aes: [u8; 16] = if is_file {
            file_key_parts(&key.clone().try_into().unwrap()).0
        } else {
            key.clone().try_into().unwrap()
        };
        let data = item.data.map(|plain| {
            let (k, nonce) = file_key_parts(&key.clone().try_into().unwrap());
            let mut d = plain;
            ctr_xor(&k, &nonce, 0, &mut d);
            d
        });
        out.push(Node {
            k_account: format!("{USER}:{}", b64_encode(&ecb_encrypt(&MK, &key).unwrap())),
            k_link: format!(
                "{BASE_ID}:{}",
                b64_encode(&ecb_encrypt(&FOLDER_KEY, &key).unwrap())
            ),
            attrs: encrypt_attributes(&aes, &json!({"n": item.name, "c": "ignored"})),
            size: data.as_ref().map_or(0, Vec::len),
            id: item.id,
            parent: item.parent,
            kind: if is_file { 0 } else { 1 },
            data,
        });
    }
    // The Rubbish Bin and a node someone else shared (no key for us).
    out.push(Node {
        id: "RUBBISH1".into(),
        parent: String::new(),
        kind: 4,
        k_account: String::new(),
        k_link: String::new(),
        attrs: String::new(),
        size: 0,
        data: None,
    });
    out.push(Node {
        id: "FOREIGN1".into(),
        parent: ROOT.into(),
        kind: 0,
        k_account: "OTHER001:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
        k_link: String::new(),
        attrs: "AAAA".into(),
        size: 1,
        data: Some(vec![0]),
    });
    out
}

/// The temporary session id: a block and its encryption with the master
/// key.
fn tsid() -> String {
    let r = *b"random-block-16b";
    let mut raw = r.to_vec();
    raw.extend(ecb_encrypt(&MK, &r).unwrap());
    b64_encode(&raw)
}

struct Mock {
    _server: Server,
    base: String,
    /// Download requests (offset, len).
    reads: Arc<Mutex<Vec<(usize, usize)>>>,
    logins: Arc<AtomicUsize>,
}

fn mock() -> Mock {
    let nodes = Arc::new(nodes());
    let reads = Arc::new(Mutex::new(Vec::new()));
    let logins = Arc::new(AtomicUsize::new(0));
    let (rd, lg) = (Arc::clone(&reads), Arc::clone(&logins));
    let server = serve(move |r: &Req| handle(r, &nodes, &rd, &lg));
    let base = server.base.clone();
    Mock {
        _server: server,
        base,
        reads,
        logins,
    }
}

fn node_json(n: &Node, link: bool) -> Value {
    let mut v = json!({"h": n.id, "p": n.parent, "t": n.kind, "ts": MODIFIED_SECS, "u": USER});
    if n.kind <= 1 {
        v["k"] = json!(if link { &n.k_link } else { &n.k_account });
        v["a"] = json!(n.attrs);
    }
    if n.kind == 0 {
        v["s"] = json!(n.size);
    }
    v
}

fn handle(
    r: &Req,
    nodes: &[Node],
    reads: &Mutex<Vec<(usize, usize)>>,
    logins: &AtomicUsize,
) -> Resp {
    let host = format!("http://{}", r.header("host").unwrap_or(""));
    if let Some(rest) = r.path.strip_prefix("/dl/") {
        let (id, range) = rest.split_once('/').unwrap();
        let (a, b) = range.split_once('-').unwrap();
        let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
        reads.lock().unwrap().push((a, b + 1 - a));
        let data = nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.data.as_ref())
            .unwrap();
        return Resp::bytes(200, data[a..=b.min(data.len() - 1)].to_vec());
    }
    assert_eq!(r.path, "/cs");
    assert!(
        r.query.contains_key("id"),
        "every request has a sequence id"
    );
    let cmd = r.json()[0].clone();
    let link = r.query.get("n").map(String::as_str) == Some("LINK0001");
    let signed_in = r.query.get("sid") == Some(&tsid());
    let answer = |v: Value| Resp::json(&json!([v]));
    let error = |code: i64| Resp::json(&json!([code]));
    match cmd["a"].as_str().unwrap() {
        "us0" => {
            assert_eq!(cmd["user"], EMAIL);
            answer(json!({"v": 2, "s": b64_encode(&SALT)}))
        }
        "us" => {
            logins.fetch_add(1, Ordering::SeqCst);
            let (pw, uh) = v2_keys(PASSWORD, &SALT);
            if cmd["user"] != EMAIL || cmd["uh"] != uh.as_str() {
                return error(-9);
            }
            match cmd.get("mfa").and_then(Value::as_str) {
                None => return error(-26),
                Some(MFA) => {}
                Some(_) => return error(-27),
            }
            answer(json!({
                "k": b64_encode(&ecb_encrypt(&pw, &MK).unwrap()),
                "tsid": tsid(),
                "u": USER,
            }))
        }
        "ug" if signed_in => answer(json!({"u": USER, "email": EMAIL})),
        "f" if link => {
            let list: Vec<_> = nodes
                .iter()
                .filter(|n| n.kind <= 1 && n.id != "FOREIGN1")
                .map(|n| {
                    let mut v = node_json(n, true);
                    if n.id == BASE_ID {
                        // The shared folder itself; its parent is not shared.
                        v["p"] = json!("NOTSHRD1");
                    }
                    v
                })
                .collect();
            answer(json!({"f": list}))
        }
        "f" if signed_in => {
            answer(json!({"f": nodes.iter().map(|n| node_json(n, false)).collect::<Vec<_>>()}))
        }
        "g" if signed_in || link || cmd.get("p").is_some() => {
            let id = cmd
                .get("n")
                .or_else(|| cmd.get("p"))
                .and_then(Value::as_str)
                .unwrap();
            let id = if id == "PUBFILE1" { "n0001" } else { id };
            let Some(n) = nodes.iter().find(|n| n.id == id && n.kind == 0) else {
                return error(-9);
            };
            let mut v = json!({"s": n.size, "at": n.attrs});
            if cmd.get("g").is_some() {
                v["g"] = json!(format!("{host}/dl/{}", n.id));
            }
            answer(v)
        }
        _ => error(-15),
    }
}

fn session() -> Session {
    Session {
        sid: tsid(),
        mk: b64_encode(&MK),
        user: USER.into(),
    }
}

pa_vfs::vfs_contract_tests!(|| {
    let m = mock();
    let fs = CloudFs::new(MegaDrive::account(&m.base, &session()).unwrap());
    pa_vfs::contract::Harness::new(fs, VfsPath::parse(BASE_NAME).unwrap()).with_guard(m)
});

mod folder_link {
    use super::*;

    pa_vfs::vfs_contract_tests!(|| {
        let m = mock();
        let link = PublicLink::parse(
            &format!(
                "https://mega.nz/folder/LINK0001#{}",
                b64_encode(&FOLDER_KEY)
            ),
            None,
        )
        .unwrap();
        let fs = CloudFs::new(MegaDrive::public(&m.base, &link).unwrap());
        pa_vfs::contract::Harness::new(fs, VfsPath::root()).with_guard(m)
    });
}

#[test]
fn unaligned_ranges_decrypt() {
    let m = mock();
    let fs = CloudFs::new(MegaDrive::account(&m.base, &session()).unwrap());
    let path = VfsPath::parse(&format!("{BASE_NAME}/{}", pa_vfs::contract::SAMPLE)).unwrap();
    let src = fs.open(&path).unwrap();
    src.hint_random_access();
    let want = pa_vfs::contract::sample_bytes();
    for (off, len) in [
        (0usize, 1usize),
        (1, 15),
        (15, 2),
        (16, 16),
        (17, 100),
        (65_537, 4097),
        (300_000, 1),
    ] {
        let mut buf = vec![0u8; len];
        src.read_exact_at(off as u64, &mut buf).unwrap();
        assert_eq!(buf, want[off..off + len], "offset {off}");
    }
    let reads = m.reads.lock().unwrap();
    assert!(reads.contains(&(17, 100)), "{reads:?}");
}

#[test]
fn others_files_and_rubbish_are_left_out() {
    let m = mock();
    let fs = CloudFs::new(MegaDrive::account(&m.base, &session()).unwrap());
    let mut names: Vec<_> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    names.sort();
    assert_eq!(names, [BASE_NAME, "readme.txt"]);
    let st = fs.stat(&VfsPath::parse("readme.txt").unwrap()).unwrap();
    assert_eq!(st.modified_ms, Some(MODIFIED_SECS * 1000));
}

#[test]
fn sign_in_with_two_factor_code() {
    let m = mock();
    let err = login(&m.base, EMAIL, PASSWORD, None).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AuthenticationFailed);
    assert!(err.message().contains(pa_cloud::mega::MFA_REQUIRED_TEXT));
    let err = login(&m.base, EMAIL, PASSWORD, Some("000000")).unwrap_err();
    assert!(err.message().contains("two-factor"));
    let err = login(&m.base, EMAIL, "wrong", Some(MFA)).unwrap_err();
    assert!(err.message().contains("wrong e-mail or password"));
    assert!(!err.message().contains("wrong\""), "no password in errors");

    let s = login(&m.base, "  Anna@Example.org ", PASSWORD, Some(MFA)).unwrap();
    assert_eq!(s, session());
    assert_eq!(m.logins.load(Ordering::SeqCst), 4);
    // The stored form round-trips and has no password.
    let secret = s.to_secret();
    assert!(!secret.contains(PASSWORD));
    assert_eq!(Session::from_secret(&secret).unwrap(), s);
    assert!(Session::from_secret("hunter2").is_err());
    let d = MegaDrive::account(&m.base, &s).unwrap();
    assert_eq!(d.account_email().unwrap(), EMAIL);
}

#[test]
fn public_file_link() {
    let m = mock();
    let key = key_of("n0001", 32);
    let link = PublicLink::parse(
        &format!("https://mega.nz/file/PUBFILE1#{}", b64_encode(&key)),
        None,
    )
    .unwrap();
    let fs = CloudFs::new(MegaDrive::public(&m.base, &link).unwrap());
    let entries = fs.list(&VfsPath::root()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, pa_vfs::contract::SAMPLE);
    let src = fs.open(&entries[0].path).unwrap();
    let mut buf = [0u8; 32];
    src.read_exact_at(123, &mut buf).unwrap();
    assert_eq!(buf[..], pa_vfs::contract::sample_bytes()[123..155]);
}

#[test]
fn ended_session_asks_to_sign_in_again() {
    let m = mock();
    let mut s = session();
    s.sid = "expired".into();
    let err = CloudFs::new(MegaDrive::account(&m.base, &s).unwrap())
        .list(&VfsPath::root())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AuthenticationFailed);
}
