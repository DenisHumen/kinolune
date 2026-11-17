// SPDX-License-Identifier: MPL-2.0
//! Picking a folder on an SMB server without knowing what a share is
//! (UI-028): the server's shared folders, live folder listing, and finding
//! the share and folder behind a path the user typed (`home` +
//! `anna/smb_shared/films` → share `anna` (Samba `[homes]`) or a share
//! `smb_shared`, verified by connecting).
//!
//! All calls connect with an [`SmbLogin`], which is either typed in the add
//! flow or taken from a stored source ([`SourceManager::smb_login_of`]), so
//! a broken source can be fixed without typing the password again.

use std::time::Duration;

use pa_secrets::SecretString;
use pa_vfs::{Vfs, VfsError, VfsPath};
use pa_vfs_smb::{SmbConfig, SmbFs};

use crate::{Error, Result, SmbAddress, SmbEncryption, SmbVersion, SourceManager, SourceOptions};

/// Timeout of one request while browsing: short, a person is waiting.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(15);

/// Most share names tried for one typed path.
const MAX_GUESSES: usize = 8;

/// Share names people and NAS vendors use most, tried when the server
/// hides its share list (UI-031). Few, so a wrong server costs little.
const COMMON_SHARES: [&str; 6] = ["shared", "share", "public", "media", "video", "Multimedia"];

/// Most share names tried for one search, typed and common ones together.
const MAX_TRIES: usize = 14;

/// Server and logon for browsing. `Debug` hides the password.
#[derive(Clone, Default)]
pub struct SmbLogin {
    /// `host`, `host:port` or `[ipv6]:port`.
    pub server: String,
    /// `None` or empty: anonymous (guest) logon.
    pub username: Option<String>,
    pub password: Option<SecretString>,
    /// Domain, version, signing and encryption.
    pub options: SourceOptions,
}

impl std::fmt::Debug for SmbLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbLogin")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish_non_exhaustive()
    }
}

/// A shared folder of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmbShare {
    /// Share name.
    pub name: String,
    /// The server's comment, may be empty.
    pub comment: String,
}

/// A folder inside a share: what a source points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmbPlace {
    /// Share name, as the server spells it.
    pub share: String,
    /// Folder inside it, `/`-separated, empty for the share itself.
    pub folder: String,
}

impl SmbLogin {
    fn user(&self) -> Option<&str> {
        self.username
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
    }

    fn config(&self, share: &str, folder: &str) -> SmbConfig {
        let o = &self.options;
        let mut c = SmbConfig::new(self.server.trim(), share)
            .with_root(folder)
            .with_timeout(BROWSE_TIMEOUT)
            .with_max_connections(1)
            .with_version(match o.smb_version {
                SmbVersion::Any => pa_vfs_smb::Version::Any,
                SmbVersion::Smb2 => pa_vfs_smb::Version::Any2,
                SmbVersion::Smb3 => pa_vfs_smb::Version::Any3,
            })
            .with_signing(if o.smb_signing_required {
                pa_vfs_smb::Signing::Required
            } else {
                pa_vfs_smb::Signing::Auto
            })
            .with_encryption(match o.smb_encryption {
                SmbEncryption::Auto => pa_vfs_smb::Encryption::Auto,
                SmbEncryption::Required => pa_vfs_smb::Encryption::Required,
                SmbEncryption::Off => pa_vfs_smb::Encryption::Off,
            });
        if let Some(user) = self.user() {
            let pass = self.password.as_ref().map_or("", SecretString::expose);
            let mut creds = pa_vfs_smb::Credentials::new(user, pass);
            if let Some(d) = o.smb_domain.as_deref().filter(|d| !d.is_empty()) {
                creds = creds.with_domain(d);
            }
            c = c.with_credentials(creds);
        }
        c
    }

    fn error(&self, e: &VfsError) -> Error {
        Error::Probe {
            kind: e.kind(),
            message: crate::scrub(e.message(), self.password.as_ref()),
        }
    }
}

fn invalid_name(name: &str) -> bool {
    name.is_empty() || name.contains(['/', '\\'])
}

/// Splits a typed path into folder names (`/` or `\`, no `.`).
fn segments(text: &str) -> Vec<&str> {
    text.split(['/', '\\'])
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
        .collect()
}

/// Shared folders of the server, without printers and administrative
/// shares.
///
/// Some servers refuse or break the share listing (Samba with a locked-down
/// IPC$ / srvsvc: the owner's box answers `ShareEnum` with a dropped
/// connection, errno 32) although the login itself is fine. That is an
/// error of kind `PermissionDenied` with [`pa_vfs_smb::SHARE_LIST_REFUSED`]
/// in the message (UI-031), distinct from a wrong user or password
/// (`AuthenticationFailed`) and a server that does not answer
/// (`Unavailable`, `TimedOut`): the login worked, the share name has to be
/// typed, and [`smb_find`] guesses the usual ones.
pub fn smb_shares(login: &SmbLogin) -> Result<Vec<SmbShare>> {
    listed_shares(login).map_err(|e| login.error(&e))
}

/// Whether listing failed only because the server hides its shares.
fn list_refused(e: &VfsError) -> bool {
    e.message().contains(pa_vfs_smb::SHARE_LIST_REFUSED)
}

/// The server's own share list, without any fallback.
fn listed_shares(login: &SmbLogin) -> pa_vfs::Result<Vec<SmbShare>> {
    pa_vfs_smb::list_shares(&login.config("IPC$", "")).map(|shares| {
        shares
            .into_iter()
            .map(|s| SmbShare {
                name: s.name,
                comment: s.comment,
            })
            .collect()
    })
}

/// Entries of `folder` (`""` = the share itself) in `share`.
pub fn smb_list_folder(
    login: &SmbLogin,
    share: &str,
    folder: &str,
) -> Result<Vec<pa_vfs::DirEntry>> {
    if invalid_name(share) {
        return Err(Error::Invalid(format!("share name {share:?}")));
    }
    SmbFs::new(login.config(share, folder))
        .and_then(|fs| fs.list(&VfsPath::root()))
        .map_err(|e| login.error(&e))
}

/// Finds where a path the user typed really is on the server. `share` and
/// `folder` are whatever the user entered, possibly a path as it looks on
/// the server's disk (`home`, `anna/smb_shared/films`).
///
/// Tries, and verifies by connecting: the path as typed; then a share
/// named like each folder of the path (Samba `[homes]` shows the home
/// folder as a share named after the user; a share may point deep into a
/// folder); then the user's home share for `home/…`, `homes/…` or
/// `Users/…`. Places that lead to the same folder (same server file id)
/// are reported once, with the most specific share. Most specific first.
///
/// If the server hides its share list (UI-031) and the typed path fits
/// nowhere (or nothing was typed), the user's home share and a few common
/// share names (`shared`, `public`, `media`…) are tried as well; each one
/// that opens is reported with the deepest tail of the typed path found in
/// it (`shared` + `torrents/downloads` for
/// `/home/anna/smb_shared/torrents/downloads`), else as the share itself.
/// At most [`MAX_TRIES`] shares are tried.
///
/// Returns an empty list if nothing fits. A wrong user or password and an
/// unreachable server are errors, so the caller can say so.
pub fn smb_find(login: &SmbLogin, share: &str, folder: &str) -> Result<Vec<SmbPlace>> {
    let joined = format!("{share}/{folder}");
    let segs = segments(&joined);
    // The server's share list: known names get the server's spelling, and
    // other names (except the user's home share, often not listed) are not
    // worth a connection. If listing is refused, every guess is tried.
    let listing = listed_shares(login);
    let hidden = matches!(&listing, Err(e) if list_refused(e));
    if segs.is_empty() {
        return match listing {
            Ok(_) => Ok(Vec::new()),
            Err(_) if hidden => common_places(login, &segs, &[]),
            Err(e) => Err(login.error(&e)),
        };
    }
    let listed = listing.ok();
    let mut guesses: Vec<(String, String)> = Vec::new();
    for i in 0..segs.len() {
        guesses.push((segs[i].to_owned(), segs[i + 1..].join("/")));
    }
    if let Some(user) = login.user()
        && ["home", "homes", "users"].contains(&segs[0].to_ascii_lowercase().as_str())
        && !segs.iter().any(|s| s.eq_ignore_ascii_case(user))
    {
        guesses.push((user.to_owned(), segs[1..].join("/")));
    }
    guesses.truncate(MAX_GUESSES);
    let mut found: Vec<(SmbPlace, u64)> = Vec::new();
    for (n, (name, rest)) in guesses.iter().cloned().enumerate() {
        if invalid_name(&name) {
            continue;
        }
        let name = match &listed {
            Some(list) => match list.iter().find(|s| s.name.eq_ignore_ascii_case(&name)) {
                Some(s) => s.name.clone(),
                None if n == 0 || login.user().is_some_and(|u| u.eq_ignore_ascii_case(&name)) => {
                    name
                }
                None => continue,
            },
            None => name,
        };
        match pa_vfs_smb::check_folder(&login.config(&name, &rest)) {
            Ok(id) => found.push((
                SmbPlace {
                    share: name,
                    folder: rest,
                },
                id,
            )),
            Err(e) if fatal(&e) => return Err(login.error(&e)),
            Err(_) => {}
        }
    }
    if found.is_empty() && hidden {
        let tried: Vec<&str> = guesses.iter().map(|(n, _)| n.as_str()).collect();
        return common_places(login, &segs, &tried);
    }
    Ok(most_specific(found))
}

/// An error that no other share name will fix: stop guessing.
fn fatal(e: &VfsError) -> bool {
    use pa_vfs::ErrorKind as K;
    matches!(
        e.kind(),
        K::AuthenticationFailed | K::Unavailable | K::TimedOut | K::Unsupported
    )
}

/// Most specific share first (the shortest folder inside it); places with
/// the same server folder id once.
fn most_specific(mut found: Vec<(SmbPlace, u64)>) -> Vec<SmbPlace> {
    found.sort_by_key(|(p, _)| segments(&p.folder).len());
    let mut out: Vec<(SmbPlace, u64)> = Vec::new();
    for (place, id) in found {
        let same = id != 0 && out.iter().any(|(_, other)| *other == id);
        if !same && !out.iter().any(|(p, _)| *p == place) {
            out.push((place, id));
        }
    }
    out.into_iter().map(|(p, _)| p).collect()
}

/// The user's home share and [`COMMON_SHARES`] that open, for a server
/// that hides its share list (UI-031); names already in `tried` are
/// skipped. In each share the deepest tail of the typed path `segs` that
/// is a folder there is taken, else the share itself. One connection per
/// share, [`MAX_TRIES`] shares at most counting `tried`.
fn common_places(login: &SmbLogin, segs: &[&str], tried: &[&str]) -> Result<Vec<SmbPlace>> {
    let mut names: Vec<&str> = Vec::new();
    for name in login.user().into_iter().chain(COMMON_SHARES) {
        let seen = |n: &&str| n.eq_ignore_ascii_case(name);
        if !invalid_name(name) && !tried.iter().any(seen) && !names.iter().any(seen) {
            names.push(name);
        }
    }
    names.truncate(MAX_TRIES.saturating_sub(tried.len()));
    // Tails, deepest first: `a/b/c`, `b/c`, `c`, then the share itself.
    let tails: Vec<String> = (0..=segs.len()).map(|i| segs[i..].join("/")).collect();
    let mut found: Vec<(SmbPlace, u64)> = Vec::new();
    for name in names {
        match pa_vfs_smb::check_folders(&login.config(name, ""), &tails) {
            Ok(ids) => {
                if let Some((tail, id)) = tails
                    .iter()
                    .zip(ids)
                    .find_map(|(t, id)| id.map(|id| (t, id)))
                {
                    found.push((
                        SmbPlace {
                            share: name.to_owned(),
                            folder: tail.clone(),
                        },
                        id,
                    ));
                }
            }
            Err(e) if fatal(&e) => return Err(login.error(&e)),
            Err(_) => {}
        }
    }
    // Where the typed path was found, the other shares are noise.
    if found.iter().any(|(p, _)| !p.folder.is_empty()) {
        found.retain(|(p, _)| !p.folder.is_empty());
    }
    Ok(most_specific(found))
}

impl SourceManager {
    /// The server and logon of SMB source `id`, with its stored password,
    /// for browsing it again ("Choose folder" on a broken source).
    pub fn smb_login_of(&self, id: i64) -> Result<SmbLogin> {
        let stored = self.stored(id)?;
        let s = &stored.source;
        if s.kind != crate::SourceKind::Smb {
            return Err(Error::Invalid(format!("source {id} is not an SMB source")));
        }
        let a = SmbAddress::parse(&s.uri)?;
        Ok(SmbLogin {
            server: a.server,
            username: s.username.clone(),
            password: self.password_of(&stored)?,
            options: s.options.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_accept_both_slashes() {
        assert_eq!(
            segments("home/anna\\smb_shared//x/./"),
            ["home", "anna", "smb_shared", "x"]
        );
        assert!(segments(" / ").is_empty());
    }

    #[test]
    fn login_debug_hides_password() {
        let l = SmbLogin {
            server: "nas".into(),
            username: Some("anna".into()),
            password: Some(SecretString::new("hunter2-browse")),
            options: SourceOptions::default(),
        };
        assert!(!format!("{l:?}").contains("hunter2"));
    }

    #[test]
    fn bad_share_names_are_invalid() {
        let l = SmbLogin {
            server: "nas".into(),
            ..Default::default()
        };
        assert!(matches!(
            smb_list_folder(&l, "a/b", ""),
            Err(Error::Invalid(_))
        ));
    }
}
