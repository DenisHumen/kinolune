// SPDX-License-Identifier: MPL-2.0
//! [`FtpFile`]: an opened FTP file read with `REST` + `RETR`.

use std::fmt;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pa_vfs::{ByteSource, Result, VfsError};
use suppaftp::{FtpError, TransferStream};

use crate::conn::{Conn, Pool, Tls, ftp_error, lock};

/// Largest amount read by one `read_at`; bigger buffers get a short read.
const MAX_READ: usize = 8 * 1024 * 1024;
/// Positioned transfers kept per file: a player reads the stream and,
/// now and then, an index or the other end of the file.
const KEPT_READERS: usize = 2;

/// A control connection with a running download at `pos`.
struct Reader {
    conn: Conn,
    stream: Option<TransferStream<Tls>>,
    pos: u64,
}

impl Reader {
    fn new(conn: Conn) -> Self {
        Self {
            conn,
            stream: None,
            pos: 0,
        }
    }

    /// Ends the running transfer by closing its data connection and reading
    /// the server's reply (`226`, or `426` for a cut transfer). `ABOR`
    /// is not used: a transfer that already ended gets an extra reply
    /// (`225 No transfer to ABOR`) on some servers and the control
    /// connection goes out of step. `false` if the connection is unusable.
    fn stop(&mut self) -> bool {
        match self.stream.take().map(TransferStream::finish) {
            None | Some(Ok(())) | Some(Err(FtpError::UnexpectedResponse(_))) => true,
            Some(Err(_)) => false,
        }
    }

    /// Reads up to `len` bytes at `offset`, restarting the transfer with
    /// `REST` if it is elsewhere.
    fn read(
        &mut self,
        pool: &Pool,
        remote: &str,
        size: u64,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        if self.stream.is_none() || self.pos != offset {
            if !self.stop() {
                self.conn = pool.connect()?;
            }
            let rest = usize::try_from(offset).map_err(|_| {
                VfsError::new(
                    pa_vfs::ErrorKind::Other,
                    format!("{remote}: offset too large"),
                )
            })?;
            self.conn
                .ftp
                .resume_transfer(rest)
                .map_err(|e| ftp_error(remote, e))?;
            let stream = self
                .conn
                .ftp
                .retr_as_stream(remote)
                .map_err(|e| ftp_error(remote, e))?;
            self.stream = Some(stream);
            self.pos = offset;
        }
        let Some(stream) = self.stream.as_mut() else {
            return Ok(Vec::new());
        };
        let mut out = vec![0u8; len];
        let mut filled = 0;
        let mut eof = false;
        while filled < len {
            match stream.read(&mut out[filled..]) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    self.stream = None;
                    return Err(VfsError::from_io(e, remote));
                }
            }
        }
        out.truncate(filled);
        self.pos += filled as u64;
        if eof || self.pos >= size {
            // Completes the transfer (226) so the connection takes the next
            // command right away.
            if !self.stop() {
                self.conn = pool.connect()?;
            }
        }
        Ok(out)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Bytes read ahead, starting at `offset`.
struct Block {
    offset: u64,
    data: Vec<u8>,
}

pub(crate) struct FtpFile {
    pool: Arc<Pool>,
    remote: String,
    size: u64,
    random: AtomicBool,
    readers: Mutex<Vec<Reader>>,
    block: Mutex<Option<Block>>,
}

impl fmt::Debug for FtpFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FtpFile")
            .field("remote", &self.remote)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl FtpFile {
    pub(crate) fn new(pool: Arc<Pool>, remote: String, size: u64) -> Self {
        Self {
            pool,
            remote,
            size,
            random: AtomicBool::new(false),
            readers: Mutex::new(Vec::new()),
            block: Mutex::new(None),
        }
    }

    /// A reader positioned at `offset` if one is, else any kept one.
    fn take_reader(&self, offset: u64) -> Option<Reader> {
        let mut readers = lock(&self.readers);
        let i = readers
            .iter()
            .position(|r| r.stream.is_some() && r.pos == offset)
            .or_else(|| readers.len().checked_sub(1))?;
        Some(readers.swap_remove(i))
    }

    fn keep(&self, reader: Reader) {
        let mut readers = lock(&self.readers);
        if readers.len() < KEPT_READERS {
            readers.push(reader);
        }
    }

    fn read_range(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let kept = self.take_reader(offset);
        let reused = kept.is_some();
        let mut reader = match kept {
            Some(r) => r,
            None => Reader::new(self.pool.connect()?),
        };
        match reader.read(&self.pool, &self.remote, self.size, offset, len) {
            Ok(data) => {
                self.keep(reader);
                Ok(data)
            }
            // A kept connection the server dropped while idle.
            Err(e) if reused && e.kind().is_transient() => {
                drop(reader);
                let mut reader = Reader::new(self.pool.connect()?);
                let data = reader.read(&self.pool, &self.remote, self.size, offset, len)?;
                self.keep(reader);
                Ok(data)
            }
            Err(e) => Err(e),
        }
    }

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

impl ByteSource for FtpFile {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() || offset >= self.size {
            return Ok(0);
        }
        let left = usize::try_from(self.size - offset).unwrap_or(usize::MAX);
        let want = buf.len().min(left).min(MAX_READ);
        let buf = &mut buf[..want];
        if let Some(n) = self.read_block(offset, buf) {
            return Ok(n);
        }
        let read_ahead = self.pool.cfg.read_ahead;
        let ahead = !self.random.load(Ordering::Relaxed) && want < read_ahead;
        let len = if ahead { read_ahead.min(left) } else { want };
        let data = self.read_range(offset, len)?;
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
