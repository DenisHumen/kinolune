// SPDX-License-Identifier: MPL-2.0
//! anime-offline-database on the device (META-030) for Dart: the database
//! the app ships (`assets/anime-db`, copied by Dart to a file) and its
//! daily update from the upstream GitHub release.
//!
//! The metadata run ([`CoreLibrary::fetch_metadata`](crate::api::library::CoreLibrary::fetch_metadata))
//! reads it through [`titles_for_run`]; the settings screen shows
//! [`anime_db_info`] and the app asks for an update on start
//! ([`anime_db_refresh`]: the core asks GitHub at most once a day).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pa_catalog as c;

use crate::api::library::LibraryError;

/// Where the database in use comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreAnimeDbOrigin {
    /// Shipped with the app.
    Bundled,
    /// Downloaded from the upstream release by this device.
    Downloaded,
}

/// The anime database the device uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreAnimeDbInfo {
    pub origin: CoreAnimeDbOrigin,
    /// Upstream release tag, `2026-46`.
    pub tag: String,
    /// Upstream `lastUpdate`, `2026-11-15`.
    pub last_update: String,
    /// Number of entries, when known without parsing the bundled file.
    pub entries: Option<u32>,
    /// Unix seconds of the last answered release check.
    pub checked_at: Option<i64>,
}

/// Result of [`anime_db_refresh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreAnimeDbRefresh {
    /// Checked less than a day ago; nothing asked.
    Skipped,
    /// The device already has the latest release.
    UpToDate,
    /// A newer release was verified and installed.
    Updated,
}

impl From<c::AnimeDbInfo> for CoreAnimeDbInfo {
    fn from(i: c::AnimeDbInfo) -> Self {
        Self {
            origin: match i.origin {
                c::AnimeDbOrigin::Bundled => CoreAnimeDbOrigin::Bundled,
                c::AnimeDbOrigin::Downloaded => CoreAnimeDbOrigin::Downloaded,
            },
            tag: i.tag,
            last_update: i.last_update,
            entries: i.entries.and_then(|n| u32::try_from(n).ok()),
            checked_at: i.checked_at.and_then(|t| i64::try_from(t).ok()),
        }
    }
}

/// The cache directory of the updates: `anime-mappings` next to the
/// provider cache, shared with the Anime-Lists list.
pub(crate) fn updates_dir(metadata_cache_dir: &Path) -> PathBuf {
    metadata_cache_dir.join("anime-mappings")
}

fn db(metadata_cache_dir: &str, bundled: Option<String>) -> c::AnimeDb {
    c::AnimeDb::new(
        updates_dir(Path::new(metadata_cache_dir)),
        bundled.filter(|p| !p.is_empty()).map(PathBuf::from),
    )
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One update of the anime data at a time: the start-up check, a manual
/// check and a metadata run share `manifest.json` (read, change, rename).
static UPDATES: Mutex<()> = Mutex::new(());

/// Holds [`UPDATES`] while `f` refreshes something in the updates
/// directory.
pub(crate) fn exclusive<T>(f: impl FnOnce() -> T) -> T {
    let _guard = UPDATES.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// What the device uses: the newest valid of the bundled file
/// (`bundled`, a path) and the downloaded update in
/// `<metadata_cache_dir>/anime-mappings`. `None`: neither exists (a
/// development build without the bundled file, before the first update).
pub fn anime_db_info(
    metadata_cache_dir: String,
    bundled: Option<String>,
) -> Option<CoreAnimeDbInfo> {
    db(&metadata_cache_dir, bundled).info().map(Into::into)
}

/// Asks GitHub for a newer release (no more than once a day unless
/// `force`) and installs it after the checks; on any error the database in
/// use stays as it is and the error says why.
pub fn anime_db_refresh(
    metadata_cache_dir: String,
    bundled: Option<String>,
    force: bool,
) -> Result<CoreAnimeDbRefresh, LibraryError> {
    use c::pa_anime_mappings::update::Outcome;
    use c::pa_metadata::http::UreqTransport;
    let transport = UreqTransport::with_timeout(
        c::pa_metadata::USER_AGENT,
        std::time::Duration::from_secs(120),
    );
    let db = db(&metadata_cache_dir, bundled);
    let outcome = exclusive(|| db.refresh(&transport, now(), force)).map_err(LibraryError::from)?;
    Ok(match outcome {
        Outcome::Skipped => CoreAnimeDbRefresh::Skipped,
        Outcome::NotModified => CoreAnimeDbRefresh::UpToDate,
        Outcome::Updated { .. } => CoreAnimeDbRefresh::Updated,
    })
}

/// The title index of the last database loaded, by (origin, tag): building
/// it takes a moment, a metadata run follows every scan.
type CachedTitles = ((c::AnimeDbOrigin, String), Arc<c::AnimeTitles>);
static TITLES: Mutex<Option<CachedTitles>> = Mutex::new(None);

/// The anime database for a metadata run, refreshed first when `update`
/// is on (at most once a day; a failure keeps what the device has).
pub(crate) fn titles_for_run(
    metadata_cache_dir: &Path,
    bundled: Option<&str>,
    update: bool,
) -> Option<Arc<c::AnimeTitles>> {
    let db = c::AnimeDb::new(
        updates_dir(metadata_cache_dir),
        bundled.filter(|p| !p.is_empty()).map(PathBuf::from),
    );
    if update {
        use c::pa_metadata::http::UreqTransport;
        let transport = UreqTransport::with_timeout(
            c::pa_metadata::USER_AGENT,
            std::time::Duration::from_secs(120),
        );
        let _ = exclusive(|| db.refresh(&transport, now(), false));
    }
    let info = db.info()?;
    let key = (info.origin, info.tag);
    let mut cached = TITLES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, titles)) = cached.as_ref()
        && *k == key
    {
        return Some(titles.clone());
    }
    let loaded = db.load().ok().flatten()?;
    let titles = Arc::new(c::AnimeTitles::new(loaded.entries));
    *cached = Some(((loaded.info.origin, loaded.info.tag), titles.clone()));
    Some(titles)
}
