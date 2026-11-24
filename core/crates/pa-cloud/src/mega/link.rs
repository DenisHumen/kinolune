// SPDX-License-Identifier: MPL-2.0
//! Public MEGA links: `https://mega.nz/folder/<id>#<key>`,
//! `https://mega.nz/file/<id>#<key>` and the old `#F!<id>!<key>`,
//! `#!<id>!<key>` forms. The key after `#` never reaches MEGA's servers;
//! it is kept in the secret store, the link without it is the address.

use pa_vfs::{ErrorKind, VfsError};

use super::crypto::b64_decode;

/// What a public link points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A folder.
    Folder,
    /// One file.
    File,
}

/// A parsed public link. `Debug` hides the key.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicLink {
    /// Folder or file.
    pub kind: LinkKind,
    /// The public handle.
    pub id: String,
    /// The key, base64 (16 bytes for a folder, 32 for a file).
    pub key: String,
}

impl std::fmt::Debug for PublicLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicLink")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn bad() -> VfsError {
    VfsError::new(ErrorKind::InvalidPath, "not a MEGA folder or file link")
}

fn is_handle(s: &str) -> bool {
    (6..=16).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl PublicLink {
    /// Parses a link; the key may also come apart (`key`), for an address
    /// stored without it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidPath`] for anything else, or a key of the wrong
    /// size.
    pub fn parse(link: &str, key: Option<&str>) -> pa_vfs::Result<Self> {
        let (kind, id, link_key) = split(link)?;
        let key = key
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .unwrap_or(link_key)
            .to_owned();
        let want = match kind {
            LinkKind::Folder => 16,
            LinkKind::File => 32,
        };
        if b64_decode(&key).map(|k| k.len()).unwrap_or(0) != want {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                "the MEGA link has no key or a wrong one (the part after #)",
            ));
        }
        Ok(Self {
            kind,
            id: id.to_owned(),
            key,
        })
    }

    /// The address of a link given with or without its key
    /// (`https://mega.nz/folder/<id>`).
    ///
    /// # Errors
    ///
    /// Not a MEGA link.
    pub fn address_of(link: &str) -> pa_vfs::Result<String> {
        let (kind, id, _) = split(link)?;
        Ok(Self {
            kind,
            id: id.to_owned(),
            key: String::new(),
        }
        .address())
    }

    /// The key part of a link (after `#`), empty when there is none.
    #[must_use]
    pub fn key_of(link: &str) -> String {
        split(link)
            .map(|(_, _, k)| k.to_owned())
            .unwrap_or_default()
    }
}

/// Kind, handle and key text of a link.
fn split(link: &str) -> pa_vfs::Result<(LinkKind, &str, &str)> {
    {
        let t = link.trim();
        let rest = t
            .strip_prefix("https://")
            .or_else(|| t.strip_prefix("http://"))
            .unwrap_or(t);
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let rest = rest
            .strip_prefix("mega.nz")
            .or_else(|| rest.strip_prefix("mega.co.nz"))
            .or_else(|| rest.strip_prefix("mega.io"))
            .ok_or_else(bad)?;
        let (kind, id, link_key) = if let Some(r) = rest.strip_prefix("/folder/") {
            let (id, k) = r.split_once('#').unwrap_or((r, ""));
            (LinkKind::Folder, id, k)
        } else if let Some(r) = rest.strip_prefix("/file/") {
            let (id, k) = r.split_once('#').unwrap_or((r, ""));
            (LinkKind::File, id, k)
        } else if let Some(r) = rest.strip_prefix("/#F!") {
            let (id, k) = r.split_once('!').unwrap_or((r, ""));
            (LinkKind::Folder, id, k)
        } else if let Some(r) = rest.strip_prefix("/#!") {
            let (id, k) = r.split_once('!').unwrap_or((r, ""));
            (LinkKind::File, id, k)
        } else {
            return Err(bad());
        };
        let id = id.trim_end_matches('/');
        if !is_handle(id) {
            return Err(bad());
        }
        // A link to a subfolder (`…#key/folder/<sub>`): the key ends there.
        let link_key = link_key.split(['/', '!', '?']).next().unwrap_or("");
        Ok((kind, id, link_key))
    }
}

impl PublicLink {
    /// The link without the key: the source address.
    #[must_use]
    pub fn address(&self) -> String {
        match self.kind {
            LinkKind::Folder => format!("https://mega.nz/folder/{}", self.id),
            LinkKind::File => format!("https://mega.nz/file/{}", self.id),
        }
    }

    /// Whether `address` looks like a MEGA public link (with or without
    /// the key).
    #[must_use]
    pub fn is_link(address: &str) -> bool {
        let a = address.trim();
        ["mega.nz/", "mega.co.nz/", "mega.io/"]
            .iter()
            .any(|h| a.contains(h))
            && (a.contains("/folder/") || a.contains("/file/") || a.contains("/#"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FKEY: &str = "AAAAAAAAAAAAAAAAAAAAAA"; // 16 zero bytes
    const KEY32: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // 32

    #[test]
    fn parses_link_forms() {
        let l =
            PublicLink::parse(&format!("https://mega.nz/folder/AbCd1234#{FKEY}"), None).unwrap();
        assert_eq!((l.kind, l.id.as_str()), (LinkKind::Folder, "AbCd1234"));
        assert_eq!(l.address(), "https://mega.nz/folder/AbCd1234");
        let l = PublicLink::parse(&format!("mega.nz/file/Xy_-9876#{KEY32}"), None).unwrap();
        assert_eq!(l.kind, LinkKind::File);
        let l = PublicLink::parse(&format!("https://mega.co.nz/#F!AbCd1234!{FKEY}"), None).unwrap();
        assert_eq!(l.kind, LinkKind::Folder);
        let l = PublicLink::parse(&format!("https://mega.nz/#!AbCd1234!{KEY32}"), None).unwrap();
        assert_eq!(l.kind, LinkKind::File);
        let l = PublicLink::parse(
            &format!("https://mega.nz/folder/AbCd1234#{FKEY}/folder/Sub12345"),
            None,
        )
        .unwrap();
        assert_eq!(l.key, FKEY);
        // Address and key apart.
        let l = PublicLink::parse("https://mega.nz/folder/AbCd1234", Some(FKEY)).unwrap();
        assert_eq!(l.key, FKEY);
    }

    #[test]
    fn refuses_bad_links() {
        assert!(PublicLink::parse("https://mega.nz/folder/AbCd1234", None).is_err());
        assert!(PublicLink::parse(&format!("https://mega.nz/file/AbCd1234#{FKEY}"), None).is_err());
        assert!(PublicLink::parse("https://example.com/folder/AbCd1234#x", None).is_err());
        assert!(PublicLink::is_link("https://mega.nz/folder/AbCd1234"));
        assert!(!PublicLink::is_link("mega:/"));
    }
}
