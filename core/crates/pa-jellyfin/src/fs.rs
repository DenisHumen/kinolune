// SPDX-License-Identifier: MPL-2.0
//! The libraries of a Jellyfin user as a read-only [`Vfs`] tree, laid out
//! the way the scanner and the recognition already read local folders
//! (Kodi/Jellyfin naming):
//!
//! ```text
//! /Movies/Dune (2021)/Dune (2021).mkv          the media file (byte ranges)
//! /Movies/Dune (2021)/movie.nfo                TMDB/IMDb/TVDB ids, if known
//! /Movies/Dune (2021)/poster.jpg, fanart.jpg   the server's artwork
//! /Movies/Dune (2021)/Dune (2021).jellyfin.json   Media Segments (PLAY-015)
//! /Movies/Dune (2021)/Dune (2021).trickplay/320 - 10x10/0.jpg   trickplay
//! /Shows/Dark (2017)/tvshow.nfo, poster.jpg
//! /Shows/Dark (2017)/Season 01/Dark S01E01 - Secrets.mkv
//! /Music/Artist/Album (2021)/01 - Song.flac, cover.jpg
//! ```
//!
//! So a Jellyfin source is scanned, recognized, browsed, searched and
//! played through the stream proxy like any other source, and the
//! existing trickplay (ADR-0048) and skip-marker import (ADR-0061) read the
//! server's previews and segments as sidecars.
//!
//! An Emby server ([`ServerKind::Emby`]) gives the same tree with its own
//! sidecars: the preview thumbnails as `<stem>.bif` (BIF, which
//! `pa-trickplay` reads; opening it fails with "not found" when the
//! server made none, so previews are generated as for any file) and the
//! intro and credits chapter markers as `<stem>.jellyfin.json` in the
//! media segments format, only for items that have markers.
//!
//! The tree of a library comes from one paged `/Items` query and is kept
//! for a while ([`JellyfinFs::with_ttl`]); [`JellyfinFs::refresh`] drops
//! it. Media files report their real size. Artwork, trickplay sheets and
//! segment files are fetched when opened and are listed with size 0 (their
//! size is only known then); [`ByteSource::size`] of an open one is exact.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Vfs, VfsError, VfsPath};

use crate::Error;
use crate::api::{ItemQuery, bif_frames, bif_path, emby_segments_file, image_path, trickplay_path};
use crate::http::{Client, MAX_BLOB, Method, ServerKind};
use crate::model::{Item, MediaSource, UserData, parse_date_ms};
use crate::playback::static_stream_path;

/// Default read-ahead of a media file: one range request per 2 MiB.
pub const DEFAULT_READ_AHEAD: usize = 2 << 20;
/// Blocks of read-ahead kept per open file.
const CACHE_BLOCKS: usize = 4;
/// Item types placed in the tree.
const TYPES: &[&str] = &["Movie", "Video", "MusicVideo", "Series", "Episode", "Audio"];

/// The media file behind a path of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    /// Jellyfin item id.
    pub item_id: String,
    /// Version (media source) id.
    pub media_source_id: String,
    /// A track (audio stream endpoint) rather than a video.
    pub audio: bool,
    /// File size.
    pub size: u64,
    /// When the server added it, Unix ms.
    pub modified_ms: Option<i64>,
    /// Duration, ms.
    pub duration_ms: Option<i64>,
    /// The user's state when the tree was read.
    pub user_data: Option<UserData>,
}

#[derive(Debug, Clone)]
enum Node {
    Dir(BTreeSet<String>),
    Media(MediaRef),
    /// Made here (NFO).
    Text(Arc<[u8]>),
    /// Fetched from the server when opened: path and query.
    Remote(String),
    /// An Emby BIF, fetched when opened; an empty one is "not found".
    RemoteBif(String),
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
            Self::Remote(_) | Self::RemoteBif(_) => Metadata {
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

#[derive(Debug)]
struct View {
    dir: String,
    id: String,
}

#[derive(Default)]
struct Cache {
    views: Option<(Instant, Arc<Vec<View>>)>,
    trees: HashMap<String, (Instant, Arc<Tree>)>,
}

/// The libraries of one Jellyfin user as a [`Vfs`] (see the module docs).
pub struct JellyfinFs {
    client: Arc<Client>,
    user_id: String,
    libraries: Option<Vec<String>>,
    ttl: Duration,
    read_ahead: usize,
    cache: Mutex<Cache>,
}

impl fmt::Debug for JellyfinFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JellyfinFs")
            .field("client", &self.client)
            .field("user_id", &self.user_id)
            .field("libraries", &self.libraries)
            .finish_non_exhaustive()
    }
}

pub(crate) fn vfs_error(e: &Error) -> VfsError {
    let kind = match e {
        Error::Invalid(_) => ErrorKind::InvalidPath,
        Error::Unreachable(_) => ErrorKind::Unavailable,
        Error::TimedOut => ErrorKind::TimedOut,
        Error::Unauthorized => ErrorKind::AuthenticationFailed,
        Error::Forbidden => ErrorKind::PermissionDenied,
        Error::NotFound => ErrorKind::NotFound,
        Error::Unsupported(_) => ErrorKind::Unsupported,
        _ => ErrorKind::Other,
    };
    VfsError::new(kind, e.to_string())
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
    let name = clean(&item.name);
    match item.production_year {
        Some(y) if !name.ends_with(&format!("({y})")) => format!("{name} ({y})"),
        _ => name,
    }
}

/// File extension of a media source.
fn extension(item: &Item, source: &MediaSource, audio: bool) -> String {
    let from_path = source
        .path
        .as_deref()
        .or(item.path.as_deref())
        .and_then(|p| p.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()));
    from_path
        .or_else(|| {
            source
                .container
                .as_deref()
                .or(item.container.as_deref())
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
        if let Some(v) = item.provider_id(p) {
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
        .production_year
        .map(|y| format!("  <year>{y}</year>\n"))
        .unwrap_or_default();
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{root}>\n  <title>{}</title>\n{year}{ids}</{root}>\n",
        xml_escape(&item.name)
    );
    Some(Arc::from(text.into_bytes()))
}

struct Builder {
    tree: Tree,
    kind: ServerKind,
}

impl Builder {
    fn new(view_dir: &str, kind: ServerKind) -> Self {
        let mut tree = Tree::default();
        tree.nodes
            .insert(format!("/{view_dir}"), Node::Dir(BTreeSet::new()));
        Self { tree, kind }
    }

    /// Children of folder `dir`, created as needed; `None` if a file has
    /// that path.
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

    /// Adds `name` to `dir` (which is created as needed).
    fn put(&mut self, dir: &str, name: &str, node: Node) -> String {
        let path = format!("{dir}/{name}");
        let Some(children) = self.children(dir) else {
            return path;
        };
        children.insert(name.to_owned());
        if let Node::Media(m) = &node {
            self.tree
                .by_item
                .entry(m.item_id.clone())
                .or_insert(path.clone());
        }
        if matches!(node, Node::Dir(_)) {
            self.tree.nodes.entry(path.clone()).or_insert(node);
        } else {
            self.tree.nodes.insert(path.clone(), node);
        }
        path
    }

    /// A folder named `name` in `dir`, reused if it exists.
    fn folder(&mut self, dir: &str, name: &str) -> String {
        self.put(dir, name, Node::Dir(BTreeSet::new()))
    }

    /// A new folder in `dir` for an item, with a unique name.
    fn item_folder(&mut self, dir: &str, name: &str) -> String {
        let name = self.unique(dir, name);
        self.folder(dir, &name)
    }

    fn art(&mut self, dir: &str, item: &Item) {
        if let Some(tag) = item.image_tags.get("Primary") {
            let p = image_path(&item.id, "Primary", Some(tag), 1000);
            self.put(dir, "poster.jpg", Node::Remote(p));
        }
        if let Some(tag) = item.backdrop_image_tags.first() {
            let p = image_path(&item.id, "Backdrop", Some(tag), 1920);
            self.put(dir, "fanart.jpg", Node::Remote(p));
        }
    }

    /// The media files of `item` in `dir`, named `stem.<ext>` (other
    /// versions `stem - <version>.<ext>`), with their sidecars.
    fn media(&mut self, dir: &str, stem: &str, item: &Item, audio: bool) {
        let modified_ms = item.date_created.as_deref().and_then(parse_date_ms);
        for (i, source) in item.media_sources.iter().enumerate() {
            let stem = if i == 0 {
                stem.to_owned()
            } else {
                let v = source
                    .name
                    .as_deref()
                    .map(clean)
                    .filter(|n| !n.is_empty() && *n != clean(&item.name))
                    .unwrap_or_else(|| format!("v{}", i + 1));
                format!("{stem} - {v}")
            };
            let ext = extension(item, source, audio);
            let name = self.unique(dir, &format!("{stem}.{ext}"));
            let file_stem = name
                .rsplit_once('.')
                .map_or(name.as_str(), |(s, _)| s)
                .to_owned();
            self.put(
                dir,
                &name,
                Node::Media(MediaRef {
                    item_id: item.id.clone(),
                    media_source_id: source.id.clone(),
                    audio,
                    size: source.size.and_then(|s| u64::try_from(s).ok()).unwrap_or(0),
                    modified_ms,
                    duration_ms: source
                        .run_time_ticks
                        .or(item.run_time_ticks)
                        .map(crate::model::ticks_to_ms),
                    user_data: item.user_data.clone(),
                }),
            );
            if audio {
                continue;
            }
            if self.kind == ServerKind::Emby {
                // Emby: one BIF and one marker set per item.
                if i == 0 {
                    self.put(
                        dir,
                        &format!("{file_stem}.bif"),
                        Node::RemoteBif(bif_path(&item.id, 320)),
                    );
                    if let Some(json) = emby_segments_file(item) {
                        self.put(
                            dir,
                            &format!("{file_stem}.jellyfin.json"),
                            Node::Text(Arc::from(json)),
                        );
                    }
                }
                continue;
            }
            if i == 0 {
                self.put(
                    dir,
                    &format!("{file_stem}.jellyfin.json"),
                    Node::Remote(format!("/MediaSegments/{}", crate::http::encode(&item.id))),
                );
            }
            if let Some(t) = item
                .trickplay
                .get(&source.id)
                .and_then(|w| w.values().min_by_key(|t| t.width.abs_diff(320)))
            {
                let trick = self.folder(dir, &format!("{file_stem}.trickplay"));
                let res = self.folder(
                    &trick,
                    &format!("{} - {}x{}", t.width, t.tile_width, t.tile_height),
                );
                for n in 0..t.sheets() {
                    let p = trickplay_path(&item.id, &source.id, t.width, n);
                    self.put(&res, &format!("{n}.jpg"), Node::Remote(p));
                }
            }
        }
    }
}

/// Lays out the items of the library at `/<view_dir>`.
fn build(view_dir: &str, items: &[Item], kind: ServerKind) -> Tree {
    let root = format!("/{view_dir}");
    let mut b = Builder::new(view_dir, kind);
    let mut series_dirs: HashMap<String, String> = HashMap::new();
    let mut series_names: HashMap<String, String> = HashMap::new();
    for s in items.iter().filter(|i| i.kind == "Series") {
        let dir = b.item_folder(&root, &titled(s));
        if let Some(n) = nfo("tvshow", s) {
            b.put(&dir, "tvshow.nfo", Node::Text(n));
        }
        b.art(&dir, s);
        series_names.insert(s.id.clone(), clean(&s.name));
        series_dirs.insert(s.id.clone(), dir);
    }
    let mut album_covers: BTreeSet<String> = BTreeSet::new();
    for item in items {
        match item.kind.as_str() {
            "Movie" | "Video" | "MusicVideo" => {
                let name = titled(item);
                let dir = b.item_folder(&root, &name);
                let stem = dir.rsplit('/').next().unwrap_or(&name).to_owned();
                if let Some(n) = nfo("movie", item) {
                    b.put(&dir, "movie.nfo", Node::Text(n));
                }
                b.art(&dir, item);
                b.media(&dir, &stem, item, false);
            }
            "Episode" => {
                let series = item
                    .series_name
                    .as_deref()
                    .map_or_else(|| "Unknown Show".to_owned(), clean);
                let show_dir = match item.series_id.as_ref().and_then(|id| series_dirs.get(id)) {
                    Some(d) => d.clone(),
                    None => {
                        let d = b.folder(&root, &series);
                        if let Some(id) = &item.series_id {
                            series_dirs.insert(id.clone(), d.clone());
                        }
                        d
                    }
                };
                let show = item
                    .series_id
                    .as_ref()
                    .and_then(|id| series_names.get(id))
                    .cloned()
                    .unwrap_or(series);
                let season = item.parent_index_number.unwrap_or(1).max(0);
                let season_dir = b.folder(
                    &show_dir,
                    &if season == 0 {
                        "Specials".to_owned()
                    } else {
                        format!("Season {season:02}")
                    },
                );
                let title = clean(&item.name);
                let stem = match (item.index_number, item.index_number_end) {
                    (Some(e), Some(end)) if end > e => {
                        format!("{show} S{season:02}E{e:02}-E{end:02} - {title}")
                    }
                    (Some(e), _) => format!("{show} S{season:02}E{e:02} - {title}"),
                    (None, _) => format!("{show} - {title}"),
                };
                b.media(&season_dir, &stem, item, false);
            }
            "Audio" => {
                let artist = item
                    .album_artist
                    .as_deref()
                    .or(item.artists.first().map(String::as_str))
                    .filter(|a| !a.trim().is_empty())
                    .map_or_else(|| "Unknown Artist".to_owned(), clean);
                let album = item
                    .album
                    .as_deref()
                    .filter(|a| !a.trim().is_empty())
                    .map_or_else(|| "Unknown Album".to_owned(), clean);
                let artist_dir = b.folder(&root, &artist);
                let album_dir = b.folder(&artist_dir, &album);
                if let (Some(id), Some(tag)) = (&item.album_id, &item.album_primary_image_tag)
                    && album_covers.insert(album_dir.clone())
                {
                    let p = image_path(id, "Primary", Some(tag), 1000);
                    b.put(&album_dir, "cover.jpg", Node::Remote(p));
                }
                let title = clean(&item.name);
                let stem = match (item.parent_index_number, item.index_number) {
                    (Some(d), Some(n)) if d > 1 => format!("{d}-{n:02} - {title}"),
                    (_, Some(n)) => format!("{n:02} - {title}"),
                    _ => title,
                };
                b.media(&album_dir, &stem, item, true);
            }
            _ => {}
        }
    }
    b.tree
}

impl JellyfinFs {
    /// The libraries `user_id` sees through `client` (signed in).
    #[must_use]
    pub fn new(client: Arc<Client>, user_id: impl Into<String>) -> Self {
        Self {
            client,
            user_id: user_id.into(),
            libraries: None,
            ttl: Duration::from_secs(300),
            read_ahead: DEFAULT_READ_AHEAD,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// Only these libraries (ids), in place of all of the user's.
    #[must_use]
    pub fn with_libraries(mut self, ids: Vec<String>) -> Self {
        self.libraries = (!ids.is_empty()).then_some(ids);
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

    /// The client.
    #[must_use]
    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }

    /// The user.
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
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
        let libs = self
            .client
            .libraries(&self.user_id)
            .map_err(|e| vfs_error(&e))?;
        let mut views: Vec<View> = Vec::new();
        for l in libs {
            if let Some(only) = &self.libraries
                && !only.contains(&l.id)
            {
                continue;
            }
            // Playlists, collections, live TV: not files.
            if matches!(
                l.collection_type.as_deref(),
                Some("playlists" | "boxsets" | "livetv" | "books" | "photos" | "folders")
            ) {
                continue;
            }
            let base = clean(&l.name);
            let mut dir = base.clone();
            let mut n = 2;
            while views.iter().any(|v| v.dir == dir) {
                dir = format!("{base} (#{n})");
                n += 1;
            }
            views.push(View { dir, id: l.id });
        }
        let views = Arc::new(views);
        let mut c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        c.views = Some((Instant::now(), Arc::clone(&views)));
        Ok(views)
    }

    fn tree(&self, view: &View) -> pa_vfs::Result<Arc<Tree>> {
        {
            let c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, t)) = c.trees.get(&view.id)
                && at.elapsed() < self.ttl
            {
                return Ok(Arc::clone(t));
            }
        }
        let items = self
            .client
            .items(
                &self.user_id,
                &ItemQuery {
                    parent_id: Some(view.id.clone()),
                    types: TYPES.iter().map(|t| (*t).to_owned()).collect(),
                    recursive: true,
                    ..ItemQuery::default()
                },
            )
            .map_err(|e| vfs_error(&e))?;
        let tree = Arc::new(build(&view.dir, &items, self.client.kind()));
        let mut c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        c.trees
            .insert(view.id.clone(), (Instant::now(), Arc::clone(&tree)));
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

    /// Path of the first media file of `item_id` in any library.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn path_of(&self, item_id: &str) -> pa_vfs::Result<Option<VfsPath>> {
        for v in self.views()?.iter() {
            if let Some(p) = self.tree(v)?.by_item.get(item_id) {
                return VfsPath::parse(p).map(Some);
            }
        }
        Ok(None)
    }

    /// Every media file with the user's state as of the last listing
    /// (resume position, watched, favorite): for importing it into the
    /// library.
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

impl Vfs for JellyfinFs {
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
                let data = self.client.get_bytes(&p).map_err(|e| vfs_error(&e))?;
                Ok(Box::new(Memory(Arc::from(data))))
            }
            Some((_, Node::RemoteBif(p))) => {
                let data = self.client.get_bytes(&p).map_err(|e| vfs_error(&e))?;
                if bif_frames(&data) == 0 {
                    return Err(VfsError::new(
                        ErrorKind::NotFound,
                        format!("{path}: the server has no preview thumbnails"),
                    ));
                }
                Ok(Box::new(Memory(Arc::from(data))))
            }
            Some((_, Node::Media(m))) => Ok(Box::new(MediaSourceReader {
                client: Arc::clone(&self.client),
                path: static_stream_path(&m.item_id, &m.media_source_id, m.audio),
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

/// The original file of a media source, read with byte ranges and a small
/// read-ahead cache (the proxy asks for many short ranges in a row).
struct MediaSourceReader {
    client: Arc<Client>,
    path: String,
    size: u64,
    read_ahead: usize,
    blocks: Mutex<VecDeque<(u64, Arc<[u8]>)>>,
    random: AtomicBool,
}

impl fmt::Debug for MediaSourceReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaSourceReader")
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

impl MediaSourceReader {
    fn cached(&self, offset: u64, buf: &mut [u8]) -> Option<usize> {
        let blocks = self.blocks.lock().unwrap_or_else(PoisonError::into_inner);
        blocks.iter().find_map(|(start, data)| {
            let end = start + data.len() as u64;
            (offset >= *start && offset < end).then(|| copy_from(data, offset - start, buf))
        })
    }

    fn fetch(&self, offset: u64, len: usize) -> pa_vfs::Result<Vec<u8>> {
        let r = self
            .client
            .send(
                Method::Get,
                &self.path,
                None,
                Some((offset, len as u64)),
                MAX_BLOB,
            )
            .and_then(crate::http::check)
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

impl ByteSource for MediaSourceReader {
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

    fn items() -> Vec<Item> {
        let json = r#"[
          {"Id":"m1","Name":"Mission: Impossible","Type":"Movie","ProductionYear":1996,
           "ProviderIds":{"Tmdb":"954"},"ImageTags":{"Primary":"p"},"BackdropImageTags":["b"],
           "MediaSources":[{"Id":"m1","Path":"/media/MI.mkv","Size":1000,"RunTimeTicks":600000000},
                           {"Id":"m1b","Name":"4K","Path":"/media/MI.4k.mp4","Size":2000}],
           "Trickplay":{"m1":{"320":{"Width":320,"Height":180,"TileWidth":10,"TileHeight":10,
             "ThumbnailCount":150,"Interval":10000}}}},
          {"Id":"m2","Name":"Heat (1995)","Type":"Movie","ProductionYear":1995,
           "MediaSources":[{"Id":"m2","Container":"mov,mp4","Size":5}]},
          {"Id":"m3","Name":"Heat (1995)","Type":"Movie","ProductionYear":1995,
           "MediaSources":[{"Id":"m3","Path":"/x/Heat.avi","Size":6}]},
          {"Id":"s1","Name":"Dark","Type":"Series","ProductionYear":2017,"ProviderIds":{"Tvdb":"334824"}},
          {"Id":"e1","Name":"Secrets","Type":"Episode","SeriesId":"s1","SeriesName":"Dark",
           "ParentIndexNumber":1,"IndexNumber":1,
           "MediaSources":[{"Id":"e1","Path":"/tv/e1.mkv","Size":7}]},
          {"Id":"e0","Name":"Making of","Type":"Episode","SeriesId":"s1","SeriesName":"Dark",
           "ParentIndexNumber":0,"IndexNumber":1,"MediaSources":[{"Id":"e0","Size":8}]},
          {"Id":"a1","Name":"Song/One","Type":"Audio","Album":"LP","AlbumArtist":"Band",
           "AlbumId":"al","AlbumPrimaryImageTag":"c","IndexNumber":3,"ParentIndexNumber":1,
           "MediaSources":[{"Id":"a1","Path":"/m/s.flac","Size":9}]}
        ]"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn lays_out_a_library() {
        let t = build("Lib", &items(), ServerKind::Jellyfin);
        let has = |p: &str| t.nodes.contains_key(p);
        assert!(has(
            "/Lib/Mission - Impossible (1996)/Mission - Impossible (1996).mkv"
        ));
        assert!(has(
            "/Lib/Mission - Impossible (1996)/Mission - Impossible (1996) - 4K.mp4"
        ));
        assert!(has("/Lib/Mission - Impossible (1996)/movie.nfo"));
        assert!(has("/Lib/Mission - Impossible (1996)/poster.jpg"));
        assert!(has("/Lib/Mission - Impossible (1996)/fanart.jpg"));
        assert!(has(
            "/Lib/Mission - Impossible (1996)/Mission - Impossible (1996).jellyfin.json"
        ));
        assert!(has(
            "/Lib/Mission - Impossible (1996)/Mission - Impossible (1996).trickplay/320 - 10x10/1.jpg"
        ));
        assert!(!has(
            "/Lib/Mission - Impossible (1996)/Mission - Impossible (1996).trickplay/320 - 10x10/2.jpg"
        ));
        // The year is not doubled; a second movie of the same name gets
        // its own folder.
        assert!(has("/Lib/Heat (1995)/Heat (1995).mov"));
        assert!(has("/Lib/Heat (1995) (#2)/Heat (1995) (#2).avi"));
        assert!(has("/Lib/Dark (2017)/tvshow.nfo"));
        assert!(has("/Lib/Dark (2017)/Season 01/Dark S01E01 - Secrets.mkv"));
        assert!(has("/Lib/Dark (2017)/Specials/Dark S00E01 - Making of.mkv"));
        assert!(has("/Lib/Band/LP/03 - Song_One.flac"));
        assert!(has("/Lib/Band/LP/cover.jpg"));
        assert!(!has("/Lib/Band/LP/03 - Song_One.jellyfin.json"));
        assert_eq!(
            t.by_item["e1"],
            "/Lib/Dark (2017)/Season 01/Dark S01E01 - Secrets.mkv"
        );
        let Node::Text(nfo) = &t.nodes["/Lib/Mission - Impossible (1996)/movie.nfo"] else {
            panic!("nfo");
        };
        let text = std::str::from_utf8(nfo).unwrap();
        assert!(text.contains("<uniqueid type=\"tmdb\">954</uniqueid>"));
        assert!(text.contains("<title>Mission: Impossible</title>"));
        let Node::Media(m) =
            &t.nodes["/Lib/Mission - Impossible (1996)/Mission - Impossible (1996).mkv"]
        else {
            panic!("media");
        };
        assert_eq!((m.size, m.duration_ms), (1000, Some(60_000)));
    }

    #[test]
    fn lays_out_an_emby_library() {
        let mut items = items();
        // Emby: media source ids are not item ids, chapters carry markers.
        items[0].media_sources[0].id = "mediasource_m1".into();
        items[4].chapters = serde_json::from_str(
            r#"[{"StartPositionTicks":0,"MarkerType":"IntroStart"},
                {"StartPositionTicks":300000000,"MarkerType":"IntroEnd"}]"#,
        )
        .unwrap();
        let t = build("Lib", &items, ServerKind::Emby);
        let has = |p: &str| t.nodes.contains_key(p);
        let movie = "/Lib/Mission - Impossible (1996)/Mission - Impossible (1996)";
        assert!(has(&format!("{movie}.mkv")));
        assert!(has(&format!("{movie} - 4K.mp4")));
        assert!(has(&format!("{movie}.bif")));
        // No trickplay sheets, no segments file without markers.
        assert!(!has(&format!("{movie}.trickplay")));
        assert!(!has(&format!("{movie}.jellyfin.json")));
        assert!(!has(&format!("{movie} - 4K.bif")));
        let ep = "/Lib/Dark (2017)/Season 01/Dark S01E01 - Secrets";
        let Node::Text(json) = &t.nodes[&format!("{ep}.jellyfin.json")] else {
            panic!("segments");
        };
        let text = std::str::from_utf8(json).unwrap();
        assert!(text.contains("\"Type\":\"Intro\""), "{text}");
        assert!(text.contains("\"EndTicks\":300000000"), "{text}");
        let Node::RemoteBif(p) = &t.nodes[&format!("{ep}.bif")] else {
            panic!("bif");
        };
        assert_eq!(p, "/Videos/e1/index.bif?Width=320");
        let Node::Media(m) = &t.nodes[&format!("{movie}.mkv")] else {
            panic!("media");
        };
        assert_eq!(m.media_source_id, "mediasource_m1");
        assert!(!has("/Lib/Band/LP/03 - Song_One.bif"));
    }

    /// A one-thread HTTP server answering `route(path)` with JSON or
    /// bytes; returns its address.
    fn serve(route: fn(&str) -> Vec<u8>) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut reader = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = route(&path);
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(&body);
            }
        });
        addr
    }

    fn bif(frames: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'B', b'I', b'F', 0x0d, 0x0a, 0x1a, 0x0a];
        b.extend([0u8; 4]);
        b.extend(frames.to_le_bytes());
        b.extend(10_000u32.to_le_bytes());
        b.resize(64, 0);
        // Index: one entry per frame and the end marker, then the JPEGs.
        let data_at = 64 + (frames as usize + 1) * 8;
        for i in 0..=frames {
            let ts = if i == frames { u32::MAX } else { i };
            b.extend(ts.to_le_bytes());
            b.extend(u32::try_from(data_at + i as usize * 2).unwrap().to_le_bytes());
        }
        for _ in 0..frames {
            b.extend([0xFF, 0xD8]);
        }
        b
    }

    fn emby_route(path: &str) -> Vec<u8> {
        if path.starts_with("/Users/u/Views") {
            br#"{"Items":[{"Id":"v","Name":"Movies","CollectionType":"movies"}]}"#.to_vec()
        } else if path.starts_with("/Users/u/Items?") {
            br#"{"Items":[
              {"Id":"a","Name":"With","Type":"Movie","ProductionYear":2001,
               "MediaSources":[{"Id":"mediasource_a","Path":"/m/a.mkv","Size":10}]},
              {"Id":"b","Name":"Without","Type":"Movie","ProductionYear":2002,
               "MediaSources":[{"Id":"mediasource_b","Path":"/m/b.mkv","Size":10}]}
            ],"TotalRecordCount":2}"#
                .to_vec()
        } else if path.starts_with("/Videos/a/index.bif") {
            bif(2)
        } else if path.starts_with("/Videos/b/index.bif") {
            bif(0)
        } else {
            b"{}".to_vec()
        }
    }

    #[test]
    fn emby_bif_sidecars_open_only_with_thumbnails() {
        let c = Client::new(&[serve(emby_route)], crate::DeviceInfo::kinolune("d", "i"))
            .unwrap()
            .with_kind(ServerKind::Emby);
        let fs = JellyfinFs::new(Arc::new(c), "u");
        let with = VfsPath::parse("/Movies/With (2001)/With (2001).bif").unwrap();
        let data = fs.open(&with).unwrap();
        assert_eq!(data.size(), bif(2).len() as u64);
        let without = VfsPath::parse("/Movies/Without (2002)/Without (2002).bif").unwrap();
        assert_eq!(fs.open(&without).unwrap_err().kind(), ErrorKind::NotFound);
        // pa-trickplay takes the server's BIF as the preview of the file.
        let vfs: Arc<dyn Vfs> = Arc::new(fs);
        let media = VfsPath::parse("/Movies/With (2001)/With (2001).mkv").unwrap();
        let found = pa_trickplay::sidecar::find(&vfs, &media, None).unwrap();
        assert!(matches!(
            found,
            Some(pa_trickplay::sidecar::Sidecar::Bif(ref b)) if b.len() == 2
        ));
    }

    #[test]
    fn cleans_names() {
        assert_eq!(clean("a/b\\c*?"), "a_b_c__");
        assert_eq!(clean(" ..x.. "), "..x");
        assert_eq!(clean("\u{1}"), "Untitled");
        assert_eq!(range_start("bytes 100-199/1000"), Some(100));
    }

    #[test]
    fn memory_reads_within_bounds() {
        let m = Memory(Arc::from(b"abcdef".to_vec()));
        let mut buf = [0u8; 4];
        assert_eq!(m.read_at(4, &mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"ef");
        assert_eq!(m.read_at(9, &mut buf).unwrap(), 0);
    }
}
