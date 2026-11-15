// SPDX-License-Identifier: MPL-2.0
//! A minimal zip writer: stored (uncompressed) entries, UTF-8 names, no
//! zip64. The report is a few megabytes of text at most, so compression is
//! not worth a dependency; every unzip tool and GitHub accept the result.

use std::io::{self, Write};

/// CRC-32 (IEEE 802.3), as zip wants it.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// MS-DOS date and time of a Unix timestamp (UTC; zip has no zone).
fn dos_time(unix_secs: u64) -> (u16, u16) {
    let (y, mo, d, h, mi, s) = crate::time::civil(unix_secs);
    let y = y.clamp(1980, 2107);
    let time = ((h as u16) << 11) | ((mi as u16) << 5) | ((s as u16) / 2);
    let date = (((y - 1980) as u16) << 9) | ((mo as u16) << 5) | d as u16;
    (time, date)
}

struct Entry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

/// Writes entries one by one, then the central directory on [`finish`].
pub(crate) struct ZipWriter<W: Write> {
    out: W,
    written: u64,
    entries: Vec<Entry>,
    time: u16,
    date: u16,
}

impl<W: Write> ZipWriter<W> {
    pub(crate) fn new(out: W, unix_secs: u64) -> Self {
        let (time, date) = dos_time(unix_secs);
        Self {
            out,
            written: 0,
            entries: Vec::new(),
            time,
            date,
        }
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    fn too_big() -> io::Error {
        io::Error::other("diagnostic report over 4 GiB")
    }

    pub(crate) fn add(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        let offset = u32::try_from(self.written).map_err(|_| Self::too_big())?;
        let size = u32::try_from(data.len()).map_err(|_| Self::too_big())?;
        let crc = crc32(data);
        let mut h = Vec::with_capacity(30 + name.len());
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes()); // version needed
        h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        h.extend_from_slice(&0u16.to_le_bytes()); // stored
        h.extend_from_slice(&self.time.to_le_bytes());
        h.extend_from_slice(&self.date.to_le_bytes());
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(name.as_bytes());
        self.put(&h)?;
        self.put(data)?;
        self.entries.push(Entry {
            name: name.to_string(),
            crc,
            size,
            offset,
        });
        Ok(())
    }

    pub(crate) fn finish(mut self) -> io::Result<W> {
        let start = u32::try_from(self.written).map_err(|_| Self::too_big())?;
        let mut cd = Vec::new();
        for e in &self.entries {
            cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            cd.extend_from_slice(&20u16.to_le_bytes()); // made by
            cd.extend_from_slice(&20u16.to_le_bytes()); // needed
            cd.extend_from_slice(&0x0800u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&self.time.to_le_bytes());
            cd.extend_from_slice(&self.date.to_le_bytes());
            cd.extend_from_slice(&e.crc.to_le_bytes());
            cd.extend_from_slice(&e.size.to_le_bytes());
            cd.extend_from_slice(&e.size.to_le_bytes());
            cd.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&[0; 2 + 2 + 2 + 2 + 4]); // extra, comment, disk, attrs
            cd.extend_from_slice(&e.offset.to_le_bytes());
            cd.extend_from_slice(e.name.as_bytes());
        }
        let n = self.entries.len() as u16;
        let mut end = Vec::with_capacity(22);
        end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        end.extend_from_slice(&[0; 4]); // disk numbers
        end.extend_from_slice(&n.to_le_bytes());
        end.extend_from_slice(&n.to_le_bytes());
        end.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        end.extend_from_slice(&start.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        self.put(&cd)?;
        self.put(&end)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Entries of a zip written by [`ZipWriter`] (stored only): name and data.
/// For tests and for reading a report back.
pub fn read_stored(bytes: &[u8]) -> io::Result<Vec<(String, Vec<u8>)>> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "not a stored zip");
    let u16_at = |i: usize| -> io::Result<usize> {
        Ok(u16::from_le_bytes(bytes.get(i..i + 2).ok_or_else(bad)?.try_into().unwrap()) as usize)
    };
    let u32_at = |i: usize| -> io::Result<u32> {
        Ok(u32::from_le_bytes(
            bytes.get(i..i + 4).ok_or_else(bad)?.try_into().unwrap(),
        ))
    };
    let mut out = Vec::new();
    let mut i = 0;
    while u32_at(i)? == 0x0403_4b50 {
        if u16_at(i + 8)? != 0 {
            return Err(bad());
        }
        let crc = u32_at(i + 14)?;
        let size = u32_at(i + 18)? as usize;
        let name_len = u16_at(i + 26)?;
        let extra = u16_at(i + 28)?;
        let name_at = i + 30;
        let data_at = name_at + name_len + extra;
        let name = std::str::from_utf8(bytes.get(name_at..name_at + name_len).ok_or_else(bad)?)
            .map_err(|_| bad())?
            .to_string();
        let data = bytes.get(data_at..data_at + size).ok_or_else(bad)?.to_vec();
        if crc32(&data) != crc {
            return Err(bad());
        }
        out.push((name, data));
        i = data_at + size;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_known_input() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn round_trip() {
        let mut z = ZipWriter::new(Vec::new(), 1_763_164_800);
        z.add("a.txt", b"hello").unwrap();
        z.add("dir/b.txt", "привет".as_bytes()).unwrap();
        let bytes = z.finish().unwrap();
        assert_eq!(
            &bytes[bytes.len() - 22..bytes.len() - 18],
            &[0x50, 0x4b, 5, 6]
        );
        let entries = read_stored(&bytes).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], ("a.txt".into(), b"hello".to_vec()));
        assert_eq!(entries[1].0, "dir/b.txt");
    }
}
