// SPDX-License-Identifier: MPL-2.0
//! OneDrive (Microsoft Graph v1.0): the signed-in user's drive as a
//! [`Drive`]. A file is read from its pre-authenticated
//! `@microsoft.graph.downloadUrl` with `Range` (no token goes to the
//! download host); the link lives about an hour, so a refused link is
//! asked for again once. Items that are neither files nor folders (OneNote
//! packages) are [`EntryKind::Other`].

use std::sync::{Arc, Mutex, PoisonError};

use pa_vfs::{ByteSource, EntryKind};
use serde::Deserialize;

use crate::drive::{Drive, Node};
use crate::http::{Http, Request, encode, range_body, status_error};
use crate::oauth::{TokenSource, send_authed};
use crate::range::{RangeFetch, RangeFile};
use crate::time::rfc3339_ms;

/// Microsoft Graph.
pub const API: &str = "https://graph.microsoft.com/v1.0";

/// OneDrive of one account.
#[derive(Debug, Clone)]
pub struct OneDrive {
    http: Http,
    tokens: Arc<TokenSource>,
    api: String,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    value: Vec<Item>,
    #[serde(rename = "@odata.nextLink", default)]
    next: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(rename = "lastModifiedDateTime", default)]
    modified: Option<String>,
    #[serde(default)]
    folder: Option<serde_json::Value>,
    #[serde(default)]
    file: Option<serde_json::Value>,
    #[serde(rename = "@microsoft.graph.downloadUrl", default)]
    download_url: Option<String>,
}

#[derive(Deserialize)]
struct DriveInfo {
    #[serde(default)]
    owner: Option<Owner>,
}

#[derive(Deserialize)]
struct Owner {
    #[serde(default)]
    user: Option<User>,
}

#[derive(Deserialize)]
struct User {
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(default)]
    email: Option<String>,
}

fn node(i: Item) -> Node {
    let kind = if i.folder.is_some() {
        EntryKind::Dir
    } else if i.file.is_some() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Node {
        id: i.id,
        name: i.name,
        kind,
        size: i.size.unwrap_or(0),
        modified_ms: i.modified.as_deref().and_then(rfc3339_ms),
    }
}

impl OneDrive {
    /// The drive of the account `tokens` signs in to.
    #[must_use]
    pub fn new(tokens: Arc<TokenSource>) -> Self {
        Self::with_api(tokens, API)
    }

    /// Like [`new`](Self::new) with Graph at `api` (tests).
    #[must_use]
    pub fn with_api(tokens: Arc<TokenSource>, api: &str) -> Self {
        Self {
            http: Http::default(),
            tokens,
            api: api.trim_end_matches('/').to_owned(),
        }
    }

    fn get(&self, url: &str, what: &str) -> pa_vfs::Result<crate::http::Response> {
        let r = send_authed(&self.http, &self.tokens, &|t| {
            Request::get(url.to_owned()).bearer(t)
        })?;
        if r.ok() {
            Ok(r)
        } else {
            Err(status_error(what, r.status))
        }
    }

    /// The account's name (or e-mail), to show with the source.
    ///
    /// # Errors
    ///
    /// Graph cannot be reached or refuses the token.
    pub fn account(&self) -> pa_vfs::Result<String> {
        let r = self.get(&format!("{}/me/drive", self.api), "OneDrive account")?;
        let d: DriveInfo = r.json("OneDrive account")?;
        let user = d.owner.and_then(|o| o.user);
        Ok(user
            .map(|u| u.email.filter(|e| !e.is_empty()).unwrap_or(u.display_name))
            .unwrap_or_default())
    }

    fn download_url(&self, id: &str) -> pa_vfs::Result<String> {
        let r = self.get(
            &format!("{}/me/drive/items/{}", self.api, encode(id)),
            "OneDrive file",
        )?;
        let item: Item = r.json("OneDrive file")?;
        item.download_url.ok_or_else(|| {
            pa_vfs::VfsError::new(pa_vfs::ErrorKind::Unsupported, "OneDrive: not a file")
        })
    }
}

impl Drive for OneDrive {
    fn root(&self) -> pa_vfs::Result<Node> {
        Ok(Node::folder("root"))
    }

    fn children(&self, dir: &Node) -> pa_vfs::Result<Vec<Node>> {
        let mut url = format!(
            "{}/me/drive/items/{}/children?$top=1000",
            self.api,
            encode(&dir.id)
        );
        let mut out = Vec::new();
        loop {
            let page: Page = self
                .get(&url, "OneDrive listing")?
                .json("OneDrive listing")?;
            out.extend(page.value.into_iter().map(node));
            match page.next {
                // Graph hands out absolute links on its own host.
                Some(next) if next.starts_with(&self.api) => url = next,
                _ => return Ok(out),
            }
        }
    }

    fn open(&self, file: &Node) -> pa_vfs::Result<Box<dyn ByteSource>> {
        let url = self.download_url(&file.id)?;
        Ok(Box::new(RangeFile::new(
            Content {
                drive: self.clone(),
                id: file.id.clone(),
                url: Mutex::new(url),
            },
            file.size,
        )))
    }
}

struct Content {
    drive: OneDrive,
    id: String,
    /// Pre-authenticated link; a secret.
    url: Mutex<String>,
}

impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Content")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl RangeFetch for Content {
    fn fetch(&self, offset: u64, len: u64) -> pa_vfs::Result<Vec<u8>> {
        let http = &self.drive.http;
        let url = self
            .url
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let r = http.send_retrying(3, &|| Request::get(url.clone()).range(offset, len))?;
        if matches!(r.status, 401 | 403 | 404 | 410) {
            // The link expired: ask for a new one.
            let fresh = self.drive.download_url(&self.id)?;
            *self.url.lock().unwrap_or_else(PoisonError::into_inner) = fresh.clone();
            let r = http.send_retrying(3, &|| Request::get(fresh.clone()).range(offset, len))?;
            return range_body("OneDrive file", r, offset, len);
        }
        range_body("OneDrive file", r, offset, len)
    }
}
