// SPDX-License-Identifier: MPL-2.0
//! The endpoints: system, browsing by ID3 tags, search, media and
//! scrobbles.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::http::Client;
use crate::model::{Album, Artist, Extension, ServerInfo, Song};
use crate::{Error, Result};

/// Songs, albums or artists asked for per `search3` or `getAlbumList2`
/// page.
const PAGE: usize = 500;

/// Order of [`Client::album_list`] (`getAlbumList2` `type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlbumListType {
    /// By album name.
    #[default]
    AlphabeticalByName,
    /// By artist name.
    AlphabeticalByArtist,
    /// Recently added.
    Newest,
    /// Recently played.
    Recent,
    /// Most played.
    Frequent,
    /// Random.
    Random,
    /// Starred by the user.
    Starred,
}

impl AlbumListType {
    fn as_str(self) -> &'static str {
        match self {
            Self::AlphabeticalByName => "alphabeticalByName",
            Self::AlphabeticalByArtist => "alphabeticalByArtist",
            Self::Newest => "newest",
            Self::Recent => "recent",
            Self::Frequent => "frequent",
            Self::Random => "random",
            Self::Starred => "starred",
        }
    }
}

/// A `search3` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Text to look for; empty: everything (OpenSubsonic servers such as
    /// Navidrome return the whole library page by page).
    pub query: String,
    /// Artists per page.
    pub artist_count: usize,
    /// Albums per page.
    pub album_count: usize,
    /// Songs per page.
    pub song_count: usize,
    /// Offset of the artist page.
    pub artist_offset: usize,
    /// Offset of the album page.
    pub album_offset: usize,
    /// Offset of the song page.
    pub song_offset: usize,
}

impl SearchQuery {
    /// `query` with 20 results of each kind.
    #[must_use]
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            artist_count: 20,
            album_count: 20,
            song_count: 20,
            artist_offset: 0,
            album_offset: 0,
            song_offset: 0,
        }
    }
}

/// Field `name` of `v` as `T` (missing or `null`: the default).
fn field<T: DeserializeOwned + Default>(v: &Value, name: &str) -> Result<T> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(T::default()),
        Some(x) => {
            serde_json::from_value(x.clone()).map_err(|e| Error::Protocol(format!("{name}: {e}")))
        }
    }
}

/// A list field `name` of `v` that may be missing, `null` or one object.
fn items<T: DeserializeOwned>(v: &Value, name: &str) -> Vec<T> {
    match v.get(name) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| serde_json::from_value(x.clone()).ok())
            .collect(),
        Some(x @ Value::Object(_)) => serde_json::from_value(x.clone()).ok().into_iter().collect(),
        _ => Vec::new(),
    }
}

/// The object `name` of an answer, or a protocol error.
fn object<'a>(v: &'a Value, name: &str) -> Result<&'a Value> {
    v.get(name)
        .filter(|x| x.is_object())
        .ok_or_else(|| Error::Protocol(format!("no {name} in the answer")))
}

impl Client {
    /// Checks the address and the credentials; what the server is.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] for wrong credentials, network errors.
    pub fn ping(&self) -> Result<ServerInfo> {
        let r = self.call("ping", &[])?;
        serde_json::from_value(r).map_err(|e| Error::Protocol(e.to_string()))
    }

    /// The OpenSubsonic extensions of the server; empty for a plain
    /// Subsonic server (which does not know the call).
    ///
    /// # Errors
    ///
    /// Network and authentication errors.
    pub fn extensions(&self) -> Result<Vec<Extension>> {
        match self.call("getOpenSubsonicExtensions", &[]) {
            Ok(r) => Ok(items(&r, "openSubsonicExtensions")),
            Err(Error::NotFound | Error::Status { .. } | Error::Server { .. }) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Whether the server takes OpenSubsonic API keys
    /// (`apiKeyAuthentication`).
    ///
    /// # Errors
    ///
    /// As [`extensions`](Self::extensions).
    pub fn supports_api_keys(&self) -> Result<bool> {
        Ok(self
            .extensions()?
            .iter()
            .any(|e| e.name == "apiKeyAuthentication"))
    }

    /// Every artist (`getArtists`, by tags), in the server's order.
    ///
    /// # Errors
    ///
    /// Network, authentication and protocol errors.
    pub fn artists(&self) -> Result<Vec<Artist>> {
        let r = self.call("getArtists", &[])?;
        let index = object(&r, "artists")?;
        let groups: Vec<Value> = items(index, "index");
        Ok(groups.iter().flat_map(|g| items(g, "artist")).collect())
    }

    /// An artist with its albums (`getArtist`).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`]; network errors.
    pub fn artist(&self, id: &str) -> Result<Artist> {
        let r = self.call("getArtist", &[("id", id.to_owned())])?;
        field(&r, "artist")
    }

    /// An album with its songs (`getAlbum`).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`]; network errors.
    pub fn album(&self, id: &str) -> Result<Album> {
        let r = self.call("getAlbum", &[("id", id.to_owned())])?;
        field(&r, "album")
    }

    /// One song (`getSong`).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`]; network errors.
    pub fn song(&self, id: &str) -> Result<Song> {
        let r = self.call("getSong", &[("id", id.to_owned())])?;
        field(&r, "song")
    }

    /// A page of albums (`getAlbumList2`), at most 500.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn album_list(
        &self,
        kind: AlbumListType,
        size: usize,
        offset: usize,
    ) -> Result<Vec<Album>> {
        let r = self.call(
            "getAlbumList2",
            &[
                ("type", kind.as_str().to_owned()),
                ("size", size.min(PAGE).to_string()),
                ("offset", offset.to_string()),
            ],
        )?;
        Ok(items(object(&r, "albumList2")?, "album"))
    }

    /// Artists, albums and songs matching `q` (`search3`).
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn search(&self, q: &SearchQuery) -> Result<(Vec<Artist>, Vec<Album>, Vec<Song>)> {
        let r = self.call(
            "search3",
            &[
                ("query", q.query.clone()),
                ("artistCount", q.artist_count.to_string()),
                ("albumCount", q.album_count.to_string()),
                ("songCount", q.song_count.to_string()),
                ("artistOffset", q.artist_offset.to_string()),
                ("albumOffset", q.album_offset.to_string()),
                ("songOffset", q.song_offset.to_string()),
            ],
        )?;
        let res = match r.get("searchResult3") {
            Some(v) if v.is_object() => v,
            _ => return Ok(Default::default()),
        };
        Ok((
            items(res, "artist"),
            items(res, "album"),
            items(res, "song"),
        ))
    }

    /// Every song of the library. Fast path: `search3` with an empty
    /// query, 500 songs a page (Navidrome and other OpenSubsonic servers);
    /// a server that answers that with nothing is walked album by album
    /// (`getAlbumList2` pages, then `getAlbum` each). Folders, videos and
    /// podcast episodes are left out.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn all_songs(&self) -> Result<Vec<Song>> {
        let mut songs = self.songs_by_search()?;
        if songs.is_empty() {
            songs = self.songs_by_album()?;
        }
        songs.retain(Song::is_music);
        let mut seen = std::collections::HashSet::new();
        songs.retain(|s| !s.id.is_empty() && seen.insert(s.id.clone()));
        Ok(songs)
    }

    fn songs_by_search(&self) -> Result<Vec<Song>> {
        let mut out = Vec::new();
        loop {
            let q = SearchQuery {
                song_count: PAGE,
                song_offset: out.len(),
                artist_count: 0,
                album_count: 0,
                ..SearchQuery::new("")
            };
            let (_, _, page) = match self.search(&q) {
                Ok(r) => r,
                // A server that wants a query: walk albums instead.
                Err(Error::Invalid(_) | Error::Server { .. }) if out.is_empty() => {
                    return Ok(Vec::new());
                }
                Err(e) => return Err(e),
            };
            let n = page.len();
            out.extend(page);
            if n < PAGE {
                return Ok(out);
            }
        }
    }

    fn songs_by_album(&self) -> Result<Vec<Song>> {
        let mut albums = Vec::new();
        loop {
            let page = self.album_list(AlbumListType::AlphabeticalByArtist, PAGE, albums.len())?;
            let n = page.len();
            albums.extend(page);
            if n < PAGE {
                break;
            }
        }
        let mut out = Vec::new();
        for a in albums {
            match self.album(&a.id) {
                Ok(full) => out.extend(full.songs),
                // Removed since the list was read.
                Err(Error::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Parameters of the original file of song `id` (`stream` with
    /// `format=raw`: no transcoding, byte ranges work).
    pub(crate) fn stream_params(id: &str) -> Vec<(&'static str, String)> {
        vec![("id", id.to_owned()), ("format", "raw".to_owned())]
    }

    /// URL of the original file of song `id`, with the credentials.
    /// Contains a secret: do not log it.
    #[must_use]
    pub fn stream_url(&self, id: &str) -> String {
        self.url("stream", &Self::stream_params(id))
    }

    /// The cover art `id` (of a song, album or artist), at most `size`
    /// pixels wide if given.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if there is none; network errors.
    pub fn cover_art(&self, id: &str, size: Option<u32>) -> Result<Vec<u8>> {
        let mut params = vec![("id", id.to_owned())];
        if let Some(s) = size {
            params.push(("size", s.to_string()));
        }
        let r = self.binary("getCoverArt", &params, None)?;
        if r.body.is_empty() {
            return Err(Error::NotFound);
        }
        Ok(r.body)
    }

    /// Scrobbles song `id`: `submission = false` says it is playing now,
    /// `true` that it was played (at `time_ms`, Unix ms; default: now).
    /// The server counts the play and passes it on to Last.fm or
    /// ListenBrainz if the user set that up there.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`]; network errors.
    pub fn scrobble(&self, id: &str, submission: bool, time_ms: Option<i64>) -> Result<()> {
        let mut params = vec![
            ("id", id.to_owned()),
            ("submission", submission.to_string()),
        ];
        if let Some(t) = time_ms {
            params.push(("time", t.to_string()));
        }
        self.call("scrobble", &params).map(|_| ())
    }

    /// Songs playing now for the users of the server (`getNowPlaying`).
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn now_playing(&self) -> Result<Vec<Song>> {
        let r = self.call("getNowPlaying", &[])?;
        Ok(r.get("nowPlaying")
            .map(|n| items(n, "entry"))
            .unwrap_or_default())
    }
}
