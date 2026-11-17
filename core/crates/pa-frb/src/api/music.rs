// SPDX-License-Identifier: MPL-2.0
//! Types of the music views (MUS-004) for Dart. The calls are methods of
//! [`CoreLibrary`](crate::api::library::CoreLibrary): `music_tracks`,
//! `record_track_play` and the audio playlists; music metadata (META-021):
//! `fetch_music_metadata`, `music_artist_infos`.

use pa_catalog as c;

use crate::api::library::CoreItem;

/// A music track with its album, credits, stream format and plays.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreMusicTrack {
    pub item: CoreItem,
    /// `music_albums.id`.
    pub album_id: Option<i64>,
    /// Album artists, main first (compilations: "Various Artists").
    pub album_artists: Vec<String>,
    pub album_year: Option<i32>,
    pub compilation: bool,
    /// Track artists, main first.
    pub artists: Vec<String>,
    pub composers: Vec<String>,
    pub genres: Vec<String>,
    pub disc_number: Option<i32>,
    pub disc_total: Option<i32>,
    /// `flac`, `alac`, `mp3`, `aac`, `opus`, `pcm`, `dsd`…
    pub codec: Option<String>,
    pub lossless: Option<bool>,
    pub sample_rate_hz: Option<i32>,
    pub bit_depth: Option<i32>,
    pub channels: Option<i32>,
    pub bitrate_kbps: Option<i32>,
    /// The file is on this device (a local folder): playable offline.
    pub local: bool,
    pub play_count: i64,
    pub last_played_at_ms: Option<i64>,
    /// Cover Art Archive front cover of the album (META-021); shown when
    /// there is no local cover.
    pub cover_url: Option<String>,
    /// Normalized MusicBrainz genres of the album.
    pub album_genres: Vec<String>,
    /// Year of the first release of the album's release group.
    pub original_year: Option<i32>,
    /// `Album`, `EP`, `Single`…
    pub album_type: Option<String>,
    pub label: Option<String>,
    /// A track of an audiobook (LIB-025): the «Аудиокниги» section.
    pub audiobook: bool,
}

/// MusicBrainz data of an album artist (META-021).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreMusicArtistInfo {
    pub artist_id: i64,
    /// The name in the library (as tagged).
    pub name: String,
    pub mbid: String,
    pub sort_name: Option<String>,
    /// `Person`, `Group`, `Orchestra`, `Choir`, `Character`, `Other`.
    pub kind: Option<String>,
    /// ISO 3166-1 country.
    pub country: Option<String>,
    pub disambiguation: Option<String>,
    /// `1985`, `1985-03-01`.
    pub begin_date: Option<String>,
    pub end_date: Option<String>,
    pub genres: Vec<String>,
}

impl From<c::MusicArtistInfo> for CoreMusicArtistInfo {
    fn from(a: c::MusicArtistInfo) -> Self {
        Self {
            artist_id: a.artist_id,
            name: a.name,
            mbid: a.mbid,
            sort_name: a.sort_name,
            kind: a.kind,
            country: a.country,
            disambiguation: a.disambiguation,
            begin_date: a.begin_date,
            end_date: a.end_date,
            genres: a.genres,
        }
    }
}

/// Progress of `CoreLibrary::fetch_music_metadata`; the last update has
/// `done` set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoreMusicMetadataUpdate {
    pub processed: u32,
    pub total: u32,
    pub done: bool,
    pub matched: u32,
    pub not_found: u32,
    pub failed: u32,
    pub covers: u32,
    pub artists_matched: u32,
    /// Audiobooks asked for at Open Library (MUS-019), and how many matched.
    pub books: u32,
    pub books_matched: u32,
    /// Why the run stopped early (offline), or a failure.
    pub error: Option<String>,
}

impl From<c::MusicMetadataReport> for CoreMusicMetadataUpdate {
    fn from(r: c::MusicMetadataReport) -> Self {
        use crate::api::library::count;
        Self {
            processed: count(if r.stopped {
                r.matched + r.not_found + r.failed
            } else {
                r.albums + r.artists
            }),
            total: count(r.albums + r.artists),
            done: true,
            matched: count(r.matched),
            not_found: count(r.not_found),
            failed: count(r.failed),
            covers: count(r.covers),
            artists_matched: count(r.artists_matched),
            books: 0,
            books_matched: 0,
            error: r.error,
        }
    }
}

fn small(v: Option<i64>) -> Option<i32> {
    v.and_then(|v| i32::try_from(v).ok())
}

impl From<c::MusicTrack> for CoreMusicTrack {
    fn from(t: c::MusicTrack) -> Self {
        Self {
            item: t.item.into(),
            album_id: t.album_id,
            album_artists: t.album_artists,
            album_year: small(t.album_year),
            compilation: t.compilation,
            artists: t.artists,
            composers: t.composers,
            genres: t.genres,
            disc_number: small(t.disc_number),
            disc_total: small(t.disc_total),
            codec: t.codec,
            lossless: t.lossless,
            sample_rate_hz: small(t.sample_rate_hz),
            bit_depth: small(t.bit_depth),
            channels: small(t.channels),
            bitrate_kbps: small(t.bitrate_kbps),
            local: t.local,
            play_count: t.play_count,
            last_played_at_ms: t.last_played_at_ms,
            cover_url: t.cover_url,
            album_genres: t.album_genres,
            original_year: small(t.original_year),
            album_type: t.album_type,
            label: t.label,
            audiobook: t.audiobook,
        }
    }
}
