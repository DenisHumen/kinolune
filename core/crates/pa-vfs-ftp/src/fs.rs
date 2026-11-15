// SPDX-License-Identifier: MPL-2.0
//! [`FtpFs`]: the [`Vfs`] of one FTP or FTPS source.

use std::fmt;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use pa_vfs::{
    ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Result, Vfs, VfsError, VfsPath,
};
use suppaftp::list::{File, ListParser};

use crate::config::FtpConfig;
use crate::conn::{Conn, Pool, ftp_error};
use crate::source::FtpFile;

/// An FTP or FTPS source (NET-006).
///
/// Listings and metadata use pooled control connections (`MLSD`/`MLST`
/// where the server has them, `LIST`/`SIZE` otherwise). Every opened file
/// streams with `REST` + `RETR` on connections of its own, so a seek costs
/// one new transfer and sequential reads continue the running one.
pub struct FtpFs {
    pool: Arc<Pool>,
}

impl fmt::Debug for FtpFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = &self.pool.cfg;
        f.debug_struct("FtpFs")
            .field("host", &c.host)
            .field("port", &c.port)
            .field("security", &c.security)
            .field("root", &c.root)
            .finish_non_exhaustive()
    }
}

/// The server path of `path` inside the source; refuses line breaks, which
/// would end the FTP command.
pub(crate) fn remote(pool: &Pool, path: &VfsPath) -> Result<String> {
    if path.as_str().contains(['\r', '\n']) {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{path:?}: line breaks are not allowed on FTP"),
        ));
    }
    let root = pool.root();
    Ok(if path.is_root() {
        if root.is_empty() {
            "/".to_owned()
        } else {
            root.to_owned()
        }
    } else {
        format!("{root}{path}")
    })
}

fn metadata_of(f: &File) -> Metadata {
    let kind = if f.is_directory() {
        EntryKind::Dir
    } else if f.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let modified_ms = f
        .modified()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .filter(|&ms| ms > 0);
    Metadata {
        kind,
        size: if kind == EntryKind::File {
            f.size() as u64
        } else {
            0
        },
        modified_ms,
    }
}

/// `stat` of a server path on `conn`.
pub(crate) fn stat_on(conn: &mut Conn, remote: &str) -> Result<Metadata> {
    if conn.mlst {
        let line = conn
            .ftp
            .mlst(Some(remote))
            .map_err(|e| ftp_error(remote, e))?;
        if let Ok(f) = ListParser::parse_mlst(&line) {
            return Ok(metadata_of(&f));
        }
    }
    // SIZE answers for files only (binary mode); CWD tells a directory.
    match conn.ftp.size(remote) {
        Ok(size) => Ok(Metadata {
            kind: EntryKind::File,
            size: size as u64,
            modified_ms: None,
        }),
        Err(e) => {
            let err = ftp_error(remote, e);
            if err.kind() != ErrorKind::NotFound {
                return Err(err);
            }
            match conn.ftp.cwd(remote) {
                Ok(()) => Ok(Metadata {
                    kind: EntryKind::Dir,
                    size: 0,
                    modified_ms: None,
                }),
                Err(e) => Err(ftp_error(remote, e)),
            }
        }
    }
}

impl FtpFs {
    /// A source; does not connect yet.
    pub fn new(cfg: FtpConfig) -> Result<Self> {
        if cfg.host.trim().is_empty() {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                "FTP source has no host",
            ));
        }
        Ok(Self {
            pool: Arc::new(Pool::new(cfg)?),
        })
    }
}

impl Vfs for FtpFs {
    fn list(&self, dir: &VfsPath) -> Result<Vec<DirEntry>> {
        self.pool.with(|pool, conn| {
            let at = remote(pool, dir)?;
            if let Err(e) = conn.ftp.cwd(&at) {
                let err = ftp_error(&at, e);
                if err.kind() == ErrorKind::NotFound && conn.ftp.size(&at).is_ok() {
                    return Err(VfsError::new(
                        ErrorKind::NotADirectory,
                        format!("{at}: not a directory"),
                    ));
                }
                return Err(err);
            }
            let (lines, mlsd) = if conn.mlst {
                (conn.ftp.mlsd(None), true)
            } else {
                (conn.ftp.list(None), false)
            };
            let lines = lines.map_err(|e| ftp_error(&at, e))?;
            let mut out = Vec::with_capacity(lines.len());
            for line in lines {
                let parsed = if mlsd {
                    ListParser::parse_mlsd(&line)
                } else {
                    ListParser::parse_posix(&line).or_else(|_| ListParser::parse_dos(&line))
                };
                // `total 12`, `type=cdir` and lines no parser knows.
                let Ok(f) = parsed else { continue };
                let name = f.name().to_owned();
                if matches!(name.as_str(), "" | "." | "..") || name.contains(['\r', '\n']) {
                    continue;
                }
                let Ok(path) = dir.join(&name) else { continue };
                let mut m = metadata_of(&f);
                if f.is_symlink() {
                    // Follow the link; a dangling one is `Other`.
                    m = remote(pool, &path)
                        .and_then(|r| stat_on(conn, &r))
                        .unwrap_or(Metadata {
                            kind: EntryKind::Other,
                            size: 0,
                            modified_ms: None,
                        });
                }
                out.push(DirEntry {
                    name,
                    path,
                    metadata: m,
                });
            }
            Ok(out)
        })
    }

    fn stat(&self, path: &VfsPath) -> Result<Metadata> {
        self.pool.with(|pool, conn| {
            let at = remote(pool, path)?;
            if path.is_root() {
                // The root is a directory; CWD checks it exists.
                conn.ftp.cwd(&at).map_err(|e| ftp_error(&at, e))?;
                return Ok(Metadata {
                    kind: EntryKind::Dir,
                    size: 0,
                    modified_ms: None,
                });
            }
            stat_on(conn, &at)
        })
    }

    fn open(&self, path: &VfsPath) -> Result<Box<dyn ByteSource>> {
        let m = self.stat(path)?;
        match m.kind {
            EntryKind::File => {}
            EntryKind::Dir => {
                return Err(VfsError::new(
                    ErrorKind::IsADirectory,
                    format!("{path}: is a directory"),
                ));
            }
            _ => {
                return Err(VfsError::new(
                    ErrorKind::Other,
                    format!("{path}: not a regular file"),
                ));
            }
        }
        let at = remote(&self.pool, path)?;
        Ok(Box::new(FtpFile::new(Arc::clone(&self.pool), at, m.size)))
    }
}
