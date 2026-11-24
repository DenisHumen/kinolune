// SPDX-License-Identifier: MPL-2.0
//! A cloud file as a [`ByteSource`]: byte ranges fetched on demand with a
//! one-block read-ahead, so a player reading sequentially makes one request
//! per block and a seek costs one request.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pa_vfs::ByteSource;

/// Fetches bytes of one remote file.
pub trait RangeFetch: Send + Sync + fmt::Debug {
    /// Up to `len` bytes from `offset` (`offset < size`, `len > 0`). A
    /// shorter answer is fine; an empty one means the file ended.
    fn fetch(&self, offset: u64, len: u64) -> pa_vfs::Result<Vec<u8>>;
}

/// Default read-ahead block: 2 MiB, about a second of a 4K remux.
pub const DEFAULT_BLOCK: usize = 2 << 20;

/// A remote file of a fixed size read through a [`RangeFetch`].
pub struct RangeFile<F> {
    fetch: F,
    size: u64,
    block: usize,
    random: AtomicBool,
    /// The last block read: its offset and bytes.
    cache: Mutex<Option<(u64, Arc<Vec<u8>>)>>,
}

impl<F: RangeFetch> fmt::Debug for RangeFile<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RangeFile")
            .field("fetch", &self.fetch)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<F: RangeFetch> RangeFile<F> {
    /// A file of `size` bytes with the default read-ahead.
    pub fn new(fetch: F, size: u64) -> Self {
        Self::with_block(fetch, size, DEFAULT_BLOCK)
    }

    /// A file of `size` bytes reading at least `block` bytes per request.
    pub fn with_block(fetch: F, size: u64, block: usize) -> Self {
        Self {
            fetch,
            size,
            block: block.max(1),
            random: AtomicBool::new(false),
            cache: Mutex::new(None),
        }
    }
}

impl<F: RangeFetch> ByteSource for RangeFile<F> {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        if buf.is_empty() || offset >= self.size {
            return Ok(0);
        }
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some((start, bytes)) = cached
            && offset >= start
            && offset < start + bytes.len() as u64
        {
            let from = usize::try_from(offset - start).unwrap_or(usize::MAX);
            let n = buf.len().min(bytes.len() - from);
            buf[..n].copy_from_slice(&bytes[from..from + n]);
            return Ok(n);
        }
        let want = if self.random.load(Ordering::Relaxed) {
            buf.len()
        } else {
            buf.len().max(self.block)
        };
        let want = (want as u64).min(self.size - offset);
        let bytes = self.fetch.fetch(offset, want)?;
        let n = buf.len().min(bytes.len());
        buf[..n].copy_from_slice(&bytes[..n]);
        if bytes.len() > n {
            *self.cache.lock().unwrap_or_else(PoisonError::into_inner) =
                Some((offset, Arc::new(bytes)));
        }
        Ok(n)
    }

    fn hint_random_access(&self) {
        self.random.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Debug, Default)]
    struct Counting {
        calls: AtomicUsize,
    }

    impl RangeFetch for Counting {
        fn fetch(&self, offset: u64, len: u64) -> pa_vfs::Result<Vec<u8>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok((offset..offset + len).map(|i| (i % 251) as u8).collect())
        }
    }

    #[test]
    fn reads_ahead_and_serves_from_the_block() {
        let f = RangeFile::with_block(Counting::default(), 1000, 100);
        let mut b = [0u8; 10];
        assert_eq!(f.read_at(5, &mut b).unwrap(), 10);
        assert_eq!(b[0], 5);
        assert_eq!(f.read_at(15, &mut b).unwrap(), 10);
        assert_eq!(f.fetch.calls.load(Ordering::Relaxed), 1);
        // Past the block: a new request; near the end: clamped.
        assert_eq!(f.read_at(995, &mut b).unwrap(), 5);
        assert_eq!(f.read_at(1000, &mut b).unwrap(), 0);
        assert_eq!(f.fetch.calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn random_access_reads_only_what_is_asked() {
        let f = RangeFile::with_block(Counting::default(), 1000, 100);
        f.hint_random_access();
        let mut b = [0u8; 4];
        f.read_at(0, &mut b).unwrap();
        f.read_at(4, &mut b).unwrap();
        assert_eq!(f.fetch.calls.load(Ordering::Relaxed), 2);
    }
}
