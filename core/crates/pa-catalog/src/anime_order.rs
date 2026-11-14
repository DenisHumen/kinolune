// SPDX-License-Identifier: MPL-2.0
//! AniDB episode order of real anime (META-029): the Anime-Lists mappings
//! of `pa-anime-mappings` written into `episode_mappings`.
//!
//! - **The list.** Anime-Lists (`anime-list-master.xml`) has no licence, so
//!   it is never bundled or mirrored (`docs/legal/anime-mappings.md` §3):
//!   the device downloads it from upstream into its own cache
//!   ([`AnimeLists`], at most once a day, ETag, checksums) and reads the last
//!   downloaded copy. With no copy the AniDB order is simply not offered.
//! - **The rows.** For every work with a TMDB TV id (or a TVDB series id),
//!   each numbered episode (TMDB numbering, `episodes.season_number` /
//!   `episode_number`) is looked up in the [`MappingIndex`]: TMDB first
//!   (`tmdbseason` / `tmdboffset`, also the absolute number for `a`
//!   seasons), then TVDB (`defaulttvdbseason`, `episodeoffset` and the
//!   mapping list, which also maps specials). The work's own TVDB id is
//!   used, else the TVDB id the franchise entries of its TMDB id share
//!   (lower confidence: TVDB and TMDB numbering may differ).
//! - **Season = AniDB entry** (ADR-0046): AniDB has no seasons, so the row
//!   season is the position of the AniDB entry in the franchise (all
//!   entries carrying the work's TMDB/TVDB id, ordered by the season and
//!   offset they map onto), 1, 2…; AniDB specials (`S1`…) are season 0.
//!   Rows have scheme `anidb` and source `anime_lists`.
//!
//! [`Catalog::apply_anime_mappings`] rewrites all rows of the source in one
//! transaction; the metadata run calls it with
//! [`MetadataProviders::anime_mappings`](crate::MetadataProviders) so newly
//! matched works and a refreshed list both reach the library.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use pa_anime_mappings::anime_lists::{SeasonRef, Target};
use pa_anime_mappings::update::{self, Outcome, Source, Updater};
use pa_anime_mappings::{IdRef, MappingIndex, Record};
use pa_library::rusqlite::{Connection, params};
use pa_metadata::http::{HttpRequest, HttpTransport};

use crate::{Catalog, Error};

/// `episode_mappings.source` of the rows written here.
pub const SOURCE: &str = "anime_lists";

/// Confidence of a row found through the work's own TMDB or TVDB id.
const CONFIDENT: f64 = 1.0;
/// Confidence of a row found through the TVDB id of the franchise (the
/// work has only a TMDB id; numbering may differ between the two).
const BORROWED_TVDB: f64 = 0.6;

fn mappings_error(e: pa_anime_mappings::Error) -> Error {
    match e {
        pa_anime_mappings::Error::Io(e) => Error::Io(e.to_string()),
        pa_anime_mappings::Error::Http(m) => Error::Io(m),
        other => Error::Invalid(other.to_string()),
    }
}

/// The Anime-Lists list downloaded on this device: a cache directory (for
/// example `<app cache>/anime-mappings`) refreshed from upstream.
#[derive(Debug, Clone)]
pub struct AnimeLists {
    updater: Updater,
}

/// [`update::HttpClient`] over a provider transport.
struct Client<'a>(&'a dyn HttpTransport);

impl update::HttpClient for Client<'_> {
    fn get(
        &self,
        url: &str,
        if_none_match: Option<&str>,
    ) -> Result<update::HttpResponse, String> {
        let request = HttpRequest {
            base: url.to_owned(),
            path: String::new(),
            query: Vec::new(),
            headers: if_none_match
                .map(|e| vec![("if-none-match".to_owned(), e.to_owned())])
                .unwrap_or_default(),
        };
        let response = self.0.get(&request).map_err(|e| e.to_string())?;
        Ok(update::HttpResponse {
            status: response.status,
            etag: response.header("etag").map(str::to_owned),
            body: response.body.into_bytes(),
        })
    }
}

impl AnimeLists {
    /// The list cached in `dir` (created on the first refresh).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            updater: Updater::new(dir),
        }
    }

    /// The same, accepting lists with at least `min_entries` entries
    /// (tests; the real list has thousands).
    #[must_use]
    pub fn with_min_entries(mut self, min_entries: usize) -> Self {
        self.updater.min_entries_override = Some(min_entries);
        self
    }

    /// Asks upstream for a newer list (no more than once a day unless
    /// `force`; `now` in Unix seconds). On any error the last downloaded
    /// list stays in place, so a failure here only means an older list.
    pub fn refresh(
        &self,
        transport: &dyn HttpTransport,
        now: u64,
        force: bool,
    ) -> Result<Outcome, Error> {
        self.updater
            .refresh(&Client(transport), Source::AnimeLists, now, force)
            .map_err(mappings_error)
    }

    /// The last downloaded list as an index; `None` before the first
    /// download.
    pub fn load(&self) -> Result<Option<MappingIndex>, Error> {
        let Some(bytes) = self
            .updater
            .load(Source::AnimeLists)
            .map_err(mappings_error)?
        else {
            return Ok(None);
        };
        let xml = std::str::from_utf8(&bytes).map_err(|e| Error::Invalid(e.to_string()))?;
        let entries = pa_anime_mappings::anime_lists::parse(xml).map_err(mappings_error)?;
        let mut index = MappingIndex::new();
        index.add_anime_lists(entries);
        Ok(Some(index))
    }
}

/// An AniDB number found for an episode.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Found {
    /// AniDB entry id; `season` 0 = its specials, 1 = regular episodes.
    anidb: u32,
    season: u32,
    episode: u32,
    confidence: f64,
}

/// Where a franchise entry sits: (season, offset, AniDB id); seasonless
/// entries last.
fn position(r: &Record, tmdb: Option<u32>) -> (u32, i64, u32) {
    let anidb = r.ids.anidb.unwrap_or(u32::MAX);
    let Some(al) = &r.anime_list else {
        return (u32::MAX, 0, anidb);
    };
    let (season, offset) = if tmdb.is_some() && al.tmdb_tv == tmdb && al.tmdb_season.is_some() {
        (al.tmdb_season, al.tmdb_offset)
    } else {
        (al.default_tvdb_season, al.episode_offset)
    };
    let season = match season {
        Some(SeasonRef::Season(s)) if s >= 1 => s,
        Some(SeasonRef::Absolute) => 1,
        _ => u32::MAX,
    };
    (season, i64::from(offset), anidb)
}

/// The AniDB side of one work: lookups and franchise positions.
struct Franchise<'a> {
    index: &'a MappingIndex,
    tmdb: Option<u32>,
    /// TVDB ids with the confidence of a row found through them.
    tvdb: Vec<(u32, f64)>,
    /// AniDB id → entry number in the franchise (1, 2…).
    entry: HashMap<u32, i64>,
}

impl<'a> Franchise<'a> {
    fn new(index: &'a MappingIndex, tmdb: Option<u32>, tvdb: Option<u32>) -> Self {
        let mut tvdb_ids = Vec::new();
        if let Some(id) = tvdb {
            tvdb_ids.push((id, CONFIDENT));
        } else if let Some(id) = tmdb {
            let shared: HashSet<u32> = index
                .get(IdRef::TmdbTv(id))
                .filter_map(|r| r.ids.tvdb)
                .collect();
            if shared.len() == 1 {
                tvdb_ids.extend(shared.into_iter().map(|t| (t, BORROWED_TVDB)));
            }
        }
        let mut records: Vec<&Record> = tmdb
            .into_iter()
            .flat_map(|id| index.get(IdRef::TmdbTv(id)))
            .chain(
                tvdb_ids
                    .iter()
                    .flat_map(|(id, _)| index.get(IdRef::Tvdb(*id))),
            )
            .filter(|r| r.ids.anidb.is_some() && r.anime_list.is_some())
            .collect();
        records.sort_by_key(|r| position(r, tmdb));
        records.dedup_by_key(|r| r.ids.anidb);
        let mut entry = HashMap::new();
        for r in records {
            if let Some(anidb) = r.ids.anidb {
                let next = i64::try_from(entry.len()).unwrap_or(i64::MAX - 1) + 1;
                entry.entry(anidb).or_insert(next);
            }
        }
        Self {
            index,
            tmdb,
            tvdb: tvdb_ids,
            entry,
        }
    }

    fn is_empty(&self) -> bool {
        self.entry.is_empty()
    }

    /// AniDB number of the episode `season`x`episode` (TMDB numbering).
    fn find(&self, season: u32, episode: u32, absolute: Option<u32>) -> Option<Found> {
        let target = Target::Season { season, episode };
        let found = |r: pa_anime_mappings::EpisodeRef, confidence| Found {
            anidb: r.anidb,
            season: r.season,
            episode: r.episode,
            confidence,
        };
        if let Some(tmdb) = self.tmdb {
            let hit = self.index.tmdb_to_anidb(tmdb, target).or_else(|| {
                absolute
                    .filter(|_| season >= 1)
                    .and_then(|a| self.index.tmdb_to_anidb(tmdb, Target::Absolute { episode: a }))
            });
            if let Some(r) = hit {
                return Some(found(r, CONFIDENT));
            }
        }
        self.tvdb.iter().find_map(|&(tvdb, confidence)| {
            self.index
                .tvdb_to_anidb(tvdb, target)
                .map(|r| found(r, confidence))
        })
    }

    /// (row season, row episode) of a find: entry number or 0 for specials.
    fn numbers(&self, f: Found) -> Option<(i64, i64)> {
        let season = if f.season == 0 {
            0
        } else {
            *self.entry.get(&f.anidb)?
        };
        Some((season, i64::from(f.episode)))
    }
}

fn ext_id(conn: &Connection, work: i64, provider: &str, kind: &str) -> Result<Option<u32>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT ext_id FROM external_ids WHERE work_id = ?1 AND provider = ?2 AND ext_kind = ?3",
    )?;
    let ids = stmt.query_map(params![work, provider, kind], |r| r.get::<_, String>(0))?;
    for id in ids {
        if let Ok(id) = id?.trim().parse() {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Writes the AniDB rows of `work`; returns how many.
fn write_work(conn: &Connection, index: &MappingIndex, work: i64) -> Result<usize, Error> {
    let tmdb = ext_id(conn, work, "tmdb", "tv")?;
    let tvdb = ext_id(conn, work, "tvdb", "series")?;
    let franchise = Franchise::new(index, tmdb, tvdb);
    if franchise.is_empty() {
        return Ok(0);
    }
    let episodes: Vec<(i64, i64, i64, Option<i64>)> = {
        let mut stmt = conn.prepare_cached(
            "SELECT id, season_number, episode_number, absolute_number FROM episodes
             WHERE work_id = ?1 AND season_number IS NOT NULL AND episode_number IS NOT NULL",
        )?;
        stmt.query_map([work], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?
    };
    let mut insert = conn.prepare_cached(
        "INSERT OR IGNORE INTO episode_mappings
             (episode_id, scheme, season_number, episode_number, source, confidence)
         VALUES (?1, 'anidb', ?2, ?3, ?4, ?5)",
    )?;
    let mut written = 0;
    for (id, season, episode, absolute) in episodes {
        let (Ok(season), Ok(episode)) = (u32::try_from(season), u32::try_from(episode)) else {
            continue;
        };
        let absolute = absolute.and_then(|a| u32::try_from(a).ok());
        let Some(found) = franchise.find(season, episode, absolute) else {
            continue;
        };
        let Some((s, e)) = franchise.numbers(found) else {
            continue;
        };
        written += insert.execute(params![id, s, e, SOURCE, found.confidence])?;
    }
    Ok(written)
}

/// Rewrites every `anime_lists` row from `index`. An index without any
/// Anime-Lists entry leaves the rows alone (no list yet).
pub(crate) fn apply(conn: &mut Connection, index: &MappingIndex) -> Result<usize, Error> {
    if !index.records().iter().any(|r| r.anime_list.is_some()) {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM episode_mappings WHERE scheme = 'anidb' AND source = ?1",
        [SOURCE],
    )?;
    let works: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT x.work_id FROM external_ids x JOIN works w ON w.id = x.work_id
             WHERE (x.provider = 'tmdb' AND x.ext_kind = 'tv')
                OR (x.provider = 'tvdb' AND x.ext_kind = 'series')
             ORDER BY x.work_id",
        )?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    let mut written = 0;
    for work in works {
        written += write_work(&tx, index, work)?;
    }
    tx.commit()?;
    Ok(written)
}

impl Catalog {
    /// Writes the AniDB episode numbers of every anime the catalog knows by
    /// TMDB or TVDB id from the Anime-Lists `index` (see the
    /// [module docs](crate::anime_order)), replacing the rows of an older
    /// list. Returns the number of rows written. Local only, no network.
    pub fn apply_anime_mappings(&self, index: &MappingIndex) -> Result<usize, Error> {
        apply(&mut self.lock(), index)
    }
}
