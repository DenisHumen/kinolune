// SPDX-License-Identifier: MPL-2.0
//! Disc images (`.iso`) as a read-only [`Vfs`] (PLAY-017, ADR-0140).
//!
//! A Blu-ray or DVD image plays without unpacking: [`IsoFs`] reads the file
//! system of the image through any [`ByteSource`] (a local file, an SMB
//! file) and opens the files in it as byte sources of their own, so the
//! player reads `BDMV/STREAM/00001.m2ts` of an image on a share with the
//! same positional reads as a plain file.
//!
//! - **UDF** 1.02–2.60 (DVD, Blu-ray): physical, sparable (read as
//!   physical) and metadata partitions (UDF 2.50+, used by Blu-ray), file
//!   entries and extended file entries, short, long, extended and embedded
//!   allocation descriptors with continuation extents. Virtual partitions
//!   (VAT, BD-R written in increments) are not supported.
//! - **ISO 9660** with Joliet names and multi-extent files, used when the
//!   image has no UDF.
//!
//! The whole directory tree is read once by [`IsoFs::open`] (a disc holds
//! hundreds of files, not millions; limits guard against broken images).
//! The layout follows ECMA-167, the OSTA UDF specification and ECMA-119;
//! no code of other projects is used. [`build`] writes small images for
//! tests and tools.

pub mod build;
mod iso9660;
mod udf;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::error::{ErrorKind, Result, VfsError};
use crate::path::VfsPath;
use crate::source::ByteSource;
use crate::vfs::{DirEntry, EntryKind, Metadata, Vfs};

/// Bytes per sector of a disc image.
pub const SECTOR: u64 = 2048;

/// Most entries read from one image.
const MAX_ENTRIES: usize = 200_000;
/// Deepest directory nesting read.
const MAX_DEPTH: usize = 64;
/// Largest directory read, in bytes.
const MAX_DIR_BYTES: u64 = 32 * 1024 * 1024;

/// File system an image was read with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IsoFormat {
    /// UDF; `revision` from the domain identifier (`0x0250` for UDF 2.50).
    Udf { revision: u16 },
    /// ISO 9660, with Joliet names if `joliet`.
    Iso9660 { joliet: bool },
}

/// A piece of a file: bytes at an offset of the image, zeros (an extent
/// allocated but not recorded) or bytes stored in the file entry itself.
#[derive(Debug, Clone)]
pub(crate) enum Piece {
    Image { at: u64, len: u64 },
    Zero { len: u64 },
    Inline(Arc<[u8]>),
}

impl Piece {
    fn len(&self) -> u64 {
        match self {
            Self::Image { len, .. } | Self::Zero { len } => *len,
            Self::Inline(b) => b.len() as u64,
        }
    }
}

/// The pieces of a file, cut to its size.
pub(crate) fn clip(pieces: Vec<Piece>, size: u64) -> Vec<Piece> {
    let mut left = size;
    let mut out = Vec::with_capacity(pieces.len());
    for p in pieces {
        if left == 0 {
            break;
        }
        let n = p.len().min(left);
        left -= n;
        out.push(match p {
            Piece::Image { at, .. } => Piece::Image { at, len: n },
            Piece::Zero { .. } => Piece::Zero { len: n },
            Piece::Inline(b) => Piece::Inline(b[..usize::try_from(n).unwrap_or(b.len())].into()),
        });
    }
    out
}

#[derive(Debug)]
pub(crate) enum Node {
    Dir(Vec<(String, usize)>),
    File { pieces: Arc<[Piece]>, size: u64 },
}

/// The tree read from an image; node 0 is the root.
#[derive(Debug, Default)]
pub(crate) struct Tree {
    pub(crate) nodes: Vec<Node>,
}

impl Tree {
    pub(crate) fn push(&mut self, node: Node) -> Result<usize> {
        if self.nodes.len() >= MAX_ENTRIES {
            return Err(invalid("too many entries"));
        }
        self.nodes.push(node);
        Ok(self.nodes.len() - 1)
    }
}

pub(crate) fn invalid(what: &str) -> VfsError {
    VfsError::new(ErrorKind::Other, format!("broken disc image: {what}"))
}

/// Reads one sector.
pub(crate) fn sector(src: &dyn ByteSource, lba: u64) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; SECTOR as usize];
    let at = lba
        .checked_mul(SECTOR)
        .ok_or_else(|| invalid("sector out of range"))?;
    if at.saturating_add(SECTOR) > src.size() {
        return Err(invalid("sector past the end"));
    }
    src.read_exact_at(at, &mut buf)?;
    Ok(buf)
}

/// Reads the bytes of `pieces` (a directory), at most [`MAX_DIR_BYTES`].
pub(crate) fn read_pieces(src: &dyn ByteSource, pieces: &[Piece]) -> Result<Vec<u8>> {
    let total: u64 = pieces.iter().map(Piece::len).sum();
    if total > MAX_DIR_BYTES {
        return Err(invalid("directory too large"));
    }
    let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    for p in pieces {
        match p {
            Piece::Image { at, len } => {
                let start = out.len();
                out.resize(start + usize::try_from(*len).unwrap_or(0), 0);
                src.read_exact_at(*at, &mut out[start..])?;
            }
            Piece::Zero { len } => out.resize(out.len() + usize::try_from(*len).unwrap_or(0), 0),
            Piece::Inline(b) => out.extend_from_slice(b),
        }
    }
    Ok(out)
}

/// A disc image opened as a read-only file system.
pub struct IsoFs {
    src: Arc<dyn ByteSource>,
    format: IsoFormat,
    volume_id: String,
    tree: Tree,
    /// Lower-cased names of each directory, for case-insensitive lookups.
    folded: HashMap<(usize, String), usize>,
}

impl fmt::Debug for IsoFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsoFs")
            .field("format", &self.format)
            .field("volume_id", &self.volume_id)
            .field("entries", &self.tree.nodes.len())
            .finish_non_exhaustive()
    }
}

impl IsoFs {
    /// Reads the file system of the image `src`: UDF if the image has it,
    /// else ISO 9660.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unsupported`] for an image with neither file system or
    /// with a UDF feature not read here; [`ErrorKind::Other`] for a broken
    /// image; read errors of `src`.
    pub fn open(src: Arc<dyn ByteSource>) -> Result<Self> {
        let (format, volume_id, tree) = match udf::read(src.as_ref())? {
            Some((revision, volume_id, tree)) => (IsoFormat::Udf { revision }, volume_id, tree),
            None => match iso9660::read(src.as_ref())? {
                Some((joliet, volume_id, tree)) => (IsoFormat::Iso9660 { joliet }, volume_id, tree),
                None => {
                    return Err(VfsError::new(
                        ErrorKind::Unsupported,
                        "not a disc image (no UDF or ISO 9660 file system)",
                    ));
                }
            },
        };
        let mut folded = HashMap::new();
        for (i, node) in tree.nodes.iter().enumerate() {
            if let Node::Dir(children) = node {
                for (name, child) in children {
                    folded.entry((i, name.to_lowercase())).or_insert(*child);
                }
            }
        }
        Ok(Self {
            src,
            format,
            volume_id,
            tree,
            folded,
        })
    }

    /// The file system the image was read with.
    #[must_use]
    pub fn format(&self) -> IsoFormat {
        self.format
    }

    /// Name of the volume (the disc title as mastered, `MY_MOVIE`), empty
    /// if the image has none.
    #[must_use]
    pub fn volume_id(&self) -> &str {
        &self.volume_id
    }

    /// The node at `path`: names match exactly, else ignoring case (disc
    /// names are upper case, links to them often are not).
    fn find(&self, path: &VfsPath) -> Result<usize> {
        let mut at = 0;
        for name in path.components() {
            let Node::Dir(children) = &self.tree.nodes[at] else {
                return Err(VfsError::new(ErrorKind::NotADirectory, path.as_str()));
            };
            at = match children.iter().find(|(n, _)| n == name) {
                Some((_, c)) => *c,
                None => *self
                    .folded
                    .get(&(at, name.to_lowercase()))
                    .ok_or_else(|| VfsError::new(ErrorKind::NotFound, path.as_str()))?,
            };
        }
        Ok(at)
    }

    fn meta(&self, node: usize) -> Metadata {
        match &self.tree.nodes[node] {
            Node::Dir(_) => Metadata {
                kind: EntryKind::Dir,
                size: 0,
                modified_ms: None,
            },
            Node::File { size, .. } => Metadata {
                kind: EntryKind::File,
                size: *size,
                modified_ms: None,
            },
        }
    }
}

impl Vfs for IsoFs {
    fn list(&self, dir: &VfsPath) -> Result<Vec<DirEntry>> {
        let at = self.find(dir)?;
        let Node::Dir(children) = &self.tree.nodes[at] else {
            return Err(VfsError::new(ErrorKind::NotADirectory, dir.as_str()));
        };
        children
            .iter()
            .map(|(name, child)| {
                Ok(DirEntry {
                    name: name.clone(),
                    path: dir.join(name)?,
                    metadata: self.meta(*child),
                })
            })
            .collect()
    }

    fn stat(&self, path: &VfsPath) -> Result<Metadata> {
        Ok(self.meta(self.find(path)?))
    }

    fn open(&self, path: &VfsPath) -> Result<Box<dyn ByteSource>> {
        match &self.tree.nodes[self.find(path)?] {
            Node::Dir(_) => Err(VfsError::new(ErrorKind::IsADirectory, path.as_str())),
            Node::File { pieces, size } => {
                let mut starts = Vec::with_capacity(pieces.len());
                let mut at = 0;
                for p in pieces.iter() {
                    starts.push(at);
                    at += p.len();
                }
                Ok(Box::new(IsoFile {
                    src: Arc::clone(&self.src),
                    pieces: Arc::clone(pieces),
                    starts,
                    size: (*size).min(at),
                }))
            }
        }
    }
}

/// A file inside an image.
struct IsoFile {
    src: Arc<dyn ByteSource>,
    pieces: Arc<[Piece]>,
    starts: Vec<u64>,
    size: u64,
}

impl fmt::Debug for IsoFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsoFile")
            .field("size", &self.size)
            .field("pieces", &self.pieces.len())
            .finish_non_exhaustive()
    }
}

impl ByteSource for IsoFile {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= self.size || buf.is_empty() {
            return Ok(0);
        }
        let i = self.starts.partition_point(|&s| s <= offset) - 1;
        let inside = offset - self.starts[i];
        let piece = &self.pieces[i];
        let n = usize::try_from((piece.len() - inside).min(self.size - offset))
            .unwrap_or(usize::MAX)
            .min(buf.len());
        match piece {
            Piece::Image { at, .. } => self.src.read_at(at + inside, &mut buf[..n]),
            Piece::Zero { .. } => {
                buf[..n].fill(0);
                Ok(n)
            }
            Piece::Inline(b) => {
                let s = usize::try_from(inside).unwrap_or(0);
                buf[..n].copy_from_slice(&b[s..s + n]);
                Ok(n)
            }
        }
    }

    fn hint_random_access(&self) {
        self.src.hint_random_access();
    }
}

/// `true` if `name` is a disc image this module opens (`.iso`, any case).
#[must_use]
pub fn is_image_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("iso"))
}

/// Splits a path that goes into an image into the image and the path in
/// it: `/m/Film.iso/BDMV/index.bdmv` → (`/m/Film.iso`, `/BDMV/index.bdmv`).
/// `None` for a path that does not go into an image (the image itself
/// included).
#[must_use]
pub fn split_image_path(path: &VfsPath) -> Option<(VfsPath, VfsPath)> {
    let s = path.as_str();
    let mut at = 0;
    for name in s[1..].split('/') {
        at += 1 + name.len();
        if is_image_name(name) && at < s.len() {
            let outer = VfsPath::parse(&s[..at]).ok()?;
            let inner = VfsPath::parse(&s[at..]).ok()?;
            return Some((outer, inner));
        }
    }
    None
}

#[cfg(test)]
mod tests;
