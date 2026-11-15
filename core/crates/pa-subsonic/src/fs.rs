// SPDX-License-Identifier: MPL-2.0
//! The music of a Subsonic user as a read-only [`Vfs`] tree, laid out the
//! way the scanner already reads local music folders:
//!
//! ```text
//! /Band/LP/01 - Song.flac        the original file (byte ranges)
//! /Band/LP/2-01 - Other.flac     disc 2
//! /Band/LP/cover.jpg             the album's cover art
//! ```
//!
//! The folder of a song is its album artist (OpenSubsonic
//! `displayAlbumArtist`, else the artist of its album, else the track
//! artist) and its album; two albums of one name get `LP (#2)`. So a
//! Subsonic server is scanned (the tags of the original files), shown in
//! the music section, searched and played through the stream proxy like
//! any other source.
//!
//! The tree comes from [`Client::all_songs`] and is kept for a while
//! ([`SubsonicFs::with_ttl`]); [`SubsonicFs::refresh`] drops it. Songs
//! report their real size; covers are fetched when opened and are listed
//! with size 0 (their size is only known then); [`ByteSource::size`] of an
//! open one is exact.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Vfs, VfsError, VfsPath};

use crate::Error;
use crate::http::Client;
use crate::model::{Song, parse_date_ms};

/// Default read-ahead of a song: one range request per 1 MiB.
pub const DEFAULT_READ_AHEAD: usize = 1 << 20;
/// Blocks of read-ahead kept per open file.
const CACHE_BLOCKS: usize = 4;
/// Pixels of the cover art asked for.
const COVER_SIZE: u32 = 1000;

/// The song behind a path of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongRef {
    /// Song id.
    pub song_id: String,
    /// Album id, if the server has one.
    pub album_id: Option<String>,
    /// Size of the original file; `0` if the server did not say (the
    /// first read then asks).
    pub size: u64,
    /// When the server added it, Unix ms.
    pub modified_ms: Option<i64>,
    /// Length, ms.
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone)]
enum Node {
    Dir(BTreeSet<String>),
    Song(SongRef),
    /// Cover art id, fetched when opened.
    Cover(String),
}

impl Node {
    fn metadata(&self) -> Metadata {
        match self {
            Self::Dir(_) => Metadata {
                kind: EntryKind::Dir,
                size: 0,
                modified_ms: None,
            },
            Self::Song(s) => Metadata {
                kind: EntryKind::File,
                size: s.size,
                modified_ms: s.modified_ms,
            },
            Self::Cover(_) => Metadata {
                kind: EntryKind::File,
                size: 0,
                modified_ms: None,
            },
        }
    }
}

/// The whole library, laid out.
#[derive(Debug, Default)]
struct Tree {
    nodes: HashMap<String, Node>,
    /// Song id → its path.
    by_song: HashMap<String, String>,
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

fn some(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.trim().is_empty())
}

/// File extension of the original file of `song`.
fn extension(song: &Song) -> String {
    let ok = |e: &str| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric());
    if let Some(s) = some(song.suffix.as_deref()).map(str::to_ascii_lowercase)
        && ok(&s)
    {
        return s;
    }
    if let Some((_, e)) = song.path.as_deref().and_then(|p| p.rsplit_once('.'))
        && ok(e)
    {
        return e.to_ascii_lowercase();
    }
    match song.content_type.as_deref() {
        Some("audio/flac" | "audio/x-flac") => "flac",
        Some("audio/ogg" | "audio/vorbis") => "ogg",
        Some("audio/opus") => "opus",
        Some("audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac") => "m4a",
        Some("audio/wav" | "audio/x-wav") => "wav",
        _ => "mp3",
    }
    .to_owned()
}

/// `/` + `name` for the root, else `dir/name`.
fn child(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

struct Builder {
    tree: Tree,
    /// Album id (or artist and name) → its folder.
    albums: HashMap<String, String>,
}

impl Builder {
    fn new() -> Self {
        let mut tree = Tree::default();
        tree.nodes.insert("/".into(), Node::Dir(BTreeSet::new()));
        Self {
            tree,
            albums: HashMap::new(),
        }
    }

    fn taken(&self, dir: &str, name: &str) -> bool {
        self.tree.nodes.contains_key(&child(dir, name))
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

    /// Adds `name` to `dir` (an existing folder).
    fn put(&mut self, dir: &str, name: &str, node: Node) -> String {
        let path = child(dir, name);
        if let Some(Node::Dir(children)) = self.tree.nodes.get_mut(dir) {
            children.insert(name.to_owned());
        }
        if let Node::Song(s) = &node {
            self.tree
                .by_song
                .entry(s.song_id.clone())
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

    fn add(&mut self, song: &Song, album_artists: &HashMap<String, String>) {
        let album_artist = some(song.display_album_artist.as_deref())
            .or_else(|| {
                song.album_id
                    .as_deref()
                    .and_then(|id| album_artists.get(id))
                    .map(String::as_str)
            })
            .or(some(song.artist.as_deref()))
            .map_or_else(|| "Unknown Artist".to_owned(), clean);
        let album = some(song.album.as_deref()).map_or_else(|| "Unknown Album".to_owned(), clean);
        let key = song
            .album_id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| format!("{album_artist}\n{album}"));
        let album_dir = if let Some(dir) = self.albums.get(&key) {
            dir.clone()
        } else {
            let artist_dir = self.folder("/", &album_artist);
            let name = self.unique(&artist_dir, &album);
            let dir = self.folder(&artist_dir, &name);
            if let Some(cover) = some(song.cover_art.as_deref()) {
                self.put(&dir, "cover.jpg", Node::Cover(cover.to_owned()));
            }
            self.albums.insert(key, dir.clone());
            dir
        };
        let title = clean(if song.title.trim().is_empty() {
            &song.id
        } else {
            &song.title
        });
        let stem = match (song.disc_number, song.track) {
            (Some(d), Some(n)) if d > 1 => format!("{d}-{n:02} - {title}"),
            (_, Some(n)) => format!("{n:02} - {title}"),
            _ => title,
        };
        let name = self.unique(&album_dir, &format!("{stem}.{}", extension(song)));
        self.put(
            &album_dir,
            &name,
            Node::Song(SongRef {
                song_id: song.id.clone(),
                album_id: song.album_id.clone(),
                size: song.size.and_then(|s| u64::try_from(s).ok()).unwrap_or(0),
                modified_ms: song.created.as_deref().and_then(parse_date_ms),
                duration_ms: song.duration_ms(),
            }),
        );
    }
}

/// Lays out `songs`; `album_artists`: album id → album artist, for
/// servers without `displayAlbumArtist`.
fn build(songs: &[Song], album_artists: &HashMap<String, String>) -> Tree {
    let mut b = Builder::new();
    for s in songs {
        b.add(s, album_artists);
    }
    b.tree
}

/// The music of one Subsonic user as a [`Vfs`] (see the module docs).
pub struct SubsonicFs {
    client: Arc<Client>,
    ttl: Duration,
    read_ahead: usize,
    cache: Mutex<Option<(Instant, Arc<Tree>)>>,
}

impl fmt::Debug for SubsonicFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubsonicFs")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl SubsonicFs {
    /// The music `client` sees.
    #[must_use]
    pub fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            ttl: Duration::from_secs(300),
            read_ahead: DEFAULT_READ_AHEAD,
            cache: Mutex::new(None),
        }
    }

    /// Keeps the tree for `ttl` (default 5 minutes).
    #[must_use]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Reads songs `bytes` at a time ahead (default
    /// [`DEFAULT_READ_AHEAD`]).
    #[must_use]
    pub fn with_read_ahead(mut self, bytes: usize) -> Self {
        self.read_ahead = bytes.max(64 * 1024);
        self
    }

    /// The client.
    #[must_use]
    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }

    /// Drops the cached tree: the next call reads the server again.
    pub fn refresh(&self) {
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn tree(&self) -> pa_vfs::Result<Arc<Tree>> {
        if let Some((at, t)) = &*self.cache.lock().unwrap_or_else(PoisonError::into_inner)
            && at.elapsed() < self.ttl
        {
            return Ok(Arc::clone(t));
        }
        let songs = self.client.all_songs().map_err(|e| vfs_error(&e))?;
        // Album artists for servers without `displayAlbumArtist`: one
        // `getAlbumList2` walk is cheap next to the songs.
        let mut album_artists = HashMap::new();
        if songs
            .iter()
            .any(|s| some(s.display_album_artist.as_deref()).is_none())
        {
            let mut offset = 0;
            loop {
                let page = self
                    .client
                    .album_list(crate::AlbumListType::AlphabeticalByName, 500, offset)
                    .map_err(|e| vfs_error(&e))?;
                offset += page.len();
                for a in &page {
                    if let Some(artist) = some(a.artist.as_deref()) {
                        album_artists.insert(a.id.clone(), artist.to_owned());
                    }
                }
                if page.len() < 500 {
                    break;
                }
            }
        }
        let tree = Arc::new(build(&songs, &album_artists));
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), Arc::clone(&tree)));
        Ok(tree)
    }

    fn node(&self, path: &VfsPath) -> pa_vfs::Result<Node> {
        self.tree()?
            .nodes
            .get(path.as_str())
            .cloned()
            .ok_or_else(|| not_found(path.as_str()))
    }

    /// The song at `path`, `None` for folders and covers.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for a path not on the server; network
    /// errors.
    pub fn song_at(&self, path: &VfsPath) -> pa_vfs::Result<Option<SongRef>> {
        Ok(match self.node(path)? {
            Node::Song(s) => Some(s),
            _ => None,
        })
    }

    /// Path of song `song_id`.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn path_of(&self, song_id: &str) -> pa_vfs::Result<Option<VfsPath>> {
        match self.tree()?.by_song.get(song_id) {
            Some(p) => VfsPath::parse(p).map(Some),
            None => Ok(None),
        }
    }

    /// Every song with its path, sorted by path.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn songs(&self) -> pa_vfs::Result<Vec<(VfsPath, SongRef)>> {
        let tree = self.tree()?;
        let mut out = Vec::new();
        for (path, node) in &tree.nodes {
            if let Node::Song(s) = node {
                out.push((VfsPath::parse(path)?, s.clone()));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

impl Vfs for SubsonicFs {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        let tree = self.tree()?;
        let Some(node) = tree.nodes.get(dir.as_str()) else {
            return Err(not_found(dir.as_str()));
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
        Ok(self.node(path)?.metadata())
    }

    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        match self.node(path)? {
            Node::Dir(_) => Err(VfsError::new(
                ErrorKind::IsADirectory,
                format!("{path}: a folder"),
            )),
            Node::Cover(id) => {
                let data = self
                    .client
                    .cover_art(&id, Some(COVER_SIZE))
                    .map_err(|e| vfs_error(&e))?;
                Ok(Box::new(Memory(Arc::from(data))))
            }
            Node::Song(s) => {
                let reader = SongReader {
                    client: Arc::clone(&self.client),
                    song_id: s.song_id,
                    size: s.size,
                    read_ahead: self.read_ahead,
                    blocks: Mutex::new(VecDeque::new()),
                    random: AtomicBool::new(false),
                };
                let reader = if reader.size == 0 {
                    reader.with_probed_size()?
                } else {
                    reader
                };
                Ok(Box::new(reader))
            }
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

/// `bytes 100-199/1000` → (100, Some(1000)).
fn content_range(header: &str) -> Option<(u64, Option<u64>)> {
    let rest = header.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let start = range.split('-').next()?.trim().parse().ok()?;
    Some((start, total.trim().parse().ok()))
}

/// The original file of a song, read with byte ranges and a small
/// read-ahead cache (the proxy asks for many short ranges in a row).
struct SongReader {
    client: Arc<Client>,
    song_id: String,
    size: u64,
    read_ahead: usize,
    blocks: Mutex<VecDeque<(u64, Arc<[u8]>)>>,
    random: AtomicBool,
}

impl fmt::Debug for SongReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SongReader")
            .field("song_id", &self.song_id)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl SongReader {
    /// Learns the size from the first range of the file (the server did
    /// not list it).
    fn with_probed_size(mut self) -> pa_vfs::Result<Self> {
        let r = self
            .client
            .binary(
                "stream",
                &Client::stream_params(&self.song_id),
                Some((0, 1)),
            )
            .map_err(|e| vfs_error(&e))?;
        self.size = match r.content_range.as_deref().and_then(content_range) {
            Some((_, Some(total))) => total,
            _ if r.status == 200 => r.body.len() as u64,
            _ => {
                return Err(VfsError::new(
                    ErrorKind::Other,
                    "the server did not say how big the file is",
                ));
            }
        };
        Ok(self)
    }

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
            .binary(
                "stream",
                &Client::stream_params(&self.song_id),
                Some((offset, len as u64)),
            )
            .map_err(|e| vfs_error(&e))?;
        match r.status {
            206 => {
                let start = r.content_range.as_deref().and_then(content_range);
                if start.map(|s| s.0) != Some(offset) {
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

impl ByteSource for SongReader {
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use super::*;
    use crate::Credentials;

    fn songs() -> Vec<Song> {
        serde_json::from_str(
            r#"[
          {"id":"s1","title":"One","album":"LP","albumId":"al1","artist":"Guest",
           "displayAlbumArtist":"Band","track":1,"size":10,"suffix":"flac","coverArt":"al1",
           "created":"2024-01-02T03:04:05Z","duration":60},
          {"id":"s2","title":"Two/2","album":"LP","albumId":"al1","artist":"Band",
           "track":1,"discNumber":2,"size":20,"suffix":"flac"},
          {"id":"s3","title":"Same","album":"LP","albumId":"al2","artist":"Band","track":1,
           "size":30,"contentType":"audio/mpeg"},
          {"id":"s4","title":"Loose","size":5,"path":"x/Loose.OGG"}
        ]"#,
        )
        .unwrap()
    }

    #[test]
    fn lays_out_the_library() {
        let artists = HashMap::from([("al2".to_owned(), "Band".to_owned())]);
        let t = build(&songs(), &artists);
        let song = |p: &str| match t.nodes.get(p) {
            Some(Node::Song(s)) => s.clone(),
            other => panic!("{p}: {other:?}"),
        };
        assert_eq!(song("/Band/LP/01 - One.flac").song_id, "s1");
        assert_eq!(
            song("/Band/LP/01 - One.flac").modified_ms,
            parse_date_ms("2024-01-02T03:04:05Z")
        );
        assert_eq!(song("/Band/LP/2-01 - Two_2.flac").size, 20);
        // Another album of the same name gets its own folder.
        assert_eq!(song("/Band/LP (#2)/01 - Same.mp3").song_id, "s3");
        assert_eq!(
            song("/Unknown Artist/Unknown Album/Loose.ogg").song_id,
            "s4"
        );
        assert!(matches!(t.nodes.get("/Band/LP/cover.jpg"), Some(Node::Cover(c)) if c == "al1"));
        assert!(!t.nodes.contains_key("/Band/LP (#2)/cover.jpg"));
        assert_eq!(t.by_song["s2"], "/Band/LP/2-01 - Two_2.flac");
        let Some(Node::Dir(root)) = t.nodes.get("/") else {
            panic!()
        };
        assert_eq!(
            root.iter().map(String::as_str).collect::<Vec<_>>(),
            ["Band", "Unknown Artist"]
        );
    }

    #[test]
    fn reads_content_ranges() {
        assert_eq!(content_range("bytes 100-199/1000"), Some((100, Some(1000))));
        assert_eq!(content_range("bytes 0-0/*"), Some((0, None)));
        assert_eq!(content_range("items 0-1/2"), None);
    }

    /// A one-file Subsonic server on 127.0.0.1: `search3` lists one song
    /// of `data`, `stream` serves its byte ranges, `getCoverArt` a fake
    /// image. Answers `requests` requests, then stops.
    fn serve(data: &'static [u8], requests: usize) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in l.incoming().take(requests) {
                let mut s = stream.unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut range = None;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        let (a, b) = v.trim().split_once('-').unwrap();
                        range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let json = |body: String| {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .into_bytes()
                };
                let out = if path.starts_with("/rest/search3.view") {
                    json(format!(
                        r#"{{"subsonic-response":{{"status":"ok","searchResult3":{{"song":[{{"id":"s1","title":"T","album":"A","albumId":"a1","displayAlbumArtist":"B","track":1,"size":{},"suffix":"mp3","coverArt":"a1"}}]}}}}}}"#,
                        data.len()
                    ))
                } else if path.starts_with("/rest/stream.view") {
                    assert!(path.contains("format=raw"));
                    let (a, b) = range.unwrap();
                    let b = b.min(data.len() - 1);
                    let mut o = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mpeg\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len(),
                        b + 1 - a
                    )
                    .into_bytes();
                    o.extend_from_slice(&data[a..=b]);
                    o
                } else if path.starts_with("/rest/getCoverArt.view") {
                    assert!(path.contains("id=a1"));
                    b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 3\r\nConnection: close\r\n\r\njpg".to_vec()
                } else {
                    json(r#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"no"}}}"#.into())
                };
                s.write_all(&out).unwrap();
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn serves_files_with_ranges_and_covers() {
        static DATA: [u8; 300_000] = {
            let mut d = [0u8; 300_000];
            let mut i = 0;
            while i < d.len() {
                d[i] = (i % 251) as u8;
                i += 1;
            }
            d
        };
        let url = serve(&DATA, 8);
        let client = Arc::new(Client::new(&url, Credentials::password("u", "p")).unwrap());
        let fs = SubsonicFs::new(client).with_read_ahead(100_000);
        let path = VfsPath::parse("/B/A/01 - T.mp3").unwrap();
        assert_eq!(fs.stat(&path).unwrap().size, 300_000);
        assert_eq!(fs.path_of("s1").unwrap(), Some(path.clone()));
        let f = fs.open(&path).unwrap();
        let mut buf = vec![0u8; 1000];
        assert_eq!(f.read_at(150_000, &mut buf).unwrap(), 1000);
        assert_eq!(buf, DATA[150_000..151_000]);
        // Served from the read-ahead block.
        assert_eq!(f.read_at(151_000, &mut buf).unwrap(), 1000);
        assert_eq!(buf, DATA[151_000..152_000]);
        let mut tail = vec![0u8; 10];
        assert_eq!(f.read_at(299_995, &mut tail).unwrap(), 5);
        assert_eq!(f.read_at(300_000, &mut tail).unwrap(), 0);
        let mut cover = Vec::new();
        let c = fs.open(&VfsPath::parse("/B/A/cover.jpg").unwrap()).unwrap();
        let mut b = vec![0u8; 16];
        let n = c.read_at(0, &mut b).unwrap();
        cover.extend_from_slice(&b[..n]);
        assert_eq!(cover, b"jpg");
        assert_eq!(
            fs.stat(&VfsPath::parse("/B/missing").unwrap())
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn cleans_names() {
        assert_eq!(clean("AC/DC: Live?"), "AC_DC - Live_");
        assert_eq!(clean(" ... "), "Untitled");
    }
}
