// SPDX-License-Identifier: MPL-2.0
//! A file behind an HTTP link, read with byte ranges.

use std::fmt;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pa_vfs::{ByteSource, ErrorKind, VfsError};
use url::Url;

use crate::client::{HttpClient, host, start_of_content_range, status_error, transport};

/// Bytes one ranged request asks for while playing: big enough for a
/// smooth stream, small enough that a stalled request ends.
pub const CHUNK: u64 = 8 * 1024 * 1024;

/// Bytes asked for after [`ByteSource::hint_random_access`] (trickplay,
/// probing).
const RANDOM_CHUNK: u64 = 256 * 1024;

/// A server that ignores `Range` sends the file from the start; this many
/// bytes are skipped to reach an offset before giving up.
const MAX_SKIP: u64 = 64 * 1024 * 1024;

/// An open response body and where it is in the file.
struct Stream {
    pos: u64,
    end: u64,
    body: ureq::BodyReader<'static>,
}

/// A file behind an HTTP(S) link: a fixed size and positional reads.
///
/// Sequential reads continue one ranged response (`Range: bytes=a-b`, at
/// most [`CHUNK`] bytes each); a seek starts a new one. A server without
/// range support works for reading from the start, and for seeks up to 64
/// MiB in. Every connection goes through the client's link policy.
pub struct HttpFile {
    client: Arc<HttpClient>,
    url: Url,
    size: u64,
    random: AtomicBool,
    stream: Mutex<Option<Stream>>,
}

impl fmt::Debug for HttpFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The URL may carry a token: only the host.
        f.debug_struct("HttpFile")
            .field("host", &host(&self.url))
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl HttpFile {
    pub(crate) fn new(client: Arc<HttpClient>, url: Url, size: u64) -> Self {
        Self {
            client,
            url,
            size,
            random: AtomicBool::new(false),
            stream: Mutex::new(None),
        }
    }

    /// Starts a response at `offset`.
    fn start(&self, offset: u64, want: usize) -> pa_vfs::Result<Stream> {
        let chunk = if self.random.load(Ordering::Relaxed) {
            RANDOM_CHUNK.max(want as u64)
        } else {
            CHUNK.max(want as u64)
        };
        let end = self.size.min(offset.saturating_add(chunk));
        let resp = self
            .client
            .agent()
            .get(self.url.as_str())
            .header("range", format!("bytes={offset}-{}", end - 1))
            .header("accept-encoding", "identity")
            .call()
            .map_err(|e| transport(&self.url, &e))?;
        match resp.status().as_u16() {
            206 => {
                let start = resp
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(start_of_content_range);
                if start.is_some_and(|s| s != offset) {
                    return Err(VfsError::new(
                        ErrorKind::Other,
                        format!("{}: the server sent another range", host(&self.url)),
                    ));
                }
                Ok(Stream {
                    pos: offset,
                    end,
                    body: resp.into_body().into_reader(),
                })
            }
            200 => {
                if offset > MAX_SKIP {
                    return Err(VfsError::new(
                        ErrorKind::Unsupported,
                        format!("{}: the server cannot seek (no byte ranges)", host(&self.url)),
                    ));
                }
                let mut body = resp.into_body().into_reader();
                let skipped = io::copy(&mut (&mut body).take(offset), &mut io::sink())
                    .map_err(|e| VfsError::from_io(e, host(&self.url)))?;
                if skipped < offset {
                    return Err(early_end(&self.url));
                }
                Ok(Stream {
                    pos: offset,
                    end: self.size,
                    body,
                })
            }
            s => Err(status_error(&self.url, s)),
        }
    }
}

fn early_end(url: &Url) -> VfsError {
    VfsError::new(
        ErrorKind::Unavailable,
        format!("{}: the connection closed before the end", host(url)),
    )
}

impl ByteSource for HttpFile {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        if buf.is_empty() || offset >= self.size {
            return Ok(0);
        }
        let mut slot = self.stream.lock().unwrap_or_else(PoisonError::into_inner);
        // One retry: a kept-alive response may have been closed by the
        // server meanwhile.
        for attempt in 0..2 {
            let reuse = slot
                .as_ref()
                .is_some_and(|s| s.pos == offset && s.pos < s.end);
            if !reuse {
                *slot = None;
                *slot = Some(self.start(offset, buf.len())?);
            }
            let s = slot.as_mut().expect("stream was just set");
            let want = usize::try_from(s.end - s.pos)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            match s.body.read(&mut buf[..want]) {
                Ok(0) => {
                    *slot = None;
                    if attempt == 1 {
                        return Err(early_end(&self.url));
                    }
                }
                Ok(n) => {
                    s.pos += n as u64;
                    if s.pos >= s.end {
                        *slot = None;
                    }
                    return Ok(n);
                }
                Err(e) => {
                    *slot = None;
                    if attempt == 1 || !reuse {
                        return Err(VfsError::from_io(e, host(&self.url)));
                    }
                }
            }
        }
        Err(early_end(&self.url))
    }

    fn hint_random_access(&self) {
        self.random.store(true, Ordering::Relaxed);
    }
}
