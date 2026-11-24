// SPDX-License-Identifier: MPL-2.0
//! [`Drive`]: a cloud as a tree of nodes with ids; [`CloudFs`]: any drive
//! as a [`Vfs`] with paths.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Vfs, VfsError, VfsPath};

/// A file or folder of a cloud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The cloud's id of the node (what [`Drive::children`] and
    /// [`Drive::open`] take).
    pub id: String,
    /// Its name as the cloud has it.
    pub name: String,
    /// File, folder, or something that cannot be read as a file (a Google
    /// Docs document, a shortcut, a OneNote notebook).
    pub kind: EntryKind,
    /// Size of a file in bytes; `0` for folders.
    pub size: u64,
    /// Last change, Unix milliseconds.
    pub modified_ms: Option<i64>,
}

impl Node {
    /// A folder node (the root of a drive).
    #[must_use]
    pub fn folder(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: String::new(),
            kind: EntryKind::Dir,
            size: 0,
            modified_ms: None,
        }
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            kind: self.kind,
            size: if self.kind == EntryKind::File {
                self.size
            } else {
                0
            },
            modified_ms: self.modified_ms,
        }
    }
}

/// A cloud seen as nodes with ids. Methods block on the network.
pub trait Drive: Send + Sync + fmt::Debug {
    /// The root folder.
    ///
    /// # Errors
    ///
    /// The cloud cannot be reached or refuses the login.
    fn root(&self) -> pa_vfs::Result<Node>;

    /// The children of folder `dir`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] if the folder is gone, network errors.
    fn children(&self, dir: &Node) -> pa_vfs::Result<Vec<Node>>;

    /// Opens file `file` for range reads.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] if the file is gone, network errors.
    fn open(&self, file: &Node) -> pa_vfs::Result<Box<dyn ByteSource>>;
}

impl<D: Drive + ?Sized> Drive for Arc<D> {
    fn root(&self) -> pa_vfs::Result<Node> {
        (**self).root()
    }
    fn children(&self, dir: &Node) -> pa_vfs::Result<Vec<Node>> {
        (**self).children(dir)
    }
    fn open(&self, file: &Node) -> pa_vfs::Result<Box<dyn ByteSource>> {
        (**self).open(file)
    }
}

/// How long a folder listing is reused.
pub const LISTING_TTL: Duration = Duration::from_secs(60);

type Listing = Arc<Vec<(String, Node)>>;

/// A [`Drive`] as a read-only [`Vfs`]: the source root is folder `root`
/// (a path inside the drive, `/` for all of it). Listings are cached for
/// [`LISTING_TTL`], so walking a deep path does not list every parent
/// again. Names that are not valid in a path (`/`, empty, `.`, `..`) are
/// cleaned; a name used twice in a folder gets ` (2)`, ` (3)` before its
/// extension, in a stable order (by id).
pub struct CloudFs<D> {
    drive: D,
    root: VfsPath,
    ttl: Duration,
    root_node: Mutex<Option<Node>>,
    listings: Mutex<HashMap<String, (Instant, Listing)>>,
}

impl<D: Drive> fmt::Debug for CloudFs<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudFs")
            .field("drive", &self.drive)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Cleans a cloud name into a path component; `None` if nothing is left.
fn clean(name: &str) -> Option<String> {
    let n: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == '\0' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let n = n.trim_end_matches(['.', ' ']).to_owned();
    (!n.trim().is_empty()).then_some(n)
}

/// `name` with ` (k)` before its extension.
fn numbered(name: &str, k: usize) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({k}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({k})"),
    }
}

/// Unique path names of `nodes`.
fn unique_names(mut nodes: Vec<Node>) -> Vec<(String, Node)> {
    nodes.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let mut used = HashSet::new();
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        let Some(base) = clean(&node.name) else {
            continue;
        };
        let mut name = base.clone();
        let mut k = 2;
        while !used.insert(name.clone()) {
            name = numbered(&base, k);
            k += 1;
        }
        out.push((name, node));
    }
    out
}

impl<D: Drive> CloudFs<D> {
    /// The whole drive.
    pub fn new(drive: D) -> Self {
        Self::with_root(drive, VfsPath::root())
    }

    /// Folder `root` of the drive as the source root.
    pub fn with_root(drive: D, root: VfsPath) -> Self {
        Self {
            drive,
            root,
            ttl: LISTING_TTL,
            root_node: Mutex::new(None),
            listings: Mutex::new(HashMap::new()),
        }
    }

    /// The drive.
    pub fn drive(&self) -> &D {
        &self.drive
    }

    /// Forgets cached listings (after a rescan was asked for).
    pub fn refresh(&self) {
        self.listings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    fn drive_root(&self) -> pa_vfs::Result<Node> {
        let mut slot = self
            .root_node
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = &*slot {
            return Ok(n.clone());
        }
        let n = self.drive.root()?;
        *slot = Some(n.clone());
        Ok(n)
    }

    fn listing(&self, dir: &Node) -> pa_vfs::Result<Listing> {
        if let Some((at, l)) = self
            .listings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&dir.id)
            && at.elapsed() < self.ttl
        {
            return Ok(Arc::clone(l));
        }
        let l = Arc::new(unique_names(self.drive.children(dir)?));
        self.listings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(dir.id.clone(), (Instant::now(), Arc::clone(&l)));
        Ok(l)
    }

    /// The node at `path` of the source.
    fn resolve(&self, path: &VfsPath) -> pa_vfs::Result<Node> {
        let mut node = self.drive_root()?;
        for (i, part) in self.root.components().chain(path.components()).enumerate() {
            if node.kind != EntryKind::Dir {
                return Err(VfsError::new(
                    ErrorKind::NotFound,
                    format!("{path}: a parent is not a folder"),
                ));
            }
            let listing = self.listing(&node)?;
            node = listing
                .iter()
                .find(|(n, _)| n == part)
                .map(|(_, n)| n.clone())
                .ok_or_else(|| {
                    let what = if i < self.root.components().count() {
                        format!("the source folder {}", self.root)
                    } else {
                        path.to_string()
                    };
                    VfsError::new(ErrorKind::NotFound, format!("{what}: not found"))
                })?;
        }
        Ok(node)
    }
}

impl<D: Drive> Vfs for CloudFs<D> {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        let node = self.resolve(dir)?;
        if node.kind != EntryKind::Dir {
            return Err(VfsError::new(
                ErrorKind::NotADirectory,
                format!("{dir}: not a folder"),
            ));
        }
        self.listing(&node)?
            .iter()
            .map(|(name, n)| {
                Ok(DirEntry {
                    name: name.clone(),
                    path: dir.join(name)?,
                    metadata: n.metadata(),
                })
            })
            .collect()
    }

    fn stat(&self, path: &VfsPath) -> pa_vfs::Result<Metadata> {
        let mut m = self.resolve(path)?.metadata();
        if path.is_root() {
            m = Metadata {
                kind: EntryKind::Dir,
                size: 0,
                modified_ms: m.modified_ms,
            };
        }
        Ok(m)
    }

    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        let node = self.resolve(path)?;
        match node.kind {
            EntryKind::File => self.drive.open(&node),
            EntryKind::Dir => Err(VfsError::new(
                ErrorKind::IsADirectory,
                format!("{path}: is a folder"),
            )),
            _ => Err(VfsError::new(
                ErrorKind::Unsupported,
                format!("{path}: not a file that can be read"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str, name: &str) -> Node {
        Node {
            id: id.into(),
            name: name.into(),
            kind: EntryKind::File,
            size: 1,
            modified_ms: None,
        }
    }

    #[test]
    fn duplicate_and_bad_names() {
        let names: Vec<String> = unique_names(vec![
            file("b", "Movie.mkv"),
            file("a", "Movie.mkv"),
            file("c", "Movie.mkv"),
            file("d", "a/b"),
            file("e", ".."),
            file("f", "  "),
            file("g", "noext"),
            file("h", "noext"),
        ])
        .into_iter()
        .map(|(n, node)| format!("{n}={}", node.id))
        .collect();
        assert_eq!(
            names,
            [
                "Movie.mkv=a",
                "Movie (2).mkv=b",
                "Movie (3).mkv=c",
                "a_b=d",
                "noext=g",
                "noext (2)=h"
            ]
        );
    }
}
