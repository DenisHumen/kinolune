// SPDX-License-Identifier: MPL-2.0
//! Addresses of sources: validation and normalization to the `sources.uri`
//! form, which never holds credentials.
//!
//! | Kind | Accepted | Stored |
//! |---|---|---|
//! | `local` | absolute path, `file://` URI | `file:///…` (percent-encoded) |
//! | `smb` | `smb://host[:port]/share[/folder]` | as given, slashes normalized |
//! | `webdav` | `http(s)://…`, `webdav(s)://…`, `davs://…` | `http(s)://…` |
//! | `jellyfin`, `emby` | `http(s)://host[:port][/base]` | without trailing `/` |
//! | `sftp` | `sftp://host[:port][/path]`, `host[/path]` | `sftp://…`, see [`ServerAddress`] |
//! | `ftp` | `ftp://`, `ftpes://` (explicit TLS), `ftps://` (implicit TLS), `host[/path]` | as given, see [`ServerAddress`] |
//!
//! The `file://` encoding is the same as in `pa-catalog`, so sources added
//! by either crate are the same rows.

use std::path::{Path, PathBuf};

use crate::{Error, Result, SourceKind};

/// Error text for `user:pass@host` addresses. It must not echo the address:
/// the password is in it.
const CREDENTIALS_IN_ADDRESS: &str =
    "the address must not contain a user name or password; use the login fields";

/// Normalizes `address` of a source of `kind` to the stored URI.
pub(crate) fn normalize(kind: SourceKind, address: &str) -> Result<String> {
    let address = address.trim();
    if address.is_empty() {
        return Err(Error::Invalid("empty address".into()));
    }
    match kind {
        SourceKind::Local => {
            let path = if address.starts_with("file://") {
                file_uri_to_path(address)
                    .ok_or_else(|| Error::Invalid(format!("malformed file URI: {address}")))?
            } else {
                PathBuf::from(address)
            };
            if !is_absolute(&path) {
                return Err(Error::Invalid(format!("not an absolute path: {address}")));
            }
            Ok(path_to_file_uri(&path))
        }
        SourceKind::Smb => SmbAddress::parse(address).map(|a| a.to_uri()),
        SourceKind::Jellyfin | SourceKind::Emby => {
            if address.contains('@') {
                return Err(Error::Invalid(CREDENTIALS_IN_ADDRESS.into()));
            }
            pa_jellyfin::normalize_address(address).map_err(|e| Error::Invalid(e.to_string()))
        }
        SourceKind::Sftp => ServerAddress::parse(address, "sftp", &["sftp"]).map(|a| a.to_uri()),
        SourceKind::Ftp => {
            ServerAddress::parse(address, "ftp", &["ftp", "ftpes", "ftps"]).map(|a| a.to_uri())
        }
        SourceKind::WebDav => {
            let (scheme, rest) = address
                .split_once("://")
                .ok_or_else(|| Error::Invalid("WebDAV address needs http:// or https://".into()))?;
            let scheme = match scheme.to_ascii_lowercase().as_str() {
                "http" | "webdav" => "http",
                "https" | "webdavs" | "davs" => "https",
                _ => {
                    return Err(Error::Invalid(
                        "WebDAV address needs http:// or https://".into(),
                    ));
                }
            };
            let authority = authority(rest)?;
            if authority.is_empty() {
                return Err(Error::Invalid("WebDAV address has no host".into()));
            }
            Ok(format!("{scheme}://{rest}"))
        }
    }
}

/// `C:/x` counts as absolute on every platform: the database is shared.
fn is_absolute(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    path.is_absolute()
        || s.starts_with('/')
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'/' | b'\\'))
}

/// The authority part of `rest` (after `scheme://`); refuses user info.
fn authority(rest: &str) -> Result<&str> {
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains('@') {
        return Err(Error::Invalid(CREDENTIALS_IN_ADDRESS.into()));
    }
    Ok(authority)
}

/// Parts of an `smb://host[:port]/share[/folder]` address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmbAddress {
    /// `host`, `host:port` or `[ipv6]:port`.
    pub server: String,
    /// Share name.
    pub share: String,
    /// Folder inside the share, `/`-separated, without leading or trailing
    /// slashes; empty for the share root.
    pub folder: String,
}

impl SmbAddress {
    /// Parses an `smb://` address. Refuses user info, queries and fragments.
    pub fn parse(address: &str) -> Result<Self> {
        let rest = address
            .get(..6)
            .filter(|s| s.eq_ignore_ascii_case("smb://"))
            .map(|_| &address[6..])
            .ok_or_else(|| Error::Invalid("SMB address needs smb://host/share".into()))?;
        let server = authority(rest)?;
        if rest.contains(['?', '#']) {
            return Err(Error::Invalid(
                "SMB address must not have a query or fragment".into(),
            ));
        }
        let mut parts = rest[server.len()..]
            .split(['/', '\\'])
            .filter(|p| !p.is_empty());
        let share = parts.next().unwrap_or_default();
        if server.is_empty() || share.is_empty() {
            return Err(Error::Invalid("SMB address needs smb://host/share".into()));
        }
        Ok(Self {
            server: server.to_owned(),
            share: share.to_owned(),
            folder: parts.collect::<Vec<_>>().join("/"),
        })
    }

    /// `smb://server/share[/folder]`.
    pub fn to_uri(&self) -> String {
        let mut uri = format!("smb://{}/{}", self.server, self.share);
        if !self.folder.is_empty() {
            uri.push('/');
            uri.push_str(&self.folder);
        }
        uri
    }
}

/// Parts of an `sftp://` or `ftp://` / `ftpes://` / `ftps://` address.
///
/// The path is a folder on the server, as curl reads it: for SFTP it is
/// absolute, `/~/…` is under the home folder and an empty path is the home
/// folder; for FTP it is relative to the login folder (RFC 1738), an empty
/// path is the login folder and `/%2F…` is absolute. `%XX` escapes are
/// decoded; the stored URI escapes only `%` itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    /// Lower-case scheme.
    pub scheme: String,
    /// Host name or IP address, IPv6 without brackets.
    pub host: String,
    /// Port, if given.
    pub port: Option<u16>,
    /// Folder: empty, or `/`-separated starting with `/`, no trailing `/`.
    pub path: String,
    /// The path started with `/%2F` (an absolute FTP path).
    pub absolute: bool,
}

impl ServerAddress {
    /// Parses `address`; one without `scheme://` gets `default_scheme`.
    /// Refuses other schemes, user info, queries and fragments.
    pub fn parse(address: &str, default_scheme: &str, schemes: &[&str]) -> Result<Self> {
        let address = address.trim();
        let (scheme, rest) = match address.split_once("://") {
            Some((s, r)) => (s.to_ascii_lowercase(), r),
            None => (default_scheme.to_owned(), address),
        };
        let wanted = || {
            let list: Vec<_> = schemes.iter().map(|s| format!("{s}://")).collect();
            Error::Invalid(format!("the address needs {}", list.join(", ")))
        };
        if !schemes.contains(&scheme.as_str()) {
            return Err(wanted());
        }
        let auth = authority(rest)?;
        if rest.contains(['?', '#']) {
            return Err(Error::Invalid(
                "the address must not have a query or fragment".into(),
            ));
        }
        let (host, port) = match auth.strip_prefix('[') {
            Some(v6) => {
                let (host, after) = v6
                    .split_once(']')
                    .ok_or_else(|| Error::Invalid("unclosed `[` in the address".into()))?;
                (host, after.strip_prefix(':'))
            }
            None => match auth.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') => (h, Some(p)),
                _ => (auth, None),
            },
        };
        if host.is_empty() {
            return Err(Error::Invalid("the address has no host".into()));
        }
        let port = port
            .map(|p| {
                p.parse::<u16>()
                    .ok()
                    .filter(|&p| p > 0)
                    .ok_or_else(|| Error::Invalid(format!("bad port {p:?}")))
            })
            .transpose()?;
        let raw_path = &rest[auth.len()..];
        let absolute = raw_path
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case("/%2f"));
        let path = percent_decode(raw_path)
            .ok_or_else(|| Error::Invalid("bad `%` escape in the address".into()))?;
        let parts: Vec<_> = path
            .split(['/', '\\'])
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        if parts.contains(&"..") || path.contains(['\0', '\r', '\n']) {
            return Err(Error::Invalid("the folder must not contain `..`".into()));
        }
        Ok(Self {
            scheme,
            host: host.to_owned(),
            port,
            path: parts.iter().map(|p| format!("/{p}")).collect(),
            absolute,
        })
    }

    /// `scheme://host[:port][/path]`.
    pub fn to_uri(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let port = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        let path = self.path.replace('%', "%25");
        let path = if self.absolute {
            format!("/%2F{}", path.trim_start_matches('/'))
        } else {
            path
        };
        format!("{}://{host}{port}{path}", self.scheme)
    }
}

/// Decodes `%XX` escapes; `None` for a malformed escape or non-UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `file://` URI of an absolute local path (percent-encoded outside the
/// unreserved set, `/` and `:`).
pub fn path_to_file_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        // `C:/Media` → `file:///C:/Media`.
        out.push('/');
    }
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Local path of a `file://` URI; `None` for other schemes or a malformed
/// escape.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Drop an authority (`file://localhost/…`).
    let rest = if rest.starts_with('/') {
        rest
    } else {
        &rest[rest.find('/')?..]
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = rest.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let mut s = String::from_utf8(out).ok()?;
    // `/C:/Media` → `C:/Media`.
    if s.len() >= 3 && s.as_bytes()[0] == b'/' && s.as_bytes()[2] == b':' {
        s.remove(0);
    }
    Some(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(kind: SourceKind, address: &str) -> String {
        normalize(kind, address).unwrap()
    }

    fn invalid(kind: SourceKind, address: &str) -> String {
        match normalize(kind, address) {
            Err(Error::Invalid(m)) => m,
            other => panic!("{address}: {other:?}"),
        }
    }

    #[test]
    fn local_paths() {
        assert_eq!(
            ok(SourceKind::Local, "/mnt/My Movies"),
            "file:///mnt/My%20Movies"
        );
        assert_eq!(ok(SourceKind::Local, "file:///mnt/a"), "file:///mnt/a");
        assert_eq!(ok(SourceKind::Local, "C:\\Media"), "file:///C:/Media");
        assert_eq!(
            file_uri_to_path(&ok(SourceKind::Local, "/home/x/Фильмы")).unwrap(),
            PathBuf::from("/home/x/Фильмы")
        );
        invalid(SourceKind::Local, "relative/dir");
        invalid(SourceKind::Local, "  ");
    }

    #[test]
    fn smb_addresses() {
        assert_eq!(
            ok(SourceKind::Smb, "smb://nas.local/media//Movies/4K/"),
            "smb://nas.local/media/Movies/4K"
        );
        let a = SmbAddress::parse("SMB://[fe80::1]:4445/share").unwrap();
        assert_eq!(
            (a.server.as_str(), a.share.as_str(), a.folder.as_str()),
            ("[fe80::1]:4445", "share", "")
        );
        invalid(SourceKind::Smb, "smb://nas");
        invalid(SourceKind::Smb, "//nas/share");
        invalid(SourceKind::Smb, "smb://nas/share?x=1");
    }

    #[test]
    fn webdav_addresses() {
        assert_eq!(
            ok(SourceKind::WebDav, "davs://cloud.example/remote.php/dav/"),
            "https://cloud.example/remote.php/dav/"
        );
        assert_eq!(ok(SourceKind::WebDav, "http://nas:8080"), "http://nas:8080");
        invalid(SourceKind::WebDav, "ftp://nas/");
        invalid(SourceKind::WebDav, "https:///x");
    }

    #[test]
    fn sftp_and_ftp_addresses() {
        assert_eq!(
            ok(SourceKind::Sftp, "SFTP://nas.local:2222/srv//media/"),
            "sftp://nas.local:2222/srv/media"
        );
        assert_eq!(ok(SourceKind::Sftp, "nas.local"), "sftp://nas.local");
        assert_eq!(
            ok(SourceKind::Sftp, "sftp://nas/~/Фильмы и сериалы"),
            "sftp://nas/~/Фильмы и сериалы"
        );
        assert_eq!(
            ok(SourceKind::Sftp, "sftp://[fe80::1]:22/My%20Movies/100%25"),
            "sftp://[fe80::1]:22/My Movies/100%25"
        );
        let a = ServerAddress::parse("sftp://[fe80::1]:22/My Movies/100%25", "sftp", &["sftp"])
            .unwrap();
        assert_eq!(
            (a.host.as_str(), a.port, a.path.as_str()),
            ("fe80::1", Some(22), "/My Movies/100%")
        );
        assert_eq!(ok(SourceKind::Ftp, "ftpes://nas/video"), "ftpes://nas/video");
        assert_eq!(ok(SourceKind::Ftp, "nas:2121"), "ftp://nas:2121");
        assert_eq!(ok(SourceKind::Ftp, "FTPS://nas/"), "ftps://nas");
        // curl: an absolute FTP path is written `/%2F…`.
        assert_eq!(ok(SourceKind::Ftp, "ftp://nas/%2fsrv/ftp"), "ftp://nas/%2Fsrv/ftp");
        let a = ServerAddress::parse("ftp://nas/%2Fsrv/ftp", "ftp", &["ftp"]).unwrap();
        assert!(a.absolute);
        assert_eq!(a.path, "/srv/ftp");
        assert!(!ServerAddress::parse("ftp://nas/video", "ftp", &["ftp"]).unwrap().absolute);
        assert_eq!(ok(SourceKind::Ftp, "ftp://nas/%2F"), "ftp://nas/%2F");
        invalid(SourceKind::Sftp, "ftp://nas/");
        invalid(SourceKind::Ftp, "sftp://nas/");
        invalid(SourceKind::Ftp, "ftp:///x");
        invalid(SourceKind::Ftp, "ftp://nas:0/");
        invalid(SourceKind::Ftp, "ftp://nas:99999/");
        invalid(SourceKind::Sftp, "sftp://nas/a/../b");
        invalid(SourceKind::Sftp, "sftp://nas/%zz");
        invalid(SourceKind::Ftp, "ftp://nas/a?x=1");
    }

    #[test]
    fn credentials_in_addresses_are_refused_without_echo() {
        for (kind, address) in [
            (SourceKind::Smb, "smb://anna:Pa55word@nas/media"),
            (
                SourceKind::WebDav,
                "https://anna:Pa55word@cloud.example/dav/",
            ),
            (SourceKind::Smb, "smb://anna@nas/media"),
            (SourceKind::Sftp, "sftp://anna:Pa55word@nas/media"),
            (SourceKind::Ftp, "ftp://anna:Pa55word@nas/media"),
        ] {
            let message = invalid(kind, address);
            assert!(!message.contains("Pa55word"), "{message}");
            assert_eq!(message, CREDENTIALS_IN_ADDRESS);
        }
    }
}
