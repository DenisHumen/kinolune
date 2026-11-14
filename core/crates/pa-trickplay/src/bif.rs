// SPDX-License-Identifier: MPL-2.0
//! BIF ("Base Index Frames", Roku): JPEG thumbnails at a fixed interval in
//! one file. Plex and Emby write it next to the media; it is also our own
//! cache format.
//!
//! Layout (little endian): an 8-byte magic, `u32` version (0), `u32` image
//! count, `u32` interval in ms (0 means 1000), 44 reserved bytes; at offset
//! 64 an index of `count + 1` pairs `(u32 timestamp, u32 offset)` — the
//! timestamp in interval units, the last pair `0xFFFFFFFF` and the end of
//! the last image — then the JPEG data.

use std::ops::Range;

/// The file magic.
pub const MAGIC: [u8; 8] = [0x89, b'B', b'I', b'F', 0x0D, 0x0A, 0x1A, 0x0A];

const HEADER: usize = 64;
const END: u32 = u32::MAX;

/// A BIF that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid BIF: {0}")]
pub struct BifError(&'static str);

/// A parsed BIF held in memory.
#[derive(Clone, PartialEq, Eq)]
pub struct Bif {
    interval_ms: u64,
    /// Timestamp (interval units) and byte range of each image, sorted.
    frames: Vec<(u32, Range<usize>)>,
    data: Vec<u8>,
}

impl std::fmt::Debug for Bif {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bif")
            .field("interval_ms", &self.interval_ms)
            .field("frames", &self.frames.len())
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl Bif {
    /// Parses a whole BIF file.
    ///
    /// # Errors
    ///
    /// [`BifError`] for a bad magic, version, index or image range.
    pub fn parse(data: Vec<u8>) -> Result<Self, BifError> {
        if data.len() < HEADER || data[..8] != MAGIC {
            return Err(BifError("magic"));
        }
        if u32_at(&data, 8) != 0 {
            return Err(BifError("version"));
        }
        let count = u32_at(&data, 12) as usize;
        let interval = match u32_at(&data, 16) {
            0 => 1000,
            ms => u64::from(ms),
        };
        let index_end = count
            .checked_add(1)
            .and_then(|n| n.checked_mul(8))
            .and_then(|n| n.checked_add(HEADER))
            .ok_or(BifError("count"))?;
        if index_end > data.len() {
            return Err(BifError("index"));
        }
        let mut frames = Vec::with_capacity(count);
        for i in 0..count {
            let at = HEADER + i * 8;
            let ts = u32_at(&data, at);
            let start = u32_at(&data, at + 4) as usize;
            let end = u32_at(&data, at + 12) as usize;
            if ts == END || start < index_end || start > end || end > data.len() {
                return Err(BifError("image range"));
            }
            frames.push((ts, start..end));
        }
        frames.sort_by_key(|(ts, _)| *ts);
        Ok(Self {
            interval_ms: interval,
            frames,
            data,
        })
    }

    /// Time between thumbnails.
    #[must_use]
    pub fn interval_ms(&self) -> u64 {
        self.interval_ms
    }

    /// Number of thumbnails.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// `true` without thumbnails.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Thumbnail `i`: its JPEG and its time.
    #[must_use]
    pub fn frame(&self, i: usize) -> Option<(&[u8], u64)> {
        let (ts, range) = self.frames.get(i)?;
        Some((&self.data[range.clone()], u64::from(*ts) * self.interval_ms))
    }

    /// Index of the thumbnail shown at `position_ms`: the last one at or
    /// before it (the first one before the first timestamp).
    #[must_use]
    pub fn index_at(&self, position_ms: u64) -> Option<usize> {
        if self.frames.is_empty() {
            return None;
        }
        let units = position_ms / self.interval_ms;
        let i = self
            .frames
            .partition_point(|(ts, _)| u64::from(*ts) <= units);
        Some(i.saturating_sub(1))
    }
}

/// Writes a BIF of `frames` (`(timestamp in interval units, JPEG)`, sorted
/// by timestamp).
///
/// # Panics
///
/// If the file would exceed 4 GiB (BIF offsets are 32-bit); thumbnails
/// are a few kilobytes, so this means a caller bug.
#[must_use]
pub fn write(interval_ms: u32, frames: &[(u32, &[u8])]) -> Vec<u8> {
    let index_len = (frames.len() + 1) * 8;
    let data_len: usize = frames.iter().map(|(_, j)| j.len()).sum();
    let mut out = Vec::with_capacity(HEADER + index_len + data_len);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(frames.len())
            .expect("BIF count")
            .to_le_bytes(),
    );
    out.extend_from_slice(&interval_ms.to_le_bytes());
    out.resize(HEADER, 0);
    let mut offset = HEADER + index_len;
    for (ts, jpeg) in frames {
        out.extend_from_slice(&ts.to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset).expect("BIF size").to_le_bytes());
        offset += jpeg.len();
    }
    out.extend_from_slice(&END.to_le_bytes());
    out.extend_from_slice(&u32::try_from(offset).expect("BIF size").to_le_bytes());
    for (_, jpeg) in frames {
        out.extend_from_slice(jpeg);
    }
    out
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let a = b"\xFF\xD8one".as_slice();
        let b = b"\xFF\xD8second".as_slice();
        let c = b"\xFF\xD8x".as_slice();
        let bytes = write(10_000, &[(0, a), (1, b), (3, c)]);
        let bif = Bif::parse(bytes).unwrap();
        assert_eq!(bif.interval_ms(), 10_000);
        assert_eq!(bif.len(), 3);
        assert_eq!(bif.frame(1), Some((b, 10_000)));
        assert_eq!(bif.frame(2), Some((c, 30_000)));
        assert_eq!(bif.index_at(0), Some(0));
        assert_eq!(bif.index_at(19_999), Some(1));
        // The gap at timestamp 2 shows the previous image.
        assert_eq!(bif.index_at(25_000), Some(1));
        assert_eq!(bif.index_at(99_000_000), Some(2));
    }

    #[test]
    fn zero_interval_means_one_second() {
        let bif = Bif::parse(write(0, &[(5, b"j".as_slice())])).unwrap();
        assert_eq!(bif.interval_ms(), 1000);
        assert_eq!(bif.frame(0).unwrap().1, 5000);
        assert_eq!(bif.index_at(0), Some(0));
    }

    #[test]
    fn rejects_damage() {
        assert_eq!(Bif::parse(vec![0; 10]).unwrap_err(), BifError("magic"));
        let mut bytes = write(1000, &[(0, b"jpeg".as_slice())]);
        bytes.truncate(bytes.len() - 2);
        assert_eq!(Bif::parse(bytes).unwrap_err(), BifError("image range"));
        let mut bytes = write(1000, &[]);
        bytes[12] = 0xFF;
        bytes[13] = 0xFF;
        assert_eq!(Bif::parse(bytes).unwrap_err(), BifError("index"));
        let empty = Bif::parse(write(1000, &[])).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.index_at(5), None);
    }
}
