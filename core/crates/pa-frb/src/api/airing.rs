// SPDX-License-Identifier: MPL-2.0
//! When the next episode of an ongoing show or anime airs (UI-057), for
//! Dart: the show page, the card of the episode still to come at the end of
//! the episode row and the labels on the library posters. Methods of
//! [`CoreLibrary`]; the core is `pa_catalog::airing`.

use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog as c;

use crate::api::library::{
    CoreLibrary, CoreMetadataSettings, LibraryError, item_ref, live_providers,
};

type Result<T> = std::result::Result<T, LibraryError>;

/// Where a show is in its run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreAiringStatus {
    /// New episodes come; `next_episode` is set when the date is known.
    Airing,
    /// On a break, no date yet.
    Hiatus,
    /// Finished or cancelled.
    Ended,
}

/// The airing state of a show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreShowAiring {
    /// `s<work>`.
    pub show_id: String,
    pub status: CoreAiringStatus,
    /// Season of the next episode (TMDB numbering); `None` when the
    /// provider numbers episodes in one run (AniList, Shikimori).
    pub next_season: Option<i32>,
    /// The next episode still to come; `None` once it aired or its file is
    /// in the library.
    pub next_episode: Option<i32>,
    /// `YYYY-MM-DD` when only the date is known (TMDB).
    pub next_air_date: Option<String>,
    /// The exact moment (UTC), Unix ms (anime).
    pub next_air_at_ms: Option<i64>,
    /// `tmdb`, `anilist` or `shikimori`.
    pub source: String,
    /// When a provider last answered, Unix ms.
    pub checked_at_ms: i64,
    /// The last refresh failed (offline): an older answer.
    pub offline: bool,
}

impl From<c::ShowAiring> for CoreShowAiring {
    fn from(a: c::ShowAiring) -> Self {
        let small = |v: i64| i32::try_from(v).ok();
        Self {
            show_id: a.show.to_string(),
            status: match a.status {
                c::AiringStatus::Airing => CoreAiringStatus::Airing,
                c::AiringStatus::Hiatus => CoreAiringStatus::Hiatus,
                c::AiringStatus::Ended => CoreAiringStatus::Ended,
            },
            next_season: a.next.as_ref().and_then(|n| n.season).and_then(small),
            next_episode: a.next.as_ref().and_then(|n| small(n.episode)),
            next_air_date: a.next.as_ref().and_then(|n| n.air_date.clone()),
            next_air_at_ms: a.next.as_ref().and_then(|n| n.air_at_ms),
            source: a.source,
            checked_at_ms: a.checked_at_ms,
            offline: a.offline,
        }
    }
}

/// The provider clients of a metadata run, sharing its response cache.
fn providers(settings: &CoreMetadataSettings) -> Result<c::MetadataProviders> {
    let dir = PathBuf::from(&settings.cache_dir);
    std::fs::create_dir_all(&dir).map_err(|e| LibraryError::new("io", e))?;
    let cache = c::pa_metadata::cache::SqliteCache::open(&dir.join("provider-cache.sqlite"))
        .map_err(|e| LibraryError::new("io", e))?;
    Ok(live_providers(Arc::new(cache), settings))
}

impl CoreLibrary {
    /// The airing state of a show as last fetched; `None` without data.
    pub fn show_airing(&self, show_id: String, now_ms: i64) -> Result<Option<CoreShowAiring>> {
        Ok(self
            .catalog()
            .show_airing(item_ref(&show_id)?, now_ms)?
            .map(Into::into))
    }

    /// The airing state of every show that has one (poster labels).
    pub fn airing_all(&self, now_ms: i64) -> Result<Vec<CoreShowAiring>> {
        Ok(self
            .catalog()
            .airing_all(now_ms)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Asks TMDB, AniList or Shikimori about a show now (its page opened).
    /// Offline the last value comes back with `offline`.
    pub fn refresh_airing(
        &self,
        show_id: String,
        settings: CoreMetadataSettings,
        now_ms: i64,
    ) -> Result<Option<CoreShowAiring>> {
        let providers = providers(&settings)?;
        Ok(self
            .catalog()
            .refresh_airing(&providers, item_ref(&show_id)?, now_ms)?
            .map(Into::into))
    }

    /// The daily background refresh of the shows that are due. Returns how
    /// many shows a provider answered for.
    pub fn refresh_airing_due(&self, settings: CoreMetadataSettings, now_ms: i64) -> Result<u32> {
        let providers = providers(&settings)?;
        let report =
            self.catalog()
                .refresh_airing_due(&providers, now_ms, &c::CancelToken::new())?;
        Ok(u32::try_from(report.updated).unwrap_or(u32::MAX))
    }
}
