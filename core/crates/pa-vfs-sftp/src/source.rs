// SPDX-License-Identifier: MPL-2.0
//! [`SftpFile`]: an opened SFTP file with pipelined positional reads.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::future::join_all;
use pa_vfs::{ByteSource, Result, VfsPath};

use crate::fs::{Inner, Live, open_handle};
use crate::session::{is_eof, sftp_error, spawn};

/// Largest amount fetched by one `read_at`; bigger buffers get a short read.
const MAX_FETCH: usize = 8 * 1024 * 1024;
/// Read requests in flight at once.
const PIPELINE: usize = 32;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A server handle and the connection generation it belongs to.
type Handle = Option<(u64, String)>;

/// Bytes fetched ahead, starting at `offset`.
struct Block {
    offset: u64,
    data: Vec<u8>,
}

pub(crate) struct SftpFile {
    inner: Arc<Inner>,
    path: VfsPath,
    size: u64,
    read_ahead: usize,
    random: AtomicBool,
    handle: Arc<Mutex<Handle>>,
    block: Mutex<Option<Block>>,
}

impl fmt::Debug for SftpFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SftpFile")
            .field("path", &self.path)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// The handle of `path` on `live`, opening it if the slot holds none or one
/// of an older connection.
async fn handle_on(live: &Live, slot: &Mutex<Handle>, remote: &str) -> Result<String> {
    if let Some((generation, h)) = lock(slot).as_ref()
        && *generation == live.generation
    {
        return Ok(h.clone());
    }
    let h = open_handle(live, remote).await?;
    let old = lock(slot).replace((live.generation, h.clone()));
    if let Some((generation, old)) = old
        && generation == live.generation
        && old != h
    {
        // Another thread opened one at the same time.
        let _ = live.session.sftp.close(old).await;
    }
    Ok(h)
}

impl SftpFile {
    pub(crate) fn new(inner: Arc<Inner>, path: VfsPath, size: u64, read_ahead: usize) -> Self {
        Self {
            inner,
            path,
            size,
            read_ahead,
            random: AtomicBool::new(false),
            handle: Arc::new(Mutex::new(None)),
            block: Mutex::new(None),
        }
    }

    pub(crate) fn ensure_open(&self) -> Result<()> {
        let slot = Arc::clone(&self.handle);
        let path = self.path.clone();
        self.inner.call(move |live| {
            let slot = Arc::clone(&slot);
            let path = path.clone();
            async move {
                let remote = live.session.remote(&path);
                handle_on(&live, &slot, &remote).await.map(|_| ())
            }
        })
    }

    /// Reads up to `len` bytes at `offset` with pipelined requests; stops at
    /// the first short chunk.
    fn fetch(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let slot = Arc::clone(&self.handle);
        let path = self.path.clone();
        self.inner.call(move |live| {
            let slot = Arc::clone(&slot);
            let path = path.clone();
            async move {
                let remote = live.session.remote(&path);
                let handle = handle_on(&live, &slot, &remote).await?;
                let chunk = live.session.read_len.max(1) as usize;
                let mut out = Vec::with_capacity(len);
                'outer: while out.len() < len {
                    let start = offset + out.len() as u64;
                    let left = len - out.len();
                    let sizes: Vec<usize> = (0..left.div_ceil(chunk).min(PIPELINE))
                        .map(|i| chunk.min(left - i * chunk))
                        .collect();
                    let mut at = start;
                    let reads = sizes.iter().map(|&n| {
                        let r = live.session.sftp.read(
                            handle.as_str(),
                            at,
                            u32::try_from(n).unwrap_or(u32::MAX),
                        );
                        at += n as u64;
                        r
                    });
                    for (want, r) in sizes.iter().zip(join_all(reads).await) {
                        match r {
                            Ok(d) => {
                                let got = d.data.len();
                                out.extend_from_slice(&d.data[..got.min(*want)]);
                                if got < *want {
                                    break 'outer;
                                }
                            }
                            Err(e) if is_eof(&e) => break 'outer,
                            Err(e) if out.is_empty() => return Err(sftp_error(&remote, e)),
                            Err(_) => break 'outer,
                        }
                    }
                }
                Ok(out)
            }
        })
    }

    /// Copies from the read-ahead block; `None` on a miss.
    fn read_block(&self, offset: u64, buf: &mut [u8]) -> Option<usize> {
        let block = lock(&self.block);
        let b = block.as_ref()?;
        let end = b.offset + b.data.len() as u64;
        if offset < b.offset || offset >= end {
            return None;
        }
        let from = usize::try_from(offset - b.offset).ok()?;
        let n = buf.len().min(b.data.len() - from);
        buf[..n].copy_from_slice(&b.data[from..from + n]);
        Some(n)
    }
}

impl ByteSource for SftpFile {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() || offset >= self.size {
            return Ok(0);
        }
        let left = usize::try_from(self.size - offset).unwrap_or(usize::MAX);
        let want = buf.len().min(left).min(MAX_FETCH);
        let buf = &mut buf[..want];
        if let Some(n) = self.read_block(offset, buf) {
            return Ok(n);
        }
        let ahead = !self.random.load(Ordering::Relaxed) && want < self.read_ahead;
        let len = if ahead {
            self.read_ahead.min(left)
        } else {
            want
        };
        let data = self.fetch(offset, len)?;
        let n = data.len().min(want);
        buf[..n].copy_from_slice(&data[..n]);
        if ahead && data.len() > n {
            *lock(&self.block) = Some(Block { offset, data });
        }
        Ok(n)
    }

    fn hint_random_access(&self) {
        self.random.store(true, Ordering::Relaxed);
    }
}

impl Drop for SftpFile {
    fn drop(&mut self) {
        let Some((generation, handle)) = lock(&self.handle).take() else {
            return;
        };
        if let Some(live) = self.inner.current()
            && live.generation == generation
        {
            spawn(async move {
                let _ = live.session.sftp.close(handle).await;
            });
        }
    }
}
