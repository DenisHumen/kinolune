// SPDX-License-Identifier: MPL-2.0
//! ISO 9660 reader (ECMA-119) with Joliet names.
//!
//! Volume descriptors start at sector 16: the Primary Volume Descriptor
//! (type 1) and, for Joliet, a Supplementary one (type 2) with UCS-2 names.
//! Each holds the root directory record; a directory is a run of records
//! that never cross a sector. Multi-byte numbers are stored in both byte
//! orders; the little-endian half is read.

use std::collections::HashSet;

use super::{MAX_DEPTH, Node, Piece, SECTOR, Tree, invalid, read_pieces, sector};
use crate::error::Result;
use crate::source::ByteSource;

fn u32_le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// One directory record: name, extent, size, flags.
struct Record {
    name: String,
    extent: u64,
    size: u64,
    is_dir: bool,
    more: bool,
}

fn record(b: &[u8], joliet: bool) -> Option<Record> {
    let len = usize::from(*b.first()?);
    if len < 34 || len > b.len() {
        return None;
    }
    let name_len = usize::from(b[32]);
    let raw = b.get(33..33 + name_len)?;
    let flags = b[25];
    let mut name = if joliet {
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        // d-characters only by the standard; images made on Linux carry
        // UTF-8 names, older ones Latin-1.
        String::from_utf8(raw.to_vec())
            .unwrap_or_else(|_| raw.iter().map(|&c| char::from(c)).collect())
    };
    if flags & 0x02 == 0 {
        if let Some((stem, version)) = name.rsplit_once(';')
            && version.bytes().all(|c| c.is_ascii_digit())
        {
            name = stem.to_owned();
        }
        if let Some(stem) = name.strip_suffix('.') {
            name = stem.to_owned();
        }
    }
    Some(Record {
        name,
        extent: u64::from(u32_le(b, 2)),
        size: u64::from(u32_le(b, 10)),
        is_dir: flags & 0x02 != 0,
        more: flags & 0x80 != 0,
    })
}

fn dir(
    src: &dyn ByteSource,
    tree: &mut Tree,
    extent: u64,
    size: u64,
    joliet: bool,
    depth: usize,
    seen: &mut HashSet<u64>,
) -> Result<usize> {
    if depth > MAX_DEPTH || !seen.insert(extent) {
        return Err(invalid("directory loop"));
    }
    let data = read_pieces(
        src,
        &[Piece::Image {
            at: extent * SECTOR,
            len: size,
        }],
    )?;
    let me = tree.push(Node::Dir(Vec::new()))?;
    let mut children: Vec<(String, usize)> = Vec::new();
    // A file larger than 4 GiB is several records of one name, each with
    // the multi-extent flag but the last.
    let mut pending: Option<(String, Vec<Piece>, u64)> = None;
    let mut at = 0;
    while at < data.len() {
        let len = usize::from(data[at]);
        if len == 0 {
            // Records do not cross sectors: go to the next one.
            at = (at / SECTOR as usize + 1) * SECTOR as usize;
            continue;
        }
        let Some(r) = record(&data[at..], joliet) else {
            break;
        };
        at += len;
        let name_raw = &data[at - len + 33..at - len + 33 + usize::from(data[at - len + 32])];
        if name_raw == [0] || name_raw == [1] {
            continue; // `.` and `..`
        }
        if r.is_dir {
            let child = dir(src, tree, r.extent, r.size, joliet, depth + 1, seen)?;
            children.push((r.name, child));
            continue;
        }
        let piece = Piece::Image {
            at: r.extent * SECTOR,
            len: r.size,
        };
        let (name, mut pieces, total) = match pending.take() {
            Some((n, p, t)) if n == r.name => (n, p, t),
            _ => (r.name.clone(), Vec::new(), 0),
        };
        pieces.push(piece);
        let total = total + r.size;
        if r.more {
            pending = Some((name, pieces, total));
        } else {
            let child = tree.push(Node::File {
                pieces: pieces.into(),
                size: total,
            })?;
            children.push((name, child));
        }
    }
    tree.nodes[me] = Node::Dir(children);
    Ok(me)
}

/// Reads the ISO 9660 file system of `src` (Joliet names if present):
/// whether Joliet was used, the volume name and the tree; `None` if the
/// image has no ISO 9660 volume.
pub(super) fn read(src: &dyn ByteSource) -> Result<Option<(bool, String, Tree)>> {
    let mut primary = None;
    let mut joliet = None;
    for lba in 16..64 {
        let Ok(b) = sector(src, lba) else { break };
        if &b[1..6] != b"CD001" {
            break;
        }
        match b[0] {
            1 if primary.is_none() => primary = Some(b),
            2 if matches!(&b[88..91], b"%/@" | b"%/C" | b"%/E") => joliet = Some(b),
            255 => break,
            _ => {}
        }
    }
    let (is_joliet, vd) = match (joliet, primary) {
        (Some(j), _) => (true, j),
        (None, Some(p)) => (false, p),
        (None, None) => return Ok(None),
    };
    let volume_id = if is_joliet {
        let units: Vec<u16> = vd[40..72]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        vd[40..72].iter().map(|&c| char::from(c)).collect()
    };
    let root = record(&vd[156..190], is_joliet).ok_or_else(|| invalid("root directory record"))?;
    let mut tree = Tree::default();
    dir(
        src,
        &mut tree,
        root.extent,
        root.size,
        is_joliet,
        0,
        &mut HashSet::new(),
    )?;
    Ok(Some((
        is_joliet,
        volume_id.trim_end_matches([' ', '\0']).to_owned(),
        tree,
    )))
}
