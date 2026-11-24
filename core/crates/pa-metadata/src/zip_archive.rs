// SPDX-License-Identifier: MPL-2.0
//! A minimal ZIP reader for subtitle archives (PLAY-016): Subdl serves
//! every subtitle as a ZIP, a season pack holds one file per episode.
//!
//! Only what subtitle archives use: the central directory, entries
//! *stored* or *deflated* (raw deflate through `flate2`, already in the
//! tree), CRC-32 checked. No ZIP64, no encryption, no multi-disk archives:
//! those are refused, not guessed at. Every entry is read with a size
//! limit, so a "zip bomb" stops at the limit instead of filling memory.
//! The same choice as the report writer of `pa-log` (no `zip` crate): a
//! subtitle archive is small and its format simple.

use std::io::Read;

/// Most entries an archive may list (a season pack has a few dozen).
pub const MAX_ENTRIES: usize = 4096;

/// Local file header signature `PK\x03\x04`.
const LOCAL: u32 = 0x0403_4b50;
/// Central directory header signature `PK\x01\x02`.
const CENTRAL: u32 = 0x0201_4b50;
/// End of central directory signature `PK\x05\x06`.
const END: u32 = 0x0605_4b50;

/// Why an archive or an entry cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZipError {
    /// No end of central directory: not a ZIP archive.
    #[error("not a zip archive")]
    NotZip,
    /// A feature subtitle archives do not use (ZIP64, encryption, another
    /// compression method).
    #[error("unsupported zip archive: {0}")]
    Unsupported(&'static str),
    /// Offsets or sizes point outside the archive, or the CRC is wrong.
    #[error("corrupt zip archive: {0}")]
    Corrupt(&'static str),
    /// The entry unpacks to more than the limit.
    #[error("zip entry larger than {0} bytes")]
    TooLarge(usize),
}

/// One entry of the central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// Path inside the archive (`/` separated; CP437 names are read as
    /// UTF-8 where they can be, the rest lossily).
    pub name: String,
    /// Unpacked size, as the directory says.
    pub size: u64,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u64,
    offset: u64,
}

impl ZipEntry {
    /// A folder, not a file.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }

    /// The last part of [`name`](Self::name).
    #[must_use]
    pub fn base_name(&self) -> &str {
        self.name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(self.name.as_str())
    }
}

/// An archive in memory and its central directory.
#[derive(Debug)]
pub struct ZipArchive<'a> {
    bytes: &'a [u8],
    entries: Vec<ZipEntry>,
}

/// True when `bytes` start like a ZIP archive (`PK\x03\x04`, or an empty
/// archive's `PK\x05\x06`).
#[must_use]
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

/// True when `bytes` are a RAR archive (some old uploads are RAR under a
/// `.zip` name).
#[must_use]
pub fn is_rar(bytes: &[u8]) -> bool {
    bytes.starts_with(b"Rar!\x1a\x07")
}

fn u16_at(b: &[u8], i: usize) -> Result<u16, ZipError> {
    b.get(i..i + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(ZipError::Corrupt("truncated header"))
}

fn u32_at(b: &[u8], i: usize) -> Result<u32, ZipError> {
    b.get(i..i + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(ZipError::Corrupt("truncated header"))
}

impl<'a> ZipArchive<'a> {
    /// Reads the central directory of `bytes`.
    ///
    /// # Errors
    ///
    /// [`ZipError::NotZip`] without an end of central directory,
    /// [`ZipError::Unsupported`] for ZIP64 and multi-disk archives,
    /// [`ZipError::Corrupt`] for offsets outside the archive.
    pub fn open(bytes: &'a [u8]) -> Result<Self, ZipError> {
        // The end record is 22 bytes plus a comment of up to 64 KiB.
        if bytes.len() < 22 {
            return Err(ZipError::NotZip);
        }
        let lowest = bytes.len().saturating_sub(22 + usize::from(u16::MAX));
        let end = (lowest..=bytes.len() - 22)
            .rev()
            .find(|&i| u32_at(bytes, i).is_ok_and(|s| s == END))
            .ok_or(ZipError::NotZip)?;
        if u16_at(bytes, end + 4)? != 0 || u16_at(bytes, end + 6)? != 0 {
            return Err(ZipError::Unsupported("multi-disk archive"));
        }
        let count = u16_at(bytes, end + 10)?;
        let cd_size = u32_at(bytes, end + 12)?;
        let cd_offset = u32_at(bytes, end + 16)?;
        if count == u16::MAX || cd_size == u32::MAX || cd_offset == u32::MAX {
            return Err(ZipError::Unsupported("zip64"));
        }
        let count = usize::from(count);
        if count > MAX_ENTRIES {
            return Err(ZipError::Unsupported("too many entries"));
        }
        let mut entries = Vec::with_capacity(count);
        let mut i = cd_offset as usize;
        for _ in 0..count {
            if u32_at(bytes, i)? != CENTRAL {
                return Err(ZipError::Corrupt("central directory"));
            }
            let flags = u16_at(bytes, i + 8)?;
            let method = u16_at(bytes, i + 10)?;
            let crc = u32_at(bytes, i + 16)?;
            let compressed = u32_at(bytes, i + 20)?;
            let size = u32_at(bytes, i + 24)?;
            let name_len = usize::from(u16_at(bytes, i + 28)?);
            let extra_len = usize::from(u16_at(bytes, i + 30)?);
            let comment_len = usize::from(u16_at(bytes, i + 32)?);
            let offset = u32_at(bytes, i + 42)?;
            if compressed == u32::MAX || size == u32::MAX || offset == u32::MAX {
                return Err(ZipError::Unsupported("zip64"));
            }
            let raw = bytes
                .get(i + 46..i + 46 + name_len)
                .ok_or(ZipError::Corrupt("entry name"))?;
            entries.push(ZipEntry {
                name: String::from_utf8_lossy(raw).into_owned(),
                size: u64::from(size),
                method,
                flags,
                crc,
                compressed: u64::from(compressed),
                offset: u64::from(offset),
            });
            i += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { bytes, entries })
    }

    /// The entries, in the order of the central directory.
    #[must_use]
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Unpacks `entry` (one of [`entries`](Self::entries)), at most `limit`
    /// bytes.
    ///
    /// # Errors
    ///
    /// [`ZipError::TooLarge`] past `limit`, [`ZipError::Unsupported`] for
    /// an encrypted entry or a method other than stored and deflate,
    /// [`ZipError::Corrupt`] for bad offsets, a broken deflate stream or a
    /// wrong CRC.
    pub fn read(&self, entry: &ZipEntry, limit: usize) -> Result<Vec<u8>, ZipError> {
        if entry.flags & 1 != 0 {
            return Err(ZipError::Unsupported("encrypted entry"));
        }
        if entry.size > limit as u64 {
            return Err(ZipError::TooLarge(limit));
        }
        let at = usize::try_from(entry.offset).map_err(|_| ZipError::Corrupt("offset"))?;
        if u32_at(self.bytes, at)? != LOCAL {
            return Err(ZipError::Corrupt("local header"));
        }
        let name_len = usize::from(u16_at(self.bytes, at + 26)?);
        let extra_len = usize::from(u16_at(self.bytes, at + 28)?);
        let start = at + 30 + name_len + extra_len;
        let len = usize::try_from(entry.compressed).map_err(|_| ZipError::Corrupt("size"))?;
        let data = self
            .bytes
            .get(start..start.saturating_add(len))
            .ok_or(ZipError::Corrupt("entry data outside the archive"))?;
        let out = match entry.method {
            0 => {
                if data.len() > limit {
                    return Err(ZipError::TooLarge(limit));
                }
                data.to_vec()
            }
            8 => {
                let mut out = Vec::with_capacity(usize::try_from(entry.size).unwrap_or(0));
                flate2::read::DeflateDecoder::new(data)
                    .take(limit as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|_| ZipError::Corrupt("deflate stream"))?;
                if out.len() > limit {
                    return Err(ZipError::TooLarge(limit));
                }
                out
            }
            _ => return Err(ZipError::Unsupported("compression method")),
        };
        let mut crc = flate2::Crc::new();
        crc.update(&out);
        if crc.sum() != entry.crc || out.len() as u64 != entry.size {
            return Err(ZipError::Corrupt("crc"));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A ZIP with one stored and one deflated entry, written by hand.
    fn archive(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in entries {
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let packed = if *deflate {
                let mut e =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let offset = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            let mut h = Vec::new();
            h.extend_from_slice(&LOCAL.to_le_bytes());
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&[0; 4]);
            h.extend_from_slice(&crc.sum().to_le_bytes());
            h.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&h);
            out.extend_from_slice(&packed);
            central.extend_from_slice(&CENTRAL.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.sum().to_le_bytes());
            central.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&END.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let text = "1\n00:00:01,000 --> 00:00:02,000\nПривет\n".repeat(50);
        let bytes = archive(&[
            ("dir/", b"", false),
            ("a.srt", b"stored", false),
            ("dir/b.ass", text.as_bytes(), true),
        ]);
        assert!(is_zip(&bytes));
        let zip = ZipArchive::open(&bytes).unwrap();
        let names: Vec<&str> = zip.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dir/", "a.srt", "dir/b.ass"]);
        assert!(zip.entries()[0].is_dir());
        assert_eq!(zip.entries()[2].base_name(), "b.ass");
        assert_eq!(zip.read(&zip.entries()[1], 100).unwrap(), b"stored");
        assert_eq!(
            zip.read(&zip.entries()[2], 1 << 20).unwrap(),
            text.as_bytes()
        );
    }

    #[test]
    fn limits_and_corruption_are_errors() {
        let big = vec![b'x'; 10_000];
        let bytes = archive(&[("big.srt", &big, true)]);
        let zip = ZipArchive::open(&bytes).unwrap();
        assert_eq!(
            zip.read(&zip.entries()[0], 100),
            Err(ZipError::TooLarge(100))
        );
        // A wrong CRC in the directory.
        let mut bad = bytes.clone();
        let cd = bad.len() - 22 - (46 + "big.srt".len());
        bad[cd + 16] ^= 0xff;
        let zip = ZipArchive::open(&bad).unwrap();
        assert_eq!(
            zip.read(&zip.entries()[0], 1 << 20),
            Err(ZipError::Corrupt("crc"))
        );
        assert_eq!(
            ZipArchive::open(b"not a zip at all, just text").unwrap_err(),
            ZipError::NotZip
        );
        assert!(is_rar(b"Rar!\x1a\x07\x01\x00"));
    }
}
