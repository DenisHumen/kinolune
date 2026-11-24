// SPDX-License-Identifier: MPL-2.0
//! MEGA (ADR-0144): an own client of the documented JSON API instead of
//! the native SDK (BSD-2-Clause, C++ with its own dependencies): the few
//! commands a read-only player needs, in Rust.
//!
//! - [`login`] — e-mail, password, optional 2FA code → [`Session`] (the
//!   password is not kept);
//! - [`MegaDrive::account`] — the account's Cloud Drive;
//! - [`MegaDrive::public`] — a public folder or file link
//!   ([`PublicLink`]);
//! - files are fetched by byte range from the download link MEGA gives
//!   and decrypted with AES-128-CTR at their offset ([`crypto::ctr_xor`]),
//!   so seeking costs one request. The file MAC is not checked: a player
//!   reads ranges, never the whole file.
//!
//! Incoming shares from other users (keys under RSA share keys) are not
//! shown yet: only the user's own Cloud Drive.

mod api;
pub mod crypto;
mod link;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, EntryKind, ErrorKind, VfsError};
use serde_json::{Value, json};

pub use api::{API, ApiError, Client, MFA_REQUIRED_TEXT, Session, login};
use crypto::{b64_decode, decrypt_attributes, ecb_decrypt, file_key_parts, node_aes_key};
pub use link::{LinkKind, PublicLink};

use crate::drive::{Drive, Node};
use crate::http::{MAX_BLOB, Request, status_error};
use crate::range::{RangeFetch, RangeFile};

/// How long the node tree is reused before it is fetched again.
const TREE_TTL: Duration = Duration::from_secs(120);

/// AES key and CTR nonce of a file.
#[derive(Clone, Copy)]
struct FileKey {
    key: [u8; 16],
    nonce: [u8; 8],
}

#[derive(Default)]
struct Tree {
    root: Option<Node>,
    nodes: HashMap<String, Node>,
    children: HashMap<String, Vec<String>>,
    keys: HashMap<String, FileKey>,
}

enum Mode {
    Account { user: String, mk: [u8; 16] },
    Folder { key: [u8; 16] },
    File { id: String, key: [u8; 32] },
}

/// MEGA as a [`Drive`]: an account's Cloud Drive or a public link.
pub struct MegaDrive {
    client: Arc<Client>,
    mode: Mode,
    tree: Mutex<Option<(Instant, Arc<Tree>)>>,
}

impl fmt::Debug for MegaDrive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match self.mode {
            Mode::Account { .. } => "account",
            Mode::Folder { .. } => "folder link",
            Mode::File { .. } => "file link",
        };
        f.debug_struct("MegaDrive")
            .field("mode", &mode)
            .finish_non_exhaustive()
    }
}

fn malformed() -> VfsError {
    VfsError::new(ErrorKind::Other, "MEGA: unexpected answer")
}

/// The decrypted key of a node from its `k` field (`owner:key/owner:key`),
/// trying the owners `accept` takes with `decrypt_with`.
fn node_key(k: &str, accept: &dyn Fn(&str) -> bool, decrypt_with: &[u8; 16]) -> Option<Vec<u8>> {
    k.split('/').find_map(|part| {
        let (owner, enc) = part.split_once(':')?;
        if !accept(owner) {
            return None;
        }
        let raw = b64_decode(enc).ok()?;
        if raw.len() != 16 && raw.len() != 32 {
            return None;
        }
        ecb_decrypt(decrypt_with, &raw).ok()
    })
}

impl MegaDrive {
    /// The Cloud Drive of a signed-in account, API at `base` ([`API`]).
    ///
    /// # Errors
    ///
    /// A damaged session.
    pub fn account(base: &str, session: &Session) -> pa_vfs::Result<Self> {
        Ok(Self {
            client: Arc::new(Client::new(base).with_session(&session.sid)),
            mode: Mode::Account {
                user: session.user.clone(),
                mk: session.master_key()?,
            },
            tree: Mutex::new(None),
        })
    }

    /// A public folder or file link, API at `base`.
    ///
    /// # Errors
    ///
    /// A damaged key.
    pub fn public(base: &str, link: &PublicLink) -> pa_vfs::Result<Self> {
        let key = b64_decode(&link.key)?;
        let bad = || VfsError::new(ErrorKind::InvalidPath, "wrong MEGA link key");
        Ok(match link.kind {
            LinkKind::Folder => Self {
                client: Arc::new(Client::new(base).with_folder(&link.id)),
                mode: Mode::Folder {
                    key: key.try_into().map_err(|_| bad())?,
                },
                tree: Mutex::new(None),
            },
            LinkKind::File => Self {
                client: Arc::new(Client::new(base)),
                mode: Mode::File {
                    id: link.id.clone(),
                    key: key.try_into().map_err(|_| bad())?,
                },
                tree: Mutex::new(None),
            },
        })
    }

    /// The account's e-mail (for an account drive).
    ///
    /// # Errors
    ///
    /// Network errors, an ended session.
    pub fn account_email(&self) -> pa_vfs::Result<String> {
        let me = self.client.call("account", &json!({"a": "ug"}))?;
        Ok(me
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    fn tree(&self) -> pa_vfs::Result<Arc<Tree>> {
        let mut slot = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, t)) = &*slot
            && at.elapsed() < TREE_TTL
        {
            return Ok(Arc::clone(t));
        }
        let t = Arc::new(self.fetch_tree()?);
        *slot = Some((Instant::now(), Arc::clone(&t)));
        Ok(t)
    }

    fn fetch_tree(&self) -> pa_vfs::Result<Tree> {
        match &self.mode {
            Mode::File { id, key } => self.file_tree(id, key),
            Mode::Account { user, mk } => {
                let v = self.client.call("listing", &json!({"a": "f", "c": 1}))?;
                Ok(build_tree(&v, &|owner| owner == user, mk, true))
            }
            Mode::Folder { key } => {
                let v = self
                    .client
                    .call("folder link", &json!({"a": "f", "c": 1, "ca": 1, "r": 1}))?;
                Ok(build_tree(&v, &|_| true, key, false))
            }
        }
    }

    fn file_tree(&self, id: &str, key: &[u8; 32]) -> pa_vfs::Result<Tree> {
        let v = self
            .client
            .call("file link", &json!({"a": "g", "p": id, "ssl": 2}))?;
        let (aes, nonce) = file_key_parts(key);
        let name = v
            .get("at")
            .and_then(Value::as_str)
            .and_then(|at| decrypt_attributes(&aes, at).ok())
            .and_then(|a| a.get("n")?.as_str().map(str::to_owned))
            .ok_or_else(|| VfsError::new(ErrorKind::InvalidPath, "wrong MEGA link key"))?;
        let node = Node {
            id: id.to_owned(),
            name,
            kind: EntryKind::File,
            size: v.get("s").and_then(Value::as_u64).unwrap_or(0),
            modified_ms: None,
        };
        let root = Node::folder("");
        let mut t = Tree::default();
        t.children.insert(String::new(), vec![id.to_owned()]);
        t.keys.insert(id.to_owned(), FileKey { key: aes, nonce });
        t.nodes.insert(id.to_owned(), node);
        t.root = Some(root);
        Ok(t)
    }

    /// The `g` command that gives the download link of file `id`.
    fn download_cmd(&self, id: &str) -> Value {
        match &self.mode {
            Mode::File { .. } => json!({"a": "g", "g": 1, "p": id, "ssl": 2}),
            _ => json!({"a": "g", "g": 1, "n": id, "ssl": 2}),
        }
    }
}

/// The download link and size a `g` command gives.
fn download(client: &Client, cmd: &Value) -> pa_vfs::Result<(String, u64)> {
    let v = client.call("file", cmd)?;
    if let Some(e) = v.get("e").and_then(Value::as_i64) {
        return Err(ApiError(e).to_vfs("file"));
    }
    let url = v
        .get("g")
        .and_then(Value::as_str)
        .filter(|g| g.starts_with("http"))
        .ok_or_else(malformed)?;
    Ok((
        url.to_owned(),
        v.get("s").and_then(Value::as_u64).unwrap_or(0),
    ))
}

/// The tree of an `f` answer. `own`: an account (root is the node of type
/// 2, the Cloud Drive); else a public folder (root is the node whose
/// parent is not in the answer).
fn build_tree(v: &Value, accept: &dyn Fn(&str) -> bool, key: &[u8; 16], own: bool) -> Tree {
    let mut t = Tree::default();
    let empty = Vec::new();
    let list = v.get("f").and_then(Value::as_array).unwrap_or(&empty);
    let handles: std::collections::HashSet<&str> =
        list.iter().filter_map(|n| n.get("h")?.as_str()).collect();
    for n in list {
        let (Some(h), Some(ty)) = (
            n.get("h").and_then(Value::as_str),
            n.get("t").and_then(Value::as_i64),
        ) else {
            continue;
        };
        let parent = n.get("p").and_then(Value::as_str).unwrap_or_default();
        let is_root = if own {
            ty == 2
        } else {
            ty == 1 && !handles.contains(parent)
        };
        if is_root {
            t.root = Some(Node::folder(h));
            if own {
                continue;
            }
        }
        if ty != 0 && ty != 1 {
            // Inbox, Rubbish Bin.
            continue;
        }
        let Some(k) = n
            .get("k")
            .and_then(Value::as_str)
            .and_then(|k| node_key(k, accept, key))
        else {
            continue;
        };
        let Ok(aes) = node_aes_key(&k) else { continue };
        let Some(name) = n
            .get("a")
            .and_then(Value::as_str)
            .and_then(|a| decrypt_attributes(&aes, a).ok())
            .and_then(|a| a.get("n")?.as_str().map(str::to_owned))
        else {
            continue;
        };
        if is_root {
            // A public folder's own node: its children are listed.
            continue;
        }
        let kind = if ty == 1 {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        if ty == 0 {
            let Ok(k32) = <[u8; 32]>::try_from(k.as_slice()) else {
                continue;
            };
            let (fk, nonce) = file_key_parts(&k32);
            t.keys.insert(h.to_owned(), FileKey { key: fk, nonce });
        }
        t.nodes.insert(
            h.to_owned(),
            Node {
                id: h.to_owned(),
                name,
                kind,
                size: n.get("s").and_then(Value::as_u64).unwrap_or(0),
                modified_ms: n
                    .get("ts")
                    .and_then(Value::as_i64)
                    .filter(|ts| *ts > 0)
                    .map(|ts| ts * 1000),
            },
        );
        t.children
            .entry(parent.to_owned())
            .or_default()
            .push(h.to_owned());
    }
    t
}

impl Drive for MegaDrive {
    fn root(&self) -> pa_vfs::Result<Node> {
        self.tree()?
            .root
            .clone()
            .ok_or_else(|| VfsError::new(ErrorKind::NotFound, "MEGA: no root folder"))
    }

    fn children(&self, dir: &Node) -> pa_vfs::Result<Vec<Node>> {
        let t = self.tree()?;
        Ok(t.children
            .get(&dir.id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| t.nodes.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default())
    }

    fn open(&self, file: &Node) -> pa_vfs::Result<Box<dyn ByteSource>> {
        let t = self.tree()?;
        let key = *t
            .keys
            .get(&file.id)
            .ok_or_else(|| VfsError::new(ErrorKind::NotFound, "MEGA: file key not found"))?;
        let cmd = self.download_cmd(&file.id);
        let (url, size) = download(&self.client, &cmd)?;
        let size = if size > 0 { size } else { file.size };
        Ok(Box::new(RangeFile::new(
            Encrypted {
                client: Arc::clone(&self.client),
                cmd,
                id: file.id.clone(),
                url: Mutex::new(url),
                key,
            },
            size,
        )))
    }
}

/// A file being read: its download link and key.
struct Encrypted {
    client: Arc<Client>,
    /// The `g` command that gives a fresh link.
    cmd: Value,
    id: String,
    /// The download link; a secret (it grants the bytes).
    url: Mutex<String>,
    key: FileKey,
}

impl fmt::Debug for Encrypted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MegaFile")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Encrypted {
    fn get(&self, url: &str, offset: u64, len: u64) -> pa_vfs::Result<crate::http::Response> {
        // MEGA takes the range in the path: `<link>/<first>-<last>`.
        let ranged = format!("{url}/{offset}-{}", offset + len - 1);
        self.client.http().send_retrying(3, &|| {
            let mut req = Request::get(ranged.clone());
            req.limit = MAX_BLOB.max(len + 1);
            req
        })
    }
}

impl RangeFetch for Encrypted {
    fn fetch(&self, offset: u64, len: u64) -> pa_vfs::Result<Vec<u8>> {
        let url = self
            .url
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut r = self.get(&url, offset, len)?;
        if matches!(r.status, 403 | 404 | 410) {
            // The link expired: ask for a new one.
            let (fresh, _) = download(&self.client, &self.cmd)?;
            *self.url.lock().unwrap_or_else(PoisonError::into_inner) = fresh.clone();
            r = self.get(&fresh, offset, len)?;
        }
        let mut bytes = match r.status {
            // A path range answers with exactly the range.
            200 | 206 => {
                let mut b = r.body;
                b.truncate(usize::try_from(len).unwrap_or(usize::MAX));
                b
            }
            509 => return Err(ApiError(ApiError::OVER_QUOTA).to_vfs("file")),
            s => return Err(status_error("MEGA file", s)),
        };
        crypto::ctr_xor(&self.key.key, &self.key.nonce, offset, &mut bytes);
        Ok(bytes)
    }
}
