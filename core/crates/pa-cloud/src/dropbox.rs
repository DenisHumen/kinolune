// SPDX-License-Identifier: MPL-2.0
//! Dropbox (API v2): the account's files as a [`Drive`]. Nodes are keyed
//! by Dropbox ids (`id:…`, the root is `""`); files are read with
//! `/files/download` and a `Range` header.

use std::sync::Arc;

use pa_vfs::{ByteSource, EntryKind, ErrorKind, VfsError};
use serde::Deserialize;
use serde_json::json;

use crate::drive::{Drive, Node};
use crate::http::{Http, Request, range_body, status_error};
use crate::oauth::{TokenSource, send_authed};
use crate::range::{RangeFetch, RangeFile};
use crate::time::rfc3339_ms;

/// The RPC host.
pub const API: &str = "https://api.dropboxapi.com/2";
/// The content host.
pub const CONTENT: &str = "https://content.dropboxapi.com/2";

/// Dropbox of one account.
#[derive(Debug, Clone)]
pub struct Dropbox {
    http: Http,
    tokens: Arc<TokenSource>,
    api: String,
    content: String,
}

#[derive(Deserialize)]
struct ListFolder {
    #[serde(default)]
    entries: Vec<Entry>,
    #[serde(default)]
    cursor: String,
    #[serde(default)]
    has_more: bool,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = ".tag")]
    tag: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    server_modified: Option<String>,
}

#[derive(Deserialize)]
struct Account {
    #[serde(default)]
    email: String,
    #[serde(default)]
    name: Option<AccountName>,
}

#[derive(Deserialize)]
struct AccountName {
    #[serde(default)]
    display_name: String,
}

/// JSON for the `Dropbox-API-Arg` header: HTTP headers are ASCII, so
/// everything else is escaped as `\uXXXX`.
pub(crate) fn header_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    for c in value.to_string().chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for u in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{u:04x}"));
            }
        }
    }
    out
}

fn node(e: Entry) -> Option<Node> {
    let kind = match e.tag.as_str() {
        "folder" => EntryKind::Dir,
        "file" => EntryKind::File,
        // `deleted` entries only come with `include_deleted`.
        _ => return None,
    };
    Some(Node {
        id: e.id,
        name: e.name,
        kind,
        size: e.size,
        modified_ms: e.server_modified.as_deref().and_then(rfc3339_ms),
    })
}

impl Dropbox {
    /// The Dropbox of the account `tokens` signs in to.
    #[must_use]
    pub fn new(tokens: Arc<TokenSource>) -> Self {
        Self::with_api(tokens, API, CONTENT)
    }

    /// Like [`new`](Self::new) with both hosts elsewhere (tests).
    #[must_use]
    pub fn with_api(tokens: Arc<TokenSource>, api: &str, content: &str) -> Self {
        Self {
            http: Http::default(),
            tokens,
            api: api.trim_end_matches('/').to_owned(),
            content: content.trim_end_matches('/').to_owned(),
        }
    }

    fn rpc(
        &self,
        endpoint: &str,
        arg: &serde_json::Value,
        what: &str,
    ) -> pa_vfs::Result<crate::http::Response> {
        let url = format!("{}/{endpoint}", self.api);
        let r = send_authed(&self.http, &self.tokens, &|t| {
            Request::post(url.clone()).bearer(t).json(arg)
        })?;
        if r.ok() {
            return Ok(r);
        }
        Err(api_error(what, &r))
    }

    /// The account's e-mail (or name), to show with the source.
    ///
    /// # Errors
    ///
    /// Dropbox cannot be reached or refuses the token.
    pub fn account(&self) -> pa_vfs::Result<String> {
        let a: Account = self
            .rpc(
                "users/get_current_account",
                &serde_json::Value::Null,
                "Dropbox account",
            )?
            .json("Dropbox account")?;
        Ok(if a.email.is_empty() {
            a.name.map(|n| n.display_name).unwrap_or_default()
        } else {
            a.email
        })
    }
}

/// A 409 of Dropbox names what went wrong in `error_summary`
/// (`path/not_found/..`).
fn api_error(what: &str, r: &crate::http::Response) -> VfsError {
    if r.status == 409 {
        let summary = serde_json::from_slice::<serde_json::Value>(&r.body)
            .ok()
            .and_then(|v| v.get("error_summary")?.as_str().map(str::to_owned))
            .unwrap_or_default();
        let kind = if summary.contains("not_found") {
            ErrorKind::NotFound
        } else if summary.contains("not_folder") {
            ErrorKind::NotADirectory
        } else if summary.contains("not_file") {
            ErrorKind::IsADirectory
        } else {
            ErrorKind::Other
        };
        return VfsError::new(kind, format!("{what}: {}", summary.trim_end_matches('.')));
    }
    status_error(what, r.status)
}

impl Drive for Dropbox {
    fn root(&self) -> pa_vfs::Result<Node> {
        Ok(Node::folder(""))
    }

    fn children(&self, dir: &Node) -> pa_vfs::Result<Vec<Node>> {
        let mut page: ListFolder = self
            .rpc(
                "files/list_folder",
                &json!({ "path": dir.id, "limit": 2000 }),
                "Dropbox listing",
            )?
            .json("Dropbox listing")?;
        let mut out = Vec::new();
        loop {
            out.extend(page.entries.into_iter().filter_map(node));
            if !page.has_more {
                return Ok(out);
            }
            page = self
                .rpc(
                    "files/list_folder/continue",
                    &json!({ "cursor": page.cursor }),
                    "Dropbox listing",
                )?
                .json("Dropbox listing")?;
        }
    }

    fn open(&self, file: &Node) -> pa_vfs::Result<Box<dyn ByteSource>> {
        Ok(Box::new(RangeFile::new(
            Download {
                drive: self.clone(),
                id: file.id.clone(),
            },
            file.size,
        )))
    }
}

#[derive(Debug)]
struct Download {
    drive: Dropbox,
    id: String,
}

impl RangeFetch for Download {
    fn fetch(&self, offset: u64, len: u64) -> pa_vfs::Result<Vec<u8>> {
        let d = &self.drive;
        let url = format!("{}/files/download", d.content);
        let arg = header_json(&json!({ "path": self.id }));
        let r = send_authed(&d.http, &d.tokens, &|t| {
            Request::post(url.clone())
                .bearer(t)
                .header("dropbox-api-arg", arg.clone())
                .range(offset, len)
        })?;
        if r.status == 409 {
            return Err(api_error("Dropbox file", &r));
        }
        range_body("Dropbox file", r, offset, len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_json_is_ascii() {
        assert_eq!(
            header_json(&json!({"path": "/Фильм 🎬"})),
            "{\"path\":\"/\\u0424\\u0438\\u043b\\u044c\\u043c \\ud83c\\udfac\"}"
        );
    }
}
