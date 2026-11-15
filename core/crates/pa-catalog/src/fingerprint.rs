// SPDX-License-Identifier: MPL-2.0
//! Content fingerprint of a file: the basis of the `file:` media key
//! (`pa_model::MediaKey::File`, "hash of size + first and last 64 KiB").
//!
//! The value is the OSHash (OpenSubtitles hash): the file size plus the
//! little-endian 64-bit words of the first and the last 64 KiB, wrapping,
//! as 16 lowercase hex digits. It is written to both `files.oshash` and
//! `files.fingerprint`. LIB-004 owns hashing; it must keep this value, or
//! progress stored under `file:` keys loses its files.
//!
//! Files shorter than 64 KiB hash their whole content twice (start and end
//! chunks overlap), zero-padding the last partial word. From 128 KiB on the
//! value equals the OpenSubtitles reference implementation (tests below).
//!
//! OSHash reads two 64 KiB ranges, so it is cheap on network sources and is
//! computed for every file in the scan ([`fill_missing`]). A new file with
//! the OSHash of a missing one is the same file moved or renamed
//! ([`pa_library::fingerprints::follow_moves`], run right after).
//!
//! The ed2k hash ([`ed2k`]) reads the whole file and runs as a background
//! job ([`crate::ed2k`]).

use pa_library::rusqlite::{Connection, params};
use pa_vfs::{ByteSource, Vfs, VfsPath};

use crate::Error;

/// Bytes hashed at each end of the file.
pub const CHUNK: u64 = 64 * 1024;

/// OSHash of `source`, 16 lowercase hex digits.
pub fn oshash(source: &dyn ByteSource) -> Result<String, Error> {
    let size = source.size();
    let chunk = size.min(CHUNK);
    let mut hash = size;
    let mut buf = vec![0u8; usize::try_from(chunk).unwrap_or(0)];
    for offset in [0, size - chunk] {
        source.read_exact_at(offset, &mut buf)?;
        hash = buf.chunks(8).fold(hash, |h, w| {
            let mut word = [0u8; 8];
            word[..w.len()].copy_from_slice(w);
            h.wrapping_add(u64::from_le_bytes(word))
        });
    }
    Ok(format!("{hash:016x}"))
}

/// Size of an ed2k chunk: 9 500 KiB.
pub const ED2K_CHUNK: u64 = 9_728_000;

/// Bytes read at once by [`ed2k`].
const ED2K_READ: usize = 1 << 20;

/// The ed2k hash of a file (MD4 over 9 500 KiB chunks), 32 lowercase hex
/// digits.
///
/// Below one chunk it is the MD4 of the data, otherwise the MD4 of the
/// concatenated chunk MD4s. Implementations differ only for a size that is
/// an exact multiple of the chunk size: the original eDonkey client
/// appends the MD4 of an empty last chunk (the "red" hash, stored in
/// `files.ed2k`: it reproduces the published value for 9 728 000 zero
/// bytes), newer eMule versions do not ([`Ed2kHash::without_empty_chunk`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kHash {
    /// The eDonkey ("red") value, written to `files.ed2k`.
    pub hash: String,
    /// The eMule ("blue") value when it differs from [`hash`](Self::hash):
    /// only for sizes that are a positive multiple of [`ED2K_CHUNK`].
    pub without_empty_chunk: Option<String>,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// The ed2k hash of `source`, read sequentially from the start. `stop` is
/// checked before every read; `Ok(None)` means it asked to stop.
/// `on_bytes` gets the bytes read so far.
pub fn ed2k(
    source: &dyn ByteSource,
    stop: &dyn Fn() -> bool,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<Option<Ed2kHash>, Error> {
    use md4::{Digest, Md4};

    let size = source.size();
    let mut buf = vec![0u8; ED2K_READ];
    let mut chunks: Vec<[u8; 16]> = Vec::new();
    let mut offset = 0u64;
    loop {
        let mut md = Md4::new();
        let end = (offset + ED2K_CHUNK).min(size);
        while offset < end {
            if stop() {
                return Ok(None);
            }
            let n = usize::try_from((end - offset).min(ED2K_READ as u64)).unwrap_or(ED2K_READ);
            source.read_exact_at(offset, &mut buf[..n])?;
            md.update(&buf[..n]);
            offset += n as u64;
            on_bytes(offset);
        }
        chunks.push(md.finalize().into());
        // The red hash adds the empty chunk after a full last one.
        if offset >= size
            && (!size.is_multiple_of(ED2K_CHUNK) || chunks.len() as u64 > size / ED2K_CHUNK)
        {
            break;
        }
    }
    let of_list = |list: &[[u8; 16]]| -> String {
        if list.len() == 1 {
            hex(&list[0])
        } else {
            let mut md = Md4::new();
            for c in list {
                md.update(c);
            }
            hex(&md.finalize())
        }
    };
    let hash = of_list(&chunks);
    let without_empty_chunk =
        (size > 0 && size.is_multiple_of(ED2K_CHUNK)).then(|| of_list(&chunks[..chunks.len() - 1]));
    Ok(Some(Ed2kHash {
        hash,
        without_empty_chunk,
    }))
}

/// OSHash of a local file.
pub fn oshash_file(path: &std::path::Path) -> Result<String, Error> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::Invalid(format!("no parent: {}", path.display())))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Invalid(format!("bad file name: {}", path.display())))?;
    let fs = pa_vfs::LocalFs::new(dir)?;
    let source = fs.open(&VfsPath::root().join(name)?)?;
    oshash(&*source)
}

/// OSHash of one file of `vfs`, read for random access (two 64 KiB reads,
/// no read-ahead on network sources).
fn oshash_at(vfs: &dyn Vfs, path: &str) -> Result<String, Error> {
    let source = vfs.open(&VfsPath::parse(path)?)?;
    source.hint_random_access();
    oshash(&*source)
}

/// Files hashed at once (LIB-023): each hash is two small reads, so on a
/// network source the time goes into round trips that overlap well.
const WORKERS: usize = 4;

/// Fingerprints the media files of `source_id` that have none yet (new or
/// changed since the last scan), [`WORKERS`] at a time. Unreadable files are
/// skipped and counted. Returns `(hashed, failed)`.
pub(crate) fn fill_missing(
    conn: &mut Connection,
    source_id: i64,
    vfs: &dyn Vfs,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<(usize, usize), Error> {
    let files: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, path FROM files
             WHERE source_id = ?1 AND fingerprint IS NULL AND missing_since_ms IS NULL
             ORDER BY id",
        )?;
        stmt.query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let total = files.len();
    if total == 0 {
        return Ok((0, 0));
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (tx, rx) = std::sync::mpsc::channel::<(i64, Option<String>)>();
    std::thread::scope(|scope| -> Result<(usize, usize), Error> {
        for _ in 0..WORKERS.min(total) {
            let (tx, next, files) = (tx.clone(), &next, &files);
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((id, path)) = files.get(i) else {
                        break;
                    };
                    if tx.send((*id, oshash_at(vfs, path).ok())).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let (mut hashed, mut failed) = (0, 0);
        let mut done: Vec<(i64, String)> = Vec::with_capacity(100);
        let write = |conn: &mut Connection, done: &mut Vec<(i64, String)>| {
            let tx = conn.transaction()?;
            {
                let mut update = tx.prepare_cached(
                    "UPDATE files SET fingerprint = ?2, oshash = ?2, hashed_at_ms = ?3
                     WHERE id = ?1",
                )?;
                let now = crate::now_ms();
                for (id, h) in done.drain(..) {
                    update.execute(params![id, h, now])?;
                }
            }
            tx.commit()?;
            Ok::<(), Error>(())
        };
        for (id, hash) in &rx {
            match hash {
                Some(h) => {
                    done.push((id, h));
                    hashed += 1;
                }
                None => failed += 1,
            }
            if done.len() >= 100 {
                write(conn, &mut done)?;
            }
            if (hashed + failed) % 25 == 0 || hashed + failed == total {
                on_progress(hashed + failed, total);
            }
        }
        write(conn, &mut done)?;
        Ok((hashed, failed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Mem(Vec<u8>);

    impl ByteSource for Mem {
        fn size(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
            let start = usize::try_from(offset).unwrap().min(self.0.len());
            let n = buf.len().min(self.0.len() - start);
            buf[..n].copy_from_slice(&self.0[start..start + n]);
            Ok(n)
        }
    }

    #[test]
    fn zero_file_hash_is_its_size() {
        // All words are zero: the hash is the size.
        let data = vec![0u8; 200_000];
        assert_eq!(oshash(&Mem(data)).unwrap(), format!("{:016x}", 200_000));
    }

    #[test]
    fn matches_reference_sum() {
        // 256 KiB of a byte pattern; reference: size + sum of LE words of
        // both 64 KiB ends.
        let data: Vec<u8> = (0..256 * 1024).map(|i| (i * 7 % 251) as u8).collect();
        let mut expect = data.len() as u64;
        for range in [0..65536, data.len() - 65536..data.len()] {
            for w in data[range].chunks(8) {
                expect = expect.wrapping_add(u64::from_le_bytes(w.try_into().unwrap()));
            }
        }
        assert_eq!(oshash(&Mem(data)).unwrap(), format!("{expect:016x}"));
    }

    /// Test data shared with the reference implementations: an LCG
    /// (`s = s * 1103515245 + 12345 mod 2^32`, byte = `s >> 24`, `s0 = 1`).
    fn lcg(n: usize) -> Vec<u8> {
        let mut s: u32 = 1;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (s >> 24) as u8
            })
            .collect()
    }

    const C: usize = ED2K_CHUNK as usize;

    #[test]
    fn oshash_matches_opensubtitles_reference() {
        // Values of the OpenSubtitles Python sample (`hashFile`) over the
        // same bytes; it refuses files below 128 KiB.
        let data = lcg(2 * C);
        for (n, expect) in [
            (131_072, "671fc8a141cbc9df"),
            (1_000_003, "616045a5f8de7751"),
            (C - 1, "06d1632ed801d32b"),
            (C, "321c4628a90b79bf"),
            (C + 1, "7c47910ba2dc8353"),
            (2 * C, "78025a2a7c65dfe8"),
        ] {
            assert_eq!(
                oshash(&Mem(data[..n].to_vec())).unwrap(),
                expect,
                "{n} bytes"
            );
        }
    }

    fn ed2k_of(data: Vec<u8>) -> Ed2kHash {
        ed2k(&Mem(data), &|| false, &mut |_| {}).unwrap().unwrap()
    }

    #[test]
    fn ed2k_of_small_files_is_md4() {
        // RFC 1320 test suite: below one chunk ed2k is plain MD4.
        for (input, expect) in [
            (&b""[..], "31d6cfe0d16ae931b73c59d7e0c089c0"),
            (b"a", "bde52cb31de33e46245e05fbdbd6fb24"),
            (b"abc", "a448017aaf21d8525fc10ae87aa6729d"),
            (b"message digest", "d9130a8164549fe818874806e1c7014b"),
            (
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "e33b4ddc9c38f2199c3e7b164fcc0536",
            ),
        ] {
            let h = ed2k_of(input.to_vec());
            assert_eq!(h.hash, expect);
            assert_eq!(h.without_empty_chunk, None);
        }
    }

    #[test]
    fn ed2k_of_one_chunk_of_zeros() {
        // The published example: 9 728 000 zero bytes.
        let h = ed2k_of(vec![0; C]);
        assert_eq!(h.hash, "fc21d9af828f92a8df64beac3357425d");
        // eMule: the MD4 of the single chunk.
        assert_eq!(
            h.without_empty_chunk.as_deref(),
            Some("d7def262a127cd79096a108e7a9fc138")
        );
    }

    #[test]
    fn ed2k_matches_reference_around_chunk_borders() {
        // Values of a reference implementation (AniDB wiki definition,
        // pure-Python MD4 checked against RFC 1320) over the same bytes.
        let data = lcg(2 * C);
        for (n, red, blue) in [
            (131_072, "7f74bd72c692b2195a2863d2f431eb86", None),
            (1_000_003, "7afd8ccb1349c89f322c565beb4e54a3", None),
            (C - 1, "836e9ab988e322a0cde69e30fa8eead6", None),
            (
                C,
                "3c27058622c511ac31d0ae34a3edebcf",
                Some("3ac4b211f48bf88db82ee2e44b353724"),
            ),
            (C + 1, "c7efbe48d81f43e2b7d3e5ce8f92b9d3", None),
            (
                2 * C,
                "884ad2673004bdc6fae2e6ca17fa6513",
                Some("81a3e735a438a5dfd029b95dabff6cdb"),
            ),
        ] {
            let h = ed2k_of(data[..n].to_vec());
            assert_eq!(h.hash, red, "{n} bytes");
            assert_eq!(h.without_empty_chunk.as_deref(), blue, "{n} bytes");
        }
    }

    #[test]
    fn ed2k_stops_and_reports_bytes() {
        let src = Mem(vec![7; 3 * ED2K_READ + 5]);
        let mut seen = Vec::new();
        let h = ed2k(&src, &|| false, &mut |b| seen.push(b)).unwrap();
        assert!(h.is_some());
        assert_eq!(seen.last(), Some(&(3 * ED2K_READ as u64 + 5)));
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
        let calls = std::cell::Cell::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert_eq!(ed2k(&src, &stop, &mut |_| {}).unwrap(), None);
    }

    #[test]
    fn short_and_empty_files() {
        assert_eq!(oshash(&Mem(Vec::new())).unwrap(), "0000000000000000");
        // 3 bytes: size 3 + word 0x030201 twice.
        let h = oshash(&Mem(vec![1, 2, 3])).unwrap();
        assert_eq!(h, format!("{:016x}", 3 + 2 * 0x0003_0201u64));
    }
}
