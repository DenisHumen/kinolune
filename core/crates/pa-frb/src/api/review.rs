// SPDX-License-Identifier: MPL-2.0
//! The review inbox («На проверку», META-016) for Dart: works whose match
//! the user should check (`auto_review`) or make (no result good enough),
//! the provider results to choose from with the signals behind each
//! score, and the user's final answers. The calls are methods of
//! [`CoreLibrary`]; see `pa_catalog::review`.

use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog as c;
use pa_catalog::metadata::MatchSignals;

use crate::api::library::{CoreLibrary, CoreMetadataSettings, LibraryError, item_ref};

type Result<T> = std::result::Result<T, LibraryError>;

/// Why a candidate scored what it did (`explanation_json`).
#[derive(Debug, Clone, PartialEq)]
pub struct CoreMatchSignals {
    /// `tmdb`, `anime_offline_database`, `anilist`.
    pub provider: String,
    /// Title similarity, `0..=1`.
    pub title: f64,
    /// `exact`, `loose`, `compact`, `base`, `fuzzy`.
    pub tier: String,
    /// The candidate's title that matched.
    pub matched_title: Option<String>,
    /// Year in the file names.
    pub file_year: Option<i32>,
    /// Year of the candidate.
    pub year: Option<i32>,
    /// `movie`, `show`, `anime`.
    pub file_kind: String,
    /// `movie` or `tv`.
    pub kind: String,
    /// `|duration − runtime| / runtime`.
    pub duration_error: Option<f64>,
    /// Written episodes the candidate has / does not have.
    pub episodes_ok: u32,
    pub episodes_bad: u32,
    /// An audio track is in the candidate's original language.
    pub language: Option<bool>,
    /// Another work scores the same.
    pub tied: bool,
    /// A word of the file names (`ova`, `movie`) the candidate lacks.
    pub lost_marker: Option<String>,
}

fn year(y: Option<i64>) -> Option<i32> {
    y.and_then(|y| i32::try_from(y).ok())
}

impl From<MatchSignals> for CoreMatchSignals {
    fn from(s: MatchSignals) -> Self {
        Self {
            provider: s.provider,
            title: s.title,
            tier: s.tier,
            matched_title: s.matched_title,
            file_year: year(s.file_year),
            year: year(s.year),
            file_kind: s.file_kind,
            kind: s.kind,
            duration_error: s.duration_error,
            episodes_ok: s.episodes_ok,
            episodes_bad: s.episodes_bad,
            language: s.language,
            tied: s.tied,
            lost_marker: s.lost_marker,
        }
    }
}

/// A provider result offered for a work.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreMatchCandidate {
    /// `tmdb`; `anilist` for an anime matched by AniList alone.
    pub provider: String,
    /// TMDB `movie` / `tv`.
    pub ext_kind: String,
    pub ext_id: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub poster_url: Option<String>,
    pub overview: Option<String>,
    /// Ranking score, `0..=1`.
    pub score: f64,
    pub signals: CoreMatchSignals,
    /// The work shows this result now.
    pub current: bool,
}

impl From<c::ReviewCandidate> for CoreMatchCandidate {
    fn from(c: c::ReviewCandidate) -> Self {
        Self {
            provider: c.provider,
            ext_kind: c.ext_kind,
            ext_id: c.ext_id,
            title: c.title,
            original_title: c.original_title,
            year: year(c.year),
            poster_url: c.poster_url,
            overview: c.overview,
            score: c.score,
            signals: c.signals.into(),
            current: c.current,
        }
    }
}

/// A work waiting in the inbox.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreReviewItem {
    pub work_id: i64,
    /// Library item id (`m<work>`, `s<work>`).
    pub item_id: String,
    /// `movie`, `show`, `anime`.
    pub kind: String,
    /// `check` (a result is shown, «проверьте») or `manual`.
    pub state: String,
    pub title: String,
    /// The title the files are named by.
    pub local_title: String,
    pub year: Option<i32>,
    pub poster_url: Option<String>,
    pub confidence: Option<f64>,
    pub signals: Option<CoreMatchSignals>,
    pub files: u32,
    /// One file, relative to its source.
    pub sample_path: String,
    /// Best first; the shown one is `current`.
    pub candidates: Vec<CoreMatchCandidate>,
}

impl From<c::ReviewItem> for CoreReviewItem {
    fn from(i: c::ReviewItem) -> Self {
        Self {
            work_id: i.work_id,
            item_id: i.item_id,
            kind: i.kind,
            state: i.state.as_str().to_owned(),
            title: i.title,
            local_title: i.local_title,
            year: year(i.year),
            poster_url: i.poster_url,
            confidence: i.confidence,
            signals: i.signals.map(Into::into),
            files: i.files,
            sample_path: i.sample_path,
            candidates: i.candidates.into_iter().map(Into::into).collect(),
        }
    }
}

/// How many works wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreReviewCount {
    /// Shown results to check.
    pub check: u32,
    /// Works without a result.
    pub manual: u32,
}

impl CoreLibrary {
    /// How many works wait in the review inbox.
    pub fn review_count(&self) -> Result<CoreReviewCount> {
        let n = self.catalog().review_count()?;
        Ok(CoreReviewCount {
            check: n.check,
            manual: n.manual,
        })
    }

    /// The works of the inbox: results to check first, then the manual
    /// queue; at most `limit`.
    pub fn review_items(&self, limit: u32) -> Result<Vec<CoreReviewItem>> {
        Ok(self
            .catalog()
            .review_items(limit as usize)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The inbox entry of an item's work, if it is in the inbox.
    pub fn review_item(&self, item_id: String) -> Result<Option<CoreReviewItem>> {
        Ok(self
            .catalog()
            .review_item(item_ref(&item_id)?)?
            .map(Into::into))
    }

    /// The results the last search of an item's work weighed.
    pub fn match_candidates(&self, item_id: String) -> Result<Vec<CoreMatchCandidate>> {
        Ok(self
            .catalog()
            .match_candidates(item_ref(&item_id)?)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// «Верно»: the shown result stays; final.
    pub fn review_confirm(&self, work_id: i64) -> Result<()> {
        Ok(self.catalog().review_confirm(work_id)?)
    }

    /// Another result (a candidate or a manual search result); final. The
    /// next metadata run reads it. Returns the work that carries it now
    /// (another work with the same result absorbs this one).
    pub fn review_choose(
        &self,
        work_id: i64,
        provider: String,
        ext_kind: String,
        ext_id: String,
    ) -> Result<i64> {
        Ok(self
            .catalog()
            .review_choose(work_id, &provider, &ext_kind, &ext_id)?)
    }

    /// «Это не фильм»: the files stay under their own names and are never
    /// sent to a provider again; final.
    pub fn review_not_media(&self, work_id: i64) -> Result<()> {
        Ok(self.catalog().review_not_media(work_id)?)
    }

    /// Manual search by title and year for a work: TMDB, ranked with what
    /// its files say, and the anime database on the device (UI-012).
    /// Writes nothing.
    pub fn review_search(
        &self,
        work_id: i64,
        settings: CoreMetadataSettings,
        title: String,
        year: Option<i32>,
    ) -> Result<Vec<CoreMatchCandidate>> {
        let providers = review_providers(&settings)?;
        Ok(self
            .catalog()
            .review_search(&providers, work_id, &title, year.map(i64::from))?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// «Вставить ID или ссылку» (UI-012): the results a pasted TMDB,
    /// TheTVDB, IMDb, AniDB, MyAnimeList, Shikimori or AniList link or id
    /// names for a work. Writes nothing.
    pub fn review_lookup(
        &self,
        work_id: i64,
        settings: CoreMetadataSettings,
        text: String,
    ) -> Result<CoreIdLookup> {
        let providers = review_providers(&settings)?;
        let found = self.catalog().review_lookup(&providers, work_id, &text)?;
        Ok(CoreIdLookup {
            source: found.source,
            candidates: found.candidates.into_iter().map(Into::into).collect(),
        })
    }

    /// Takes a result for a work in one step (UI-012): another kind
    /// switches the work's type, every file of the work takes the result
    /// and its folder learns the rule. Final.
    pub fn review_apply(
        &self,
        work_id: i64,
        provider: String,
        ext_kind: String,
        ext_id: String,
    ) -> Result<CoreFixReport> {
        Ok(self
            .catalog()
            .review_apply(work_id, &provider, &ext_kind, &ext_id)?
            .into())
    }

    /// What follows a work's decision: its files and the folders whose
    /// rule names it (after «Верно»).
    pub fn review_fix_report(&self, work_id: i64) -> Result<CoreFixReport> {
        Ok(self.catalog().fix_report(work_id)?.into())
    }
}

/// What a pasted id or link names.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreIdLookup {
    /// `tmdb`, `tvdb`, `imdb`, `anidb`, `anilist`, `mal`, `shikimori`;
    /// `None`: the text is not an id or link the app reads.
    pub source: Option<String>,
    /// Those of the work's kind first; empty: nobody knows the id.
    pub candidates: Vec<CoreMatchCandidate>,
}

/// What a fix did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFixReport {
    /// The work that carries the result now.
    pub work_id: i64,
    /// `movie`, `show` or `anime`.
    pub kind: String,
    /// Video files that follow the fix.
    pub files: u32,
    /// Folders whose rule names the work (relative to their source).
    pub folders: Vec<String>,
}

impl From<c::FixReport> for CoreFixReport {
    fn from(r: c::FixReport) -> Self {
        Self {
            work_id: r.work_id,
            kind: r.kind,
            files: r.files,
            folders: r.folders,
        }
    }
}

/// Providers of the manual search and the lookup: TMDB, AniList and
/// Shikimori with the provider cache, the anime database and the
/// Anime-Lists list the device has (no update here).
fn review_providers(settings: &CoreMetadataSettings) -> Result<c::MetadataProviders> {
    let cache_dir = PathBuf::from(&settings.cache_dir);
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| LibraryError::new("io", format!("provider cache: {e}")))?;
    let cache = c::pa_metadata::cache::SqliteCache::open(&cache_dir.join("provider-cache.sqlite"))
        .map_err(|e| LibraryError::new("database", format!("provider cache: {e}")))?;
    let key = settings
        .tmdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    let mut providers = c::MetadataProviders::live(Arc::new(cache), key, &settings.language);
    providers.anime_mappings =
        crate::api::library::anime_lists(&crate::api::anime_db::updates_dir(&cache_dir), false);
    providers.anime_titles = crate::api::anime_db::titles_for_run(
        &cache_dir,
        settings.anime_db_bundled.as_deref(),
        false,
    );
    Ok(providers)
}
