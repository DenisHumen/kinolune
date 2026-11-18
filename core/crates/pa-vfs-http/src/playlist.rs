// SPDX-License-Identifier: MPL-2.0
//! An M3U/M3U8 playlist as a read-only source.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Vfs, VfsError, VfsPath};
use url::Url;

use crate::client::{HttpClient, Timeouts, check_link, host};
use crate::m3u::{PlaylistEntry, parse_m3u};
use crate::names::{
    UniqueNames, extension_of_mime, extension_of_url, file_name, is_media_mime, sanitize,
    stem_of_url,
};
use crate::policy::{LinkPolicy, Zone};

/// Largest playlist read (IPTV lists run to a few MiB).
pub const MAX_PLAYLIST_BYTES: u64 = 16 * 1024 * 1024;
/// Links of one playlist that become files; the rest are ignored.
pub const MAX_ENTRIES: usize = 5000;
/// Links probed at once while the playlist is listed.
const PROBE_THREADS: usize = 8;

/// Where the playlist is.
#[derive(Clone, PartialEq, Eq)]
pub enum PlaylistLocation {
    /// An `http(s)` link the user typed.
    Url(Url),
    /// A file on this device.
    File(PathBuf),
}

impl fmt::Debug for PlaylistLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(u) => write!(f, "Url({})", host(u)),
            Self::File(_) => f.write_str("File(..)"),
        }
    }
}

#[derive(Debug, Clone)]
enum Node {
    Dir,
    File { url: Url, size: u64 },
}

/// The playlist as folders (`group-title`) of files.
#[derive(Debug)]
struct Tree {
    links: Arc<HttpClient>,
    /// `""` for the root, else the folder name → entries.
    dirs: BTreeMap<String, Vec<(String, Node)>>,
}

/// What probing a link found: its size and the extension of its file name.
type Probed = Option<(u64, &'static str)>;

/// An M3U/M3U8 playlist read as a folder of files (NET-018, ADR-0124).
///
/// Every `http(s)` link with a size becomes a file named after its
/// `#EXTINF` title and the media extension of its link or type; links
/// with a `group-title` go into a folder of that name. Live channels and
/// HLS streams (no size) are left out. The links are fetched under the
/// [`LinkPolicy`] of where the playlist came from: a list from the
/// Internet cannot point into the LAN or at this device, a file on the
/// device cannot point at the device itself.
///
/// The list is read again after a minute; sizes are kept.
pub struct PlaylistFs {
    location: PlaylistLocation,
    fetcher: Arc<HttpClient>,
    timeouts: Timeouts,
    ttl: Duration,
    tree: Mutex<Option<(Instant, Arc<Tree>)>>,
    probes: Mutex<HashMap<String, Probed>>,
}

impl fmt::Debug for PlaylistFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlaylistFs")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl PlaylistFs {
    /// A playlist at `location`; nothing is read until the first call.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidPath`] for a link that is not `http(s)` or has
    /// credentials.
    pub fn new(location: PlaylistLocation) -> pa_vfs::Result<Self> {
        Self::with_timeouts(location, Timeouts::default())
    }

    /// Like [`new`](Self::new) with the timeouts of every request.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_timeouts(location: PlaylistLocation, timeouts: Timeouts) -> pa_vfs::Result<Self> {
        if let PlaylistLocation::Url(u) = &location {
            check_link(u)?;
        }
        Ok(Self {
            location,
            fetcher: Arc::new(HttpClient::with_timeouts(LinkPolicy::typed(), timeouts)),
            timeouts,
            ttl: Duration::from_secs(60),
            tree: Mutex::new(None),
            probes: Mutex::new(HashMap::new()),
        })
    }

    /// Reads the playlist: its text, the base of relative links and the
    /// policy of its links.
    fn read(&self) -> pa_vfs::Result<(String, Option<Url>, LinkPolicy)> {
        match &self.location {
            PlaylistLocation::Url(url) => {
                let (text, fin) = self.fetcher.get_text(url, MAX_PLAYLIST_BYTES)?;
                let zone = fin
                    .host_str()
                    .and_then(|h| Zone::of_host(h, fin.port_or_known_default().unwrap_or(80)))
                    .unwrap_or(Zone::Public);
                Ok((text, Some(fin), LinkPolicy::from(zone)))
            }
            PlaylistLocation::File(path) => {
                let file = std::fs::File::open(path)
                    .map_err(|e| VfsError::from_io(e, "the playlist file"))?;
                let mut bytes = Vec::new();
                file.take(MAX_PLAYLIST_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| VfsError::from_io(e, "the playlist file"))?;
                if bytes.len() as u64 > MAX_PLAYLIST_BYTES {
                    return Err(VfsError::new(
                        ErrorKind::Unsupported,
                        "the playlist file is too large",
                    ));
                }
                Ok((
                    String::from_utf8_lossy(&bytes).into_owned(),
                    None,
                    LinkPolicy::local_file(),
                ))
            }
        }
    }

    fn tree(&self) -> pa_vfs::Result<Arc<Tree>> {
        {
            let t = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, tree)) = t.as_ref()
                && at.elapsed() < self.ttl
            {
                return Ok(Arc::clone(tree));
            }
        }
        let (text, base, policy) = self.read()?;
        let list = parse_m3u(&text, base.as_ref())?;
        let links = Arc::new(HttpClient::with_timeouts(policy, self.timeouts));
        let entries: Vec<PlaylistEntry> = list
            .entries
            .into_iter()
            .filter(|e| !e.is_live())
            .take(MAX_ENTRIES)
            .collect();
        let probed = self.probe_all(&links, &entries);
        let tree = Arc::new(build_tree(links, &entries, &probed));
        *self.tree.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), Arc::clone(&tree)));
        Ok(tree)
    }

    /// Size and extension of every entry, from the cache or a `HEAD`,
    /// [`PROBE_THREADS`] at a time.
    fn probe_all(&self, links: &Arc<HttpClient>, entries: &[PlaylistEntry]) -> Vec<Probed> {
        let mut out: Vec<Option<Probed>> = {
            let cache = self.probes.lock().unwrap_or_else(PoisonError::into_inner);
            entries
                .iter()
                .map(|e| cache.get(e.url.as_str()).copied())
                .collect()
        };
        let todo: Vec<usize> = (0..entries.len()).filter(|&i| out[i].is_none()).collect();
        let next = Mutex::new(todo.into_iter());
        let found = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..PROBE_THREADS.min(entries.len()) {
                s.spawn(|| {
                    loop {
                        let Some(i) = next.lock().unwrap_or_else(PoisonError::into_inner).next()
                        else {
                            break;
                        };
                        let p = probe_entry(links, &entries[i]);
                        found
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push((i, p));
                    }
                });
            }
        });
        let mut cache = self.probes.lock().unwrap_or_else(PoisonError::into_inner);
        for (i, p) in found.into_inner().unwrap_or_else(PoisonError::into_inner) {
            // Network errors are not remembered: the next listing retries.
            if let Ok(p) = p {
                cache.insert(entries[i].url.as_str().to_owned(), p);
                out[i] = Some(p);
            }
        }
        out.into_iter().map(Option::flatten).collect()
    }

    fn node(&self, path: &VfsPath) -> pa_vfs::Result<(Arc<Tree>, Node)> {
        let tree = self.tree()?;
        if path.is_root() {
            return Ok((tree, Node::Dir));
        }
        let parts: Vec<&str> = path.components().collect();
        let (dir, name) = match parts.as_slice() {
            [name] => ("", *name),
            [dir, name] => (*dir, *name),
            _ => return Err(not_found(path)),
        };
        let node = tree
            .dirs
            .get(dir)
            .and_then(|d| d.iter().find(|(n, _)| n == name))
            .map(|(_, node)| node.clone())
            .ok_or_else(|| not_found(path))?;
        Ok((tree, node))
    }
}

fn not_found(path: &VfsPath) -> VfsError {
    VfsError::new(ErrorKind::NotFound, format!("{path}: not in the playlist"))
}

/// Size and extension of one link; `Ok(None)` for a link that is not a
/// media file (a web page, a live stream without a size).
fn probe_entry(links: &HttpClient, e: &PlaylistEntry) -> pa_vfs::Result<Probed> {
    let p = match links.probe(&e.url) {
        Ok(p) => p,
        // Gone or refused: not a file of this playlist (not retried).
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::InvalidPath
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let Some(size) = p.size.filter(|&s| s > 0) else {
        return Ok(None);
    };
    let mime = p.content_type.as_deref().unwrap_or("");
    let generic = mime.is_empty() || mime == "application/octet-stream";
    if !generic && !is_media_mime(mime) {
        return Ok(None);
    }
    let ext = extension_of_url(&e.url)
        .or_else(|| extension_of_mime(mime))
        .unwrap_or(if mime.starts_with("audio/") {
            "mp3"
        } else {
            "mkv"
        });
    Ok(Some((size, ext)))
}

fn build_tree(links: Arc<HttpClient>, entries: &[PlaylistEntry], probed: &[Probed]) -> Tree {
    let mut dirs: BTreeMap<String, Vec<(String, Node)>> = BTreeMap::new();
    let mut root_names = UniqueNames::default();
    let mut group_dirs: HashMap<String, String> = HashMap::new();
    let mut names: HashMap<String, UniqueNames> = HashMap::new();
    dirs.insert(String::new(), Vec::new());
    for (e, p) in entries.iter().zip(probed) {
        let Some((size, ext)) = *p else { continue };
        let dir = match e.group.as_deref() {
            Some(g) => {
                let key = sanitize(g).to_lowercase();
                group_dirs
                    .entry(key)
                    .or_insert_with(|| {
                        let name = root_names.take(&sanitize(g));
                        dirs.get_mut("")
                            .expect("root")
                            .push((name.clone(), Node::Dir));
                        dirs.insert(name.clone(), Vec::new());
                        name
                    })
                    .clone()
            }
            None => String::new(),
        };
        let stem = e
            .title
            .clone()
            .or_else(|| stem_of_url(&e.url))
            .unwrap_or_else(|| "Untitled".to_owned());
        let wanted = file_name(&stem, ext);
        let name = if dir.is_empty() {
            root_names.take(&wanted)
        } else {
            names.entry(dir.clone()).or_default().take(&wanted)
        };
        dirs.get_mut(&dir).expect("folder exists").push((
            name,
            Node::File {
                url: e.url.clone(),
                size,
            },
        ));
    }
    Tree { links, dirs }
}

fn metadata(node: &Node) -> Metadata {
    match node {
        Node::Dir => Metadata {
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

impl Vfs for PlaylistFs {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        let (tree, node) = self.node(dir)?;
        if let Node::File { .. } = node {
            return Err(VfsError::new(
                ErrorKind::NotADirectory,
                format!("{dir}: a file"),
            ));
        }
        let key = dir.file_name().unwrap_or("");
        let entries = tree.dirs.get(key).ok_or_else(|| not_found(dir))?;
        entries
            .iter()
            .map(|(name, node)| {
                Ok(DirEntry {
                    name: name.clone(),
                    path: dir.join(name)?,
                    metadata: metadata(node),
                })
            })
            .collect()
    }

    fn stat(&self, path: &VfsPath) -> pa_vfs::Result<Metadata> {
        self.node(path).map(|(_, n)| metadata(&n))
    }

    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        match self.node(path)? {
            (tree, Node::File { url, size }) => Ok(Box::new(tree.links.open(url, size)?)),
            (_, Node::Dir) => Err(VfsError::new(
                ErrorKind::IsADirectory,
                format!("{path}: a folder"),
            )),
        }
    }
}
