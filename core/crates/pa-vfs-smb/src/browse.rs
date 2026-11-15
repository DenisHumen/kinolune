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

/// Shares of `config`'s server, logging on with its credentials; the share
/// and folder of `config` are ignored. Only folders a person would pick are
/// returned (no printers, `IPC$`, `C$`, `ADMIN$`, `print$`), in the
/// server's order.
///
/// Errors as for connecting: [`ErrorKind::AuthenticationFailed`] for a
/// wrong user or password, [`ErrorKind::Unavailable`] or
/// [`ErrorKind::TimedOut`] for a server that does not answer,
/// [`ErrorKind::PermissionDenied`] if the server does not let this user
/// list its shares.
pub fn list_shares(config: &SmbConfig) -> Result<Vec<ShareInfo>> {
    let lib = lib_of(config)?;
    let mut ipc = config.clone();
    ipc.share = "IPC$".into();
    ipc.root.clear();
    let pool = Pool::new(lib, ipc);
    let mut conn = pool.connect()?;
    let url = format!("smb://{}/", config.server);
    let shares = conn.ctx.list_shares().map_err(|e| map_error(&url, &e))?;
    Ok(shares
        .into_iter()
        .filter(ShareInfo::is_user_folder)
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
