// SPDX-License-Identifier: MPL-2.0
//! MusicBrainz metadata of albums and artists, Cover Art Archive covers and
//! normalized genres (META-021, research 03 §3.4).
//!
//! [`Catalog::fetch_music_metadata`] runs in the background after a scan:
//!
//! 1. **Albums** without fresh metadata ([`MusicMetadataOptions`]):
//!    - the release MBID of the tags → `/release/{mbid}`;
//!    - else the release-group MBID → `/release-group/{mbid}`;
//!    - else a release search by title and album artist ("Various Artists"
//!      for compilations). Results are ranked by [`search_score`] — title
//!      (edition suffixes such as "(Deluxe Edition)" ignored), artist,
//!      track count and year — and the best few ([`MAX_LOOKUPS`]) are looked
//!      up with their tracks: [`duration_score`] compares the durations of
//!      the files with the track lengths by disc and number. The best
//!      [`combined_score`] of at least [`ACCEPT`] is the match.
//!
//!    Genres come from the release group (else the release), normalized
//!    ([`pa_library::genre`]). An album artist without an MBID gets the
//!    MBID of the credit with the same name.
//! 2. **Covers**: the front cover of the matched release group in the Cover
//!    Art Archive. Only its URL is stored; the image cache (`pa-imagecache`,
//!    LIB-013) downloads it on the device when the UI shows it.
//! 3. **Artists** (album artists except "Various Artists"): the MBID of the
//!    tags or of a matched album, else the only exact name match of an
//!    artist search; `/artist/{mbid}` gives type, country, dates,
//!    disambiguation and genres. Artist photos are not fetched: no source
//!    has a licence compatible with showing them without a key and
//!    per-image attribution (docs/legal/providers.md).
//!
//! The network runs without holding the database; results are written one
//! album or artist at a time. MusicBrainz allows one request per second
//! ([`pa_metadata::musicbrainz`]), so a large library takes a while — the
//! run is cancellable and later runs continue where it stopped. Network
//! failures mark the album `failed` (retried on a later run); several in a
//! row stop the run.

use std::collections::HashSet;
use std::sync::Arc;

use pa_library::genre;
use pa_library::music::{VARIOUS_ARTISTS, fold};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_metadata::cache::ProviderCache;
use pa_metadata::coverart::CoverArtClient;
use pa_metadata::fetch::FetchError;
use pa_metadata::musicbrainz::{
    self as mb, ArtistCredit, MusicBrainzClient, Release, ReleaseGroup,
};
use pa_scanner::CancelToken;

use crate::{Catalog, Error, now_ms};

/// Lowest [`combined_score`] of a searched release accepted as the album.
pub const ACCEPT: f64 = 0.8;

/// Search results looked up with their tracks per album.
pub const MAX_LOOKUPS: usize = 3;

/// A file duration matches a track length within this.
pub const DURATION_TOLERANCE_MS: u64 = 3000;

/// Genres kept per album or artist.
const MAX_GENRES: usize = 5;

/// Network failures in a row that end a run (offline).
const MAX_FAILURES_IN_A_ROW: usize = 5;

/// Lowest MusicBrainz search score of an artist accepted by name.
const ARTIST_SEARCH_SCORE: u32 = 90;

const DAY_MS: i64 = 24 * 3600 * 1000;

/// The clients a music metadata run uses.
#[derive(Clone, Default)]
pub struct MusicProviders {
    pub musicbrainz: Option<Arc<MusicBrainzClient>>,
    pub cover_art: Option<Arc<CoverArtClient>>,
}

impl std::fmt::Debug for MusicProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MusicProviders")
            .field("musicbrainz", &self.musicbrainz.is_some())
            .field("cover_art", &self.cover_art.is_some())
            .finish()
    }
}

impl MusicProviders {
    /// Live clients (HTTPS) sharing one response cache.
    pub fn live(cache: Arc<dyn ProviderCache>) -> Self {
        Self {
            musicbrainz: Some(Arc::new(MusicBrainzClient::live(
                Default::default(),
                cache.clone(),
            ))),
            cover_art: Some(Arc::new(CoverArtClient::live(Default::default(), cache))),
        }
    }
}

/// When albums and artists are asked for again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MusicMetadataOptions {
    /// Matched albums and artists are refreshed after this long.
    pub refresh_after_ms: i64,
    /// Albums and artists MusicBrainz did not know are retried after this
    /// long (or when their tags change).
    pub retry_not_found_after_ms: i64,
    /// Failed fetches (network) are retried after this long.
    pub retry_failed_after_ms: i64,
}

impl Default for MusicMetadataOptions {
    fn default() -> Self {
        Self {
            refresh_after_ms: 90 * DAY_MS,
            retry_not_found_after_ms: 30 * DAY_MS,
            retry_failed_after_ms: 3600 * 1000,
        }
    }
}

/// Result of [`Catalog::fetch_music_metadata`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MusicMetadataReport {
    /// Albums that needed metadata.
    pub albums: usize,
    pub matched: usize,
    pub not_found: usize,
    pub failed: usize,
    /// Albums that got a cover URL.
    pub covers: usize,
    /// Artists that needed metadata, and how many were matched.
    pub artists: usize,
    pub artists_matched: usize,
    /// The run was cancelled or stopped early.
    pub stopped: bool,
    /// Why the run stopped early (offline, no client), if it did.
    pub error: Option<String>,
}

/// A track of an album as the matcher sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrackQuery {
    /// Disc number (1 without a tag).
    pub disc: u32,
    pub number: Option<u32>,
    /// Track total of the disc from the tags.
    pub total: Option<u32>,
    pub duration_ms: Option<u64>,
}

/// An album as the matcher sees it: its tags and present tracks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AlbumQuery {
    pub title: String,
    /// Album artists, main first.
    pub artists: Vec<String>,
    pub compilation: bool,
    pub year: Option<i64>,
    pub tracks: Vec<TrackQuery>,
    pub release_mbid: Option<String>,
    pub release_group_mbid: Option<String>,
}

impl AlbumQuery {
    /// Tracks the release should have: the track totals of the tags when
    /// every disc has one, else the present tracks.
    pub fn expected_tracks(&self) -> u32 {
        let mut discs: Vec<(u32, Option<u32>)> = Vec::new();
        for t in &self.tracks {
            match discs.iter_mut().find(|(d, _)| *d == t.disc) {
                Some((_, total)) => *total = (*total).max(t.total),
                None => discs.push((t.disc, t.total)),
            }
        }
        let present = u32::try_from(self.tracks.len()).unwrap_or(u32::MAX);
        if !discs.is_empty() && discs.iter().all(|(_, t)| t.is_some()) {
            discs.iter().filter_map(|(_, t)| *t).sum::<u32>().max(present)
        } else {
            present
        }
    }
}

/// Edition words: a bracketed suffix with one of them is not part of the
/// title (`OK Computer (Collector's Edition)`).
const EDITION_WORDS: &[&str] = &[
    "deluxe",
    "edition",
    "remaster",
    "remastered",
    "expanded",
    "bonus",
    "anniversary",
    "version",
    "reissue",
    "special",
    "disc",
    "cd",
];

/// Title for comparison: folded, edition suffixes and punctuation removed.
pub fn clean_title(title: &str) -> String {
    let mut t = fold(title);
    loop {
        let trimmed = t.trim_end();
        let Some(close) = trimmed.chars().last().filter(|c| matches!(c, ')' | ']')) else {
            break;
        };
        let open = if close == ')' { '(' } else { '[' };
        let Some(start) = trimmed.rfind(open) else {
            break;
        };
        let inner = &trimmed[start + 1..trimmed.len() - 1];
        if inner
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| EDITION_WORDS.contains(&w))
        {
            t = trimmed[..start].to_owned();
        } else {
            break;
        }
    }
    t.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Similarity of two cleaned names: 1 when equal, else the Dice coefficient
/// of their words, scaled below 1.
fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let wa: HashSet<&str> = a.split(' ').filter(|w| !w.is_empty()).collect();
    let wb: HashSet<&str> = b.split(' ').filter(|w| !w.is_empty()).collect();
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let common = wa.intersection(&wb).count() as f64;
    0.9 * 2.0 * common / (wa.len() + wb.len()) as f64
}

fn artist_score(q: &AlbumQuery, credits: &[ArtistCredit]) -> f64 {
    let credit = clean_title(&mb::credit_string(credits));
    let names: Vec<String> = credits
        .iter()
        .flat_map(|c| [clean_title(&c.name), clean_title(&c.artist.name)])
        .filter(|n| !n.is_empty())
        .collect();
    if q.compilation || q.artists.iter().any(|a| fold(a) == fold(VARIOUS_ARTISTS)) {
        let various = clean_title(VARIOUS_ARTISTS);
        return if credit == various || names.contains(&various) {
            1.0
        } else {
            0.3
        };
    }
    if q.artists.is_empty() {
        return 0.5;
    }
    let ours: Vec<String> = q.artists.iter().map(|a| clean_title(a)).collect();
    if ours.iter().any(|a| *a == credit || names.contains(a)) {
        return 1.0;
    }
    similarity(&ours.join(" "), &credit)
}

fn year_of(date: Option<&str>) -> Option<i64> {
    let d = date?.get(..4)?;
    d.parse().ok().filter(|y| (1000..=3000).contains(y))
}

/// Score of a release for an album before its durations are known (0–1):
/// title 0.4, artist 0.3, track count 0.2, year 0.1.
pub fn search_score(q: &AlbumQuery, r: &Release) -> f64 {
    let title = similarity(&clean_title(&q.title), &clean_title(&r.title));
    let artist = artist_score(q, &r.artist_credit);
    let expected = q.expected_tracks();
    let total = r.total_tracks();
    let tracks = match (expected, total) {
        (_, 0) | (0, _) => 0.5,
        (e, t) if e == t => 1.0,
        // Some files missing: the release may still be this one.
        (e, t) if e < t => 0.6 * f64::from(e) / f64::from(t),
        _ => 0.0,
    };
    let release_year = year_of(r.date.as_deref()).or_else(|| {
        r.release_group
            .as_ref()
            .and_then(|g| year_of(g.first_release_date.as_deref()))
    });
    let year = match (q.year, release_year) {
        (Some(a), Some(b)) if a == b => 1.0,
        (Some(a), Some(b)) if (a - b).abs() <= 1 => 0.7,
        (Some(_), Some(_)) => 0.2,
        _ => 0.5,
    };
    0.4 * title + 0.3 * artist + 0.2 * tracks + 0.1 * year
}

/// Share of the album's numbered tracks whose duration matches the length
/// of the release track with the same disc and number (within
/// [`DURATION_TOLERANCE_MS`]); `None` when nothing can be compared.
pub fn duration_score(q: &AlbumQuery, r: &Release) -> Option<f64> {
    let mut compared = 0u32;
    let mut matching = 0u32;
    for t in &q.tracks {
        let (Some(number), Some(ours)) = (t.number, t.duration_ms) else {
            continue;
        };
        let disc = if r.media.len() == 1 { 1 } else { t.disc.max(1) };
        let theirs = r
            .media
            .iter()
            .find(|m| m.position == disc)
            .and_then(|m| m.tracks.iter().find(|x| x.position == number))
            .and_then(|x| x.length.or(x.recording.length));
        compared += 1;
        if theirs.is_some_and(|l| l.abs_diff(ours) <= DURATION_TOLERANCE_MS) {
            matching += 1;
        }
    }
    (compared > 0).then(|| f64::from(matching) / f64::from(compared))
}

/// The match confidence of a looked-up release: [`search_score`] and
/// [`duration_score`] (0.6 / 0.4), or the search score alone without
/// durations.
pub fn combined_score(q: &AlbumQuery, r: &Release) -> f64 {
    let base = search_score(q, r);
    match duration_score(q, r) {
        Some(d) => 0.6 * base + 0.4 * d,
        None => base,
    }
}

/// What MusicBrainz said about an album.
#[derive(Debug, Clone)]
struct AlbumMatch {
    decided_by: &'static str,
    confidence: f64,
    release: Option<Release>,
    group: ReleaseGroup,
    credits: Vec<ArtistCredit>,
    genres: Vec<String>,
}

#[derive(Debug)]
enum Outcome<T> {
    Matched(Box<T>),
    NotFound,
    Failed(String),
}

fn failed<T>(e: &FetchError) -> Outcome<T> {
    Outcome::Failed(format!("MusicBrainz: {e}"))
}

fn normalized(raw: Vec<String>) -> Vec<String> {
    genre::normalize_all(raw).into_iter().take(MAX_GENRES).collect()
}

/// A looked-up release as the match, with release-group genres (asked for
/// when the release lookup carries none; best effort).
fn release_match(
    client: &MusicBrainzClient,
    release: Release,
    decided_by: &'static str,
    confidence: f64,
) -> Outcome<AlbumMatch> {
    let Some(mut group) = release.release_group.clone().filter(|g| !g.id.is_empty()) else {
        return Outcome::NotFound;
    };
    let mut raw = mb::top_genres(&group.genres, MAX_GENRES * 2);
    if raw.is_empty() {
        raw = mb::top_genres(&release.genres, MAX_GENRES * 2);
    }
    if raw.is_empty()
        && let Ok(full) = client.release_group(&group.id)
    {
        raw = mb::top_genres(&full.genres, MAX_GENRES * 2);
        if group.first_release_date.is_none() {
            group.first_release_date = full.first_release_date;
        }
    }
    Outcome::Matched(Box::new(AlbumMatch {
        decided_by,
        confidence,
        credits: release.artist_credit.clone(),
        genres: normalized(raw),
        release: Some(release),
        group,
    }))
}

/// Matches one album; see the [module docs](self).
fn match_album(client: &MusicBrainzClient, q: &AlbumQuery) -> Outcome<AlbumMatch> {
    if let Some(id) = &q.release_mbid {
        match client.release(id) {
            Ok(r) => return release_match(client, r, "release_mbid", 1.0),
            Err(FetchError::NotFound { .. }) => {}
            Err(e) => return failed(&e),
        }
    }
    if let Some(id) = &q.release_group_mbid {
        match client.release_group(id) {
            Ok(group) if !group.id.is_empty() => {
                return Outcome::Matched(Box::new(AlbumMatch {
                    decided_by: "release_group_mbid",
                    confidence: 1.0,
                    release: None,
                    credits: group.artist_credit.clone(),
                    genres: normalized(mb::top_genres(&group.genres, MAX_GENRES * 2)),
                    group,
                }));
            }
            Ok(_) | Err(FetchError::NotFound { .. }) => {}
            Err(e) => return failed(&e),
        }
    }
    if clean_title(&q.title).is_empty() {
        return Outcome::NotFound;
    }
    let artist = if q.compilation {
        Some(VARIOUS_ARTISTS.to_owned())
    } else {
        q.artists.first().cloned()
    };
    let results = match client.search_releases(&q.title, artist.as_deref()) {
        Ok(r) => r,
        Err(FetchError::NotFound { .. }) => return Outcome::NotFound,
        Err(e) => return failed(&e),
    };
    let mut ranked: Vec<(f64, &Release)> = results
        .iter()
        .map(|r| (search_score(q, r), r))
        .filter(|(s, _)| *s >= ACCEPT - 0.25)
        .collect();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.score.unwrap_or(0).cmp(&a.1.score.unwrap_or(0)))
    });
    let Some(&(top, _)) = ranked.first() else {
        return Outcome::NotFound;
    };
    let mut best: Option<(f64, Release)> = None;
    for (pre, candidate) in ranked.into_iter().take(MAX_LOOKUPS) {
        if pre < top - 0.2 {
            break;
        }
        let release = match client.release(&candidate.id) {
            Ok(r) => r,
            Err(FetchError::NotFound { .. }) => continue,
            Err(e) => return failed(&e),
        };
        let score = combined_score(q, &release);
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, release));
        }
        // An exact match needs no further lookups.
        if score >= 0.999 {
            break;
        }
    }
    match best {
        Some((score, release)) if score >= ACCEPT => {
            release_match(client, release, "search", score.clamp(0.0, 1.0))
        }
        _ => Outcome::NotFound,
    }
}

/// What MusicBrainz said about an artist.
#[derive(Debug, Clone)]
struct ArtistMatch {
    decided_by: &'static str,
    artist: mb::Artist,
    genres: Vec<String>,
}

fn match_artist(
    client: &MusicBrainzClient,
    name: &str,
    mbid: Option<&str>,
    from_album: bool,
) -> Outcome<ArtistMatch> {
    let (id, decided_by) = match mbid {
        Some(id) => (id.to_owned(), if from_album { "album" } else { "mbid" }),
        None => {
            let found = match client.search_artists(name) {
                Ok(f) => f,
                Err(FetchError::NotFound { .. }) => return Outcome::NotFound,
                Err(e) => return failed(&e),
            };
            let key = fold(name);
            let exact: Vec<&mb::Artist> = found
                .iter()
                .filter(|a| fold(&a.name) == key && a.score.unwrap_or(0) >= ARTIST_SEARCH_SCORE)
                .collect();
            // Two artists with this exact name: ambiguous, leave it.
            match exact.as_slice() {
                [one] => (one.id.clone(), "search"),
                _ => return Outcome::NotFound,
            }
        }
    };
    match client.artist(&id) {
        Ok(artist) if mb::mbid(&artist.id).is_some() => Outcome::Matched(Box::new(ArtistMatch {
            decided_by,
            genres: normalized(mb::top_genres(&artist.genres, MAX_GENRES * 2)),
            artist,
        })),
        Ok(_) | Err(FetchError::NotFound { .. }) => Outcome::NotFound,
        Err(e) => failed(&e),
    }
}

// --- Database --------------------------------------------------------------

fn pending_albums(
    conn: &Connection,
    options: &MusicMetadataOptions,
    now: i64,
) -> Result<Vec<i64>, Error> {
    let mut stmt = conn.prepare(
        "SELECT a.id FROM music_albums a
         LEFT JOIN music_album_metadata m ON m.album_id = a.id
         WHERE EXISTS (SELECT 1 FROM music_tracks t
                       JOIN files f ON f.id = t.file_id AND f.missing_since_ms IS NULL
                       WHERE t.album_id = a.id)
           AND (m.album_id IS NULL
                OR (m.outcome = 'matched' AND m.fetched_at_ms < ?1 - ?2)
                OR (m.outcome = 'not_found' AND m.fetched_at_ms < ?1 - ?3)
                OR (m.outcome = 'failed' AND m.fetched_at_ms < ?1 - ?4)
                OR (m.outcome <> 'matched' AND a.updated_at_ms > m.fetched_at_ms))
         ORDER BY a.id",
    )?;
    let rows = stmt.query_map(
        params![
            now,
            options.refresh_after_ms,
            options.retry_not_found_after_ms,
            options.retry_failed_after_ms
        ],
        |r| r.get(0),
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Matched albums whose cover was not asked for yet.
fn pending_covers(conn: &Connection) -> Result<Vec<(i64, String)>, Error> {
    let mut stmt = conn.prepare(
        "SELECT album_id, release_group_mbid FROM music_album_metadata
         WHERE outcome = 'matched' AND cover_checked = 0 ORDER BY album_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Album artists (not "Various Artists") without fresh metadata, with
/// their name and MBID.
fn pending_artists(
    conn: &Connection,
    options: &MusicMetadataOptions,
    now: i64,
) -> Result<Vec<(i64, String, Option<String>)>, Error> {
    let mut stmt = conn.prepare(
        "SELECT ar.id, ar.name, ar.mbid FROM music_artists ar
         LEFT JOIN music_artist_metadata m ON m.artist_id = ar.id
         WHERE EXISTS (SELECT 1 FROM music_album_artists x
                       JOIN music_tracks t ON t.album_id = x.album_id
                       JOIN files f ON f.id = t.file_id AND f.missing_since_ms IS NULL
                       WHERE x.artist_id = ar.id)
           AND ar.name_key <> ?5
           AND (m.artist_id IS NULL
                OR (m.outcome = 'matched' AND m.fetched_at_ms < ?1 - ?2)
                OR (m.outcome = 'not_found' AND m.fetched_at_ms < ?1 - ?3)
                OR (m.outcome = 'failed' AND m.fetched_at_ms < ?1 - ?4))
         ORDER BY ar.id",
    )?;
    let rows = stmt.query_map(
        params![
            now,
            options.refresh_after_ms,
            options.retry_not_found_after_ms,
            options.retry_failed_after_ms,
            fold(VARIOUS_ARTISTS)
        ],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn load_album(conn: &Connection, id: i64) -> Result<Option<AlbumQuery>, Error> {
    let Some((title, compilation, year, release_mbid, release_group_mbid)) = conn
        .query_row(
            "SELECT title, compilation, year, release_mbid, release_group_mbid
             FROM music_albums WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let artists = conn
        .prepare(
            "SELECT ar.name FROM music_album_artists x JOIN music_artists ar ON ar.id = x.artist_id
             WHERE x.album_id = ?1 ORDER BY x.position",
        )?
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let to_u32 = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());
    let tracks = conn
        .prepare(
            "SELECT t.disc_number, t.track_number, t.track_total, t.duration_ms FROM music_tracks t
             JOIN files f ON f.id = t.file_id AND f.missing_since_ms IS NULL
             WHERE t.album_id = ?1 ORDER BY t.disc_number, t.track_number, t.id",
        )?
        .query_map([id], |r| {
            Ok(TrackQuery {
                disc: to_u32(r.get(0)?).unwrap_or(1),
                number: to_u32(r.get(1)?),
                total: to_u32(r.get(2)?),
                duration_ms: r
                    .get::<_, Option<i64>>(3)?
                    .and_then(|v| u64::try_from(v).ok()),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(AlbumQuery {
        title,
        artists,
        compilation,
        year,
        tracks,
        release_mbid,
        release_group_mbid,
    }))
}

fn json_list(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".to_owned())
}

/// Writes the outcome of an album; returns the ids of album artists that
/// got an MBID from the release credit.
fn write_album(
    conn: &mut Connection,
    album_id: i64,
    outcome: &Outcome<AlbumMatch>,
    now: i64,
) -> Result<Vec<i64>, Error> {
    let tx = conn.transaction()?;
    let mut learned = Vec::new();
    match outcome {
        Outcome::Matched(m) => {
            let release = m.release.as_ref();
            let original_year = year_of(m.group.first_release_date.as_deref())
                .or_else(|| release.and_then(|r| year_of(r.date.as_deref())));
            tx.execute(
                "INSERT OR REPLACE INTO music_album_metadata
                     (album_id, outcome, decided_by, confidence, release_mbid, release_group_mbid,
                      title, artist_credit, release_date, original_year, primary_type,
                      secondary_types, country, label, genres, cover_url, cover_checked,
                      error, fetched_at_ms)
                 VALUES (?1, 'matched', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         NULL, 0, NULL, ?15)",
                params![
                    album_id,
                    m.decided_by,
                    m.confidence,
                    release.and_then(|r| mb::mbid(&r.id)),
                    mb::mbid(&m.group.id),
                    release.map_or(m.group.title.as_str(), |r| r.title.as_str()),
                    Some(mb::credit_string(&m.credits)).filter(|c| !c.is_empty()),
                    release.and_then(|r| r.date.clone()),
                    original_year,
                    m.group.primary_type,
                    json_list(&m.group.secondary_types),
                    release.and_then(|r| r.country.clone()),
                    release.and_then(|r| r.label().map(str::to_owned)),
                    json_list(&m.genres),
                    now,
                ],
            )?;
            // Album artists without an MBID take the one of the credit with
            // the same name.
            let artists: Vec<(i64, String)> = tx
                .prepare(
                    "SELECT ar.id, ar.name FROM music_album_artists x
                     JOIN music_artists ar ON ar.id = x.artist_id
                     WHERE x.album_id = ?1 AND ar.mbid IS NULL",
                )?
                .query_map([album_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            for (artist_id, name) in artists {
                let key = fold(&name);
                let credit = m
                    .credits
                    .iter()
                    .find(|c| fold(&c.name) == key || fold(&c.artist.name) == key)
                    .and_then(|c| mb::mbid(&c.artist.id));
                if let Some(id) = credit {
                    tx.execute(
                        "UPDATE music_artists SET mbid = ?2 WHERE id = ?1 AND mbid IS NULL",
                        params![artist_id, id],
                    )?;
                    // A name search that found nothing is retried by MBID.
                    tx.execute(
                        "DELETE FROM music_artist_metadata WHERE artist_id = ?1 AND outcome <> 'matched'",
                        [artist_id],
                    )?;
                    learned.push(artist_id);
                }
            }
        }
        Outcome::NotFound | Outcome::Failed(_) => {
            let (state, error) = match outcome {
                Outcome::Failed(e) => ("failed", Some(e.as_str())),
                _ => ("not_found", None),
            };
            tx.execute(
                "INSERT OR REPLACE INTO music_album_metadata
                     (album_id, outcome, error, fetched_at_ms) VALUES (?1, ?2, ?3, ?4)",
                params![album_id, state, error, now],
            )?;
        }
    }
    tx.commit()?;
    Ok(learned)
}

fn write_cover(conn: &Connection, album_id: i64, cover: Option<&str>) -> Result<(), Error> {
    conn.execute(
        "UPDATE music_album_metadata SET cover_url = ?2, cover_checked = 1
         WHERE album_id = ?1 AND outcome = 'matched'",
        params![album_id, cover],
    )?;
    Ok(())
}

fn write_artist(
    conn: &Connection,
    artist_id: i64,
    outcome: &Outcome<ArtistMatch>,
    now: i64,
) -> Result<(), Error> {
    match outcome {
        Outcome::Matched(m) => {
            let a = &m.artist;
            let text = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
            conn.execute(
                "INSERT OR REPLACE INTO music_artist_metadata
                     (artist_id, outcome, decided_by, mbid, name, sort_name, artist_type, country,
                      disambiguation, begin_date, end_date, genres, error, fetched_at_ms)
                 VALUES (?1, 'matched', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12)",
                params![
                    artist_id,
                    m.decided_by,
                    mb::mbid(&a.id),
                    text(&a.name),
                    text(&a.sort_name),
                    a.kind,
                    a.country,
                    text(&a.disambiguation),
                    a.life_span.begin,
                    a.life_span.end,
                    json_list(&m.genres),
                    now,
                ],
            )?;
            conn.execute(
                "UPDATE music_artists SET mbid = coalesce(mbid, ?2),
                     sort_name = coalesce(sort_name, ?3)
                 WHERE id = ?1",
                params![artist_id, mb::mbid(&a.id), text(&a.sort_name)],
            )?;
        }
        Outcome::NotFound | Outcome::Failed(_) => {
            let (state, error) = match outcome {
                Outcome::Failed(e) => ("failed", Some(e.as_str())),
                _ => ("not_found", None),
            };
            conn.execute(
                "INSERT OR REPLACE INTO music_artist_metadata
                     (artist_id, outcome, error, fetched_at_ms) VALUES (?1, ?2, ?3, ?4)",
                params![artist_id, state, error, now],
            )?;
        }
    }
    Ok(())
}

/// MusicBrainz data of an album artist ([`Catalog::music_artist_infos`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MusicArtistInfo {
    /// `music_artists.id`.
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
    /// Normalized genres, most votes first.
    pub genres: Vec<String>,
}

/// JSON array of strings, empty when it is not one.
pub(crate) fn parse_list(json: Option<String>) -> Vec<String> {
    json.and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

impl Catalog {
    /// Fetches MusicBrainz metadata and Cover Art Archive covers for the
    /// albums and album artists that need them; see the
    /// [module docs](self). Blocking and slow (one MusicBrainz request per
    /// second): run it in the background after a scan. `progress` gets
    /// `(done, total)`; `total` grows when the artists are counted after the
    /// albums.
    pub fn fetch_music_metadata(
        &self,
        providers: &MusicProviders,
        options: &MusicMetadataOptions,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<MusicMetadataReport, Error> {
        let mut report = MusicMetadataReport::default();
        let Some(client) = providers.musicbrainz.as_deref() else {
            report.error = Some("no MusicBrainz client".into());
            return Ok(report);
        };
        let now = now_ms();
        let albums = pending_albums(&self.lock(), options, now)?;
        report.albums = albums.len();
        let mut total = albums.len();
        let mut done = 0;
        let mut failures = 0;
        let mut learned: HashSet<i64> = HashSet::new();
        progress(done, total);

        let stop = |report: &mut MusicMetadataReport, failures: usize| {
            if failures >= MAX_FAILURES_IN_A_ROW {
                report.stopped = true;
                report.error = Some("MusicBrainz unreachable (offline?)".into());
                true
            } else if cancel.is_cancelled() {
                report.stopped = true;
                true
            } else {
                false
            }
        };

        for id in albums {
            if stop(&mut report, failures) {
                return Ok(report);
            }
            let query = load_album(&self.lock(), id)?;
            let Some(query) = query else {
                done += 1;
                continue;
            };
            let outcome = match_album(client, &query);
            match &outcome {
                Outcome::Matched(_) => {
                    report.matched += 1;
                    failures = 0;
                }
                Outcome::NotFound => {
                    report.not_found += 1;
                    failures = 0;
                }
                Outcome::Failed(_) => {
                    report.failed += 1;
                    failures += 1;
                }
            }
            learned.extend(write_album(&mut self.lock(), id, &outcome, now_ms())?);
            done += 1;
            progress(done, total);
        }

        if let Some(covers) = providers.cover_art.as_deref() {
            // Bound first: the lock must not live through the loop.
            let todo = pending_covers(&self.lock())?;
            for (album_id, group) in todo {
                if stop(&mut report, 0) {
                    return Ok(report);
                }
                // A network failure leaves `cover_checked = 0` for later.
                if let Ok(url) = covers.front(&group) {
                    report.covers += usize::from(url.is_some());
                    write_cover(&self.lock(), album_id, url.as_deref())?;
                }
            }
        }

        let artists = pending_artists(&self.lock(), options, now)?;
        report.artists = artists.len();
        total += artists.len();
        progress(done, total);
        failures = 0;
        for (artist_id, name, mbid) in artists {
            if stop(&mut report, failures) {
                return Ok(report);
            }
            let outcome = match_artist(
                client,
                &name,
                mbid.as_deref(),
                learned.contains(&artist_id),
            );
            match &outcome {
                Outcome::Matched(_) => {
                    report.artists_matched += 1;
                    failures = 0;
                }
                Outcome::NotFound => failures = 0,
                Outcome::Failed(_) => failures += 1,
            }
            write_artist(&self.lock(), artist_id, &outcome, now_ms())?;
            done += 1;
            progress(done, total);
        }
        Ok(report)
    }

    /// Makes the next music metadata run ask MusicBrainz again for every
    /// album and artist.
    pub fn reset_music_metadata(&self) -> Result<(), Error> {
        let conn = self.lock();
        conn.execute_batch(
            "DELETE FROM music_album_metadata; DELETE FROM music_artist_metadata;",
        )?;
        Ok(())
    }

    /// MusicBrainz data of the matched album artists, by name.
    pub fn music_artist_infos(&self) -> Result<Vec<MusicArtistInfo>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT ar.id, ar.name, m.mbid, m.sort_name, m.artist_type, m.country,
                    m.disambiguation, m.begin_date, m.end_date, m.genres
             FROM music_artist_metadata m JOIN music_artists ar ON ar.id = m.artist_id
             WHERE m.outcome = 'matched'
             ORDER BY ar.name COLLATE NOCASE, ar.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MusicArtistInfo {
                artist_id: r.get(0)?,
                name: r.get(1)?,
                mbid: r.get(2)?,
                sort_name: r.get(3)?,
                kind: r.get(4)?,
                country: r.get(5)?,
                disambiguation: r.get(6)?,
                begin_date: r.get(7)?,
                end_date: r.get(8)?,
                genres: parse_list(r.get(9)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edition_suffixes_are_ignored() {
        assert_eq!(clean_title("OK Computer (Collector's Edition)"), "ok computer");
        assert_eq!(clean_title("Album [Remastered 2011] (Deluxe)"), "album");
        assert_eq!(clean_title("Live (at Home)"), "live at home");
        assert_eq!(clean_title("Синтетика: Часть 1"), "синтетика часть 1");
    }

    #[test]
    fn expected_tracks_use_totals_of_every_disc() {
        let t = |disc, total| TrackQuery {
            disc,
            number: Some(1),
            total,
            duration_ms: None,
        };
        let q = AlbumQuery {
            tracks: vec![t(1, Some(10)), t(2, Some(8))],
            ..Default::default()
        };
        assert_eq!(q.expected_tracks(), 18);
        let q = AlbumQuery {
            tracks: vec![t(1, Some(10)), t(2, None)],
            ..Default::default()
        };
        assert_eq!(q.expected_tracks(), 2);
    }
}
