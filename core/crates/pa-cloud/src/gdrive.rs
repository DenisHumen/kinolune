// SPDX-License-Identifier: MPL-2.0
//! Google Drive (Drive API v3): "My Drive" as a [`Drive`]. Files are read
//! with `alt=media` and a `Range` header. Google Docs, Sheets and other
//! documents without bytes, and shortcuts, are listed as
//! [`EntryKind::Other`] and skipped by the scanner. Trashed files are not
//! listed.

use std::sync::Arc;

use pa_vfs::{ByteSource, EntryKind};
use serde::Deserialize;

use crate::drive::{Drive, Node};
use crate::http::{Http, Request, encode, range_body, status_error};
use crate::oauth::{TokenSource, send_authed};
use crate::range::{RangeFetch, RangeFile};
use crate::time::rfc3339_ms;

/// The Drive API.
pub const API: &str = "https://www.googleapis.com/drive/v3";

const FOLDER: &str = "application/vnd.google-apps.folder";

/// Google Drive of one account.
#[derive(Debug, Clone)]
pub struct GoogleDrive {
    http: Http,
    tokens: Arc<TokenSource>,
    api: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    #[serde(default)]
    files: Vec<File>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    id: String,
    name: String,
    mime_type: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    modified_time: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct About {
    user: AboutUser,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AboutUser {
    #[serde(default)]
    email_address: String,
    #[serde(default)]
    display_name: String,
}

fn node(f: File) -> Node {
    let kind = if f.mime_type == FOLDER {
        EntryKind::Dir
    } else if f.mime_type.starts_with("application/vnd.google-apps.") {
        EntryKind::Other
    } else {
        EntryKind::File
    };
    Node {
        id: f.id,
        name: f.name,
        kind,
        size: f.size.and_then(|s| s.parse().ok()).unwrap_or(0),
        modified_ms: f.modified_time.as_deref().and_then(rfc3339_ms),
    }
}

impl GoogleDrive {
    /// The drive of the account `tokens` signs in to.
    #[must_use]
    pub fn new(tokens: Arc<TokenSource>) -> Self {
        Self::with_api(tokens, API)
    }

    /// Like [`new`](Self::new) with the API at `api` (tests).
    #[must_use]
    pub fn with_api(tokens: Arc<TokenSource>, api: &str) -> Self {
        Self {
            http: Http::default(),
            tokens,
            api: api.trim_end_matches('/').to_owned(),
        }
    }

    /// The account's e-mail (or name), to show with the source.
    ///
    /// # Errors
    ///
    /// The API cannot be reached or refuses the token.
    pub fn account(&self) -> pa_vfs::Result<String> {
        let url = format!("{}/about?fields=user(emailAddress,displayName)", self.api);
        let r = send_authed(&self.http, &self.tokens, &|t| {
            Request::get(url.clone()).bearer(t)
        })?;
        if !r.ok() {
            return Err(status_error("Google Drive account", r.status));
        }
        let a: About = r.json("Google Drive account")?;
        Ok(if a.user.email_address.is_empty() {
            a.user.display_name
        } else {
            a.user.email_address
        })
    }
}

impl Drive for GoogleDrive {
    fn root(&self) -> pa_vfs::Result<Node> {
        Ok(Node::folder("root"))
    }

    fn children(&self, dir: &Node) -> pa_vfs::Result<Vec<Node>> {
        let q = format!(
            "'{}' in parents and trashed = false",
            dir.id.replace('\'', "\\'")
        );
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut url = format!(
                "{}/files?q={}&pageSize=1000&fields={}&supportsAllDrives=true&includeItemsFromAllDrives=true",
                self.api,
                encode(&q),
                encode("nextPageToken,files(id,name,mimeType,size,modifiedTime)"),
            );
            if let Some(p) = &page {
                url.push_str(&format!("&pageToken={}", encode(p)));
            }
            let r = send_authed(&self.http, &self.tokens, &|t| {
                Request::get(url.clone()).bearer(t)
            })?;
            if !r.ok() {
                return Err(status_error("Google Drive listing", r.status));
            }
            let list: FileList = r.json("Google Drive listing")?;
            out.extend(list.files.into_iter().map(node));
            match list.next_page_token.filter(|p| !p.is_empty()) {
                Some(p) => page = Some(p),
                None => return Ok(out),
            }
        }
    }

    fn open(&self, file: &Node) -> pa_vfs::Result<Box<dyn ByteSource>> {
        Ok(Box::new(RangeFile::new(
            Media {
                drive: self.clone(),
                id: file.id.clone(),
            },
            file.size,
        )))
    }
}

#[derive(Debug)]
struct Media {
    drive: GoogleDrive,
    id: String,
}

impl RangeFetch for Media {
    fn fetch(&self, offset: u64, len: u64) -> pa_vfs::Result<Vec<u8>> {
        let d = &self.drive;
        let url = format!(
            "{}/files/{}?alt=media&supportsAllDrives=true",
            d.api,
            encode(&self.id)
        );
        let r = send_authed(&d.http, &d.tokens, &|t| {
            Request::get(url.clone()).bearer(t).range(offset, len)
        })?;
        range_body("Google Drive file", r, offset, len)
    }
}
