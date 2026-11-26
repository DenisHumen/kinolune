// SPDX-License-Identifier: MPL-2.0
//! UDF reader (ECMA-167 parts 3 and 4, OSTA UDF 1.02–2.60).
//!
//! The path from the anchor to a file: Anchor Volume Descriptor Pointer
//! (sector 256) → Volume Descriptor Sequence (Partition Descriptor,
//! Logical Volume Descriptor with the partition maps) → File Set
//! Descriptor → root File Entry → File Identifier Descriptors of each
//! directory → File Entries with allocation descriptors. All numbers are
//! little-endian.

use std::collections::HashSet;
use std::sync::Arc;

use super::{MAX_DEPTH, Node, Piece, SECTOR, Tree, clip, invalid, read_pieces, sector};
use crate::error::{ErrorKind, Result, VfsError};
use crate::source::ByteSource;

const TAG_AVDP: u16 = 2;
const TAG_PD: u16 = 5;
const TAG_LVD: u16 = 6;
const TAG_TD: u16 = 8;
const TAG_FSD: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_AED: u16 = 258;
const TAG_FE: u16 = 261;
const TAG_EFE: u16 = 266;

/// Most allocation descriptors read for one file.
const MAX_EXTENTS: usize = 1 << 16;

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(x)
}

/// The id of the descriptor tag at the start of `b`, if its checksum holds.
fn tag_id(b: &[u8]) -> Option<u16> {
    if b.len() < 16 {
        return None;
    }
    let sum = b[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |a, (_, x)| a.wrapping_add(*x));
    (sum == b[4]).then(|| u16_at(b, 0))
}

/// An OSTA CS0 string: a compression id (8: one byte per character, 16:
/// UTF-16BE) and the characters.
pub(super) fn cs0(b: &[u8]) -> String {
    match b.split_first() {
        Some((8, rest)) => rest.iter().map(|&c| char::from(c)).collect(),
        Some((16, rest)) => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::new(),
    }
}

/// A fixed-size dstring: CS0 bytes with the used length in the last byte.
fn dstring(b: &[u8]) -> String {
    let Some((&len, body)) = b.split_last() else {
        return String::new();
    };
    let len = usize::from(len).min(body.len());
    cs0(&body[..len]).trim_end_matches(['\0', ' ']).to_owned()
}

#[derive(Debug)]
enum PartMap {
    /// Blocks of the partition with this starting sector.
    Physical { start: u64 },
    /// Metadata partition: blocks of the metadata file, which lies in the
    /// physical partition `start`.
    Metadata {
        start: u64,
        extents: Vec<(u64, u64)>,
    },
}

struct Udf<'a> {
    src: &'a dyn ByteSource,
    maps: Vec<PartMap>,
}

impl Udf<'_> {
    /// Sector of block `lbn` of partition `part`.
    fn sector_of(&self, part: u16, lbn: u64) -> Result<u64> {
        match self.maps.get(usize::from(part)) {
            Some(PartMap::Physical { start }) => Ok(start + lbn),
            Some(PartMap::Metadata { start, extents }) => {
                let mut at = lbn;
                for (phys, blocks) in extents {
                    if at < *blocks {
                        return Ok(start + phys + at);
                    }
                    at -= blocks;
                }
                Err(invalid("block outside the metadata partition"))
            }
            None => Err(invalid("unknown partition")),
        }
    }

    /// Image pieces of `len` bytes from block `lbn` of partition `part`.
    fn resolve(&self, part: u16, lbn: u64, len: u64, out: &mut Vec<Piece>) -> Result<()> {
        match self.maps.get(usize::from(part)) {
            Some(PartMap::Physical { start }) => {
                out.push(Piece::Image {
                    at: (start + lbn) * SECTOR,
                    len,
                });
                Ok(())
            }
            Some(PartMap::Metadata { start, extents }) => {
                // Metadata extents need not be contiguous on the image:
                // cut the run at their ends.
                let mut skip = lbn;
                let mut left = len;
                for (phys, blocks) in extents {
                    if left == 0 {
                        break;
                    }
                    if skip >= *blocks {
                        skip -= blocks;
                        continue;
                    }
                    let n = left.min((blocks - skip) * SECTOR);
                    out.push(Piece::Image {
                        at: (start + phys + skip) * SECTOR,
                        len: n,
                    });
                    left -= n;
                    skip = 0;
                }
                if left > 0 {
                    return Err(invalid("extent outside the metadata partition"));
                }
                Ok(())
            }
            None => Err(invalid("unknown partition")),
        }
    }

    fn block(&self, part: u16, lbn: u64) -> Result<Vec<u8>> {
        sector(self.src, self.sector_of(part, lbn)?)
    }

    /// Reads the file entry at block `lbn` of `part`: whether it is a
    /// directory, its size and its pieces.
    fn entry(&self, part: u16, lbn: u64) -> Result<(bool, u64, Vec<Piece>)> {
        let b = self.block(part, lbn)?;
        let (len_at, ad_at) = match tag_id(&b) {
            Some(TAG_FE) => (168, 176),
            Some(TAG_EFE) => (208, 216),
            _ => return Err(invalid("expected a file entry")),
        };
        let file_type = b[27];
        let flags = u16_at(&b, 34);
        let size = u64_at(&b, 56);
        let l_ea = u32_at(&b, len_at) as usize;
        let l_ad = u32_at(&b, len_at + 4) as usize;
        let start = ad_at + l_ea;
        let ads = b
            .get(start..start + l_ad)
            .ok_or_else(|| invalid("allocation descriptors past the entry"))?;
        let is_dir = file_type == 4;
        let pieces = match flags & 7 {
            3 => vec![Piece::Inline(Arc::from(ads))],
            kind => self.extents(part, kind, ads)?,
        };
        Ok((is_dir, size, clip(pieces, size)))
    }

    /// Pieces of the allocation descriptors `ads` of kind `kind` (0 short,
    /// 1 long, 2 extended), following continuation extents.
    fn extents(&self, part: u16, kind: u16, ads: &[u8]) -> Result<Vec<Piece>> {
        let size = match kind {
            0 => 8,
            1 => 16,
            2 => 20,
            _ => return Err(invalid("allocation descriptor type")),
        };
        let mut out = Vec::new();
        let mut data = ads.to_vec();
        let mut seen = 0usize;
        'next: loop {
            for ad in data.chunks_exact(size) {
                seen += 1;
                if seen > MAX_EXTENTS {
                    return Err(invalid("too many extents"));
                }
                let raw = u32_at(ad, 0);
                let len = u64::from(raw & 0x3FFF_FFFF);
                let ty = raw >> 30;
                let (lbn, p) = match kind {
                    0 => (u64::from(u32_at(ad, 4)), part),
                    1 => (u64::from(u32_at(ad, 4)), u16_at(ad, 8)),
                    _ => (u64::from(u32_at(ad, 12)), u16_at(ad, 16)),
                };
                if len == 0 {
                    break 'next;
                }
                match ty {
                    0 => self.resolve(p, lbn, len, &mut out)?,
                    1 | 2 => out.push(Piece::Zero { len }),
                    _ => {
                        // The rest of the descriptors are in an Allocation
                        // Extent Descriptor.
                        let b = self.block(p, lbn)?;
                        if tag_id(&b) != Some(TAG_AED) {
                            return Err(invalid("expected an allocation extent"));
                        }
                        let n = u32_at(&b, 20) as usize;
                        data = b
                            .get(24..24 + n)
                            .ok_or_else(|| invalid("allocation extent length"))?
                            .to_vec();
                        continue 'next;
                    }
                }
            }
            break;
        }
        Ok(out)
    }

    /// Adds the directory at (`part`, `lbn`) and everything under it.
    fn dir(
        &self,
        tree: &mut Tree,
        part: u16,
        lbn: u64,
        depth: usize,
        seen: &mut HashSet<(u16, u64)>,
    ) -> Result<usize> {
        if depth > MAX_DEPTH || !seen.insert((part, lbn)) {
            return Err(invalid("directory loop"));
        }
        let (_, _, pieces) = self.entry(part, lbn)?;
        let data = read_pieces(self.src, &pieces)?;
        let me = tree.push(Node::Dir(Vec::new()))?;
        let mut children = Vec::new();
        let mut at = 0;
        while at + 38 <= data.len() {
            let fid = &data[at..];
            if tag_id(fid) != Some(TAG_FID) {
                break;
            }
            let chars = fid[18];
            let l_fi = usize::from(fid[19]);
            let l_iu = usize::from(u16_at(fid, 36));
            let len = (38 + l_iu + l_fi + 3) & !3;
            if at + 38 + l_iu + l_fi > data.len() {
                return Err(invalid("file identifier past the directory"));
            }
            let icb_lbn = u64::from(u32_at(fid, 24));
            let icb_part = u16_at(fid, 28);
            let name = cs0(&fid[38 + l_iu..38 + l_iu + l_fi]);
            at += len;
            // Parent entry, deleted files, hidden entries without a name.
            if chars & 0x0C != 0 || name.is_empty() || name.contains('/') {
                continue;
            }
            let child = if chars & 0x02 != 0 {
                self.dir(tree, icb_part, icb_lbn, depth + 1, seen)?
            } else {
                let (is_dir, size, pieces) = self.entry(icb_part, icb_lbn)?;
                if is_dir {
                    continue;
                }
                tree.push(Node::File {
                    pieces: pieces.into(),
                    size,
                })?
            };
            children.push((name, child));
        }
        tree.nodes[me] = Node::Dir(children);
        Ok(me)
    }
}

/// Whether the image has a UDF volume recognition sequence (`NSR02` or
/// `NSR03` among the descriptors from sector 16).
fn has_nsr(src: &dyn ByteSource) -> Result<bool> {
    for lba in 16..48 {
        let Ok(b) = sector(src, lba) else {
            return Ok(false);
        };
        match &b[1..6] {
            b"NSR02" | b"NSR03" => return Ok(true),
            b"BEA01" | b"CD001" | b"CDW02" | b"BOOT2" | b"TEA01" => {}
            _ => return Ok(false),
        }
    }
    Ok(false)
}

/// Reads the UDF file system of `src`: its revision, volume name and tree;
/// `None` if the image has no UDF.
pub(super) fn read(src: &dyn ByteSource) -> Result<Option<(u16, String, Tree)>> {
    if !has_nsr(src)? {
        return Ok(None);
    }
    let last = (src.size() / SECTOR).saturating_sub(1);
    let anchor = [256, last.saturating_sub(256), last]
        .into_iter()
        .find_map(|lba| {
            sector(src, lba)
                .ok()
                .filter(|b| tag_id(b) == Some(TAG_AVDP))
        });
    let Some(anchor) = anchor else {
        return Ok(None);
    };
    let vds_len = u64::from(u32_at(&anchor, 16)) / SECTOR;
    let vds_at = u64::from(u32_at(&anchor, 20));

    let mut partitions: Vec<(u16, u64)> = Vec::new();
    let mut lvd: Option<Vec<u8>> = None;
    for lba in vds_at..vds_at + vds_len.min(64) {
        let b = sector(src, lba)?;
        match tag_id(&b) {
            Some(TAG_PD) => partitions.push((u16_at(&b, 22), u64::from(u32_at(&b, 188)))),
            Some(TAG_LVD) => lvd = Some(b),
            Some(TAG_TD) | None => break,
            _ => {}
        }
    }
    let lvd = lvd.ok_or_else(|| invalid("no logical volume descriptor"))?;
    if u32_at(&lvd, 212) != 2048 {
        return Err(VfsError::new(
            ErrorKind::Unsupported,
            "UDF block size other than 2048",
        ));
    }
    let revision = u16_at(&lvd, 240);
    let volume_id = dstring(&lvd[84..212]);
    let start_of = |number: u16| {
        partitions
            .iter()
            .find(|(n, _)| *n == number)
            .map(|(_, s)| *s)
            .ok_or_else(|| invalid("partition map without a partition"))
    };

    let n_maps = u32_at(&lvd, 268) as usize;
    let mut udf = Udf {
        src,
        maps: Vec::new(),
    };
    let mut metadata_files = Vec::new();
    let mut at = 440;
    for _ in 0..n_maps.min(64) {
        let map = lvd
            .get(at..at + 2)
            .ok_or_else(|| invalid("partition maps past the descriptor"))?;
        let (ty, len) = (map[0], usize::from(map[1]));
        let map = lvd
            .get(at..at + len)
            .ok_or_else(|| invalid("partition map length"))?;
        match ty {
            1 if len >= 6 => udf.maps.push(PartMap::Physical {
                start: start_of(u16_at(map, 4))?,
            }),
            2 if len >= 64 => {
                let ident = &map[5..28];
                let start = start_of(u16_at(map, 38))?;
                if ident.starts_with(b"*UDF Metadata Partition") {
                    metadata_files.push((udf.maps.len(), u64::from(u32_at(map, 40))));
                    udf.maps.push(PartMap::Metadata {
                        start,
                        extents: Vec::new(),
                    });
                } else if ident.starts_with(b"*UDF Sparable Partition") {
                    // Read as physical: sparing only remaps defective
                    // packets of rewritable media.
                    udf.maps.push(PartMap::Physical { start });
                } else {
                    return Err(VfsError::new(
                        ErrorKind::Unsupported,
                        "UDF virtual partitions (VAT) are not supported",
                    ));
                }
            }
            _ => return Err(invalid("partition map type")),
        }
        at += len;
    }
    // The metadata file (a file entry in the physical partition) maps the
    // blocks of a metadata partition.
    for (index, lbn) in metadata_files {
        let PartMap::Metadata { start, .. } = udf.maps[index] else {
            continue;
        };
        let phys = udf
            .maps
            .iter()
            .position(|m| matches!(m, PartMap::Physical { start: s } if *s == start))
            .ok_or_else(|| invalid("metadata partition without a physical map"))?;
        let (_, _, pieces) = udf.entry(u16::try_from(phys).unwrap_or(0), lbn)?;
        let mut extents = Vec::new();
        for p in pieces {
            match p {
                Piece::Image { at, len } => {
                    extents.push((at / SECTOR - start, len.div_ceil(SECTOR)));
                }
                _ => return Err(invalid("metadata file extent")),
            }
        }
        udf.maps[index] = PartMap::Metadata { start, extents };
    }

    // File Set Descriptor, from the logical volume contents use (a long_ad).
    let fsd_lbn = u64::from(u32_at(&lvd, 252));
    let fsd_part = u16_at(&lvd, 256);
    let fsd = udf.block(fsd_part, fsd_lbn)?;
    if tag_id(&fsd) != Some(TAG_FSD) {
        return Err(invalid("no file set descriptor"));
    }
    let root_lbn = u64::from(u32_at(&fsd, 404));
    let root_part = u16_at(&fsd, 408);
    let mut tree = Tree::default();
    udf.dir(&mut tree, root_part, root_lbn, 0, &mut HashSet::new())?;
    Ok(Some((revision, volume_id, tree)))
}
