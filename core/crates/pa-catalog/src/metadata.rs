// SPDX-License-Identifier: MPL-2.0
//! Provider metadata for recognized works (META-013, basic matcher).
//!
//! [`Catalog::fetch_metadata`] takes every work with a live match that has
//! no fresh metadata ([`MetadataOptions`]) and:
//!
//! 1. **TMDB.** A work with a TMDB id (a user fix, an NFO file or an
//!    earlier match) is read by id; a work with only an IMDb or TVDB id
//!    from an NFO file is looked up by it (`/find`). Otherwise TMDB is
//!    searched by its titles (main first) and
//!    year — `/search/movie` for movies, `/search/tv` for shows and anime —
//!    and the results are ranked with the local title index
//!    ([`pa_recognition::title::TitleIndex`]: title tiers, year ±1, kind);
//!    the best one is taken if it scores at least [`ACCEPT`]
//!    ([`match_search`], also run offline by `pa-eval`). Details,
//!    translations (titles in the metadata language, English and Russian)
//!    and the seasons of the work's episodes are fetched.
//! 2. **Anime** (works of kind `anime`): AniList by id or by title (same
//!    ranking) for romaji / English / native titles, the MAL id and a cover;
//!    Shikimori by MAL id for the Russian title and synopsis. They fill what
//!    TMDB did not (no TMDB match, no Russian overview).
//! 3. **Writing** (one transaction per work): external ids, titles (the
//!    provider title becomes `main`, local titles stay as synonyms so files
//!    named the old way still join the work), overview, year, runtime,
//!    status, artwork (TMDB poster and backdrop, AniList cover; episode
//!    stills), episode titles, overviews, air dates and TMDB ids. Matches
//!    decided by the parser become `decided_by = scorer`,
//!    `model_version` = [`MODEL_VERSION`] with the match score as confidence
//!    and `auto_review` below [`AUTO`]; user decisions are left alone. A
//!    work no provider knew (`not_found`) gets the same `model_version` on
//!    its automatic matches, so a newer scorer asks again at once instead of
//!    after [`MetadataOptions::retry_not_found_after_ms`] (BUG-009). A TMDB id
//!    already owned by another work merges the two works (two folders of
//!    one show named differently).
//!
//! The network runs on worker threads without holding the database;
//! results are written one work at a time, so the library stays readable.
//! A rejected TMDB key stops the run; network failures mark the work
//! `failed` (retried on a later run) and several in a row stop the run.
//!
//! **Two stages** (LIB-023, ADR-0095). AniList allows 30 requests a minute
//! in its degraded mode, so a library with dozens of anime used to keep
//! every worker waiting on it while films and series queued behind. The run
//! now goes in two stages ([`MetadataStage`]):
//! 1. *Titles*: steps 1 and 3 for every work — the local anime database and
//!    TMDB, [`TITLE_WORKERS`] works at a time. Titles, posters and
//!    descriptions appear for the whole library within TMDB's rate.
//! 2. *Anime*: step 2 for anime works, at AniList's and Shikimori's pace;
//!    each work is written again with everything both stages found. A work
//!    whose TMDB data is written but whose anime stage has not run keeps
//!    `work_metadata.anime_pending`, so a stopped run is finished by the
//!    next one.
//!
//! Identical requests of several works go out once (`pa-metadata` keeps
//! one request per cache key in flight), and progress
//! ([`MetadataProgress`]) carries the stage, the counts and how long a
//! provider makes us wait ([`ProviderWait`]).
//!
//! 3a. **Episode numbers of anime** (META-015): the episodes of an anime
//!    work matched to a TMDB series are renumbered into TMDB order from
//!    what the file names say ([`crate::numbering`]: running counts, entry
//!    titles through the Anime-Lists list), and the seasons they land in
//!    are fetched too.
//!
//! 4. **AniDB order** (META-029): with the Anime-Lists list downloaded on
//!    the device ([`MetadataProviders::anime_mappings`]) the run ends by
//!    writing the AniDB episode numbers of anime known by TMDB or TVDB id
//!    into `episode_mappings` ([`crate::anime_order`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use pa_library::rusqlite::{Connection, OptionalExtension, Transaction, params};
use pa_metadata::anilist::{AniListClient, Media};
use pa_metadata::cache::ProviderCache;
use pa_metadata::fetch::FetchError;
use pa_metadata::shikimori::{RussianInfo, ShikimoriClient};
use pa_metadata::tmdb::{
    self, Credits, Details, ExternalSource, MediaKind, SearchResult, Season, TmdbClient, TmdbError,
};
use pa_recognition::parse::MediaHint;
use pa_recognition::score::pair;
use pa_recognition::title::{MatchTier, TitleIndex, TitleQuery, TitleRecord, WorkKind};
use pa_scanner::CancelToken;
use serde_json::json;

use crate::{Catalog, Error};

#[path = "metadata_extra.rs"]
mod extra;
pub use extra::ExtraProviders;

/// Lowest ranking score of a search result that is accepted as the work.
pub const ACCEPT: f64 = 0.75;

/// Lowest confidence of a match that stays `auto`; below it the match is
/// `auto_review` ("check this", research 03 §4.2 [7]).
pub const AUTO: f64 = 0.92;

/// Lowest ranking score of a search result that is still shown, below
/// [`ACCEPT`]: the provider's best answer when nothing ranks well
/// enough, with its title and poster, as `auto_review` (META-033;
/// META-016 «please check»). Only an untied result counts. Below it the
/// work goes to the manual queue of the review inbox with the results
/// the search weighed ([`crate::review`]). META-016 raised it from 0.6
/// to the 0.70 of research 03 §4.2: corpus v0 has no result between 0.6
/// and 0.85, so the floor follows the research, not a measurement.
pub const REVIEW: f64 = 0.70;

/// `matches.model_version` of decisions made here.
///
/// Bump it whenever the ranking can decide a work differently (new
/// inputs, weights, thresholds): the next metadata run decides every
/// automatic decision of an older version again — `auto` and
/// `auto_review` scorer matches and `not_found` works — so a library
/// that outlived an update (the database stays when the app is removed and
/// installed again) gets the better answers without the user's help.
/// User decisions, NFO ids and folder rules are never decided again.
///
/// - `scorer_v1` (META-015): episode numbering;
/// - `scorer_v2` (BUG-009): titles in other languages (META-033) and the
///   review thresholds (META-016) had changed the ranking under `scorer_v1`,
///   so decisions of builds before them stayed in the review inbox;
/// - `scorer_v3` (BUG-011): a file named by the title before the subtitle
///   (`Агент 327 (2017)` → `Агент 327: Операция «Парикмахерская»`) matches
///   it as a head title, decided alone with the same year.
pub const MODEL_VERSION: &str = "scorer_v3";

/// SQL (on `matches AS m`): a match of a `not_found` work that carries
/// the version of the scorer that found nothing (BUG-009): an automatic
/// one. A folder rule above the file does not keep it: the rule did not
/// decide it (its matches are `user_set`), and only provider data is
/// written to a work no provider knew.
macro_rules! not_found_matches {
    () => {
        "(m.state IN ('auto', 'auto_review') AND m.decided_by NOT IN ('user', 'nfo'))"
    };
}

/// Title queries tried per work.
const MAX_QUERIES: usize = 3;

/// Network failures in a row that end a run (offline).
const MAX_FAILURES_IN_A_ROW: usize = 5;

/// Worker threads of the titles stage (TMDB at 20 requests a second, a
/// handful of requests per work).
pub const TITLE_WORKERS: usize = 8;

/// Worker threads of the anime stage (AniList and Shikimori pace them).
const ANIME_WORKERS: usize = 4;

/// How often a waiting run reports progress (provider waits, liveness).
const TICK: Duration = Duration::from_millis(500);

/// Stage of a metadata run (see the [module docs](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataStage {
    /// Local anime database and TMDB for every work.
    Titles,
    /// AniList and Shikimori for anime works.
    Anime,
}

/// A provider that makes the run wait (rate limit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderWait {
    /// `tmdb`, `anilist` or `shikimori`.
    pub provider: &'static str,
    /// Time until its next request goes out.
    pub wait: Duration,
    /// The provider asked for it (HTTP 429 with `Retry-After`, quota used
    /// up); otherwise it is our pacing under its published limit.
    pub provider_asked: bool,
}

/// Progress of [`Catalog::fetch_metadata_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataProgress {
    pub stage: MetadataStage,
    /// Works done in this stage.
    pub done: usize,
    /// Works of this stage.
    pub total: usize,
    /// Works whose outcome is final (all stages).
    pub decided: usize,
    /// Works of the run.
    pub works: usize,
    /// Providers that make the run wait at least a second.
    pub waiting: Vec<ProviderWait>,
}

/// TMDB image sizes stored in `artwork.url`.
const POSTER_SIZE: &str = "w500";
const BACKDROP_SIZE: &str = "w1280";
const STILL_SIZE: &str = "w300";
/// Cast and crew photos (UI-009).
const PROFILE_SIZE: &str = "w185";

const DAY_MS: i64 = 24 * 3600 * 1000;

/// The provider clients a metadata run uses; any of them may be absent.
#[derive(Clone, Default)]
pub struct MetadataProviders {
    pub tmdb: Option<Arc<TmdbClient>>,
    pub anilist: Option<Arc<AniListClient>>,
    pub shikimori: Option<Arc<ShikimoriClient>>,
    /// The Anime-Lists list last downloaded on this device
    /// ([`AnimeLists::load`](crate::AnimeLists::load), META-029). With it
    /// the run ends by writing the AniDB episode numbers of anime
    /// ([`Catalog::apply_anime_mappings`]); without it the AniDB order is
    /// not offered.
    pub anime_mappings: Option<Arc<pa_anime_mappings::MappingIndex>>,
    /// anime-offline-database on the device (bundled or updated, META-030,
    /// [`AnimeDb::load`](crate::AnimeDb::load)): anime works are recognised
    /// by its titles without a request ([`local_anime`]) and the episode
    /// numbering takes entry sizes and titles from it.
    pub anime_titles: Option<Arc<crate::AnimeTitles>>,
    /// TVmaze, TheTVDB, Fanart.tv and the OpenSubtitles hash lookup
    /// (META-012, [`ExtraProviders`]).
    pub extra: ExtraProviders,
}

impl std::fmt::Debug for MetadataProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataProviders")
            .field("tmdb", &self.tmdb.is_some())
            .field("anilist", &self.anilist.is_some())
            .field("shikimori", &self.shikimori.is_some())
            .field("anime_mappings", &self.anime_mappings.is_some())
            .field("anime_titles", &self.anime_titles.as_ref().map(|t| t.len()))
            .field("extra", &self.extra)
            .finish()
    }
}

impl MetadataProviders {
    /// Live clients (HTTPS) sharing one response cache: TMDB with the
    /// build's project key or `tmdb_user_key`, in `language` (`ru-RU`), the
    /// keyless AniList and Shikimori, and the [`ExtraProviders`] without
    /// user keys.
    pub fn live(
        cache: Arc<dyn ProviderCache>,
        tmdb_user_key: Option<&str>,
        language: &str,
    ) -> Self {
        Self::live_with(cache, tmdb_user_key, None, None, None, language)
    }

    /// [`live`](Self::live) with the user's TheTVDB key and personal
    /// Fanart.tv key from Settings (META-012).
    pub fn live_with(
        cache: Arc<dyn ProviderCache>,
        tmdb_user_key: Option<&str>,
        tvdb_key: Option<&str>,
        tvdb_pin: Option<&str>,
        fanart_personal_key: Option<&str>,
        language: &str,
    ) -> Self {
        let config = tmdb::TmdbConfig {
            language: language.to_owned(),
            ..Default::default()
        };
        Self {
            tmdb: Some(Arc::new(TmdbClient::live(
                config,
                tmdb_user_key,
                cache.clone(),
            ))),
            anilist: Some(Arc::new(AniListClient::live(
                Default::default(),
                cache.clone(),
            ))),
            shikimori: Some(Arc::new(ShikimoriClient::live(
                Default::default(),
                cache.clone(),
            ))),
            anime_mappings: None,
            anime_titles: None,
            extra: ExtraProviders::live(cache, tvdb_key, tvdb_pin, fanart_personal_key),
        }
    }

    /// The metadata language: TMDB's, `ru-RU` without TMDB.
    pub fn language(&self) -> String {
        self.tmdb
            .as_ref()
            .map_or_else(|| "ru-RU".to_owned(), |t| t.config().language.clone())
    }

    /// TMDB, if it has a key.
    pub(crate) fn tmdb(&self) -> Option<&TmdbClient> {
        self.tmdb.as_deref().filter(|t| t.key_source().is_some())
    }
}

/// When works are asked for again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataOptions {
    /// Matched works are refreshed after this long (well below TMDB's
    /// six-month cache limit).
    pub refresh_after_ms: i64,
    /// Works no provider knew are retried after this long.
    pub retry_not_found_after_ms: i64,
    /// Works whose fetch failed (network) are retried after this long.
    pub retry_failed_after_ms: i64,
}

impl Default for MetadataOptions {
    fn default() -> Self {
        Self {
            refresh_after_ms: 30 * DAY_MS,
            retry_not_found_after_ms: 7 * DAY_MS,
            retry_failed_after_ms: 3600 * 1000,
        }
    }
}

/// Result of [`Catalog::fetch_metadata`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataReport {
    /// Works that needed metadata.
    pub works: usize,
    pub matched: usize,
    pub not_found: usize,
    pub failed: usize,
    /// Works merged into another work with the same TMDB id.
    pub merged: usize,
    /// The run was cancelled or stopped early.
    pub stopped: bool,
    /// Why the run stopped early (rejected key, offline), if it did.
    pub error: Option<String>,
}

/// A work as the matcher sees it.
#[derive(Debug, Clone)]
struct WorkInfo {
    id: i64,
    kind: String,
    year: Option<i64>,
    /// All titles, main first.
    titles: Vec<String>,
    tmdb_id: Option<u64>,
    /// IMDb / TVDB ids (from NFO files), looked up when there is no TMDB id.
    imdb_id: Option<String>,
    tvdb_id: Option<String>,
    anilist_id: Option<u64>,
    /// Season numbers of numbered episodes.
    seasons: Vec<u32>,
    /// What the work's files say (pair features of the ranking, META-014).
    evidence: WorkEvidence,
    /// `(file id, relative path)` of the numbered files of an anime work
    /// (META-015 renumbering); empty for other kinds.
    files: Vec<(i64, String)>,
    /// The work's TMDB id came from an older scorer and nothing but
    /// automatic, unconfirmed scorer matches speak for it: the current
    /// model decides again instead of reading the id (META-015).
    rescore: bool,
    /// The work's AniList id came from its frames (META-031): it is read
    /// by that id, and TMDB is not searched by the file names that failed.
    frames: bool,
    /// Air dates of the unnumbered episodes (daily shows, META-012).
    dates: Vec<String>,
    /// OSHashes of the work's files, for the OpenSubtitles lookup
    /// (META-012).
    hashes: Vec<String>,
}

impl WorkInfo {
    fn query(&self) -> WorkQuery {
        WorkQuery {
            kind: self.kind.clone(),
            year: self.year,
            titles: self.titles.clone(),
            evidence: self.evidence.clone(),
        }
    }
}

/// A work as a provider search sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkQuery {
    /// `movie`, `show` or `anime`.
    pub kind: String,
    pub year: Option<i64>,
    /// Titles to search by, main first; the first [`MAX_QUERIES`] are tried.
    pub titles: Vec<String>,
    /// What the work's files say, compared with each result's details
    /// (META-014 pair features); empty when unknown.
    pub evidence: WorkEvidence,
}

/// What the files of a work say about it (META-014): the group-level
/// input of the pair features in [`pa_recognition::score::pair`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkEvidence {
    /// `(season, episode)` of its numbered episodes, as written.
    pub episodes: Vec<(u32, u32)>,
    /// Container durations of its main files (movie: the feature or its
    /// parts; series: the episodes), milliseconds.
    pub durations_ms: Vec<u64>,
    /// Audio languages of its files as probed (`jpn`, `eng`).
    pub audio_langs: Vec<String>,
}

impl WorkEvidence {
    /// Nothing known.
    pub fn is_empty(&self) -> bool {
        self.episodes.is_empty() && self.durations_ms.is_empty() && self.audio_langs.is_empty()
    }

    /// Pair-feature evidence: the movie's longest file or the episodes'
    /// median duration.
    fn pair(&self, movie: bool) -> pair::Evidence {
        let mut d = self.durations_ms.clone();
        d.sort_unstable();
        let duration = if movie {
            d.last().copied()
        } else {
            d.get(d.len() / 2).copied()
        };
        pair::Evidence::of_file(duration, &self.audio_langs, self.episodes.clone())
    }
}

/// What a details request adds to a search result for the ranking
/// (runtime, season list, original language).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultFacts {
    /// Film runtime, or a typical episode's, in minutes.
    pub runtime_min: Option<u32>,
    /// Regular episodes per season.
    pub seasons: BTreeMap<u32, u32>,
    /// ISO 639-1 original language.
    pub original_language: Option<String>,
}

impl ResultFacts {
    /// Facts of TMDB details: movie runtime or median episode runtime,
    /// regular seasons (`S00` specials left out).
    pub fn of_details(d: &Details) -> Self {
        let mut runs = d.episode_run_time.clone();
        runs.sort_unstable();
        Self {
            runtime_min: d
                .runtime
                .filter(|&r| r > 0)
                .or_else(|| runs.get(runs.len() / 2).copied().filter(|&r| r > 0)),
            seasons: d
                .seasons
                .iter()
                .filter(|s| s.season_number >= 1 && s.episode_count > 0)
                .map(|s| (s.season_number, s.episode_count))
                .collect(),
            original_language: d.original_language.clone(),
        }
    }

    fn apply(&self, record: &mut TitleRecord) {
        if self.runtime_min.is_some() {
            record.runtime_min = self.runtime_min;
        }
        if !self.seasons.is_empty() {
            record.seasons = self.seasons.clone();
        }
        if self.original_language.is_some() {
            record.original_language = self.original_language.clone();
        }
    }
}

/// The search result a work was matched to ([`match_search`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    pub result: SearchResult,
    /// Ranking score, at least [`ACCEPT`]; the match confidence.
    pub score: f64,
    /// Title tier of the ranking (`exact`, `loose`…).
    pub tier: String,
    /// The title the result was found by.
    pub query: String,
    /// Why (META-016 explanation).
    pub signals: MatchSignals,
}

/// The signals behind a candidate's score (META-016 `explanation_json`):
/// what the inbox shows as «почему».
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MatchSignals {
    /// `tmdb` (search), `anime_offline_database` (the local anime
    /// database), `anilist`.
    pub provider: String,
    /// Title similarity of the title index (title tiers, year ±1, kind),
    /// `0..=1`.
    pub title: f64,
    /// How the title matched (`exact`, `loose`, `compact`, `base`, `head`, `fuzzy`).
    pub tier: String,
    /// The candidate's title that matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_title: Option<String>,
    /// Year in the file or folder names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_year: Option<i64>,
    /// Year of the candidate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i64>,
    /// What the files look like: `movie`, `show`, `anime`.
    pub file_kind: String,
    /// The candidate: `movie` or `tv`.
    pub kind: String,
    /// `|duration − runtime| / runtime` (META-014 pair feature).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_error: Option<f64>,
    /// Written episodes the candidate's seasons have.
    pub episodes_ok: u32,
    /// Written episodes it does not have.
    pub episodes_bad: u32,
    /// An audio track is in the candidate's original language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<bool>,
    /// Another work scores the same: the title alone cannot tell them
    /// apart.
    pub tied: bool,
    /// A word of the files' name (`ova`, `movie`) that the candidate's
    /// title does not have: perhaps a special or film of the franchise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lost_marker: Option<String>,
    /// The title the candidate was found by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

/// A ranked search result ([`search_ranked`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredResult {
    pub result: SearchResult,
    /// Ranking score with the pair features.
    pub score: f64,
    pub signals: MatchSignals,
}

/// The decision of a search and the results it weighed (META-016).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchDecision {
    /// The result taken, `auto` or `auto_review` by its score; `None`:
    /// nothing scored [`REVIEW`] untied, the work goes to the manual
    /// queue.
    pub chosen: Option<SearchMatch>,
    /// The best results of the final ranking (at most [`CANDIDATES`]),
    /// best first: what the inbox offers.
    pub candidates: Vec<ScoredResult>,
}

/// Candidates kept per work for the inbox.
pub const CANDIDATES: usize = 5;

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// What TMDB said about a work.
#[derive(Debug)]
struct TmdbData {
    kind: MediaKind,
    details: Details,
    /// `(ISO 639-1, title)` from translations.
    titles: Vec<(String, String)>,
    /// Seasons of the work's numbered episodes.
    seasons: Vec<Season>,
    /// Cast and crew (UI-009); empty when TMDB has none or the request
    /// failed: credits are best effort and never fail a fetch.
    credits: Credits,
    /// `(country, certification)` content ratings (LIB-012), best effort
    /// like the credits.
    certifications: Vec<(String, String)>,
    /// The anime work's files in TMDB numbering (META-015).
    renumbered: Vec<crate::numbering::Renumbered>,
}

#[derive(Debug)]
struct AnimeData {
    media: Media,
    russian: Option<RussianInfo>,
}

/// The fetched metadata of one work.
#[derive(Debug, Default)]
struct Fetched {
    tmdb: Option<TmdbData>,
    anime: Option<AnimeData>,
    /// Match confidence when a search decided; `None` when the work was
    /// read by a known id (the decision stays as it was).
    confidence: Option<f64>,
    explanation: serde_json::Value,
    /// The TMDB results a search weighed (META-016 inbox); empty when the
    /// work was read by a known id.
    candidates: Vec<ScoredResult>,
    /// What TVmaze, TheTVDB and Fanart.tv added (META-012).
    extras: extra::Extras,
}

#[derive(Debug)]
enum Outcome {
    Matched(Box<Fetched>),
    /// No provider result was good enough: the work goes to the manual
    /// queue with the results the search weighed. Dated episodes may still
    /// have their titles from TVmaze or TheTVDB (META-012).
    NotFound(Vec<ScoredResult>, Box<extra::Extras>),
    Failed(String),
}

/// A provider failure: transient (retry later) or fatal (stop the run).
enum Failure {
    Transient(String),
    Fatal(String),
}

fn tmdb_failure(e: TmdbError) -> Failure {
    match e {
        TmdbError::Unauthorized => Failure::Fatal("TMDB rejected the API key".into()),
        e => Failure::Transient(format!("TMDB: {e}")),
    }
}

fn fetch_failure(e: FetchError) -> Failure {
    Failure::Transient(e.to_string())
}

/// Works that need metadata, oldest first.
pub(crate) fn pending(
    conn: &Connection,
    language: &str,
    options: &MetadataOptions,
    now: i64,
) -> Result<Vec<i64>, Error> {
    let mut stmt = conn.prepare(concat!(
        "SELECT w.id FROM works w
         LEFT JOIN work_metadata wm ON wm.work_id = w.id
         -- UI-025: a work whose files are all personal media is not looked
         -- up (an item moved to a personal library).
         WHERE EXISTS (SELECT 1 FROM matches m JOIN files mf ON mf.id = m.file_id
                       WHERE m.work_id = w.id AND m.state <> 'rejected' AND NOT ",
        pa_library::file_personal!("mf"),
        ")
           -- META-016: «not a movie» — never asked again.
           AND NOT EXISTS (SELECT 1 FROM work_reviews wr WHERE wr.work_id = w.id)
           AND (wm.work_id IS NULL
                OR wm.language <> ?1
                OR wm.anime_pending = 1
                OR (wm.outcome = 'matched' AND wm.fetched_at_ms < ?2 - ?3)
                OR (wm.outcome = 'not_found' AND wm.fetched_at_ms < ?2 - ?4)
                OR (wm.outcome = 'failed' AND wm.fetched_at_ms < ?2 - ?5)
                OR (wm.outcome = 'matched' AND EXISTS (
                      SELECT 1 FROM matches m JOIN files f ON f.id = m.file_id
                      WHERE m.work_id = w.id AND m.state <> 'rejected'
                        AND f.added_at_ms > wm.fetched_at_ms))
                -- BUG-009: a work no provider knew under an older scorer is
                -- asked again now, not after the not-found retry period.
                -- Nothing the user decided is in it: the files are still
                -- named only by the parser.
                OR (wm.outcome = 'not_found' AND EXISTS (
                      SELECT 1 FROM matches m
                      WHERE m.work_id = w.id AND ifnull(m.model_version, '') <> ?6
                        AND ",
        not_found_matches!(),
        "))
                -- A newer scorer (META-015): automatic matches of an older
                -- model are decided again; user and rule decisions are not.
                OR EXISTS (
                      SELECT 1 FROM matches m
                      WHERE m.work_id = w.id AND m.decided_by = 'scorer'
                        AND ifnull(m.model_version, '') <> ?6 AND ",
        crate::numbering::recomputable!(),
        "))
         ORDER BY w.id"
    ))?;
    let rows = stmt.query_map(
        params![
            language,
            now,
            options.refresh_after_ms,
            options.retry_not_found_after_ms,
            options.retry_failed_after_ms,
            MODEL_VERSION
        ],
        |r| r.get(0),
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What a provider search of work `id` starts from (META-016 manual
/// search).
pub(crate) fn work_query(conn: &Connection, id: i64) -> Result<Option<WorkQuery>, Error> {
    Ok(load_work(conn, id)?.map(|w| w.query()))
}

fn load_work(conn: &Connection, id: i64) -> Result<Option<WorkInfo>, Error> {
    let Some((kind, title, year)) = conn
        .query_row(
            "SELECT kind, title, year FROM works WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut titles = vec![title];
    {
        let mut stmt = conn.prepare(
            "SELECT text FROM titles WHERE work_id = ?1
             ORDER BY kind = 'main' DESC, source = 'local' DESC, id",
        )?;
        for t in stmt.query_map([id], |r| r.get::<_, String>(0))? {
            let t = t?;
            if !titles.contains(&t) {
                titles.push(t);
            }
        }
    }
    let ext_str = |provider: &str, ext_kind: &str| -> Result<Option<String>, Error> {
        Ok(conn
            .query_row(
                "SELECT ext_id FROM external_ids
                 WHERE work_id = ?1 AND provider = ?2 AND ext_kind = ?3",
                params![id, provider, ext_kind],
                |r| r.get(0),
            )
            .optional()?)
    };
    let ext = |provider: &str, ext_kind: &str| -> Result<Option<u64>, Error> {
        Ok(ext_str(provider, ext_kind)?.and_then(|v| v.parse().ok()))
    };
    let tmdb_kind = if kind == "movie" { "movie" } else { "tv" };
    let tmdb_id = ext("tmdb", tmdb_kind)?;
    let imdb_id = ext_str("imdb", "")?;
    let tvdb_id = ext_str("tvdb", if kind == "movie" { "movie" } else { "series" })?;
    let anilist_id = ext("anilist", "")?;
    let mut seasons = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT season_number FROM episodes
             WHERE work_id = ?1 AND season_number IS NOT NULL AND episode_number IS NOT NULL
             ORDER BY 1",
        )?;
        for s in stmt.query_map([id], |r| r.get::<_, i64>(0))? {
            if let Ok(s) = u32::try_from(s?) {
                seasons.push(s);
            }
        }
    }
    let evidence = load_evidence(conn, id, kind == "movie")?;
    let mut files = Vec::new();
    if kind == "anime" {
        let mut stmt = conn.prepare(concat!(
            "SELECT DISTINCT f.id, f.path FROM matches m JOIN files f ON f.id = m.file_id
                 WHERE m.work_id = ?1 AND ",
            crate::numbering::recomputable!(),
            " AND EXISTS (SELECT 1 FROM match_episodes me WHERE me.match_id = m.id)
                 ORDER BY f.id"
        ))?;
        for row in stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))? {
            files.push(row?);
        }
    }
    let rescore: bool = conn.query_row(
        concat!(
            "SELECT EXISTS (SELECT 1 FROM matches m WHERE m.work_id = ?1 AND m.decided_by = 'scorer'
                              AND ifnull(m.model_version, '') <> ?2 AND ",
            crate::numbering::recomputable!(),
            ")
             AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.work_id = ?1 AND m.state <> 'rejected'
                              AND (m.decided_by IN ('user', 'nfo') OR NOT ",
            crate::numbering::recomputable!(),
            "))"
        ),
        params![id, MODEL_VERSION],
        |r| r.get(0),
    )?;
    let frames = anilist_id.is_some() && crate::frame_search::decided_by_frames(conn, id)?;
    let mut dates = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT air_date FROM episodes
             WHERE work_id = ?1 AND air_date IS NOT NULL AND episode_type = 'regular'
               AND season_number IS NULL AND episode_number IS NULL AND absolute_number IS NULL
             ORDER BY 1",
        )?;
        for d in stmt.query_map([id], |r| r.get::<_, String>(0))? {
            dates.push(d?);
        }
    }
    let mut hashes = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT f.oshash FROM matches m JOIN files f ON f.id = m.file_id
             WHERE m.work_id = ?1 AND m.state <> 'rejected' AND f.oshash IS NOT NULL
             ORDER BY f.size_bytes DESC, f.id LIMIT 3",
        )?;
        for h in stmt.query_map([id], |r| r.get::<_, String>(0))? {
            hashes.push(h?);
        }
    }
    Ok(Some(WorkInfo {
        id,
        kind,
        year,
        titles,
        tmdb_id,
        imdb_id,
        tvdb_id,
        anilist_id,
        seasons,
        evidence,
        files,
        rescore,
        frames,
        dates,
        hashes,
    }))
}

/// What the files of a work say (META-014 pair features): written
/// episodes, durations (a movie: all its files, the longest one counts; a
/// series: the files with episodes) and audio languages of its live matches.
fn load_evidence(conn: &Connection, work: i64, movie: bool) -> Result<WorkEvidence, Error> {
    let mut ev = WorkEvidence::default();
    let mut stmt = conn.prepare(
        "SELECT e.season_number, e.episode_number FROM matches m
         JOIN match_episodes me ON me.match_id = m.id
         JOIN episodes e ON e.id = me.episode_id
         WHERE m.work_id = ?1 AND m.state <> 'rejected'
           AND e.season_number IS NOT NULL AND e.episode_number IS NOT NULL
         ORDER BY 1, 2",
    )?;
    for row in stmt.query_map([work], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (s, e) = row?;
        if let (Ok(s), Ok(e)) = (u32::try_from(s), u32::try_from(e)) {
            ev.episodes.push((s, e));
        }
    }
    let mut stmt = conn.prepare(
        "SELECT f.duration_ms, f.audio_langs,
                EXISTS (SELECT 1 FROM match_episodes me WHERE me.match_id = m.id)
         FROM matches m JOIN files f ON f.id = m.file_id
         WHERE m.work_id = ?1 AND m.state <> 'rejected'
         ORDER BY f.id",
    )?;
    let rows = stmt.query_map([work], |r| {
        Ok((
            r.get::<_, Option<i64>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })?;
    for row in rows {
        let (duration, langs, episodic) = row?;
        if let Some(d) = duration.and_then(|d| u64::try_from(d).ok())
            && (movie || episodic)
        {
            ev.durations_ms.push(d);
        }
        let langs: Vec<String> = langs
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        for l in langs {
            if !ev.audio_langs.contains(&l) {
                ev.audio_langs.push(l);
            }
        }
    }
    Ok(ev)
}

/// The query of a work for the title index.
pub(crate) fn title_query(work: &WorkQuery) -> TitleQuery {
    TitleQuery {
        titles: work
            .titles
            .iter()
            .enumerate()
            .map(|(i, t)| (t.clone(), if i == 0 { 1.0 } else { 0.9 }))
            .collect(),
        year: work.year.and_then(|y| u16::try_from(y).ok()),
        season: None,
        media: if work.kind == "movie" {
            MediaHint::Movie
        } else {
            MediaHint::Episode
        },
        anime_hint: work.kind == "anime",
    }
}

/// Candidates compared by [`rank`].
const RANKED: usize = 5;

/// Two results this close are a tie the title cannot break.
const TIE: f64 = 0.01;

/// A lead that comes from the pair features alone (duration, episode
/// range, language) breaks a title tie only from this size on: the audio
/// language alone (a weak hint) does not.
const PAIR_TIE: f64 = 0.03;

/// Results whose details are fetched to break a tie (META-014).
const DETAILED: usize = 3;

/// Confidence factor of a tie between different works (a remake or a
/// same-named work without a year): below [`AUTO`], so the match is
/// `auto_review`.
const AMBIGUOUS: f64 = 0.85;

/// Highest title score of a result whose title lacks the type marker of
/// the work's name ([`lost_marker`]): below [`AUTO`], so the match is
/// `auto_review`.
const MARKER_CAP: f64 = 0.85;

/// Words of a name that say the files are not the series itself.
const TYPE_MARKERS: &[&str] = &[
    "ova",
    "oad",
    "ona",
    "special",
    "specials",
    "sp",
    "movie",
    "film",
    "gekijouban",
    "recap",
];

/// The type marker (`ova`, `movie`…) of the work's main title that the
/// candidate's `matched` title does not have, if any (META-016).
fn lost_marker(work: &WorkQuery, matched: &str) -> Option<&'static str> {
    let words = |s: &str| -> Vec<String> {
        pa_recognition::title::normalize::fold(s)
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    };
    let title = words(work.titles.first()?);
    let matched = words(matched);
    TYPE_MARKERS
        .iter()
        .copied()
        .find(|m| title.iter().any(|w| w == m) && !matched.iter().any(|w| w == m))
}

/// The ranking of a work's search results.
#[derive(Debug, Clone)]
struct Ranked {
    /// Position of the best record.
    best: usize,
    /// Signals of `order` (META-016 explanation), same order.
    signals: Vec<MatchSignals>,
    /// Its score (lowered by [`AMBIGUOUS`] on a tie).
    score: f64,
    /// Title tier of the best record.
    tier: String,
    /// Two works tie.
    tied: bool,
    /// Positions of the ranked records, best first.
    order: Vec<usize>,
    /// Scores of `order` (with the pair features, before [`AMBIGUOUS`]).
    scores: Vec<f64>,
}

/// Largest gap between the file's year and the year of a movie with
/// exactly its title that is still the same movie when no other work has
/// that title (BUG-023): premiere against wide or local release.
const YEAR_SLACK: u64 = 2;

/// The title tier names the work's title itself: the same words up to
/// punctuation, case, articles or spacing.
fn same_title(tier: MatchTier) -> bool {
    matches!(
        tier,
        MatchTier::Exact | MatchTier::Loose | MatchTier::Compact
    )
}

/// Ranks `records` for `work`: title tiers, year ±1 and kind from the local
/// title index, plus the META-014 pair features of the work's files against
/// each record (duration against runtime, written episodes against the
/// season list, audio language; [`pair::relative`]). Ties keep the
/// provider's order, but a tie between two works lowers the score below
/// [`AUTO`].
fn rank_all(work: &WorkQuery, records: &[TitleRecord]) -> Option<Ranked> {
    if records.is_empty() {
        return None;
    }
    let index = TitleIndex::from_records(records.to_vec());
    let candidates = index.search(&title_query(work), RANKED);
    let evidence = work.evidence.pair(work.kind == "movie");
    let pairs: Vec<(usize, &TitleRecord)> = candidates
        .iter()
        .map(|c| (c.record, index.record(c.record)))
        .collect();
    let relative = pair::relative(&evidence, &pairs);
    // Records whose title is the work's title itself (not a head, a
    // fuzzy or a partial match): more than one is a remake or a
    // same-named work (BUG-023).
    let same_titled = candidates.iter().filter(|c| same_title(c.tier)).count();
    // (candidate, title score, score with pair features)
    let scored: Vec<_> = candidates
        .into_iter()
        .map(|c| {
            let mut score = f64::from(c.score);
            // A movie of another year is another movie (remakes). The
            // only work with exactly the file's title, a year or two off,
            // is the same movie dated by another release (`Plan 9 from
            // Outer Space (1959)`: premiere 1957, wide release 1959;
            // BUG-023): the title index's year penalty is enough.
            let record = index.record(c.record);
            if work.kind == "movie"
                && let (Some(a), Some(b)) = (work.year, record.year)
                && a.abs_diff(i64::from(b)) > 1
                && !(a.abs_diff(i64::from(b)) <= YEAR_SLACK
                    && same_title(c.tier)
                    && same_titled == 1)
            {
                score *= 0.8;
            }
            // `Fairy Tail OVA` is not the series `Fairy Tail` for sure
            // (META-016): an OVA, a special or a film of a franchise that
            // TMDB lists inside the series, or not at all.
            if lost_marker(work, record.titles.get(c.title).map_or("", String::as_str)).is_some() {
                score = score.min(MARKER_CAP);
            }
            let with_pair = score + f64::from(relative.get(&c.record).copied().unwrap_or(0.0));
            (c, score, with_pair)
        })
        .collect();
    let mut best = 0;
    for (i, (_, _, score)) in scored.iter().enumerate() {
        if *score > scored[best].2 {
            best = i;
        }
    }
    let (candidate, title_score, mut score) = scored.get(best).cloned()?;
    let tied = scored.iter().enumerate().any(|(i, (_, t, s))| {
        i != best && (*s >= score - TIE || (*t >= title_score - TIE && *s >= score - PAIR_TIE))
    });
    if tied {
        score *= AMBIGUOUS;
    }
    let mut order: Vec<usize> = (0..scored.len()).collect();
    order.sort_by(|&a, &b| scored[b].2.total_cmp(&scored[a].2).then(a.cmp(&b)));
    let signals = order
        .iter()
        .map(|&i| {
            let (c, title_score, _) = &scored[i];
            let record = index.record(c.record);
            let f = pair::features(&evidence, record);
            MatchSignals {
                provider: "tmdb".to_owned(),
                title: round3(*title_score),
                tier: format!("{:?}", c.tier).to_lowercase(),
                matched_title: record.titles.get(c.title).cloned(),
                file_year: work.year,
                year: record.year.map(i64::from),
                file_kind: work.kind.clone(),
                kind: if record.kind == WorkKind::Movie {
                    "movie"
                } else {
                    "tv"
                }
                .to_owned(),
                duration_error: f.duration_error.map(|e| round3(f64::from(e))),
                episodes_ok: f.episodes_ok,
                episodes_bad: f.episodes_bad,
                language: f.language,
                tied: tied && scored[i].2 >= scored[best].2 - PAIR_TIE,
                lost_marker: lost_marker(
                    work,
                    record.titles.get(c.title).map_or("", String::as_str),
                )
                .map(str::to_owned),
                query: None,
            }
        })
        .collect();
    Some(Ranked {
        best: candidate.record,
        signals,
        score,
        tier: format!("{:?}", candidate.tier).to_lowercase(),
        tied,
        scores: order.iter().map(|&i| scored[i].2).collect(),
        order: order.into_iter().map(|i| scored[i].0.record).collect(),
    })
}

/// Index of the best record for `work` and its score, if any reaches
/// [`ACCEPT`] ([`rank_all`]).
fn rank(work: &WorkQuery, records: Vec<TitleRecord>) -> Option<(usize, f64, String)> {
    rank_all(work, &records)
        .filter(|r| r.score >= ACCEPT)
        .map(|r| (r.best, r.score, r.tier))
}

/// An anime work recognised by the local anime-offline-database
/// ([`local_anime`], META-030).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalAnime {
    pub anidb: Option<u32>,
    pub anilist: Option<u32>,
    pub mal: Option<u32>,
    /// Main title of the entry.
    pub title: String,
    /// Title-index score of the entry.
    pub score: f64,
    /// TMDB TV series of the entry from the Anime-Lists list, if any.
    pub tmdb_tv: Option<u32>,
}

/// Two local entries this close are a tie: nothing is decided locally.
const LOCAL_TIE: f64 = 0.03;

/// The anime-offline-database entry of an anime work, by its titles and
/// year alone (no request): the best entry of the title index if it scores
/// at least [`AUTO`] and no other entry comes within [`LOCAL_TIE`]. With
/// the Anime-Lists list (`mappings`) the entry's AniDB id also gives the
/// TMDB series — a season named as its own title (`Bleach - Sennen
/// Kessen-hen`, `Kimetsu no Yaiba - Mugen Ressha-hen`) that the TMDB search
/// does not find under that name.
#[must_use]
pub fn local_anime(
    work: &WorkQuery,
    titles: &crate::AnimeTitles,
    mappings: Option<&pa_anime_mappings::MappingIndex>,
) -> Option<LocalAnime> {
    if work.kind != "anime" || titles.is_empty() {
        return None;
    }
    let found = titles.index().search(&title_query(work), RANKED);
    let best = found
        .iter()
        .max_by(|a, b| a.score.total_cmp(&b.score).then(b.record.cmp(&a.record)))?;
    let score = f64::from(best.score);
    if score < AUTO {
        return None;
    }
    let tied: Vec<usize> = found
        .iter()
        .filter(|c| f64::from(c.score) >= score - LOCAL_TIE)
        .map(|c| c.record)
        .collect();
    let chosen = match tied.as_slice() {
        [one] => *one,
        // A TV series and its film of the same name (`Kimetsu no Yaiba:
        // Mugen Ressha-hen`): the durations of the files tell them apart;
        // without durations nothing is decided.
        _ => {
            let mut d = work.evidence.durations_ms.clone();
            if d.is_empty() {
                return None;
            }
            d.sort_unstable();
            let movie = d[d.len() / 2] > 60 * 60 * 1000;
            let fits: Vec<usize> = tied
                .iter()
                .copied()
                .filter(|&r| (titles.index().record(r).kind == WorkKind::Movie) == movie)
                .collect();
            match fits.as_slice() {
                [one] => *one,
                _ => return None,
            }
        }
    };
    let score = found
        .iter()
        .find(|c| c.record == chosen)
        .map_or(score, |c| f64::from(c.score));
    if score < AUTO {
        return None;
    }
    let entry = titles.entry_at(chosen)?;
    let tmdb_tv = entry.ids.anidb.and_then(|a| {
        mappings?
            .find(pa_anime_mappings::IdRef::AniDb(a))?
            .anime_list
            .as_ref()?
            .tmdb_tv
    });
    Some(LocalAnime {
        anidb: entry.ids.anidb,
        anilist: entry.ids.anilist,
        mal: entry.ids.mal,
        title: entry.title.clone(),
        score,
        tmdb_tv,
    })
}

fn tmdb_kind(work: &WorkQuery) -> MediaKind {
    if work.kind == "movie" {
        MediaKind::Movie
    } else {
        MediaKind::Tv
    }
}

/// A TMDB search: `(kind, title, year)` → results.
pub type SearchFn<'a, E> =
    dyn FnMut(MediaKind, &str, Option<u32>) -> Result<Vec<SearchResult>, E> + 'a;

/// Titles of a TMDB result in other languages for the ranking: `(kind,
/// id)` → titles (translations and alternative titles; empty: unknown).
pub type TitlesFn<'a, E> = dyn FnMut(MediaKind, u64) -> Result<Vec<String>, E> + 'a;

/// Results of a search page (TMDB's order) whose titles in other languages
/// are fetched when nothing ranks [`AUTO`] (META-033).
const TITLED: usize = 3;

/// TMDB details of a result for the ranking: `(kind, id)` → facts
/// (`None`: unknown).
pub type DetailsFn<'a, E> = dyn FnMut(MediaKind, u64) -> Result<Option<ResultFacts>, E> + 'a;

/// Searches a work by its titles and year and ranks the results with the
/// local title index: per title (at most [`MAX_QUERIES`]), `search(kind,
/// title, year)` with the year and, if that finds nothing, without it; the
/// best result scoring at least [`ACCEPT`] wins, a score of [`AUTO`] stops
/// the search. `search` is TMDB `/search/movie` or `/search/tv`: live in a
/// metadata run, recorded or synthetic in `pa-eval`. Without details
/// ([`match_search_with`]) the pair features see only what search results
/// carry (the original language).
pub fn match_search<E>(
    work: &WorkQuery,
    search: &mut SearchFn<'_, E>,
) -> Result<Option<SearchMatch>, E> {
    match_search_with(work, search, &mut |_, _| Ok(None))
}

/// [`match_search`] that breaks ties with details (META-014): when two
/// works tie and the work's files say something ([`WorkEvidence`]), the
/// details of the best [`DETAILED`] results (runtime, seasons) join the
/// ranking — `Dune.mkv` of 155 minutes is the 2021 film, `Shameless
/// 9x14` the series with a 14-episode ninth season. Details are requested
/// only on such ties; a metadata run reads the chosen work's details anyway.
pub fn match_search_with<E>(
    work: &WorkQuery,
    search: &mut SearchFn<'_, E>,
    details: &mut DetailsFn<'_, E>,
) -> Result<Option<SearchMatch>, E> {
    match_search_titled(work, search, details, &mut |_, _| Ok(Vec::new()))
}

/// [`match_search_with`] that also compares the work's titles with the
/// titles TMDB knows in other languages (META-033). Search results come in
/// the metadata language (`ru-RU`): a result carries only its Russian and
/// original titles, so `12 Monkeys` (`12 обезьян`, `Twelve Monkeys`) or
/// `Youjo Senki` (`Военная хроника маленькой девочки`, `幼女戦記`) rank
/// low. When nothing ranks [`AUTO`], the first [`TITLED`] results of the
/// page get their translations and alternative titles (`titles`: two
/// cached requests each, the winner's translations are read anyway) and
/// the page is ranked again. When still nothing reaches [`ACCEPT`], the
/// best untied result from [`REVIEW`] on is returned (`auto_review`).
pub fn match_search_titled<E>(
    work: &WorkQuery,
    search: &mut SearchFn<'_, E>,
    details: &mut DetailsFn<'_, E>,
    titles: &mut TitlesFn<'_, E>,
) -> Result<Option<SearchMatch>, E> {
    Ok(search_ranked(work, search, details, titles)?.chosen)
}

/// [`match_search_titled`] with the results it weighed: the best
/// [`CANDIDATES`] of the final ranking with their scores and signals, for
/// the review inbox (META-016) — also when nothing is chosen.
pub fn search_ranked<E>(
    work: &WorkQuery,
    search: &mut SearchFn<'_, E>,
    details: &mut DetailsFn<'_, E>,
    titles: &mut TitlesFn<'_, E>,
) -> Result<SearchDecision, E> {
    let kind = tmdb_kind(work);
    let year = work.year.and_then(|y| u32::try_from(y).ok());
    let mut results: Vec<SearchResult> = Vec::new();
    let mut seen = HashSet::new();
    let mut facts: HashMap<u64, Option<ResultFacts>> = HashMap::new();
    let mut other_titles: HashMap<u64, Vec<String>> = HashMap::new();
    let mut best = None;
    let mut fallback: Option<SearchMatch> = None;
    // The last ranking (every result seen so far) and its query.
    let mut last: Option<(Ranked, String)> = None;
    // The best ranking seen (score, query, top candidates) for the log.
    let mut seen_best: Option<(f64, String, String)> = None;
    for query in work.titles.iter().take(MAX_QUERIES) {
        let mut years = vec![year];
        if year.is_some() {
            years.push(None);
        }
        let mut page_ids = Vec::new();
        for y in years {
            let page = search(kind, query, y)?;
            let found = !page.is_empty();
            page_ids = page.iter().map(|r| r.id).collect();
            for r in page {
                if seen.insert(r.id) {
                    results.push(r);
                }
            }
            if found {
                break;
            }
        }
        let mut records: Vec<TitleRecord> = results
            .iter()
            .map(|r| {
                tmdb::titles::title_record(r, None, None).unwrap_or_else(|| TitleRecord {
                    titles: vec![r.title.clone().unwrap_or_default()],
                    ..Default::default()
                })
            })
            .collect();
        let apply = |records: &mut [TitleRecord],
                     facts: &HashMap<u64, Option<ResultFacts>>,
                     other: &HashMap<u64, Vec<String>>| {
            for (r, rec) in results.iter().zip(records.iter_mut()) {
                if let Some(Some(f)) = facts.get(&r.id) {
                    f.apply(rec);
                }
                for t in other.get(&r.id).into_iter().flatten() {
                    if !rec.titles.contains(t) {
                        rec.titles.push(t.clone());
                    }
                }
            }
        };
        apply(&mut records, &facts, &other_titles);
        let mut ranked = rank_all(work, &records);
        if ranked.as_ref().is_none_or(|r| r.score < AUTO) {
            let mut asked = false;
            for &id in page_ids.iter().take(TITLED) {
                if let std::collections::hash_map::Entry::Vacant(e) = other_titles.entry(id) {
                    e.insert(with_heads(titles(kind, id)?));
                    asked = true;
                }
            }
            if asked {
                apply(&mut records, &facts, &other_titles);
                ranked = rank_all(work, &records);
            }
        }
        if let Some(r) = &ranked
            && r.tied
            && !work.evidence.is_empty()
        {
            let mut asked = false;
            for &i in r.order.iter().take(DETAILED) {
                let id = results[i].id;
                if let std::collections::hash_map::Entry::Vacant(e) = facts.entry(id) {
                    e.insert(details(kind, id)?);
                    asked = true;
                }
            }
            if asked {
                apply(&mut records, &facts, &other_titles);
                ranked = rank_all(work, &records);
            }
        }
        if let Some(r) = &ranked
            && seen_best.as_ref().is_none_or(|(s, _, _)| *s < r.score)
        {
            let top: Vec<String> = r
                .order
                .iter()
                .zip(&r.scores)
                .take(3)
                .map(|(&i, score)| {
                    let x = &results[i];
                    format!(
                        "#{} {:?}/{:?} ({}) {score:.3}",
                        x.id,
                        x.title.as_deref().unwrap_or_default(),
                        x.original_title.as_deref().unwrap_or_default(),
                        x.year().map_or_else(|| "-".to_owned(), |y| y.to_string()),
                    )
                })
                .collect();
            seen_best = Some((r.score, query.clone(), top.join("; ")));
        }
        if let Some(r) = ranked
            .as_ref()
            .filter(|r| (REVIEW..ACCEPT).contains(&r.score) && !r.tied)
            && fallback.as_ref().is_none_or(|f| f.score < r.score)
        {
            fallback = Some(SearchMatch {
                result: results[r.best].clone(),
                score: r.score,
                tier: r.tier.clone(),
                query: query.clone(),
                signals: r.signals.first().cloned().unwrap_or_default(),
            });
        }
        if let Some(r) = &ranked {
            last = Some((r.clone(), query.clone()));
        }
        if let Some(r) = ranked.filter(|r| r.score >= ACCEPT) {
            best = Some(SearchMatch {
                result: results[r.best].clone(),
                score: r.score,
                tier: r.tier,
                query: query.clone(),
                signals: r.signals.first().cloned().unwrap_or_default(),
            });
            if r.score >= AUTO {
                break;
            }
        }
    }
    for m in [&mut best, &mut fallback].into_iter().flatten() {
        m.signals.query = Some(m.query.clone());
    }
    let candidates: Vec<ScoredResult> = last
        .map(|(r, query)| {
            r.order
                .iter()
                .zip(&r.scores)
                .zip(r.signals)
                .take(CANDIDATES)
                .map(|((&i, &score), mut signals)| {
                    signals.query = Some(query.clone());
                    ScoredResult {
                        result: results[i].clone(),
                        score: round3(score.clamp(0.0, 1.0)),
                        signals,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let decision = match (&best, &fallback) {
        (Some(m), _) if m.score >= AUTO => "auto".to_owned(),
        (Some(_), _) => "auto_review".to_owned(),
        (None, Some(_)) => format!("auto_review (below {ACCEPT}, the best untied result)"),
        (None, None) if results.is_empty() => "rejected: no results".to_owned(),
        (None, None) => format!("rejected: no untied result scores {REVIEW}"),
    };
    let chosen = best.as_ref().or(fallback.as_ref());
    let (best_score, best_query, top) = seen_best.unwrap_or_default();
    tracing::info!(
        target: "pa_catalog::metadata",
        kind = %work.kind,
        title = ?work.titles.first(),
        year = ?work.year,
        provider = "tmdb",
        query = %chosen.map_or(best_query.as_str(), |m| m.query.as_str()),
        score = chosen.map_or(best_score, |m| m.score),
        candidates = %top,
        alt_titles_for = other_titles.len(),
        "search decision: {decision}"
    );
    Ok(SearchDecision {
        chosen: best.or(fallback),
        candidates,
    })
}

/// TMDB search for a work ([`match_search_with`] on the client).
fn tmdb_search(client: &TmdbClient, work: &WorkInfo) -> Result<SearchDecision, Failure> {
    search_ranked(
        &work.query(),
        &mut |kind, query, year| {
            match kind {
                MediaKind::Movie => client.search_movie(query, year),
                MediaKind::Tv => client.search_tv(query, year),
            }
            .map(|page| page.results)
            .map_err(tmdb_failure)
        },
        &mut |kind, id| match client.details(kind, id) {
            Ok(d) => Ok(Some(ResultFacts::of_details(&d))),
            Err(TmdbError::NotFound(_)) => Ok(None),
            Err(e) => Err(tmdb_failure(e)),
        },
        &mut |kind, id| tmdb_other_titles(client, kind, id),
    )
}

/// [`tmdb_search`] whose weighed results go to `candidates`.
fn search_into(
    client: &TmdbClient,
    work: &WorkInfo,
    candidates: &mut Vec<ScoredResult>,
) -> Result<Option<SearchMatch>, Failure> {
    let decision = tmdb_search(client, work)?;
    *candidates = decision.candidates;
    Ok(decision.chosen)
}

/// `titles` plus the head of each one that has a subtitle after a dash
/// (`Steins;Gate: The Movie − Load Region of Déjà Vu` →
/// `Steins;Gate: The Movie`): files are often named by the head alone.
fn with_heads(titles: Vec<String>) -> Vec<String> {
    let mut out = titles.clone();
    for t in &titles {
        for sep in [" − ", " - ", " – ", " — "] {
            if let Some((head, _)) = t.split_once(sep) {
                let head = head.trim();
                if head.chars().filter(|c| c.is_alphanumeric()).count() >= 4
                    && !out.iter().any(|x| x == head)
                {
                    out.push(head.to_owned());
                }
            }
        }
    }
    out
}

/// Titles of a TMDB work in every language TMDB knows: translations and
/// alternative titles (META-033); a missing list is no titles.
fn tmdb_other_titles(
    client: &TmdbClient,
    kind: MediaKind,
    id: u64,
) -> Result<Vec<String>, Failure> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        let t = t.trim();
        if !t.is_empty() && !out.iter().any(|x| x == t) {
            out.push(t.to_owned());
        }
    };
    match client.translations(kind, id) {
        Ok(tr) => tr
            .translations
            .iter()
            .filter_map(|t| t.data.title.as_deref())
            .for_each(&mut push),
        Err(TmdbError::NotFound(_)) => {}
        Err(e) => return Err(tmdb_failure(e)),
    }
    match client.alternative_titles(kind, id) {
        Ok(alt) => alt.titles.iter().for_each(|t| push(&t.title)),
        Err(TmdbError::NotFound(_)) => {}
        Err(e) => return Err(tmdb_failure(e)),
    }
    Ok(out)
}

/// The TMDB id of a work that has only an IMDb or TVDB id (an NFO file).
fn tmdb_find(client: &TmdbClient, work: &WorkInfo) -> Result<Option<u64>, Failure> {
    let kind = tmdb_kind(&work.query());
    for (id, source) in [
        (&work.imdb_id, ExternalSource::Imdb),
        (&work.tvdb_id, ExternalSource::Tvdb),
    ] {
        let Some(id) = id else { continue };
        let found = match client.find(id, source) {
            Ok(f) => f,
            Err(TmdbError::NotFound(_)) => continue,
            Err(e) => return Err(tmdb_failure(e)),
        };
        let results = match kind {
            MediaKind::Movie => found.movie_results,
            MediaKind::Tv => found.tv_results,
        };
        if let Some(r) = results.first() {
            return Ok(Some(r.id));
        }
    }
    Ok(None)
}

fn fetch_tmdb(
    client: &TmdbClient,
    work: &WorkInfo,
    mappings: Option<&pa_anime_mappings::MappingIndex>,
    local: Option<&LocalAnime>,
    anime: Option<&crate::AnimeTitles>,
    explanation: &mut serde_json::Value,
    candidates: &mut Vec<ScoredResult>,
) -> Result<Option<(TmdbData, Option<f64>)>, Failure> {
    let kind = tmdb_kind(&work.query());
    let known = work.tmdb_id.filter(|_| !work.rescore);
    let found = match known {
        Some(id) => Some(id),
        None if work.rescore => None,
        None => tmdb_find(client, work)?,
    };
    let (id, confidence) = match found {
        // Known ids keep the decision as it is (user, NFO).
        Some(id) => (id, None),
        // Recognised by its frames (META-031): the names TMDB did not know
        // are not asked again; AniList is read by the id.
        None if work.frames => return Ok(None),
        None => match search_into(client, work, candidates)? {
            Some(m) => {
                explanation["tmdb"] = json!({
                    "id": m.result.id,
                    "kind": kind.as_str(),
                    "query": m.query,
                    "score": (m.score * 1000.0).round() / 1000.0,
                    "tier": m.tier,
                    "year": m.result.year(),
                });
                explanation["signals"] = serde_json::to_value(&m.signals).unwrap_or_default();
                (m.result.id, Some(m.score))
            }
            // TMDB does not know the title; the local database knows the
            // AniDB entry and the Anime-Lists list its TMDB series
            // (META-030).
            None => match local
                .filter(|_| kind == MediaKind::Tv)
                .and_then(|l| l.tmdb_tv.map(|t| (l, t)))
            {
                Some((l, tmdb)) => {
                    explanation["tmdb"] = json!({
                        "id": tmdb,
                        "kind": kind.as_str(),
                        "via": "anime_offline_database",
                        "anidb": l.anidb,
                        "score": (l.score * 1000.0).round() / 1000.0,
                    });
                    explanation["signals"] = serde_json::to_value(MatchSignals {
                        provider: "anime_offline_database".to_owned(),
                        title: round3(l.score),
                        matched_title: Some(l.title.clone()),
                        file_year: work.year,
                        file_kind: work.kind.clone(),
                        kind: "tv".to_owned(),
                        ..Default::default()
                    })
                    .unwrap_or_default();
                    (u64::from(tmdb), Some(l.score))
                }
                None => return Ok(None),
            },
        },
    };
    let details = match client.details(kind, id) {
        Ok(d) => d,
        Err(TmdbError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(tmdb_failure(e)),
    };
    let mut titles = Vec::new();
    match client.translations(kind, id) {
        Ok(tr) => {
            for lang in ["en", "ru"] {
                if let Some(t) = tr.find(lang).and_then(|t| t.data.title.clone()) {
                    titles.push((lang.to_owned(), t));
                }
            }
        }
        Err(TmdbError::NotFound(_)) => {}
        Err(e) => return Err(tmdb_failure(e)),
    }
    // Anime files written as running counts or entry titles, in TMDB order.
    let renumbered = if kind == MediaKind::Tv && work.kind == "anime" && !work.files.is_empty() {
        crate::numbering::series_of_details_with(&details, mappings, anime)
            .map(|s| {
                crate::numbering::renumber_with(
                    &s,
                    &work.files,
                    &work.titles,
                    local.and_then(|l| l.anidb),
                )
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut wanted = work.seasons.clone();
    wanted.extend(crate::numbering::seasons(&renumbered));
    // Daily shows: the seasons that aired the dated episodes (META-012).
    if kind == MediaKind::Tv {
        wanted.extend(extra::seasons_for_dates(&details, &work.dates));
    }
    wanted.sort_unstable();
    wanted.dedup();
    let mut seasons = Vec::new();
    if kind == MediaKind::Tv {
        for &s in &wanted {
            match client.season(id, s) {
                Ok(season) => seasons.push(season),
                Err(TmdbError::NotFound(_)) => {}
                Err(e) => return Err(tmdb_failure(e)),
            }
        }
    }
    let credits = client.credits(kind, id).unwrap_or_default();
    let certifications = client.certifications(kind, id).unwrap_or_default();
    Ok(Some((
        TmdbData {
            kind,
            details,
            titles,
            seasons,
            credits,
            certifications,
            renumbered,
        },
        confidence,
    )))
}

fn anilist_record(m: &Media) -> TitleRecord {
    TitleRecord {
        kind: if m.is_movie() {
            WorkKind::Movie
        } else {
            WorkKind::Series
        },
        anime: true,
        year: m.year().and_then(|y| u16::try_from(y).ok()),
        titles: m.all_titles().into_iter().map(str::to_owned).collect(),
        ..Default::default()
    }
}

fn fetch_anime(
    anilist: &AniListClient,
    shikimori: Option<&ShikimoriClient>,
    work: &WorkInfo,
    local: Option<&LocalAnime>,
    explanation: &mut serde_json::Value,
) -> Result<Option<(AnimeData, Option<f64>)>, Failure> {
    // The local database names the AniList entry: read it by id instead of
    // searching (META-030).
    let local = local.and_then(|l| l.anilist.map(|id| (id, l.score)));
    let (media, confidence) = match (work.anilist_id, local) {
        (Some(id), _) => match anilist.media(id) {
            Ok(m) => (m, None),
            Err(FetchError::NotFound { .. }) => return Ok(None),
            Err(e) => return Err(fetch_failure(e)),
        },
        (None, Some((id, score))) => match anilist.media(u64::from(id)) {
            Ok(m) => {
                explanation["anilist"] = json!({
                    "id": m.id,
                    "via": "anime_offline_database",
                    "score": (score * 1000.0).round() / 1000.0,
                    "year": m.year(),
                });
                (m, Some(score))
            }
            Err(FetchError::NotFound { .. }) => return Ok(None),
            Err(e) => return Err(fetch_failure(e)),
        },
        (None, None) => {
            let mut found: Vec<Media> = Vec::new();
            let mut best = None;
            for query in work.titles.iter().take(MAX_QUERIES) {
                for m in anilist.search(query).map_err(fetch_failure)? {
                    if !found.iter().any(|f| f.id == m.id) {
                        found.push(m);
                    }
                }
                if let Some((i, score, tier)) =
                    rank(&work.query(), found.iter().map(anilist_record).collect())
                {
                    best = Some((i, score, tier, query.clone()));
                    if score >= AUTO {
                        break;
                    }
                }
            }
            let Some((i, score, tier, query)) = best else {
                return Ok(None);
            };
            let media = found.swap_remove(i);
            explanation["anilist"] = json!({
                "id": media.id,
                "query": query,
                "score": (score * 1000.0).round() / 1000.0,
                "tier": tier,
                "year": media.year(),
            });
            (media, Some(score))
        }
    };
    let russian = match (shikimori, media.id_mal) {
        (Some(s), Some(mal)) => match s.russian(mal) {
            Ok(r) => Some(r),
            Err(FetchError::NotFound { .. }) => None,
            Err(e) => return Err(fetch_failure(e)),
        },
        _ => None,
    };
    Ok(Some((AnimeData { media, russian }, confidence)))
}

/// A Japanese animated series or film on TMDB (genre 16 `Animation`).
fn is_anime(d: &Details) -> bool {
    d.original_language.as_deref() == Some("ja") && d.genres.iter().any(|g| g.id == 16)
}

/// The titles stage of `work`: the local anime database and TMDB.
struct TitlesPart {
    fetched: Fetched,
    /// The local anime database's entry, for the anime stage.
    local: Option<LocalAnime>,
    /// A transient TMDB failure.
    error: Option<String>,
    /// The run must stop (rejected key).
    fatal: Option<String>,
}

fn fetch_titles(providers: &MetadataProviders, work: &WorkInfo) -> TitlesPart {
    let mut part = TitlesPart {
        fetched: Fetched {
            explanation: json!({}),
            ..Default::default()
        },
        local: None,
        error: None,
        fatal: None,
    };
    // Works someone decided (user fix, NFO ids) keep that decision.
    let undecided = (work.tmdb_id.is_none() || work.rescore) && work.anilist_id.is_none();
    // A show folder may be anime named in Russian (`Гильгамеш/`): the local
    // database is asked as for anime; its answer counts only if TMDB does
    // not know a live-action work of that name (META-032).
    let anime_query = {
        let mut q = work.query();
        if q.kind == "show" {
            q.kind = "anime".to_owned();
        }
        q
    };
    part.local = providers
        .anime_titles
        .as_deref()
        .filter(|_| undecided)
        .and_then(|t| local_anime(&anime_query, t, providers.anime_mappings.as_deref()));
    if let Some(l) = &part.local {
        part.fetched.explanation["local"] = json!({
            "anidb": l.anidb,
            "title": l.title,
            "score": (l.score * 1000.0).round() / 1000.0,
        });
    }
    if let Some(client) = providers.tmdb() {
        match fetch_tmdb(
            client,
            work,
            providers.anime_mappings.as_deref(),
            part.local.as_ref(),
            providers.anime_titles.as_deref(),
            &mut part.fetched.explanation,
            &mut part.fetched.candidates,
        ) {
            Ok(Some((data, confidence))) => {
                part.fetched.tmdb = Some(data);
                part.fetched.confidence = confidence;
            }
            // TMDB does not know the names: perhaps OpenSubtitles knows the
            // file by its hash (META-012).
            Ok(None) => {
                if let Some(os) = providers.extra.opensubtitles.as_deref()
                    && undecided
                    && !work.frames
                    && part.local.is_none()
                    && matches!(work.kind.as_str(), "movie" | "show")
                    && let Some((id, confidence, explain)) = extra::hash_match(os, work)
                {
                    let mut by_id = work.clone();
                    by_id.tmdb_id = Some(id);
                    by_id.rescore = false;
                    let mut ignored = json!({});
                    match fetch_tmdb(
                        client,
                        &by_id,
                        providers.anime_mappings.as_deref(),
                        None,
                        providers.anime_titles.as_deref(),
                        &mut ignored,
                        &mut Vec::new(),
                    ) {
                        Ok(Some((data, _))) => {
                            let ex = &mut part.fetched.explanation;
                            ex["tmdb"] = json!({
                                "id": id,
                                "kind": data.kind.as_str(),
                                "via": "opensubtitles",
                                "score": confidence,
                            });
                            ex["opensubtitles"] = explain;
                            ex["signals"] = serde_json::to_value(MatchSignals {
                                provider: "opensubtitles".to_owned(),
                                title: confidence,
                                tier: "hash".to_owned(),
                                matched_title: data.details.title.clone(),
                                file_year: work.year,
                                year: data.details.year().map(i64::from),
                                file_kind: work.kind.clone(),
                                kind: data.kind.as_str().to_owned(),
                                ..Default::default()
                            })
                            .unwrap_or_default();
                            part.fetched.tmdb = Some(data);
                            part.fetched.confidence = Some(confidence);
                        }
                        Ok(None) => {}
                        Err(Failure::Fatal(e)) => part.fatal = Some(e),
                        Err(Failure::Transient(e)) => part.error = Some(e),
                    }
                }
            }
            Err(Failure::Fatal(e)) => part.fatal = Some(e),
            Err(Failure::Transient(e)) => part.error = Some(e),
        }
    }
    // TVmaze, TheTVDB, Fanart.tv (META-012); never on a failed TMDB call.
    if part.error.is_none() && part.fatal.is_none() {
        let facts = part.fetched.tmdb.as_ref().and_then(|t| {
            Some(extra::TmdbFacts {
                client: providers.tmdb()?,
                kind: t.kind,
                id: t.details.id,
                dates: t
                    .seasons
                    .iter()
                    .flat_map(|s| &s.episodes)
                    .filter_map(|e| valid_date(e.air_date.as_deref()).map(str::to_owned))
                    .collect(),
            })
        });
        part.fetched.extras = extra::fetch(
            &providers.extra,
            work,
            facts.as_ref(),
            &providers.language(),
            &mut part.fetched.explanation,
        );
    }
    part
}

/// Whether `work` has an anime stage in this run, given its titles stage:
/// anime, or a show that TMDB calls Japanese animation or, when TMDB found
/// nothing, the local anime database knows (META-032).
fn has_anime_stage(providers: &MetadataProviders, work: &WorkInfo, part: &TitlesPart) -> bool {
    let anime_like = work.kind == "anime"
        || (work.kind == "show"
            && match &part.fetched.tmdb {
                Some(t) => is_anime(&t.details),
                None => part.local.is_some(),
            });
    anime_like && providers.anilist.is_some()
}

/// The anime stage of `work`: AniList and Shikimori, added to what the
/// titles stage found. Returns the AniList failure, if any.
fn fetch_anime_part(
    providers: &MetadataProviders,
    work: &WorkInfo,
    part: &mut TitlesPart,
) -> Result<(), Failure> {
    let Some(anilist) = providers.anilist.as_deref() else {
        return Ok(());
    };
    // A show TMDB named: AniList knows it by TMDB's English or original
    // title, not by the Russian folder name (META-032).
    let anime_work = match &part.fetched.tmdb {
        Some(t) if work.kind == "show" => {
            let mut w = work.clone();
            let mut titles: Vec<String> = t
                .titles
                .iter()
                .filter(|(l, _)| l == "en")
                .map(|(_, title)| title.clone())
                .chain(t.details.original_title.clone())
                .collect();
            titles.extend(work.titles.iter().cloned());
            let mut seen = HashSet::new();
            titles.retain(|t| !t.trim().is_empty() && seen.insert(t.clone()));
            w.titles = titles;
            std::borrow::Cow::Owned(w)
        }
        _ => std::borrow::Cow::Borrowed(work),
    };
    let fetched = &mut part.fetched;
    if let Some((data, confidence)) = fetch_anime(
        anilist,
        providers.shikimori.as_deref(),
        &anime_work,
        part.local.as_ref(),
        &mut fetched.explanation,
    )? {
        fetched.anime = Some(data);
        if fetched.tmdb.is_none() {
            fetched.confidence = confidence;
        }
    }
    Ok(())
}

/// One log line per written work: what was decided and why.
fn log_outcome(work: &WorkInfo, outcome: &Outcome) {
    const T: &str = "pa_catalog::metadata";
    let title = work.titles.first();
    match outcome {
        Outcome::Matched(f) => {
            let state = match f.confidence {
                None => "kept (known id)",
                Some(c) if c >= AUTO => "auto",
                Some(_) => "auto_review",
            };
            let shown = f
                .tmdb
                .as_ref()
                .and_then(|t| t.details.title.clone())
                .or_else(|| f.anime.as_ref().and_then(|a| a.media.title.romaji.clone()));
            tracing::info!(
                target: T,
                work = work.id,
                kind = %work.kind,
                title = ?title,
                year = ?work.year,
                tmdb = f.tmdb.as_ref().map(|t| t.details.id),
                anime = f.anime.is_some(),
                confidence = ?f.confidence,
                shown = ?shown,
                explanation = %f.explanation,
                "metadata: {state}"
            );
        }
        Outcome::NotFound(..) => tracing::info!(
            target: T,
            work = work.id,
            kind = %work.kind,
            title = ?title,
            year = ?work.year,
            "metadata: rejected, no provider result ranks high enough"
        ),
        Outcome::Failed(e) => tracing::warn!(
            target: T,
            work = work.id,
            kind = %work.kind,
            title = ?title,
            "metadata: failed, retried on a later run: {e}"
        ),
    }
}

/// The outcome of everything fetched for a work (both stages).
/// A fatal titles failure stops the run before this is asked.
fn outcome(part: TitlesPart, anime_error: Option<String>) -> Outcome {
    let fetched = part.fetched;
    if fetched.tmdb.is_some() || fetched.anime.is_some() {
        Outcome::Matched(Box::new(fetched))
    } else if let Some(e) = part.error.or(anime_error) {
        Outcome::Failed(e)
    } else {
        Outcome::NotFound(fetched.candidates, Box::new(fetched.extras))
    }
}

/// Runs `fetch` over `items` on `workers` threads and hands the results to
/// `on_event` on this thread, in completion order; `on_event(None)` when
/// nothing arrived for [`TICK`]. Workers stop taking items once `stop` is
/// set or `cancel` fires.
fn run_stage<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    stop: &AtomicBool,
    cancel: &CancelToken,
    fetch: &(dyn Fn(&T) -> R + Sync),
    on_event: &mut dyn FnMut(Option<(usize, R)>) -> Result<(), Error>,
) -> Result<(), Error> {
    if items.is_empty() {
        return Ok(());
    }
    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, R)>();
    std::thread::scope(|scope| -> Result<(), Error> {
        for _ in 0..workers.clamp(1, items.len()) {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) && !cancel.is_cancelled() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    if tx.send((i, fetch(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let result = (|| -> Result<(), Error> {
            loop {
                match rx.recv_timeout(TICK) {
                    Ok((i, r)) => {
                        if !stop.load(Ordering::Relaxed) {
                            on_event(Some((i, r)))?;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => on_event(None)?,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                }
            }
        })();
        if result.is_err() {
            stop.store(true, Ordering::Relaxed);
            // Let the workers see the stop and the channel drain.
            for _ in rx.try_iter() {}
        }
        result
    })
}

/// Providers that make the run wait at least a second.
fn provider_waits(providers: &MetadataProviders) -> Vec<ProviderWait> {
    let waits = [
        ("tmdb", providers.tmdb().and_then(TmdbClient::waiting)),
        (
            "anilist",
            providers
                .anilist
                .as_deref()
                .and_then(AniListClient::waiting),
        ),
        (
            "shikimori",
            providers
                .shikimori
                .as_deref()
                .and_then(ShikimoriClient::waiting),
        ),
    ];
    waits
        .into_iter()
        .filter_map(|(provider, w)| {
            w.filter(|w| w.duration >= Duration::from_secs(1))
                .map(|w| ProviderWait {
                    provider,
                    wait: w.duration,
                    provider_asked: w.provider_asked,
                })
        })
        .collect()
}

// --- Writing -----------------------------------------------------------------

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// `(id, season, episode, absolute, episode_type)` of an episode row.
type EpisodeRow = (i64, Option<i64>, Option<i64>, Option<i64>, String);

/// Moves everything of work `from` to work `into` and deletes `from`.
/// Episodes with the same numbers become one.
pub(crate) fn merge_work(tx: &Transaction<'_>, from: i64, into: i64) -> Result<(), Error> {
    let episodes: Vec<EpisodeRow> = {
        let mut stmt = tx.prepare(
            "SELECT id, season_number, episode_number, absolute_number, episode_type
             FROM episodes WHERE work_id = ?1",
        )?;
        stmt.query_map([from], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?
    };
    for (id, season, episode, absolute, kind) in episodes {
        let same: Option<i64> = tx
            .query_row(
                "SELECT id FROM episodes
                 WHERE work_id = ?1 AND season_number IS ?2 AND episode_number IS ?3
                   AND absolute_number IS ?4 AND episode_type = ?5
                 ORDER BY id LIMIT 1",
                params![into, season, episode, absolute, kind],
                |r| r.get(0),
            )
            .optional()?;
        match same {
            Some(target) => {
                tx.execute(
                    "UPDATE OR IGNORE match_episodes SET episode_id = ?2 WHERE episode_id = ?1",
                    params![id, target],
                )?;
                tx.execute("DELETE FROM episodes WHERE id = ?1", [id])?;
            }
            None => {
                tx.execute(
                    "UPDATE episodes SET work_id = ?2, season_id = NULL, canonical_ref = NULL
                     WHERE id = ?1",
                    params![id, into],
                )?;
            }
        }
    }
    for sql in [
        "UPDATE matches SET work_id = ?2 WHERE work_id = ?1",
        "UPDATE match_candidates SET work_id = ?2 WHERE work_id = ?1",
        "UPDATE OR IGNORE folder_rules SET work_id = ?2 WHERE work_id = ?1",
        "UPDATE OR IGNORE favorites SET work_id = ?2 WHERE work_id = ?1",
        "UPDATE OR IGNORE collection_items SET work_id = ?2 WHERE work_id = ?1",
        "UPDATE OR IGNORE display_preferences SET work_id = ?2 WHERE work_id = ?1",
        "UPDATE OR IGNORE version_prefs SET work_id = ?2 WHERE work_id = ?1",
        "INSERT OR IGNORE INTO titles (work_id, lang, script, text, kind, source)
         SELECT ?2, lang, script, text, CASE kind WHEN 'main' THEN 'synonym' ELSE kind END, source
         FROM titles WHERE work_id = ?1",
        "INSERT OR IGNORE INTO titles (work_id, text, kind, source)
         SELECT ?2, title, 'synonym', 'local' FROM works WHERE id = ?1",
    ] {
        tx.execute(sql, params![from, into])?;
    }
    tx.execute("DELETE FROM works WHERE id = ?1", [from])?;
    Ok(())
}

/// Sets `provider:ext_kind:ext_id` on `work`. If another work owns the id,
/// `work` is merged into it; returns the work that carries the id now.
pub(crate) fn claim_external(
    tx: &Transaction<'_>,
    work: i64,
    provider: &str,
    ext_kind: &str,
    ext_id: &str,
    merged: &mut usize,
) -> Result<i64, Error> {
    let owner: Option<i64> = tx
        .query_row(
            "SELECT work_id FROM external_ids WHERE provider = ?1 AND ext_kind = ?2 AND ext_id = ?3",
            params![provider, ext_kind, ext_id],
            |r| r.get(0),
        )
        .optional()?;
    match owner {
        Some(o) if o == work => Ok(work),
        Some(o) => {
            merge_work(tx, work, o)?;
            *merged += 1;
            Ok(o)
        }
        None => {
            tx.execute(
                "DELETE FROM external_ids WHERE work_id = ?1 AND provider = ?2 AND ext_kind = ?3",
                params![work, provider, ext_kind],
            )?;
            tx.execute(
                "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id) VALUES (?1, ?2, ?3, ?4)",
                params![work, provider, ext_kind, ext_id],
            )?;
            Ok(work)
        }
    }
}

fn add_title(
    tx: &Transaction<'_>,
    work: i64,
    text: Option<&str>,
    kind: &str,
    lang: &str,
    script: &str,
    source: &str,
) -> Result<(), Error> {
    if let Some(t) = non_empty(text) {
        tx.execute(
            "INSERT OR IGNORE INTO titles (work_id, lang, script, text, kind, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![work, lang, script, t, kind, source],
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn set_artwork(
    tx: &Transaction<'_>,
    work: Option<i64>,
    episode: Option<i64>,
    kind: &str,
    provider: &str,
    url: Option<String>,
    lang: &str,
    priority: i64,
) -> Result<(), Error> {
    let Some(url) = url.filter(|u| !u.is_empty()) else {
        return Ok(());
    };
    tx.execute(
        "DELETE FROM artwork WHERE work_id IS ?1 AND episode_id IS ?2 AND kind = ?3 AND provider = ?4",
        params![work, episode, kind, provider],
    )?;
    tx.execute(
        "INSERT INTO artwork (work_id, episode_id, kind, provider, url, lang, priority)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![work, episode, kind, provider, url, lang, priority],
    )?;
    Ok(())
}

/// Actors shown on the Detail screen, in billing order.
const MAX_CAST: usize = 20;
/// Crew jobs shown on the Detail screen, in this order.
const CREW_JOBS: [&str; 6] = [
    "Director",
    "Screenplay",
    "Writer",
    "Novel",
    "Original Music Composer",
    "Director of Photography",
];
/// Crew members shown on the Detail screen.
const MAX_CREW: usize = 8;

/// Rewrites the TMDB cast and crew of a work (UI-009): the first
/// [`MAX_CAST`] actors and up to [`MAX_CREW`] people of [`CREW_JOBS`].
fn set_credits(tx: &Transaction<'_>, work: i64, credits: &Credits) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM work_credits WHERE work_id = ?1 AND provider = 'tmdb'",
        [work],
    )?;
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO work_credits
             (work_id, provider, role, position, person_ref, name, character, job, profile_url)
         VALUES (?1, 'tmdb', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    let profile = |p: Option<&str>| p.map(|p| tmdb::image_url(PROFILE_SIZE, p));
    let mut cast: Vec<_> = credits
        .cast
        .iter()
        .filter(|c| !c.name.trim().is_empty())
        .collect();
    cast.sort_by_key(|c| c.order);
    for (i, c) in (0_i64..).zip(cast.into_iter().take(MAX_CAST)) {
        insert.execute(params![
            work,
            "cast",
            i,
            c.id.to_string(),
            c.name.trim(),
            non_empty(c.character.as_deref()),
            Option::<&str>::None,
            profile(c.profile_path.as_deref()),
        ])?;
    }
    let mut seen = HashSet::new();
    let mut crew = Vec::new();
    for job in CREW_JOBS {
        for c in credits.crew.iter().filter(|c| c.job == job) {
            if !c.name.trim().is_empty() && seen.insert((c.id, job)) {
                crew.push((c, job));
            }
        }
    }
    for (i, (c, job)) in (0_i64..).zip(crew.into_iter().take(MAX_CREW)) {
        insert.execute(params![
            work,
            "crew",
            i,
            c.id.to_string(),
            c.name.trim(),
            Option::<&str>::None,
            job,
            profile(c.profile_path.as_deref()),
        ])?;
    }
    Ok(())
}

/// Stores or clears the rating of `provider` (UI-009); a rating without
/// votes is no rating.
fn set_rating(
    tx: &Transaction<'_>,
    work: i64,
    provider: &str,
    value: Option<f64>,
    votes: Option<u32>,
) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM work_ratings WHERE work_id = ?1 AND provider = ?2",
        params![work, provider],
    )?;
    if let Some(v) = value.filter(|v| *v > 0.0 && *v <= 10.0)
        && votes.is_none_or(|n| n > 0)
    {
        tx.execute(
            "INSERT INTO work_ratings (work_id, provider, value, votes) VALUES (?1, ?2, ?3, ?4)",
            params![work, provider, v, votes.map(i64::from)],
        )?;
    }
    Ok(())
}

fn valid_date(d: Option<&str>) -> Option<&str> {
    d.filter(|d| {
        let b = d.as_bytes();
        b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    })
}

/// AniList descriptions carry `<br>` and `<i>` markup.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for c in s.chars() {
        match (in_tag, c) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                in_tag = false;
                if tag.trim_start_matches('/').starts_with("br") {
                    out.push('\n');
                }
            }
            (true, c) => tag.push(c.to_ascii_lowercase()),
            (false, c) => out.push(c),
        }
    }
    let mut text = out.replace("\n\n\n", "\n\n");
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    text.trim().to_owned()
}

fn lang_of(language: &str) -> &str {
    language.split('-').next().unwrap_or("")
}

fn record_state(
    tx: &Transaction<'_>,
    work: i64,
    outcome: &str,
    language: &str,
    error: Option<&str>,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO work_metadata (work_id, outcome, language, fetched_at_ms, error)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (work_id) DO UPDATE SET outcome = excluded.outcome,
             language = excluded.language, fetched_at_ms = excluded.fetched_at_ms,
             error = excluded.error",
        params![work, outcome, language, now, error],
    )?;
    Ok(())
}

/// Writes the metadata of `work` (or of the work it is merged into).
fn apply(
    tx: &Transaction<'_>,
    work: &WorkInfo,
    fetched: &Fetched,
    language: &str,
    now: i64,
    merged: &mut usize,
) -> Result<Option<i64>, Error> {
    // The work may have been merged away or removed since it was read.
    let exists: Option<i64> = tx
        .query_row("SELECT id FROM works WHERE id = ?1", [work.id], |r| {
            r.get(0)
        })
        .optional()?;
    if exists.is_none() {
        return Ok(None);
    }
    let lang = lang_of(language);
    let mut target = work.id;
    if let Some(t) = &fetched.tmdb {
        target = claim_external(
            tx,
            target,
            "tmdb",
            t.kind.as_str(),
            &t.details.id.to_string(),
            merged,
        )?;
        if let Some(imdb) = non_empty(t.details.imdb_id.as_deref())
            && tx
                .query_row(
                    "SELECT 1 FROM external_ids WHERE provider = 'imdb' AND ext_kind = '' AND ext_id = ?1",
                    [imdb],
                    |_| Ok(()),
                )
                .optional()?
                .is_none()
        {
            tx.execute(
                "INSERT OR IGNORE INTO external_ids (work_id, provider, ext_kind, ext_id)
                 VALUES (?1, 'imdb', '', ?2)",
                params![target, imdb],
            )?;
        }
    }
    if let Some(a) = &fetched.anime {
        target = claim_external(tx, target, "anilist", "", &a.media.id.to_string(), merged)?;
        for (provider, id) in [
            ("mal", a.media.id_mal),
            ("shikimori", a.russian.as_ref().map(|r| r.id)),
        ] {
            if let Some(id) = id {
                tx.execute(
                    "INSERT OR IGNORE INTO external_ids (work_id, provider, ext_kind, ext_id)
                     VALUES (?1, ?2, '', ?3)",
                    params![target, provider, id.to_string()],
                )?;
            }
        }
    }

    // Episode numbers of anime files in TMDB order (META-015), before the
    // TMDB episode data below is written onto the rows.
    if let Some(t) = &fetched.tmdb
        && !t.renumbered.is_empty()
    {
        crate::numbering::apply(tx, target, &t.renumbered)?;
    }

    // Titles: provider rows are rewritten; local and user titles stay.
    tx.execute(
        "DELETE FROM titles WHERE work_id = ?1 AND source IN ('tmdb', 'anilist', 'shikimori')",
        [target],
    )?;
    let tmdb = fetched.tmdb.as_ref().map(|t| &t.details);
    let anime = fetched.anime.as_ref();
    let russian = anime.and_then(|a| a.russian.as_ref());
    // The title in the metadata language when any provider knows one, else
    // the English or original one (META-032): TMDB answers in the original
    // language when it has no translation, so its translations and the
    // Russian title of Shikimori come first.
    let translated = |l: &str| {
        fetched.tmdb.as_ref().and_then(|t| {
            t.titles
                .iter()
                .find(|(tl, _)| tl == l)
                .and_then(|(_, title)| non_empty(Some(title)))
        })
    };
    let main_title = translated(lang)
        .or_else(|| {
            if lang == "ru" {
                russian.and_then(|r| non_empty(r.title.as_deref()))
            } else {
                None
            }
        })
        .or_else(|| {
            tmdb.filter(|d| {
                d.original_language.as_deref() == Some(lang)
                    || d.title.as_deref() != d.original_title.as_deref()
            })
            .and_then(|d| non_empty(d.title.as_deref()))
        })
        .or_else(|| translated("en"))
        .or_else(|| {
            anime.and_then(|a| {
                non_empty(a.media.title.english.as_deref())
                    .or(non_empty(a.media.title.romaji.as_deref()))
            })
        })
        .or_else(|| tmdb.and_then(|d| non_empty(d.title.as_deref())))
        .map(str::to_owned);
    // A provider without a translation leaves a Latin title in a non-Latin
    // UI (TMDB «The Animatrix» for ru): prefer the file's own name in the
    // UI's script when the scan saved one («Аниматрица»).
    let main_title = match main_title {
        Some(t) if lang != "en" && is_latin_only(&t) => {
            local_script_synonym(tx, target)?.or(Some(t))
        }
        other => other,
    };
    let original_title = tmdb
        .and_then(|d| non_empty(d.original_title.as_deref()))
        .or_else(|| anime.and_then(|a| non_empty(a.media.title.native.as_deref())))
        .map(str::to_owned);
    if let Some(title) = &main_title {
        tx.execute(
            "UPDATE titles SET kind = 'synonym' WHERE work_id = ?1 AND kind = 'main'",
            [target],
        )?;
        let source = if tmdb.is_some() { "tmdb" } else { "anilist" };
        add_title(tx, target, Some(title), "main", lang, "", source)?;
    }
    if let Some(d) = tmdb {
        let orig_lang = d.original_language.as_deref().unwrap_or("");
        add_title(
            tx,
            target,
            d.original_title.as_deref(),
            "official",
            orig_lang,
            "",
            "tmdb",
        )?;
    }
    if let Some(t) = &fetched.tmdb {
        for (l, title) in &t.titles {
            add_title(tx, target, Some(title), "official", l, "", "tmdb")?;
        }
    }
    if let Some(a) = anime {
        let t = &a.media.title;
        add_title(
            tx,
            target,
            t.romaji.as_deref(),
            "romaji",
            "ja",
            "Latn",
            "anilist",
        )?;
        add_title(
            tx,
            target,
            t.english.as_deref(),
            "official",
            "en",
            "",
            "anilist",
        )?;
        add_title(
            tx,
            target,
            t.native.as_deref(),
            "official",
            "ja",
            "",
            "anilist",
        )?;
        for s in &a.media.synonyms {
            add_title(tx, target, Some(s), "synonym", "", "", "anilist")?;
        }
        if let Some(r) = &a.russian {
            add_title(
                tx,
                target,
                r.title.as_deref(),
                "official",
                "ru",
                "",
                "shikimori",
            )?;
            add_title(
                tx,
                target,
                r.license_name.as_deref(),
                "official",
                "ru",
                "",
                "shikimori",
            )?;
        }
    }

    // The work row.
    let overview = tmdb
        .and_then(|d| non_empty(d.overview.as_deref()))
        .map(str::to_owned)
        .or_else(|| {
            if lang == "ru" {
                russian.and_then(|r| non_empty(r.description.as_deref()).map(str::to_owned))
            } else {
                None
            }
        })
        .or_else(|| {
            anime
                .and_then(|a| a.media.description.as_deref())
                .map(strip_html)
                .filter(|s| !s.is_empty())
        });
    let year = tmdb
        .and_then(Details::year)
        .or_else(|| anime.and_then(|a| a.media.year()))
        .map(i64::from)
        .filter(|y| (1800..=3000).contains(y));
    let runtime_min = tmdb
        .and_then(|d| d.runtime.or_else(|| d.episode_run_time.first().copied()))
        .or_else(|| anime.and_then(|a| a.media.duration))
        .filter(|m| *m > 0);
    let status = tmdb
        .and_then(|d| d.status.clone())
        .or_else(|| anime.and_then(|a| a.media.status.clone()));
    let original_lang =
        tmdb.and_then(|d| non_empty(d.original_language.as_deref()).map(str::to_owned));
    tx.execute(
        "UPDATE works SET title = ifnull(?2, title), original_title = ifnull(?3, original_title),
             original_lang = ifnull(?4, original_lang), year = ifnull(?5, year),
             overview = ifnull(?6, overview), runtime_ms = ifnull(?7, runtime_ms),
             status = ifnull(?8, status), updated_at_ms = ?9
         WHERE id = ?1",
        params![
            target,
            main_title,
            original_title,
            original_lang,
            year,
            overview,
            runtime_min.map(|m| i64::from(m) * 60_000),
            status,
            now
        ],
    )?;
    // A show folder the anime providers recognised is anime (META-032).
    if anime.is_some() {
        tx.execute(
            "UPDATE works SET kind = 'anime' WHERE id = ?1 AND kind = 'show'",
            [target],
        )?;
    }
    if let Some(a) = anime
        && let Some(format) = a.media.format.as_deref()
    {
        let anime_type = match format {
            "TV" | "TV_SHORT" => Some("tv"),
            "MOVIE" => Some("movie"),
            "OVA" => Some("ova"),
            "ONA" => Some("ona"),
            "SPECIAL" => Some("special"),
            "MUSIC" => Some("music_video"),
            _ => None,
        };
        tx.execute(
            "UPDATE works SET anime_type = ?2 WHERE id = ?1 AND kind = 'anime'",
            params![target, anime_type],
        )?;
    }

    // Genres (LIB-009): TMDB names are in the metadata language; AniList
    // names (English) only when TMDB has none.
    let tmdb_genres: Vec<&str> = tmdb
        .map(|d| d.genres.iter().map(|g| g.name.as_str()).collect())
        .unwrap_or_default();
    let anilist_genres: Vec<&str> = match anime {
        Some(a) if tmdb_genres.is_empty() => a.media.genres.iter().map(String::as_str).collect(),
        _ => Vec::new(),
    };
    pa_library::views::set_work_genres(tx, target, "tmdb", &tmdb_genres)?;
    pa_library::views::set_work_genres(tx, target, "anilist", &anilist_genres)?;

    // TMDB collection of a movie (LIB-010).
    if let Some(t) = &fetched.tmdb
        && t.kind.as_str() == "movie"
    {
        let collection = t
            .details
            .belongs_to_collection
            .as_ref()
            .map(|c| (c.id, c.name.as_deref().unwrap_or("")));
        crate::lists::link_tmdb_collection(tx, target, collection, now)?;
    }

    // Cast, crew and the TMDB rating (UI-009).
    if let Some(t) = &fetched.tmdb {
        set_credits(tx, target, &t.credits)?;
        // Content ratings for the profiles' filters (LIB-012).
        pa_library::profiles::set_certifications(tx, target, "tmdb", &t.certifications)?;
        set_rating(
            tx,
            target,
            "tmdb",
            t.details.vote_average,
            t.details.vote_count,
        )?;
    }

    // Artwork.
    tx.execute(
        "DELETE FROM artwork WHERE work_id = ?1 AND provider IN ('tmdb', 'anilist')",
        [target],
    )?;
    if let Some(d) = tmdb {
        let poster = d
            .poster_path
            .as_deref()
            .map(|p| tmdb::image_url(POSTER_SIZE, p));
        let backdrop = d
            .backdrop_path
            .as_deref()
            .map(|p| tmdb::image_url(BACKDROP_SIZE, p));
        set_artwork(tx, Some(target), None, "poster", "tmdb", poster, lang, 0)?;
        set_artwork(tx, Some(target), None, "backdrop", "tmdb", backdrop, "", 0)?;
    }
    if let Some(a) = anime {
        let cover = a.media.cover_image.as_ref().and_then(|c| c.large.clone());
        set_artwork(tx, Some(target), None, "poster", "anilist", cover, "", 10)?;
        // The last fallback: the Shikimori poster (META-032).
        let poster = a.russian.as_ref().and_then(|r| r.poster.clone());
        set_artwork(
            tx,
            Some(target),
            None,
            "poster",
            "shikimori",
            poster,
            "",
            20,
        )?;
    }

    // Episodes of the TMDB seasons; dated episodes (daily shows) first take
    // the numbers of TMDB's episode of their date (META-012).
    if let Some(t) = &fetched.tmdb {
        if !work.dates.is_empty() {
            extra::apply_tmdb_dates(tx, target, &t.seasons)?;
        }
        for season in &t.seasons {
            for ep in &season.episodes {
                let rows: Vec<i64> = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM episodes
                         WHERE work_id = ?1 AND season_number = ?2 AND episode_number = ?3
                           AND episode_type = 'regular'",
                    )?;
                    stmt.query_map(params![target, ep.season_number, ep.episode_number], |r| {
                        r.get(0)
                    })?
                    .collect::<Result<_, _>>()?
                };
                for id in rows {
                    tx.execute(
                        "UPDATE episodes SET title = ifnull(?2, title), overview = ifnull(?3, overview),
                             air_date = ifnull(?4, air_date), runtime_ms = ifnull(?5, runtime_ms)
                         WHERE id = ?1",
                        params![
                            id,
                            non_empty(ep.name.as_deref()),
                            non_empty(ep.overview.as_deref()),
                            valid_date(ep.air_date.as_deref()),
                            ep.runtime.filter(|m| *m > 0).map(|m| i64::from(m) * 60_000),
                        ],
                    )?;
                    let canonical = ep.id.to_string();
                    tx.execute(
                        "UPDATE episodes SET canonical_ref = ?2 WHERE id = ?1
                           AND NOT EXISTS (SELECT 1 FROM episodes e2
                                           WHERE e2.work_id = ?3 AND e2.canonical_ref = ?2)",
                        params![id, canonical, target],
                    )?;
                    let still = ep
                        .still_path
                        .as_deref()
                        .map(|p| tmdb::image_url(STILL_SIZE, p));
                    set_artwork(tx, None, Some(id), "still", "tmdb", still, "", 0)?;
                }
            }
        }
    }

    // TVmaze / TheTVDB episodes of dates TMDB lacks, Fanart.tv art, ids
    // (META-012).
    extra::apply(tx, target, &fetched.extras)?;

    // The results the search weighed, for the inbox (META-016).
    if fetched.confidence.is_some() {
        crate::review::set_candidates(tx, target, &fetched.candidates)?;
    }
    // Parser decisions become scorer decisions; user decisions and matches
    // under a user folder rule stay (META-015).
    if let Some(c) = fetched.confidence {
        let c = c.clamp(0.0, 1.0);
        let state = if c >= AUTO { "auto" } else { "auto_review" };
        tx.execute(
            concat!(
                "UPDATE matches AS m SET decided_by = 'scorer', model_version = ?2,
                     confidence = ?3, state = ?4, explanation_json = ?5, updated_at_ms = ?6
                 WHERE m.work_id = ?1 AND ",
                crate::numbering::recomputable!()
            ),
            params![
                target,
                MODEL_VERSION,
                c,
                state,
                fetched.explanation.to_string(),
                now
            ],
        )?;
    }
    // A known id (the user's, an NFO's, an anime's frames) keeps the work;
    // its automatic scorer matches of an older model have now been
    // renumbered by this one. Also without TMDB (an AniList id from frames):
    // otherwise the work would be taken again on every run.
    if fetched.confidence.is_none() {
        tx.execute(
            concat!(
                "UPDATE matches AS m SET model_version = ?2, updated_at_ms = ?3
                 WHERE m.work_id = ?1 AND m.decided_by = 'scorer'
                   AND ifnull(m.model_version, '') <> ?2 AND ",
                crate::numbering::recomputable!()
            ),
            params![target, MODEL_VERSION, now],
        )?;
    }
    record_state(tx, target, "matched", language, None, now)?;
    Ok(Some(target))
}

/// Marks whether the anime stage of `work` is still to run.
fn set_anime_pending(tx: &Transaction<'_>, work: i64, pending: bool) -> Result<(), Error> {
    tx.execute(
        "UPDATE work_metadata SET anime_pending = ?2 WHERE work_id = ?1",
        params![work, pending],
    )?;
    Ok(())
}

impl Catalog {
    /// Fetches provider metadata for works that need it (see the [module
    /// docs](self)). Runs are serialized with scans. `on_progress(done,
    /// total)` is called as works are decided, and while the run waits.
    pub fn fetch_metadata(
        &self,
        providers: &MetadataProviders,
        options: &MetadataOptions,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(usize, usize),
    ) -> Result<MetadataReport, Error> {
        self.fetch_metadata_with(providers, options, cancel, &mut |p| {
            on_progress(p.decided, p.works);
        })
    }

    /// [`fetch_metadata`](Self::fetch_metadata) with the stage, the counts
    /// of the stage and the provider waits (LIB-023).
    pub fn fetch_metadata_with(
        &self,
        providers: &MetadataProviders,
        options: &MetadataOptions,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(&MetadataProgress),
    ) -> Result<MetadataReport, Error> {
        let report = self.fetch_provider_metadata(providers, options, cancel, on_progress)?;
        // AniDB numbers (META-029): for the works matched now and for a
        // list refreshed since the last run. Local, no network.
        if let Some(index) = &providers.anime_mappings {
            self.apply_anime_mappings(index)?;
        }
        // LIB-025: clips the provider found nothing for go to «Мои видео».
        if report.not_found > 0 {
            self.reclassify_not_found()?;
        }
        Ok(report)
    }

    #[allow(clippy::too_many_lines)]
    fn fetch_provider_metadata(
        &self,
        providers: &MetadataProviders,
        options: &MetadataOptions,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(&MetadataProgress),
    ) -> Result<MetadataReport, Error> {
        let _serial = self.scan_lock.lock().unwrap_or_else(|e| e.into_inner());
        let language = providers.language();
        let mut report = MetadataReport::default();
        if providers.tmdb().is_none() && providers.anilist.is_none() && !providers.extra.dates() {
            tracing::info!(target: "pa_catalog::metadata", "metadata run skipped: no provider");
            return Ok(report);
        }
        let works: Vec<WorkInfo> = {
            let conn = self.lock();
            let ids = pending(&conn, &language, options, crate::now_ms())?;
            let mut works = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(w) = load_work(&conn, id)? {
                    // Works without TMDB support (non-anime without a TMDB
                    // key) are not asked for, but daily shows still get
                    // their episodes from TVmaze / TheTVDB (META-012).
                    let dated = w.kind == "show" && !w.dates.is_empty() && providers.extra.dates();
                    if providers.tmdb().is_some() || w.kind == "anime" || dated {
                        works.push(w);
                    }
                }
            }
            works
        };
        report.works = works.len();
        let total = works.len();
        let started = std::time::Instant::now();
        // Diagnostics (LIB-024, CORE-016): every run says what it starts
        // with and how it ends, so an exported log shows whether
        // recognition resumed.
        tracing::info!(
            target: "pa_catalog::metadata",
            works = total,
            anime = works.iter().filter(|w| w.kind == "anime").count(),
            language = %language,
            tmdb = providers.tmdb().is_some(),
            anilist = providers.anilist.is_some(),
            "metadata run start"
        );
        let mut progress = MetadataProgress {
            stage: MetadataStage::Titles,
            done: 0,
            total,
            decided: 0,
            works: total,
            waiting: Vec::new(),
        };
        on_progress(&progress);
        if total == 0 {
            return Ok(report);
        }

        // A decided work: written in its own transaction.
        let write = |work: &WorkInfo,
                     outcome: &Outcome,
                     anime_pending: Option<bool>,
                     report: &mut MetadataReport|
         -> Result<(), Error> {
            let now = crate::now_ms();
            let mut conn = self.lock();
            let t = conn.transaction()?;
            let exists = t
                .query_row("SELECT 1 FROM works WHERE id = ?1", [work.id], |_| Ok(()))
                .optional()?
                .is_some();
            match outcome {
                Outcome::Matched(fetched) => {
                    let target = apply(&t, work, fetched, &language, now, &mut report.merged)?;
                    if let (Some(target), Some(pending)) = (target, anime_pending) {
                        set_anime_pending(&t, target, pending)?;
                    }
                }
                Outcome::NotFound(candidates, extras) if exists => {
                    record_state(&t, work.id, "not_found", &language, None, now)?;
                    // The scorer that found nothing (BUG-009): a newer one
                    // asks again on its first run.
                    t.execute(
                        concat!(
                            "UPDATE matches AS m SET model_version = ?2
                             WHERE m.work_id = ?1 AND ",
                            not_found_matches!()
                        ),
                        params![work.id, MODEL_VERSION],
                    )?;
                    crate::review::set_candidates(&t, work.id, candidates)?;
                    // Daily-show episodes TVmaze or TheTVDB named (META-012).
                    extra::apply(&t, work.id, extras)?;
                    set_anime_pending(&t, work.id, false)?;
                }
                Outcome::Failed(e) if exists => {
                    record_state(&t, work.id, "failed", &language, Some(e), now)?;
                    set_anime_pending(&t, work.id, false)?;
                }
                _ => {}
            }
            t.commit()?;
            log_outcome(work, outcome);
            Ok(())
        };

        // Stage 1: titles.
        let stop = AtomicBool::new(false);
        let mut anime: Vec<(usize, TitlesPart)> = Vec::new();
        let mut failures_in_a_row = 0;
        run_stage(
            &works,
            TITLE_WORKERS,
            &stop,
            cancel,
            &|w: &WorkInfo| fetch_titles(providers, w),
            &mut |event| {
                let Some((i, mut part)) = event else {
                    progress.waiting = provider_waits(providers);
                    on_progress(&progress);
                    return Ok(());
                };
                let work = &works[i];
                if let Some(e) = &part.fatal {
                    stop.store(true, Ordering::Relaxed);
                    report.stopped = true;
                    report.error = Some(e.clone());
                    return Ok(());
                }
                match &part.error {
                    Some(e) if part.fetched.tmdb.is_none() => {
                        failures_in_a_row += 1;
                        if failures_in_a_row >= MAX_FAILURES_IN_A_ROW {
                            stop.store(true, Ordering::Relaxed);
                            report.stopped = true;
                            report.error = Some(e.clone());
                        }
                    }
                    _ => failures_in_a_row = 0,
                }
                progress.done += 1;
                if has_anime_stage(providers, work, &part) {
                    // Titles and posters now; the anime stage writes the
                    // work again with everything.
                    if part.fetched.tmdb.is_some() {
                        let titles = Outcome::Matched(Box::new(std::mem::take(&mut part.fetched)));
                        write(work, &titles, Some(true), &mut report)?;
                        if let Outcome::Matched(f) = titles {
                            part.fetched = *f;
                        }
                    }
                    anime.push((i, part));
                } else {
                    let outcome = outcome(part, None);
                    match &outcome {
                        Outcome::Matched(_) => report.matched += 1,
                        Outcome::NotFound(..) => report.not_found += 1,
                        Outcome::Failed(_) => report.failed += 1,
                    }
                    // LIB-024: no anime stage for this work (any more):
                    // a flag left by an earlier run must not keep it
                    // pending forever.
                    write(work, &outcome, Some(false), &mut report)?;
                    progress.decided += 1;
                }
                progress.waiting = provider_waits(providers);
                on_progress(&progress);
                Ok(())
            },
        )?;

        // Stage 2: anime, unless the run stopped (offline, rejected key).
        if !stop.load(Ordering::Relaxed) && !cancel.is_cancelled() && !anime.is_empty() {
            progress.stage = MetadataStage::Anime;
            progress.done = 0;
            progress.total = anime.len();
            progress.waiting = provider_waits(providers);
            on_progress(&progress);
            // Work index and titles stage of each anime work.
            let (index, parts): (Vec<usize>, Vec<Mutex<Option<TitlesPart>>>) = anime
                .into_iter()
                .map(|(i, part)| (i, Mutex::new(Some(part))))
                .unzip();
            let order: Vec<usize> = (0..parts.len()).collect();
            let mut failures_in_a_row = 0;
            run_stage(
                &order,
                ANIME_WORKERS,
                &stop,
                cancel,
                &|&k: &usize| {
                    let mut part = parts[k]
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                        .expect("each part is fetched once");
                    let result = fetch_anime_part(providers, &works[index[k]], &mut part);
                    (part, result)
                },
                &mut |event| {
                    let Some((k, (part, result))) = event else {
                        progress.waiting = provider_waits(providers);
                        on_progress(&progress);
                        return Ok(());
                    };
                    let work = &works[index[k]];
                    let anime_error = match result {
                        Ok(()) => {
                            failures_in_a_row = 0;
                            None
                        }
                        Err(Failure::Fatal(e)) => {
                            stop.store(true, Ordering::Relaxed);
                            report.stopped = true;
                            report.error = Some(e);
                            return Ok(());
                        }
                        Err(Failure::Transient(e)) => {
                            failures_in_a_row += 1;
                            if failures_in_a_row >= MAX_FAILURES_IN_A_ROW {
                                // AniList is out for now; the works keep
                                // their titles stage and wait for the next
                                // run.
                                stop.store(true, Ordering::Relaxed);
                                report.stopped = true;
                                report.error = Some(e.clone());
                            }
                            Some(e)
                        }
                    };
                    // AniList failed but TMDB matched: try AniList again on
                    // the next run.
                    let retry_anime = anime_error.is_some();
                    let outcome = outcome(part, anime_error);
                    match &outcome {
                        Outcome::Matched(_) => report.matched += 1,
                        Outcome::NotFound(..) => report.not_found += 1,
                        Outcome::Failed(_) => report.failed += 1,
                    }
                    write(work, &outcome, Some(retry_anime), &mut report)?;
                    progress.done += 1;
                    progress.decided += 1;
                    progress.waiting = provider_waits(providers);
                    on_progress(&progress);
                    Ok(())
                },
            )?;
        }
        if cancel.is_cancelled() {
            report.stopped = true;
        }
        pa_library::search::sync(&self.lock())?;
        let left = {
            let conn = self.lock();
            pending(&conn, &language, options, crate::now_ms())?.len()
        };
        tracing::info!(
            target: "pa_catalog::metadata",
            works = report.works,
            matched = report.matched,
            not_found = report.not_found,
            failed = report.failed,
            merged = report.merged,
            stopped = report.stopped,
            cancelled = cancel.is_cancelled(),
            error = ?report.error,
            still_pending = left,
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "metadata run end"
        );
        Ok(report)
    }

    /// Forgets the metadata fetch state so the next
    /// [`fetch_metadata`](Self::fetch_metadata) asks again: of every work
    /// (`all`) or of works no provider matched (after a new API key).
    pub fn reset_metadata(&self, all: bool) -> Result<(), Error> {
        let conn = self.lock();
        if all {
            conn.execute("DELETE FROM work_metadata", [])?;
        } else {
            conn.execute("DELETE FROM work_metadata WHERE outcome <> 'matched'", [])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_anilist_markup() {
        assert_eq!(
            strip_html("Line one.<br><br>\n<i>Line</i> two."),
            "Line one.\n\nLine two."
        );
    }

    #[test]
    fn dates_are_checked() {
        assert_eq!(valid_date(Some("2021-04-02")), Some("2021-04-02"));
        assert_eq!(valid_date(Some("2021")), None);
        assert_eq!(valid_date(Some("2021-4-2x")), None);
    }

    #[test]
    fn ranking_prefers_title_and_year() {
        let work = WorkQuery {
            kind: "movie".into(),
            year: Some(2021),
            titles: vec!["Dune".into()],
            ..Default::default()
        };
        let rec = |title: &str, year: u16| TitleRecord {
            kind: WorkKind::Movie,
            year: Some(year),
            titles: vec![title.into()],
            ..Default::default()
        };
        let (i, score, _) = rank(&work, vec![rec("Dune", 1984), rec("Dune", 2021)]).unwrap();
        assert_eq!(i, 1);
        assert!(score >= AUTO, "{score}");
        assert!(rank(&work, vec![rec("Dunkirk", 2017)]).is_none());
        // Without a year two same-named works tie: not auto.
        let no_year = WorkQuery {
            year: None,
            ..work.clone()
        };
        let (i, score, _) = rank(&no_year, vec![rec("Dune", 2021), rec("Dune", 1984)]).unwrap();
        assert_eq!(i, 0, "the provider's order breaks the tie");
        assert!((ACCEPT..AUTO).contains(&score), "{score}");
        // Only a remake of another year: penalised below auto.
        let (_, score, _) = rank(&work, vec![rec("Dune", 1984)]).unwrap_or((0, 0.0, String::new()));
        assert!(score < AUTO, "{score}");
    }

    #[test]
    fn match_search_tries_the_year_then_without_it() {
        let work = WorkQuery {
            kind: "movie".into(),
            year: Some(2021),
            titles: vec!["Dune".into()],
            ..Default::default()
        };
        let result = |id: u64, year: &str| SearchResult {
            id,
            media_type: Some("movie".into()),
            title: Some("Dune".into()),
            original_title: None,
            original_language: None,
            release_date: Some(format!("{year}-01-01")),
            origin_country: vec![],
            genre_ids: vec![],
            overview: None,
            poster_path: None,
            backdrop_path: None,
            popularity: 0.0,
            adult: false,
        };
        let mut calls = Vec::new();
        let m = match_search::<()>(&work, &mut |kind, q, y| {
            calls.push((kind, q.to_owned(), y));
            Ok(match y {
                Some(_) => vec![],
                None => vec![result(841, "1984"), result(438631, "2021")],
            })
        })
        .unwrap()
        .unwrap();
        assert_eq!(m.result.id, 438631);
        assert_eq!(m.query, "Dune");
        assert_eq!(
            calls,
            [
                (MediaKind::Movie, "Dune".to_owned(), Some(2021)),
                (MediaKind::Movie, "Dune".to_owned(), None)
            ]
        );
        assert_eq!(
            match_search::<()>(&work, &mut |_, _, _| Ok(vec![])).unwrap(),
            None
        );
    }

    /// BUG-011: `Агент 327 (2017)` against its only candidate `Агент 327:
    /// Операция «Парикмахерская»` (2017) is decided alone; a shared head
    /// or another year still asks.
    #[test]
    fn title_before_the_subtitle_is_decided_alone_with_the_year() {
        let work = |year: Option<i64>| WorkQuery {
            kind: "movie".into(),
            year,
            titles: vec!["Агент 327".into()],
            ..Default::default()
        };
        let search = |work: &WorkQuery, page: Vec<SearchResult>| {
            match_search::<()>(work, &mut |_, _, _| Ok(page.clone()))
                .unwrap()
                .unwrap()
        };
        let agent = movie_result(457_784, "Агент 327: Операция «Парикмахерская»", "2017");
        let m = search(&work(Some(2017)), vec![agent.clone()]);
        assert_eq!(m.result.id, 457_784);
        assert!(m.score >= AUTO, "{m:?}");
        assert_eq!(m.tier, "head");

        // The original title's head (`Agent 327: Operatie Barbershop`).
        let mut original = agent.clone();
        original.title = Some("Агент 327. Операция".into());
        original.original_title = Some("Agent 327: Operatie Barbershop".into());
        let latin = WorkQuery {
            titles: vec!["Agent 327".into()],
            ..work(Some(2017))
        };
        let m = search(&latin, vec![original]);
        assert!(m.score >= AUTO, "{m:?}");

        // No year, or another one: a suggestion to check, not a decision.
        for year in [None, Some(2016), Some(2019)] {
            let m =
                match_search::<()>(&work(year), &mut |_, _, _| Ok(vec![agent.clone()])).unwrap();
            assert!(m.as_ref().is_none_or(|m| m.score < AUTO), "{year:?}: {m:?}");
        }

        // Two films of the franchise share the head and the year: a tie.
        let m = search(
            &work(Some(2017)),
            vec![
                agent.clone(),
                movie_result(900_001, "Агент 327: Операция «Ледокол»", "2017"),
            ],
        );
        assert!(m.score < AUTO, "{m:?}");

        // The film named exactly like the file wins over a subtitled one.
        let exact = WorkQuery {
            titles: vec!["Сага".into()],
            ..work(Some(2017))
        };
        let m = search(
            &exact,
            vec![
                movie_result(900_002, "Сага: Часть 2", "2017"),
                movie_result(900_003, "Сага", "2017"),
            ],
        );
        assert_eq!(m.result.id, 900_003);
        assert!(m.score >= AUTO, "{m:?}");
    }

    fn movie_result(id: u64, title: &str, year: &str) -> SearchResult {
        SearchResult {
            id,
            media_type: Some("movie".into()),
            title: Some(title.into()),
            original_title: None,
            original_language: None,
            release_date: Some(format!("{year}-01-01")),
            origin_country: vec![],
            genre_ids: vec![],
            overview: None,
            poster_path: None,
            backdrop_path: None,
            popularity: 0.0,
            adult: false,
        }
    }

    #[test]
    fn details_break_a_remake_tie_by_duration() {
        // `Dune.mkv` without a year: a tie, unless the file's duration
        // says which film it is.
        let mut work = WorkQuery {
            kind: "movie".into(),
            year: None,
            titles: vec!["Dune".into()],
            ..Default::default()
        };
        let page = || {
            vec![
                movie_result(841, "Dune", "1984"),
                movie_result(438631, "Dune", "2021"),
            ]
        };
        let runtime = |id: u64| Some(if id == 841 { 137 } else { 155 });
        let asked = std::cell::RefCell::new(Vec::new());
        let mut details = |_: MediaKind, id: u64| {
            asked.borrow_mut().push(id);
            Ok::<_, ()>(Some(ResultFacts {
                runtime_min: runtime(id),
                ..Default::default()
            }))
        };
        // No evidence: a tie, no details requested.
        let m = match_search_with(&work, &mut |_, _, _| Ok(page()), &mut details)
            .unwrap()
            .unwrap();
        assert!(m.score < AUTO, "{m:?}");
        assert!(asked.borrow().is_empty());
        // A 137-minute file is the 1984 film.
        work.evidence.durations_ms = vec![137 * 60_000 + 30_000];
        let m = match_search_with(&work, &mut |_, _, _| Ok(page()), &mut details)
            .unwrap()
            .unwrap();
        assert_eq!(m.result.id, 841);
        assert!(m.score >= AUTO, "{m:?}");
        assert_eq!(*asked.borrow(), [841, 438631]);
    }

    #[test]
    fn season_list_breaks_a_same_name_series_tie() {
        let mut work = WorkQuery {
            kind: "show".into(),
            titles: vec!["Shameless".into()],
            ..Default::default()
        };
        work.evidence.episodes = vec![(9, 14)];
        let tv = |id: u64, year: &str| SearchResult {
            media_type: Some("tv".into()),
            ..movie_result(id, "Shameless", year)
        };
        let page = || vec![tv(1906, "2004"), tv(34307, "2011")];
        let m = match_search_with(&work, &mut |_, _, _| Ok(page()), &mut |_, id| {
            let seasons: BTreeMap<u32, u32> = if id == 1906 {
                [(9, 10)].into()
            } else {
                [(9, 14)].into()
            };
            Ok::<_, ()>(Some(ResultFacts {
                seasons,
                ..Default::default()
            }))
        })
        .unwrap()
        .unwrap();
        assert_eq!(m.result.id, 34307);
        assert!(m.score >= AUTO, "{m:?}");
    }

    #[test]
    fn details_facts_of_tmdb() {
        let d: Details = serde_json::from_str(
            r#"{"id":1,"original_language":"en","episode_run_time":[60,55,50],
                "seasons":[{"season_number":0,"episode_count":3},{"season_number":1,"episode_count":12}]}"#,
        )
        .unwrap();
        let f = ResultFacts::of_details(&d);
        assert_eq!(f.runtime_min, Some(55));
        assert_eq!(f.seasons, BTreeMap::from([(1, 12)]));
        assert_eq!(f.original_language.as_deref(), Some("en"));
    }
}

/// No letters outside ASCII.
fn is_latin_only(s: &str) -> bool {
    s.chars().all(|c| !c.is_alphabetic() || c.is_ascii())
}

/// A synonym the scan took from the file name with letters outside ASCII
/// (Cyrillic, CJK, ...), the oldest first.
fn local_script_synonym(tx: &Transaction<'_>, work: i64) -> Result<Option<String>, Error> {
    let mut stmt = tx.prepare_cached(
        "SELECT text FROM titles WHERE work_id = ?1 AND source = 'local' AND kind = 'synonym' \
         ORDER BY id",
    )?;
    let rows = stmt.query_map([work], |r| r.get::<_, String>(0))?;
    for text in rows {
        let text = text?;
        if !is_latin_only(&text) && !text.trim().is_empty() {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod local_title_tests {
    use super::is_latin_only;

    #[test]
    fn latin_only_ignores_digits_and_punctuation() {
        assert!(is_latin_only("The Animatrix"));
        assert!(is_latin_only("Blade Runner 2049: Part 1"));
        assert!(!is_latin_only("Аниматрица"));
        assert!(!is_latin_only("Pokémon"));
        assert!(!is_latin_only("攻殻機動隊"));
    }
}
