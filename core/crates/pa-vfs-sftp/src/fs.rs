// SPDX-License-Identifier: MPL-2.0
//! [`SftpFs`]: the [`Vfs`] of one SFTP source.

use std::fmt;
use std::future::Future;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::future::join_all;
use pa_vfs::{
    ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Result, Vfs, VfsError, VfsPath, VfsWrite,
};
use russh_sftp::protocol::{FileAttributes, OpenFlags};

use crate::config::{HostKey, HostKeyTrust, SftpConfig};
use crate::session::{self, Session, block, is_eof, sftp_error};
use crate::source::SftpFile;

/// The current connection and its generation number: handles of files are
/// bound to the generation that opened them.
pub(crate) struct Live {
    pub(crate) generation: u64,
    pub(crate) session: Session,
}

pub(crate) struct Inner {
    cfg: Arc<SftpConfig>,
    live: Mutex<Option<Arc<Live>>>,
    /// Serializes connecting, so concurrent callers share one connection.
    connecting: Mutex<()>,
    generation: AtomicU64,
    /// The pinned key: configured, or learned on the first connect.
    pin: Mutex<Option<HostKey>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    pub(crate) fn current(&self) -> Option<Arc<Live>> {
        lock(&self.live).clone()
    }

    /// The connection, connecting if there is none.
    pub(crate) fn live(&self) -> Result<Arc<Live>> {
        if let Some(live) = self.current() {
            return Ok(live);
        }
        let _connecting = lock(&self.connecting);
        if let Some(live) = self.current() {
            return Ok(live);
        }
        let expected = lock(&self.pin).clone();
        let (session, key) = block(session::connect(Arc::clone(&self.cfg), expected.clone()))?;
        if expected.is_none() {
            *lock(&self.pin) = Some(key.clone());
            if let HostKeyTrust::FirstUseWith(store) = &self.cfg.host_key {
                store(&key);
            }
        }
        let live = Arc::new(Live {
            generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
            session,
        });
        *lock(&self.live) = Some(Arc::clone(&live));
        Ok(live)
    }

    /// Forgets the connection of `generation` after a connection error.
    pub(crate) fn drop_live(&self, generation: u64) {
        let mut live = lock(&self.live);
        if live.as_ref().is_some_and(|l| l.generation == generation) {
            *live = None;
        }
    }

    /// Runs `op` on the connection; on a connection error reconnects and
    /// tries once more (the server closed an idle connection, the NAS
    /// slept, Wi-Fi changed).
    pub(crate) fn call<T, F, Fut>(&self, op: F) -> Result<T>
    where
        T: Send + 'static,
        F: Fn(Arc<Live>) -> Fut,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let live = self.live()?;
        let generation = live.generation;
        match block(op(live)) {
            Err(e) if e.kind().is_transient() => {
                self.drop_live(generation);
                block(op(self.live()?))
            }
            r => r,
        }
    }
}

/// An SFTP source (NET-006): a folder on an SSH server.
///
/// One SSH connection with one SFTP channel serves every call and every
/// opened file; requests are pipelined on it. The connection opens on the
/// first call and reopens after a network error. The host key is checked
/// against [`SftpConfig::host_key`] before anything is sent.
pub struct SftpFs {
    inner: Arc<Inner>,
}

impl fmt::Debug for SftpFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SftpFs")
            .field("host", &self.inner.cfg.host)
            .field("port", &self.inner.cfg.port)
            .field("root", &self.inner.cfg.root)
            .field("username", &self.inner.cfg.username)
            .finish_non_exhaustive()
    }
}

impl SftpFs {
    /// A source; does not connect yet.
    pub fn new(cfg: SftpConfig) -> Result<Self> {
        if cfg.host.trim().is_empty() {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                "SFTP source has no host",
            ));
        }
        let pin = match &cfg.host_key {
            HostKeyTrust::Pinned(k) => Some(k.clone()),
            _ => None,
        };
        Ok(Self {
            inner: Arc::new(Inner {
                cfg: Arc::new(cfg),
                live: Mutex::new(None),
                connecting: Mutex::new(()),
                generation: AtomicU64::new(0),
                pin: Mutex::new(pin),
            }),
        })
    }

    /// The host key in use: the pinned one, or the one learned on the first
    /// connect; `None` before the first connect of a trust-on-first-use
    /// source.
    #[must_use]
    pub fn host_key(&self) -> Option<HostKey> {
        lock(&self.inner.pin).clone()
    }

    /// Connects now (if not connected yet) and returns the server's host key.
    pub fn connect(&self) -> Result<HostKey> {
        self.inner.live()?;
        self.host_key()
            .ok_or_else(|| VfsError::new(ErrorKind::Other, "no host key"))
    }
}

/// Metadata from SFTP attributes (symbolic links are resolved by the caller).
pub(crate) fn metadata(a: &FileAttributes) -> Metadata {
    let t = a.file_type();
    let kind = if t.is_dir() {
        EntryKind::Dir
    } else if t.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Metadata {
        kind,
        size: if kind == EntryKind::File {
            a.size.unwrap_or(0)
        } else {
            0
        },
        modified_ms: a.mtime.map(|t| i64::from(t) * 1000),
    }
}

/// `stat` (follows symbolic links) of a remote path.
async fn stat_remote(live: &Live, remote: &str) -> Result<Metadata> {
    live.session
        .sftp
        .stat(remote)
        .await
        .map(|a| metadata(&a.attrs))
        .map_err(|e| sftp_error(remote, e))
}

impl Vfs for SftpFs {
    fn list(&self, dir: &VfsPath) -> Result<Vec<DirEntry>> {
        let dir = dir.clone();
        self.inner.call(|live| {
            let dir = dir.clone();
            async move {
                let remote = live.session.remote(&dir);
                let sftp = &live.session.sftp;
                let handle = match sftp.opendir(remote.as_str()).await {
                    Ok(h) => h.handle,
                    Err(e) => {
                        let err = sftp_error(&remote, e);
                        // A file reads as "no such file" or a generic failure,
                        // depending on the server.
                        if stat_remote(&live, &remote).await.is_ok_and(|m| !m.is_dir()) {
                            return Err(VfsError::new(
                                ErrorKind::NotADirectory,
                                format!("{remote}: not a directory"),
                            ));
                        }
                        return Err(err);
                    }
                };
                let mut files = Vec::new();
                let read = loop {
                    match sftp.readdir(handle.as_str()).await {
                        Ok(name) => files.extend(name.files),
                        Err(e) if is_eof(&e) => break Ok(()),
                        Err(e) => break Err(sftp_error(&remote, e)),
                    }
                };
                let _ = sftp.close(handle).await;
                read?;
                let mut out = Vec::with_capacity(files.len());
                for f in files {
                    if matches!(f.filename.as_str(), "." | "..") {
                        continue;
                    }
                    let Ok(path) = dir.join(&f.filename) else {
                        continue;
                    };
                    let mut m = metadata(&f.attrs);
                    if f.attrs.file_type().is_symlink() {
                        // Follow the link; a dangling one is `Other`.
                        m = stat_remote(&live, &live.session.remote(&path))
                            .await
                            .unwrap_or(Metadata {
                                kind: EntryKind::Other,
                                size: 0,
                                modified_ms: None,
                            });
                    }
                    out.push(DirEntry {
                        name: f.filename,
                        path,
                        metadata: m,
                    });
                }
                Ok(out)
            }
        })
    }

    fn stat(&self, path: &VfsPath) -> Result<Metadata> {
        let path = path.clone();
        self.inner.call(|live| {
            let path = path.clone();
            async move {
                let remote = live.session.remote(&path);
                stat_remote(&live, &remote).await
            }
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
        let file = SftpFile::new(
            Arc::clone(&self.inner),
            path.clone(),
            m.size,
            self.inner.cfg.read_ahead,
        );
        // Open the handle now, so a file that cannot be read fails here.
        file.ensure_open()?;
        Ok(Box::new(file))
    }

    fn as_write(&self) -> Option<&dyn VfsWrite> {
        Some(self)
    }
}

/// Size of one write request: every server takes 32 KiB (draft-ietf-secsh-filexfer).
const WRITE_LEN: usize = 32 * 1024;
/// Write requests in flight at once.
const WRITES_IN_FLIGHT: usize = 32;

impl SftpFs {
    /// Server path of an entry a write creates or removes (never the root).
    fn write_target(path: &VfsPath) -> Result<()> {
        if path.is_root() {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                "the source root cannot be written",
            ));
        }
        Ok(())
    }

    /// Runs one path operation (no retry: a write may have happened).
    fn path_call<F, Fut>(&self, path: &VfsPath, op: F) -> Result<()>
    where
        F: FnOnce(Arc<Live>, String) -> Fut,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self::write_target(path)?;
        let live = self.inner.live()?;
        let generation = live.generation;
        let remote = live.session.remote(path);
        let r = block(op(live, remote));
        if r.as_ref().is_err_and(|e| e.kind().is_transient()) {
            self.inner.drop_live(generation);
        }
        r
    }
}

impl VfsWrite for SftpFs {
    fn create_dir(&self, path: &VfsPath) -> Result<()> {
        self.path_call(path, |live, remote| async move {
            match live
                .session
                .sftp
                .mkdir(remote.as_str(), FileAttributes::empty())
                .await
            {
                Ok(_) => Ok(()),
                Err(e) => {
                    let err = sftp_error(&remote, e);
                    // OpenSSH answers a generic failure for an existing name.
                    if !err.kind().is_transient() && stat_remote(&live, &remote).await.is_ok() {
                        return Err(VfsError::new(ErrorKind::AlreadyExists, remote));
                    }
                    Err(err)
                }
            }
        })
    }

    fn write_file(
        &self,
        path: &VfsPath,
        offset: u64,
        data: &mut dyn Read,
        len: u64,
    ) -> Result<u64> {
        Self::write_target(path)?;
        let live = self.inner.live()?;
        let generation = live.generation;
        let remote = live.session.remote(path);
        let r = write_on(&live, &remote, offset, data, len);
        if r.as_ref().is_err_and(|e| e.kind().is_transient()) {
            self.inner.drop_live(generation);
        }
        r
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        Self::write_target(to)?;
        let to = to.clone();
        self.path_call(from, move |live, remote| async move {
            let target = live.session.remote(&to);
            live.session
                .sftp
                .rename(remote.as_str(), target.as_str())
                .await
                .map(|_| ())
                .map_err(|e| sftp_error(&remote, e))
        })
    }

    fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.path_call(path, |live, remote| async move {
            live.session
                .sftp
                .remove(remote.as_str())
                .await
                .map(|_| ())
                .map_err(|e| sftp_error(&remote, e))
        })
    }

    fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.path_call(path, |live, remote| async move {
            live.session
                .sftp
                .rmdir(remote.as_str())
                .await
                .map(|_| ())
                .map_err(|e| sftp_error(&remote, e))
        })
    }

    fn is_symlink(&self, path: &VfsPath) -> Result<bool> {
        let path = path.clone();
        self.inner.call(|live| {
            let path = path.clone();
            async move {
                let remote = live.session.remote(&path);
                live.session
                    .sftp
                    .lstat(remote.as_str())
                    .await
                    .map(|a| a.attrs.file_type().is_symlink())
                    .map_err(|e| sftp_error(&remote, e))
            }
        })
    }
}

/// Opens `remote` for writing, sends `len` bytes of `data` from `offset`
/// in pipelined requests and closes it.
fn write_on(
    live: &Arc<Live>,
    remote: &str,
    offset: u64,
    data: &mut dyn Read,
    len: u64,
) -> Result<u64> {
    let flags = if offset == 0 {
        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE
    } else {
        OpenFlags::WRITE
    };
    let handle = {
        let live = Arc::clone(live);
        let remote = remote.to_owned();
        block(async move {
            if offset > 0 {
                let size = stat_remote(&live, &remote).await?.size;
                if size != offset {
                    return Err(VfsError::new(
                        ErrorKind::Other,
                        format!("{remote}: {size} bytes, cannot append at {offset}"),
                    ));
                }
            }
            live.session
                .sftp
                .open(remote.as_str(), flags, FileAttributes::empty())
                .await
                .map(|h| h.handle)
                .map_err(|e| sftp_error(&remote, e))
        })?
    };
    let r = (|| {
        let mut done = 0u64;
        while done < len {
            let mut batch = Vec::with_capacity(WRITES_IN_FLIGHT);
            let mut at = offset + done;
            let mut ended = false;
            while batch.len() < WRITES_IN_FLIGHT && done < len {
                let want = usize::try_from(len - done)
                    .unwrap_or(usize::MAX)
                    .min(WRITE_LEN);
                let mut chunk = vec![0u8; want];
                let n = fill(data, &mut chunk)?;
                if n == 0 {
                    ended = true;
                    break;
                }
                chunk.truncate(n);
                batch.push((at, chunk));
                at += n as u64;
                done += n as u64;
            }
            if !batch.is_empty() {
                let live = Arc::clone(live);
                let handle = handle.clone();
                let remote = remote.to_owned();
                block(async move {
                    let writes = batch
                        .into_iter()
                        .map(|(at, chunk)| live.session.sftp.write(handle.as_str(), at, chunk));
                    for r in join_all(writes).await {
                        r.map_err(|e| sftp_error(&remote, e))?;
                    }
                    Ok(())
                })?;
            }
            if ended {
                break;
            }
        }
        Ok(done)
    })();
    // Close even after an error: the server keeps what was written.
    let closed = {
        let live = Arc::clone(live);
        let remote = remote.to_owned();
        block(async move {
            live.session
                .sftp
                .close(handle)
                .await
                .map(|_| ())
                .map_err(|e| sftp_error(&remote, e))
        })
    };
    let done = r?;
    closed?;
    Ok(done)
}

/// Reads until `buf` is full or `data` ends.
fn fill(data: &mut dyn Read, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match data.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            // The data failed, not the connection: not a transient error.
            Err(e) => {
                return Err(
                    VfsError::new(ErrorKind::Other, format!("upload data: {e}")).with_source(e)
                );
            }
        }
    }
    Ok(n)
}

/// Opens `remote` for reading on `live`.
pub(crate) async fn open_handle(live: &Live, remote: &str) -> Result<String> {
    live.session
        .sftp
        .open(remote, OpenFlags::READ, FileAttributes::empty())
        .await
        .map(|h| h.handle)
        .map_err(|e| sftp_error(remote, e))
}
