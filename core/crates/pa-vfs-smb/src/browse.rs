// SPDX-License-Identifier: MPL-2.0
//! Helpers for picking a folder on a server (UI-028): the list of its
//! shares and a one-shot check of a folder inside a share.

use pa_smb2::{FileType, ShareInfo, Smb2Lib};
use pa_vfs::{ErrorKind, Result, VfsError};

use crate::config::SmbConfig;
use crate::pool::{Pool, map_error};

fn lib_of(config: &SmbConfig) -> Result<&'static Smb2Lib> {
    match config.lib {
        Some(lib) => Ok(lib),
        None => Smb2Lib::shared().map_err(|e| map_error("libsmb2", &e)),
    }
}

/// Marker in the message of the error of [`list_shares`] when the server
/// let the user in but would not tell its shares (UI-031): the login is
/// fine, the share name has to be typed or guessed.
pub const SHARE_LIST_REFUSED: &str = "share list refused";

/// Shares of `config`'s server, logging on with its credentials; the share
/// and folder of `config` are ignored. Only folders a person would pick are
/// returned (no printers, `IPC$`, `C$`, `ADMIN$`, `print$`), in the
/// server's order.
///
/// Errors as for connecting: [`ErrorKind::AuthenticationFailed`] for a
/// wrong user or password, [`ErrorKind::Unavailable`] or
/// [`ErrorKind::TimedOut`] for a server that does not answer. A server
/// that accepts the login but refuses the listing (denies `IPC$` to the
/// user, drops the srvsvc request) gives [`ErrorKind::PermissionDenied`]
/// with [`SHARE_LIST_REFUSED`] in the message.
pub fn list_shares(config: &SmbConfig) -> Result<Vec<ShareInfo>> {
    let lib = lib_of(config)?;
    let mut ipc = config.clone();
    ipc.share = "IPC$".into();
    ipc.root.clear();
    let pool = Pool::new(lib, ipc);
    let url = format!("smb://{}/", config.server);
    let named = config.credentials.is_some();
    let mut conn = pool.connect().map_err(|e| listing_error(false, named, e))?;
    let shares = conn
        .ctx
        .list_shares()
        .map_err(|e| listing_error(true, named, map_error(&url, &e)))?;
    Ok(shares
        .into_iter()
        .filter(ShareInfo::is_user_folder)
        .collect())
}

/// Tells "the login is fine, only the listing is refused" apart from a
/// login or network error (UI-031). `connected`: the logon and the `IPC$`
/// tree connect worked and the share enumeration itself failed (the
/// owner's Samba drops it, `ShareEnum failed … errno 32`, which libsmb2
/// reports like a lost server). `named`: a user logged on; for a guest a
/// refused `IPC$` means "no guests here" and stays as it is.
fn listing_error(connected: bool, named: bool, e: VfsError) -> VfsError {
    // A wrong password or a dead server fails the session setup; a denied
    // or missing `IPC$` comes after a successful logon.
    let refused = connected
        || (named && matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::NotFound));
    if !refused || e.kind() == ErrorKind::AuthenticationFailed {
        return e;
    }
    VfsError::new(
        ErrorKind::PermissionDenied,
        format!("{SHARE_LIST_REFUSED}: {}", e.message()),
    )
}

/// Connects once to `config`'s share and checks which of `folders` (inside
/// it, `/`-separated, `""` for the share itself) are folders: the server's
/// id of each that is (as [`check_folder`]), `None` for the others. The
/// folder of `config` is ignored. Connection errors (no such share, wrong
/// login) are errors.
pub fn check_folders(config: &SmbConfig, folders: &[String]) -> Result<Vec<Option<u64>>> {
    let lib = lib_of(config)?;
    let mut share = config.clone();
    share.root.clear();
    let pool = Pool::new(lib, share);
    let mut conn = pool.connect()?;
    Ok(folders
        .iter()
        .map(|f| {
            conn.ctx
                .stat(f.trim_matches('/'))
                .ok()
                .filter(|st| st.kind == FileType::Dir)
                .map(|st| st.ino)
        })
        .collect())
}

/// Connects to `config`'s share and checks that its folder
/// ([`SmbConfig::with_root`]) is a folder. Returns the server's id of the
/// folder (Samba: the inode; 0 if the server reports none), which tells
/// two shares that lead to the same folder apart from different ones.
pub fn check_folder(config: &SmbConfig) -> Result<u64> {
    let lib = lib_of(config)?;
    let pool = Pool::new(lib, config.clone());
    let mut conn = pool.connect()?;
    let url = format!("smb://{}/{}/{}", config.server, config.share, config.root);
    let st = conn
        .ctx
        .stat(&config.root)
        .map_err(|e| map_error(&url, &e))?;
    if st.kind != FileType::Dir {
        return Err(VfsError::new(ErrorKind::NotADirectory, url));
    }
    Ok(st.ino)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(kind: ErrorKind) -> VfsError {
        VfsError::new(kind, "smb://nas/: something")
    }

    fn refused(e: &VfsError) -> bool {
        e.message().contains(SHARE_LIST_REFUSED)
    }

    #[test]
    fn dropped_share_enum_after_login_is_a_refused_listing() {
        // The owner's Samba: logon and IPC$ fine, then `ShareEnum failed:
        // libsmb2 returned 0 (errno 32)`, which maps to "unavailable".
        for kind in [
            ErrorKind::Unavailable,
            ErrorKind::TimedOut,
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
        ] {
            let e = listing_error(true, true, err(kind));
            assert_eq!(e.kind(), ErrorKind::PermissionDenied, "{kind:?}");
            assert!(refused(&e), "{e}");
        }
    }

    #[test]
    fn denied_ipc_for_a_user_is_a_refused_listing() {
        for kind in [ErrorKind::PermissionDenied, ErrorKind::NotFound] {
            let e = listing_error(false, true, err(kind));
            assert_eq!(e.kind(), ErrorKind::PermissionDenied);
            assert!(refused(&e), "{e}");
        }
    }

    #[test]
    fn login_and_network_errors_stay() {
        for kind in [
            ErrorKind::AuthenticationFailed,
            ErrorKind::Unavailable,
            ErrorKind::TimedOut,
            ErrorKind::Unsupported,
            ErrorKind::Other,
        ] {
            let e = listing_error(false, true, err(kind));
            assert_eq!(e.kind(), kind);
            assert!(!refused(&e), "{e}");
        }
        // A guest kept out of IPC$ is a refused guest, not a hidden list.
        let e = listing_error(false, false, err(ErrorKind::PermissionDenied));
        assert!(!refused(&e), "{e}");
        // A wrong password reported late is still a wrong password.
        let e = listing_error(true, true, err(ErrorKind::AuthenticationFailed));
        assert_eq!(e.kind(), ErrorKind::AuthenticationFailed);
    }
}
