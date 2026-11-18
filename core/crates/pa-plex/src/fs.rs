// SPDX-License-Identifier: MPL-2.0
//! The libraries of a Plex Media Server as a read-only [`Vfs`] tree, laid
//! out the way the scanner and the recognition already read local folders
//! (Kodi/Plex naming), like the Jellyfin tree (ADR-0075):
//!
//! ```text
//! /Movies/Dune (2021)/Dune (2021).mkv          the file (byte ranges)
//! /Movies/Dune (2021)/movie.nfo                TMDB/IMDb/TVDB ids, if known
//! /Movies/Dune (2021)/poster.jpg, fanart.jpg   the server's artwork
//! /TV Shows/Dark (2017)/tvshow.nfo, poster.jpg
//! /TV Shows/Dark (2017)/Season 01/Dark S01E01 - Secrets.mkv
//! /Music/Artist/Album/01 - Song.flac, cover.jpg
//! ```
//!
//! Another version of a movie is `Dune (2021) - 4K.mkv`; a movie split into
//! files is `Dune (2021) - part1.mkv`, `… - part2.mkv` (the scanner joins
//! parts, ADR-0045). Photo libraries are left out.
//!
//! The tree of a library comes from a few paged listings and is kept for a
//! while ([`PlexFs::with_ttl`]); [`PlexFs::refresh`] drops it. Media files
//! report their real size; artwork is fetched when opened and is listed
//! with size 0.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Vfs, VfsError, VfsPath};

use crate::http::{MAX_BLOB, check};
use crate::model::{Item, Media, Part, Section};
use crate::server::Server;
use crate::vfs_error;

/// Default read-ahead of a media file: one range request per 2 MiB.
pub const DEFAULT_READ_AHEAD: usize = 2 << 20;
/// Blocks of read-ahead kept per open file.
const CACHE_BLOCKS: usize = 4;

/// The media file behind a path of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    /// Plex item id (`ratingKey`).
    pub rating_key: String,
    /// Path of the file on the server's API (`/library/parts/…`).
    pub part_key: String,
    /// A track rather than a video.
    pub audio: bool,
    /// File size.
    pub size: u64,
    /// When the server added it, Unix ms.
    pub modified_ms: Option<i64>,
    /// Duration, ms.
    pub duration_ms: Option<i64>,
    /// The user's resume position when the tree was read, ms.
    pub view_offset_ms: Option<i64>,
    /// Whether the user had watched it when the tree was read.
    pub watched: bool,
}

#[derive(Debug, Clone)]
enum Node {
    Dir(BTreeSet<String>),
    Media(MediaRef),
    /// Made here (NFO).
    Text(Arc<[u8]>),
    /// Fetched from the server when opened: its path.
    Remote(String),
}

impl Node {
    fn metadata(&self) -> Metadata {
        match self {
            Self::Dir(_) => Metadata {
                kind: EntryKind::Dir,
                size: 0,
                modified_ms: None,
            },
            Self::Media(m) => Metadata {
                kind: EntryKind::File,
                size: m.size,
                modified_ms: m.modified_ms,
            },
            Self::Text(t) => Metadata {
                kind: EntryKind::File,
                size: t.len() as u64,
                modified_ms: None,
            },
            Self::Remote(_) => Metadata {
                kind: EntryKind::File,
                size: 0,
                modified_ms: None,
            },
        }
    }
}

/// One library, laid out.
#[derive(Debug, Default)]
struct Tree {
    nodes: HashMap<String, Node>,
    /// Item id → path of its first media file.
    by_item: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct View {
    dir: String,
    key: String,
    kind: String,
}

#[derive(Default)]
struct Cache {
    views: Option<(Instant, Arc<Vec<View>>)>,
    trees: HashMap<String, (Instant, Arc<Tree>)>,
}

/// The libraries of one Plex server as a [`Vfs`] (see the module docs).
pub struct PlexFs {
    server: Arc<Server>,
    libraries: Option<Vec<String>>,
    ttl: Duration,
    read_ahead: usize,
    cache: Mutex<Cache>,
}

impl fmt::Debug for PlexFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlexFs")
            .field("server", &self.server)
            .field("libraries", &self.libraries)
            .finish_non_exhaustive()
    }
}

fn not_found(path: &str) -> VfsError {
    VfsError::new(ErrorKind::NotFound, format!("{path}: not on the server"))
}

/// A name usable as one path component on every platform the files may be
/// copied to (offline copies, NET-019).
fn clean(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            ':' => out.push_str(" -"),
            '/' | '\\' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let t = out.trim().trim_end_matches('.').trim();
    if t.is_empty() {
        "Untitled".to_owned()
    } else {
        t.to_owned()
    }
}

/// `Name (Year)`, unless the name already ends with the year.
fn titled(item: &Item) -> String {
    let name = clean(&item.title);
    match item.year {
        Some(y) if !name.ends_with(&format!("({y})")) => format!("{name} ({y})"),
        _ => name,
    }
}

/// File extension of a part.
fn extension(part: &Part, media: &Media, audio: bool) -> String {
    part.file
        .as_deref()
        .or(Some(part.key.as_str()))
        .and_then(|p| p.rsplit('/').next())
        .and_then(|p| p.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .or_else(|| {
            part.container
                .as_deref()
                .or(media.container.as_deref())
                .and_then(|c| c.split(',').next())
                .map(str::to_ascii_lowercase)
                .filter(|e| !e.is_empty())
        })
        .unwrap_or_else(|| if audio { "mp3" } else { "mkv" }.to_owned())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `movie.nfo` / `tvshow.nfo` with the external ids, `None` without ids.
fn nfo(root: &str, item: &Item) -> Option<Arc<[u8]>> {
    let mut ids = String::new();
    for p in ["tmdb", "imdb", "tvdb"] {
        if let Some(v) = item.guid(p) {
            ids.push_str(&format!(
                "  <uniqueid type=\"{p}\">{}</uniqueid>\n",
                xml_escape(v)
            ));
        }
    }
    if ids.is_empty() {
        return None;
    }
    let year = item
        .year
        .map(|y| format!("  <year>{y}</year>\n"))
        .unwrap_or_default();
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{root}>\n  <title>{}</title>\n{year}{ids}</{root}>\n",
        xml_escape(&item.title)
    );
    Some(Arc::from(text.into_bytes()))
}

/// A server path for artwork: only the server's own paths, never a URL
/// (an absolute URL would be fetched from the server's address).
fn art_path(path: Option<&str>) -> Option<String> {
    path.map(str::trim)
        .filter(|p| p.starts_with('/') && !p.starts_with("//"))
        .map(str::to_owned)
}

#[derive(Default)]
struct Builder {
    tree: Tree,
}

impl Builder {
    fn new(view_dir: &str) -> Self {
        let mut b = Self::default();
        b.tree
            .nodes
            .insert(format!("/{view_dir}"), Node::Dir(BTreeSet::new()));
        b
    }

    fn children(&mut self, dir: &str) -> Option<&mut BTreeSet<String>> {
        match self
            .tree
            .nodes
            .entry(dir.to_owned())
            .or_insert_with(|| Node::Dir(BTreeSet::new()))
        {
            Node::Dir(c) => Some(c),
            _ => None,
        }
    }

    fn taken(&self, dir: &str, name: &str) -> bool {
        self.tree.nodes.contains_key(&format!("{dir}/{name}"))
    }

    /// A name in `dir` not used yet: `name`, else `name (#2)`, …
    fn unique(&self, dir: &str, name: &str) -> String {
        if !self.taken(dir, name) {
            return name.to_owned();
        }
        (2..)
            .map(|n| format!("{name} (#{n})"))
            .find(|n| !self.taken(dir, n))
            .unwrap_or_default()
    }

    fn put(&mut self, dir: &str, name: &str, node: Node) -> String {
        let path = format!("{dir}/{name}");
        let Some(children) = self.children(dir) else {
            return path;
        };
        children.insert(name.to_owned());
        if let Node::Media(m) = &node {
            self.tree
                .by_item
                .entry(m.rating_key.clone())
                .or_insert(path.clone());
        }
        if matches!(node, Node::Dir(_)) {
            self.tree.nodes.entry(path.clone()).or_insert(node);
        } else {
            self.tree.nodes.insert(path.clone(), node);
        }
        path
    }

    fn folder(&mut self, dir: &str, name: &str) -> String {
        self.put(dir, name, Node::Dir(BTreeSet::new()))
    }

    fn item_folder(&mut self, dir: &str, name: &str) -> String {
        let name = self.unique(dir, name);
        self.folder(dir, &name)
    }

    fn art(&mut self, dir: &str, item: &Item) {
        if let Some(p) = art_path(item.thumb.as_deref()) {
            self.put(dir, "poster.jpg", Node::Remote(p));
        }
        if let Some(p) = art_path(item.art.as_deref()) {
            self.put(dir, "fanart.jpg", Node::Remote(p));
        }
    }

    /// The files of `item` in `dir`, named `stem.<ext>`; other versions
    /// `stem - <version>.<ext>`, parts `stem - partN.<ext>`.
    fn media(&mut self, dir: &str, stem: &str, item: &Item, audio: bool) {
        let modified_ms = item.added_at.map(|s| s.saturating_mul(1000));
        for (i, media) in item.media.iter().enumerate() {
            let version_stem = if i == 0 {
                stem.to_owned()
            } else {
                let v = media
                    .title
                    .as_deref()
                    .map(clean)
                    .filter(|t| !t.is_empty() && *t != "Untitled")
                    .or_else(|| {
                        let r = media.video_resolution.trim();
                        (!r.is_empty()).then(|| {
                            if r.chars().all(|c| c.is_ascii_digit()) {
                                format!("{r}p")
                            } else {
                                r.to_ascii_uppercase()
                            }
                        })
                    })
                    .unwrap_or_else(|| format!("v{}", i + 1));
                format!("{stem} - {v}")
            };
            let many = media.parts.len() > 1;
            for (n, part) in media.parts.iter().enumerate() {
                if part.key.trim().is_empty() || !part.key.starts_with('/') {
                    continue;
                }
                let part_stem = if many {
                    format!("{version_stem} - part{}", n + 1)
                } else {
                    version_stem.clone()
                };
                let ext = extension(part, media, audio);
                let name = self.unique(dir, &format!("{part_stem}.{ext}"));
                self.put(
                    dir,
                    &name,
                    Node::Media(MediaRef {
                        rating_key: item.rating_key.clone(),
                        part_key: part.key.clone(),
                        audio,
                        size: part.size.and_then(|s| u64::try_from(s).ok()).unwrap_or(0),
                        modified_ms,
                        duration_ms: part.duration.or(media.duration).or(item.duration),
                        view_offset_ms: item.view_offset,
                        watched: item.view_count.unwrap_or(0) > 0,
                    }),
                );
            }
        }
    }
}

/// Lays out a movie (or "other videos") library.
fn build_movies(view_dir: &str, movies: &[Item]) -> Tree {
    let root = format!("/{view_dir}");
    let mut b = Builder::new(view_dir);
    for m in movies {
        let name = titled(m);
        let dir = b.item_folder(&root, &name);
        let stem = dir.rsplit('/').next().unwrap_or(&name).to_owned();
        if let Some(n) = nfo("movie", m) {
            b.put(&dir, "movie.nfo", Node::Text(n));
        }
        b.art(&dir, m);
        b.media(&dir, &stem, m, false);
    }
    b.tree
}

/// Lays out a TV library from its shows and episodes.
fn build_shows(view_dir: &str, shows: &[Item], episodes: &[Item]) -> Tree {
    let root = format!("/{view_dir}");
    let mut b = Builder::new(view_dir);
    let mut dirs: HashMap<String, (String, String)> = HashMap::new();
    for s in shows {
        let dir = b.item_folder(&root, &titled(s));
        if let Some(n) = nfo("tvshow", s) {
            b.put(&dir, "tvshow.nfo", Node::Text(n));
        }
        b.art(&dir, s);
        dirs.insert(s.rating_key.clone(), (dir, clean(&s.title)));
    }
    for e in episodes {
        let fallback = e
            .grandparent_title
            .as_deref()
            .map_or_else(|| "Unknown Show".to_owned(), clean);
        let (show_dir, show) = match dirs.get(&e.grandparent_rating_key) {
            Some(d) => d.clone(),
            None => {
                let d = b.folder(&root, &fallback);
                if !e.grandparent_rating_key.is_empty() {
                    dirs.insert(
                        e.grandparent_rating_key.clone(),
                        (d.clone(), fallback.clone()),
                    );
                }
                (d, fallback)
            }
        };
        let season = e.parent_index.unwrap_or(1).max(0);
        let season_dir = b.folder(
            &show_dir,
            &if season == 0 {
                "Specials".to_owned()
            } else {
                format!("Season {season:02}")
            },
        );
        let title = clean(&e.title);
        let stem = match e.index {
            Some(n) => format!("{show} S{season:02}E{n:02} - {title}"),
            None => format!("{show} - {title}"),
        };
        b.media(&season_dir, &stem, e, false);
    }
    b.tree
}

/// Lays out a music library from its tracks.
fn build_music(view_dir: &str, tracks: &[Item]) -> Tree {
    let root = format!("/{view_dir}");
    let mut b = Builder::new(view_dir);
    let mut covers: BTreeSet<String> = BTreeSet::new();
    for t in tracks {
        let artist = t
            .grandparent_title
            .as_deref()
            .or(t.original_title.as_deref())
            .filter(|a| !a.trim().is_empty())
            .map_or_else(|| "Unknown Artist".to_owned(), clean);
        let album = t
            .parent_title
            .as_deref()
            .filter(|a| !a.trim().is_empty())
            .map_or_else(|| "Unknown Album".to_owned(), clean);
        let artist_dir = b.folder(&root, &artist);
        let album_dir = b.folder(&artist_dir, &album);
        if let Some(p) = art_path(t.parent_thumb.as_deref())
            && covers.insert(album_dir.clone())
        {
            b.put(&album_dir, "cover.jpg", Node::Remote(p));
        }
        let title = clean(&t.title);
        let stem = match (t.parent_index, t.index) {
            (Some(d), Some(n)) if d > 1 => format!("{d}-{n:02} - {title}"),
            (_, Some(n)) => format!("{n:02} - {title}"),
            _ => title,
        };
        b.media(&album_dir, &stem, t, true);
    }
    b.tree
}

impl PlexFs {
    /// The libraries of `server` (with a server token).
    #[must_use]
    pub fn new(server: Arc<Server>) -> Self {
        Self {
            server,
            libraries: None,
            ttl: Duration::from_secs(300),
            read_ahead: DEFAULT_READ_AHEAD,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// Only these libraries (section keys), in place of all.
    #[must_use]
    pub fn with_libraries(mut self, keys: Vec<String>) -> Self {
        self.libraries = (!keys.is_empty()).then_some(keys);
        self
    }

    /// How long a listing of the server is reused (default 5 min).
    #[must_use]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Bytes fetched per range request of a media file.
    #[must_use]
    pub fn with_read_ahead(mut self, bytes: usize) -> Self {
        self.read_ahead = bytes.max(64 << 10);
        self
    }

    /// The server.
    #[must_use]
    pub fn server(&self) -> &Arc<Server> {
        &self.server
    }

    /// Forgets the cached listings: the next call reads the server again.
    pub fn refresh(&self) {
        let mut c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        *c = Cache::default();
    }

    fn views(&self) -> pa_vfs::Result<Arc<Vec<View>>> {
        {
            let c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, v)) = &c.views
                && at.elapsed() < self.ttl
            {
                return Ok(Arc::clone(v));
            }
        }
        let sections: Vec<Section> = self.server.sections().map_err(|e| vfs_error(&e))?;
        let mut views: Vec<View> = Vec::new();
        for s in sections {
            if let Some(only) = &self.libraries
                && !only.contains(&s.key)
            {
                continue;
            }
            if !matches!(s.kind.as_str(), "movie" | "show" | "artist") {
                continue;
            }
            let base = clean(&s.title);
            let mut dir = base.clone();
            let mut n = 2;
            while views.iter().any(|v| v.dir == dir) {
                dir = format!("{base} (#{n})");
                n += 1;
            }
            views.push(View {
                dir,
                key: s.key,
                kind: s.kind,
            });
        }
        let views = Arc::new(views);
        let mut c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        c.views = Some((Instant::now(), Arc::clone(&views)));
        Ok(views)
    }

    fn tree(&self, view: &View) -> pa_vfs::Result<Arc<Tree>> {
        {
            let c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, t)) = c.trees.get(&view.key)
                && at.elapsed() < self.ttl
            {
                return Ok(Arc::clone(t));
            }
        }
        let items = |t: u8| self.server.items(&view.key, t).map_err(|e| vfs_error(&e));
        let tree = Arc::new(match view.kind.as_str() {
            "show" => build_shows(&view.dir, &items(2)?, &items(4)?),
            "artist" => build_music(&view.dir, &items(10)?),
            _ => build_movies(&view.dir, &items(1)?),
        });
        let mut c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        c.trees
            .insert(view.key.clone(), (Instant::now(), Arc::clone(&tree)));
        Ok(tree)
    }

    /// The node at `path` (`None`: the root).
    fn node(&self, path: &VfsPath) -> pa_vfs::Result<Option<(Arc<Tree>, Node)>> {
        let Some(first) = path.components().next() else {
            return Ok(None);
        };
        let views = self.views()?;
        let view = views
            .iter()
            .find(|v| v.dir == first)
            .ok_or_else(|| not_found(path.as_str()))?;
        let tree = self.tree(view)?;
        let node = tree
            .nodes
            .get(path.as_str())
            .cloned()
            .ok_or_else(|| not_found(path.as_str()))?;
        Ok(Some((tree, node)))
    }

    /// The media file at `path`, `None` for folders and sidecars.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for a path not on the server; network
    /// errors.
    pub fn media_at(&self, path: &VfsPath) -> pa_vfs::Result<Option<MediaRef>> {
        Ok(match self.node(path)? {
            Some((_, Node::Media(m))) => Some(m),
            _ => None,
        })
    }

    /// Path of the first media file of item `rating_key` in any library.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn path_of(&self, rating_key: &str) -> pa_vfs::Result<Option<VfsPath>> {
        for v in self.views()?.iter() {
            if let Some(p) = self.tree(v)?.by_item.get(rating_key) {
                return VfsPath::parse(p).map(Some);
            }
        }
        Ok(None)
    }

    /// Every media file with the user's state as of the last listing.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn media_files(&self) -> pa_vfs::Result<Vec<(VfsPath, MediaRef)>> {
        let mut out = Vec::new();
        for v in self.views()?.iter() {
            let tree = self.tree(v)?;
            for (path, node) in &tree.nodes {
                if let Node::Media(m) = node {
                    out.push((VfsPath::parse(path)?, m.clone()));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

impl Vfs for PlexFs {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        let Some((tree, node)) = self.node(dir)? else {
            return self
                .views()?
                .iter()
                .map(|v| {
                    Ok(DirEntry {
                        name: v.dir.clone(),
                        path: dir.join(&v.dir)?,
                        metadata: Node::Dir(BTreeSet::new()).metadata(),
                    })
                })
                .collect();
        };
        let Node::Dir(children) = node else {
            return Err(VfsError::new(
                ErrorKind::NotADirectory,
                format!("{dir}: not a folder"),
            ));
        };
        children
            .iter()
            .map(|name| {
                let path = dir.join(name)?;
                let metadata = tree
                    .nodes
                    .get(path.as_str())
                    .map_or_else(|| Node::Dir(BTreeSet::new()).metadata(), Node::metadata);
                Ok(DirEntry {
                    name: name.clone(),
                    path,
                    metadata,
                })
            })
            .collect()
    }

    fn stat(&self, path: &VfsPath) -> pa_vfs::Result<Metadata> {
        Ok(match self.node(path)? {
            None => Node::Dir(BTreeSet::new()).metadata(),
            Some((_, n)) => n.metadata(),
        })
    }

    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        match self.node(path)? {
            None | Some((_, Node::Dir(_))) => Err(VfsError::new(
                ErrorKind::IsADirectory,
                format!("{path}: a folder"),
            )),
            Some((_, Node::Text(t))) => Ok(Box::new(Memory(t))),
            Some((_, Node::Remote(p))) => {
                let data = self.server.get_bytes(&p).map_err(|e| vfs_error(&e))?;
                Ok(Box::new(Memory(Arc::from(data))))
            }
            Some((_, Node::Media(m))) => Ok(Box::new(PartReader {
                server: Arc::clone(&self.server),
                path: m.part_key,
                size: m.size,
                read_ahead: self.read_ahead,
                blocks: Mutex::new(VecDeque::new()),
                random: AtomicBool::new(false),
            })),
        }
    }
}

/// A small file held in memory.
struct Memory(Arc<[u8]>);

impl fmt::Debug for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Memory({} bytes)", self.0.len())
    }
}

impl ByteSource for Memory {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        Ok(copy_from(&self.0, offset, buf))
    }
}

fn copy_from(data: &[u8], offset: u64, buf: &mut [u8]) -> usize {
    let Ok(start) = usize::try_from(offset) else {
        return 0;
    };
    if start >= data.len() {
        return 0;
    }
    let n = buf.len().min(data.len() - start);
    buf[..n].copy_from_slice(&data[start..start + n]);
    n
}

/// A file of the server, read with byte ranges and a small read-ahead
/// cache (the proxy asks for many short ranges in a row).
struct PartReader {
    server: Arc<Server>,
    path: String,
    size: u64,
    read_ahead: usize,
    blocks: Mutex<VecDeque<(u64, Arc<[u8]>)>>,
    random: AtomicBool,
}

impl fmt::Debug for PartReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PartReader")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// `bytes 100-199/1000` → 100.
fn range_start(header: &str) -> Option<u64> {
    header
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

impl PartReader {
    fn cached(&self, offset: u64, buf: &mut [u8]) -> Option<usize> {
        let blocks = self.blocks.lock().unwrap_or_else(PoisonError::into_inner);
        blocks.iter().find_map(|(start, data)| {
            let end = start + data.len() as u64;
            (offset >= *start && offset < end).then(|| copy_from(data, offset - start, buf))
        })
    }

    fn fetch(&self, offset: u64, len: usize) -> pa_vfs::Result<Vec<u8>> {
        let r = self
            .server
            .send(&self.path, Some((offset, len as u64)), MAX_BLOB)
            .and_then(check)
            .map_err(|e| vfs_error(&e))?;
        match r.status {
            206 => {
                let start = r.content_range.as_deref().and_then(range_start);
                if start != Some(offset) {
                    return Err(VfsError::new(
                        ErrorKind::Other,
                        "the server answered another byte range",
                    ));
                }
                Ok(r.body)
            }
            // The whole file (a server ignoring Range): cut the part.
            _ => {
                let start = usize::try_from(offset).unwrap_or(usize::MAX);
                Ok(r.body
                    .get(start..)
                    .map_or_else(Vec::new, |b| b[..b.len().min(len)].to_vec()))
            }
        }
    }
}

impl ByteSource for PartReader {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        if buf.is_empty() || offset >= self.size {
            return Ok(0);
        }
        let left = self.size - offset;
        let want = usize::try_from(left).map_or(buf.len(), |l| l.min(buf.len()));
        let buf = &mut buf[..want];
        if let Some(n) = self.cached(offset, buf) {
            return Ok(n);
        }
        if want >= self.read_ahead || self.random.load(Ordering::Relaxed) {
            let data = self.fetch(offset, want)?;
            return Ok(copy_from(&data, 0, buf));
        }
        let block = usize::try_from(left).map_or(self.read_ahead, |l| l.min(self.read_ahead));
        let data: Arc<[u8]> = self.fetch(offset, block)?.into();
        let n = copy_from(&data, 0, buf);
        if !data.is_empty() {
            let mut blocks = self.blocks.lock().unwrap_or_else(PoisonError::into_inner);
            blocks.push_front((offset, data));
            blocks.truncate(CACHE_BLOCKS);
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

    fn item(json: &str) -> Item {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn lays_out_movies_with_versions_and_parts() {
        let movies = [
            item(
                r#"{"ratingKey":"1","type":"movie","title":"Dune: Part One","year":2021,"thumb":"/library/metadata/1/thumb/9","art":"/library/metadata/1/art/9","addedAt":1700000000,"viewCount":1,
                "Guid":[{"id":"tmdb://438631"},{"id":"imdb://tt1160419"}],
                "Media":[{"id":10,"container":"mkv","Part":[{"id":100,"key":"/library/parts/100/1/file.mkv","size":1000,"file":"/m/Dune.mkv"}]},
                         {"id":11,"videoResolution":"4k","Part":[{"id":101,"key":"/library/parts/101/1/file.mp4","size":2000,"file":"/m/Dune 4K.mp4"}]}]}"#,
            ),
            item(
                r#"{"ratingKey":"2","type":"movie","title":"Long","thumb":"http://elsewhere/x.jpg",
                "Media":[{"id":12,"Part":[{"id":102,"key":"/library/parts/102/1/a.avi","size":5},{"id":103,"key":"/library/parts/103/1/b.avi","size":6}]}]}"#,
            ),
        ];
        let t = build_movies("Movies", &movies);
        let dir = "/Movies/Dune - Part One (2021)";
        let Some(Node::Dir(children)) = t.nodes.get(dir) else {
            panic!("no movie folder: {:?}", t.nodes.keys());
        };
        let names: Vec<&str> = children.iter().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "Dune - Part One (2021) - 4K.mp4",
                "Dune - Part One (2021).mkv",
                "fanart.jpg",
                "movie.nfo",
                "poster.jpg"
            ]
        );
        let Some(Node::Media(m)) = t.nodes.get(&format!("{dir}/Dune - Part One (2021).mkv")) else {
            panic!("no file");
        };
        assert_eq!(m.part_key, "/library/parts/100/1/file.mkv");
        assert_eq!(m.size, 1000);
        assert_eq!(m.modified_ms, Some(1_700_000_000_000));
        assert!(m.watched);
        let Some(Node::Text(nfo)) = t.nodes.get(&format!("{dir}/movie.nfo")) else {
            panic!("no nfo");
        };
        let nfo = String::from_utf8_lossy(nfo);
        assert!(nfo.contains("<uniqueid type=\"tmdb\">438631</uniqueid>"));
        assert!(nfo.contains("<year>2021</year>"));
        // Parts, and no artwork from another host.
        assert!(t.nodes.contains_key("/Movies/Long/Long - part1.avi"));
        assert!(t.nodes.contains_key("/Movies/Long/Long - part2.avi"));
        assert!(!t.nodes.contains_key("/Movies/Long/poster.jpg"));
        assert_eq!(t.by_item["1"], format!("{dir}/Dune - Part One (2021).mkv"));
    }

    #[test]
    fn lays_out_shows_and_music() {
        let shows = [item(
            r#"{"ratingKey":"5","type":"show","title":"Dark","year":2017,"thumb":"/t","Guid":[{"id":"tvdb://334824"}]}"#,
        )];
        let episodes = [
            item(
                r#"{"ratingKey":"6","type":"episode","title":"Secrets","grandparentRatingKey":"5","grandparentTitle":"Dark","parentIndex":1,"index":1,"viewOffset":60000,
                "Media":[{"Part":[{"key":"/library/parts/6/1/e.mkv","size":9}]}]}"#,
            ),
            item(
                r#"{"ratingKey":"7","type":"episode","title":"Pilot","grandparentRatingKey":"8","grandparentTitle":"Other","parentIndex":0,"index":2,
                "Media":[{"Part":[{"key":"/library/parts/7/1/e.mp4","size":9}]}]}"#,
            ),
        ];
        let t = build_shows("TV", &shows, &episodes);
        assert!(t.nodes.contains_key("/TV/Dark (2017)/tvshow.nfo"));
        let Some(Node::Media(m)) = t
            .nodes
            .get("/TV/Dark (2017)/Season 01/Dark S01E01 - Secrets.mkv")
        else {
            panic!("no episode: {:?}", t.nodes.keys());
        };
        assert_eq!(m.view_offset_ms, Some(60_000));
        assert!(!m.watched);
        assert!(
            t.nodes
                .contains_key("/TV/Other/Specials/Other S00E02 - Pilot.mp4")
        );
        let tracks = [
            item(
                r#"{"ratingKey":"20","type":"track","title":"Song","grandparentTitle":"Artist","parentTitle":"Album","parentThumb":"/a","parentIndex":1,"index":3,
                "Media":[{"Part":[{"key":"/library/parts/20/1/s.flac","size":9,"container":"flac"}]}]}"#,
            ),
            item(
                r#"{"ratingKey":"21","type":"track","title":"B-side","grandparentTitle":"Artist","parentTitle":"Album","parentIndex":2,"index":1,
                "Media":[{"Part":[{"key":"/library/parts/21/1/file","size":9,"container":"mp3"}]}]}"#,
            ),
        ];
        let t = build_music("Music", &tracks);
        let Some(Node::Media(m)) = t.nodes.get("/Music/Artist/Album/03 - Song.flac") else {
            panic!("no track: {:?}", t.nodes.keys());
        };
        assert!(m.audio);
        assert!(
            t.nodes
                .contains_key("/Music/Artist/Album/2-01 - B-side.mp3")
        );
        assert!(t.nodes.contains_key("/Music/Artist/Album/cover.jpg"));
    }

    #[test]
    fn cleans_names() {
        assert_eq!(clean("a/b: c?"), "a_b - c_");
        assert_eq!(clean(" .. "), "Untitled");
        assert_eq!(art_path(Some("//evil/x")), None);
        assert_eq!(art_path(Some("/library/x")), Some("/library/x".into()));
    }
}
