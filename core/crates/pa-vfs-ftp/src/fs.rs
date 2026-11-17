// SPDX-License-Identifier: MPL-2.0
//! [`FtpFs`]: the [`Vfs`] of one FTP or FTPS source.

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use pa_vfs::{
    ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Result, Vfs, VfsError, VfsPath,
    VfsWrite,
};
use suppaftp::FtpError;
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

    fn as_write(&self) -> Option<&dyn VfsWrite> {
        Some(self)
    }
}

/// Upload buffer.
const WRITE_BUF: usize = 256 * 1024;

/// Which write a 550/553 reply refused (the code alone does not say why).
#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteOp {
    Mkdir,
    Store,
    Remove,
}

/// Maps the error of a write command on `at`: `550`/`553` mean "exists",
/// "no permission" or "missing" depending on the command and on what is
/// there now, `552` a full disk.
fn write_error(conn: &mut Conn, at: &str, op: WriteOp, e: FtpError) -> VfsError {
    let code = match &e {
        FtpError::UnexpectedResponse(r) => Some(r.status.code()),
        _ => None,
    };
    let err = ftp_error(at, e);
    let kind = match code {
        Some(552) => ErrorKind::StorageFull,
        Some(550 | 553) => {
            let exists = stat_on(conn, at).is_ok();
            match (op, exists) {
                (WriteOp::Mkdir, true) => ErrorKind::AlreadyExists,
                (WriteOp::Remove, false) => ErrorKind::NotFound,
                _ => ErrorKind::PermissionDenied,
            }
        }
        _ => return err,
    };
    VfsError::new(kind, err.message().to_owned())
}

/// Server path of an entry a write creates or removes (never the root).
fn write_remote(pool: &crate::conn::Pool, path: &VfsPath) -> Result<String> {
    if path.is_root() {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            "the source root cannot be written",
        ));
    }
    remote(pool, path)
}

impl VfsWrite for FtpFs {
    fn create_dir(&self, path: &VfsPath) -> Result<()> {
        self.pool.with(|pool, conn| {
            let at = write_remote(pool, path)?;
            conn.ftp
                .mkdir(&at)
                .map_err(|e| write_error(conn, &at, WriteOp::Mkdir, e))
        })
    }

    fn write_file(&self, path: &VfsPath, offset: u64, data: &mut dyn Read, len: u64) -> Result<u64> {
        // A connection of its own: the transfer cannot be retried, and a
        // broken one is not handed back to the pool.
        let mut conn = self.pool.connect()?;
        let at = write_remote(&self.pool, path)?;
        if offset > 0 {
            let size = conn.ftp.size(&at).map_err(|e| ftp_error(&at, e))? as u64;
            if size != offset {
                return Err(VfsError::new(
                    ErrorKind::Other,
                    format!("{at}: {size} bytes, cannot append at {offset}"),
                ));
            }
        }
        let opened = if offset == 0 {
            conn.ftp.put_with_stream(&at)
        } else {
            conn.ftp.append_with_stream(&at)
        };
        let mut stream = match opened {
            Ok(s) => s,
            Err(e) => return Err(write_error(&mut conn, &at, WriteOp::Store, e)),
        };
        let mut buf = vec![0u8; WRITE_BUF];
        let mut done = 0u64;
        while done < len {
            let want = usize::try_from(len - done).unwrap_or(usize::MAX).min(buf.len());
            let n = match data.read(&mut buf[..want]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // The data failed: the server keeps what it got (the
                // dropped stream ends the transfer), the connection goes.
                Err(e) => {
                    return Err(VfsError::new(ErrorKind::Other, format!("upload data: {e}"))
                        .with_source(e));
                }
            };
            stream
                .write_all(&buf[..n])
                .map_err(|e| VfsError::from_io(e, &at))?;
            done += n as u64;
        }
        stream.finish().map_err(|e| ftp_error(&at, e))?;
        self.pool.put(conn);
        Ok(done)
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        self.pool.with(|pool, conn| {
            let a = write_remote(pool, from)?;
            let b = write_remote(pool, to)?;
            // RNTO replaces an existing file on most servers: refuse first.
            if stat_on(conn, &b).is_ok() {
                return Err(VfsError::new(ErrorKind::AlreadyExists, b));
            }
            conn.ftp
                .rename(&a, &b)
                .map_err(|e| write_error(conn, &a, WriteOp::Remove, e))
        })
    }

    fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.pool.with(|pool, conn| {
            let at = write_remote(pool, path)?;
            conn.ftp
                .rm(&at)
                .map_err(|e| write_error(conn, &at, WriteOp::Remove, e))
        })
    }

    fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.pool.with(|pool, conn| {
            let at = write_remote(pool, path)?;
            conn.ftp
                .rmdir(&at)
                .map_err(|e| write_error(conn, &at, WriteOp::Remove, e))
        })
    }

    fn is_symlink(&self, path: &VfsPath) -> Result<bool> {
        let Some(parent) = path.parent() else {
            return Ok(false);
        };
        let name = path.file_name().unwrap_or_default();
        self.pool.with(|pool, conn| {
            let at = remote(pool, &parent)?;
            conn.ftp.cwd(&at).map_err(|e| ftp_error(&at, e))?;
            let lines = if conn.mlst {
                conn.ftp.mlsd(None)
            } else {
                conn.ftp.list(None)
            }
            .map_err(|e| ftp_error(&at, e))?;
            for line in lines {
                let parsed = if conn.mlst {
                    ListParser::parse_mlsd(&line)
                } else {
                    ListParser::parse_posix(&line).or_else(|_| ListParser::parse_dos(&line))
                };
                if let Ok(f) = parsed
                    && f.name() == name
                {
                    return Ok(f.is_symlink());
                }
            }
            Ok(false)
        })
    }
}
