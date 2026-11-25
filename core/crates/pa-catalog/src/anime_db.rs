// SPDX-License-Identifier: MPL-2.0
//! anime-offline-database on the device (META-030).
//!
//! - **Bundled.** The app ships the derived ODbL bundle
//!   (`tools/anime-mappings/build.sh`, zstd) and passes its path; it is read
//!   in place, never modified.
//! - **Updated.** Once a day [`AnimeDb::refresh`] asks GitHub for the
//!   latest upstream release and installs it only if it is newer than what
//!   the device has and passes every check
//!   ([`Updater::refresh_offline_db`]: size and SHA-256 published with the
//!   release, zstd, ODbL upstream JSONL, entry count). The update lives in
//!   the cache directory next to the Anime-Lists list.
//! - **Read.** [`AnimeDb::load`] takes the newest valid copy: the update
//!   when its file checks out and its tag is not older than the bundled
//!   one, else the bundled file. A damaged update is never read.
//!
//! [`AnimeTitles`] is the local title index of the database: the metadata
//! run recognises anime works by it without a network request (AniDB,
//! AniList and MAL ids; with the Anime-Lists list also the TMDB series),
//! and the episode numbering takes entry sizes and titles from it.

use std::collections::HashMap;
use std::path::PathBuf;

use pa_anime_mappings::offline_db::{self, BundleHeader, OfflineEntry};
use pa_anime_mappings::release;
use pa_anime_mappings::update::{Outcome, Updater};
use pa_metadata::http::HttpTransport;
use pa_recognition::title::TitleIndex;

use crate::Error;
use crate::anime_order::{Client, mappings_error};

/// Where the database in use comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimeDbOrigin {
    /// Shipped with the app.
    Bundled,
    /// Downloaded from the upstream release by this device.
    Downloaded,
}

/// What the device has, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimeDbInfo {
    pub origin: AnimeDbOrigin,
    /// Upstream release tag, `2026-46`.
    pub tag: String,
    /// Upstream `lastUpdate`, `2026-11-15`.
    pub last_update: String,
    /// Entries (known only once loaded or for a downloaded update).
    pub entries: Option<usize>,
    /// Unix seconds of the last answered release check.
    pub checked_at: Option<u64>,
}

/// The database read by [`AnimeDb::load`].
#[derive(Debug, Clone)]
pub struct LoadedAnimeDb {
    pub info: AnimeDbInfo,
    pub entries: Vec<OfflineEntry>,
}

/// The bundled database and its updates in a cache directory (for example
/// `<app cache>/anime-mappings`, shared with [`crate::AnimeLists`]).
#[derive(Debug, Clone)]
pub struct AnimeDb {
    updater: Updater,
    bundled: Option<PathBuf>,
}

impl AnimeDb {
    /// Updates cached in `dir`; `bundled` is the file shipped with the app,
    /// if any (development builds may have none).
    pub fn new(dir: impl Into<PathBuf>, bundled: Option<PathBuf>) -> Self {
        Self {
            updater: Updater::new(dir),
            bundled,
        }
    }

    /// The same, accepting releases with at least `min_entries` entries
    /// (tests; the real database has tens of thousands).
    #[must_use]
    pub fn with_min_entries(mut self, min_entries: usize) -> Self {
        self.updater.min_entries_override = Some(min_entries);
        self
    }

    fn bundled_bytes(&self) -> Option<Vec<u8>> {
        self.bundled.as_ref().and_then(|p| std::fs::read(p).ok())
    }

    /// Header of the bundled database.
    pub fn bundled_header(&self) -> Option<BundleHeader> {
        self.bundled_bytes()
            .and_then(|b| offline_db::read_bundle_header(&b).ok())
    }

    /// Asks upstream for a newer release (no more than once a day unless
    /// `force`; `now` in Unix seconds). On any error the database in use
    /// stays as it is.
    pub fn refresh(
        &self,
        transport: &dyn HttpTransport,
        now: u64,
        force: bool,
    ) -> Result<Outcome, Error> {
        let bundled = self.bundled_header().map(|h| h.upstream_tag);
        self.updater
            .refresh_offline_db(&Client(transport), now, force, bundled.as_deref())
            .map_err(mappings_error)
    }

    /// The update to use instead of the bundled file: installed, its file
    /// intact and its tag not older than `bundled`.
    fn downloaded(&self, bundled: Option<&str>) -> Option<(BundleHeader, Vec<OfflineEntry>)> {
        let (h, entries) = self.updater.load_offline_db().ok().flatten()?;
        (bundled.is_none_or(|b| !release::newer(b, Some(&h.upstream_tag)))).then_some((h, entries))
    }

    /// The newest valid database; `None` with no bundled file and no
    /// update.
    pub fn load(&self) -> Result<Option<LoadedAnimeDb>, Error> {
        let checked_at = self.updater.offline_db_checked_at().ok().flatten();
        let bundled = self.bundled_bytes();
        let bundled_tag = bundled
            .as_deref()
            .and_then(|b| offline_db::read_bundle_header(b).ok())
            .map(|h| h.upstream_tag);
        let (origin, (header, entries)) = match self.downloaded(bundled_tag.as_deref()) {
            Some(d) => (AnimeDbOrigin::Downloaded, d),
            None => match bundled {
                Some(b) => (
                    AnimeDbOrigin::Bundled,
                    offline_db::read_bundle_bytes(&b).map_err(mappings_error)?,
                ),
                None => return Ok(None),
            },
        };
        Ok(Some(LoadedAnimeDb {
            info: AnimeDbInfo {
                origin,
                tag: header.upstream_tag,
                last_update: header.upstream_last_update,
                entries: Some(entries.len()),
                checked_at,
            },
            entries,
        }))
    }

    /// What [`load`](Self::load) would read; of the bundled file only the
    /// header is read.
    pub fn info(&self) -> Option<AnimeDbInfo> {
        let checked_at = self.updater.offline_db_checked_at().ok().flatten();
        let bundled = self.bundled_header();
        if let Some((h, entries)) =
            self.downloaded(bundled.as_ref().map(|b| b.upstream_tag.as_str()))
        {
            return Some(AnimeDbInfo {
                origin: AnimeDbOrigin::Downloaded,
                tag: h.upstream_tag,
                last_update: h.upstream_last_update,
                entries: Some(entries.len()),
                checked_at,
            });
        }
        bundled.map(|h| AnimeDbInfo {
            origin: AnimeDbOrigin::Bundled,
            tag: h.upstream_tag,
            last_update: h.upstream_last_update,
            entries: None,
            checked_at,
        })
    }
}

/// The local title index of anime-offline-database: main titles and
/// synonyms (romaji, English, Russian, Japanese) of every entry.
#[derive(Debug)]
pub struct AnimeTitles {
    index: TitleIndex,
    /// In index record order.
    entries: Vec<OfflineEntry>,
    by_anidb: HashMap<u32, usize>,
}

impl AnimeTitles {
    #[must_use]
    pub fn new(entries: Vec<OfflineEntry>) -> Self {
        let index =
            TitleIndex::from_records(entries.iter().map(pa_recognition::title::anime::record));
        let by_anidb = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Some((e.ids.anidb?, i)))
            .collect();
        Self {
            index,
            entries,
            by_anidb,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn index(&self) -> &TitleIndex {
        &self.index
    }

    /// The entry at index record `position`.
    pub fn entry_at(&self, position: usize) -> Option<&OfflineEntry> {
        self.entries.get(position)
    }

    /// The entry with this AniDB id.
    pub fn entry(&self, anidb: u32) -> Option<&OfflineEntry> {
        self.by_anidb.get(&anidb).map(|&i| &self.entries[i])
    }

    /// The entry carrying `id` (AniDB, AniList, MAL or Kitsu; a pasted id
    /// of «Исправить распознавание», UI-012). A linear pass: it runs once
    /// per paste.
    pub fn find(&self, id: pa_anime_mappings::IdRef) -> Option<&OfflineEntry> {
        use pa_anime_mappings::IdRef;
        if let IdRef::AniDb(a) = id {
            return self.entry(a);
        }
        self.entries.iter().find(|e| match id {
            IdRef::AniList(n) => e.ids.anilist == Some(n),
            IdRef::Mal(n) => e.ids.mal == Some(n),
            IdRef::Kitsu(n) => e.ids.kitsu == Some(n),
            IdRef::Tvdb(n) => e.ids.tvdb == Some(n),
            IdRef::TmdbTv(n) => e.ids.tmdb_tv == Some(n),
            IdRef::TmdbMovie(n) => e.ids.tmdb_movie == Some(n),
            IdRef::AniDb(_) => false,
        })
    }
}
