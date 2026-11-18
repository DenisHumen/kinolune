// SPDX-License-Identifier: MPL-2.0
//! Additional providers of the metadata run (META-012, ADR-0132).
//!
//! They work by themselves whenever they can — no setup but the optional
//! TheTVDB key and the optional personal Fanart.tv key:
//!
//! - **Daily shows by date** (A11): an episode named by its air date
//!   (`The.Late.Desk.2025.03.14.Guest…`) is one catalog row with that
//!   `air_date`. A show TMDB matched gets the TMDB season that covers the
//!   date, and the row takes the numbers, title and still of TMDB's episode
//!   of that day ([`seasons_for_dates`], [`apply_tmdb_dates`]). Dates TMDB
//!   does not have (or every date, without a TMDB key) are asked from
//!   **TVmaze** (keyless) and then **TheTVDB** (only with the user's key):
//!   the row gets the episode's title, overview and runtime; TheTVDB's
//!   aired numbers go to `episode_mappings` (`tvdb_aired`). The show is
//!   found by the TheTVDB / IMDb id TMDB or an NFO file gives, else by a
//!   title search whose result must carry the same title.
//! - **Fanart.tv**: a matched movie (by TMDB id) or series (by the
//!   TheTVDB id TMDB links) gets a clear logo (`artwork.kind = 'logo'`)
//!   and clear art (`clearart`) in the metadata language, when the build
//!   has the project key or the user a personal key.
//! - **OpenSubtitles moviehash → id**: a movie or show TMDB could not find
//!   by its names is looked up by the OSHash of its files (subtitles
//!   uploaded for that very file name its TMDB id), when the build has the
//!   OpenSubtitles key and TMDB a key. Only the 16-digit hash is sent.
//!
//! Failures of these providers never fail a work: what they would add is
//! left out and asked again on a later run.

use std::collections::HashSet;
use std::sync::Arc;

use pa_library::rusqlite::{Transaction, params};
use pa_metadata::cache::ProviderCache;
use pa_metadata::fanart::{self, Art, FanartClient, Target};
use pa_metadata::opensubtitles::{HashMatch, OpenSubtitlesClient};
use pa_metadata::tmdb::{Details, MediaKind, Season, TmdbClient};
use pa_metadata::tvdb::TvdbClient;
use pa_metadata::tvmaze::{self, Lookup, TvMazeClient};
use pa_recognition::title::TitleKey;
use serde_json::json;

use super::{WorkInfo, non_empty, set_artwork, valid_date};
use crate::Error;

const T: &str = "pa_catalog::metadata";

/// Hashes of a work's files asked from OpenSubtitles at most.
const MAX_HASHES: usize = 2;

/// Confidence of a TMDB id that OpenSubtitles named for the file's hash:
/// [`super::AUTO`] and above when at least two subtitles made for that very
/// file agree and none disagrees, `auto_review` otherwise.
const HASH_SURE: f64 = 0.93;
const HASH_UNSURE: f64 = 0.8;

/// The META-012 providers of a run; each is optional.
#[derive(Clone, Default)]
pub struct ExtraProviders {
    /// TVmaze (keyless): daily shows by date.
    pub tvmaze: Option<Arc<TvMazeClient>>,
    /// TheTVDB, only with the user's own key.
    pub tvdb: Option<Arc<TvdbClient>>,
    /// Fanart.tv, with the project key and/or the user's personal key.
    pub fanart: Option<Arc<FanartClient>>,
    /// OpenSubtitles with the project key: moviehash → TMDB id.
    pub opensubtitles: Option<Arc<OpenSubtitlesClient>>,
}

impl std::fmt::Debug for ExtraProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtraProviders")
            .field("tvmaze", &self.tvmaze.is_some())
            .field("tvdb", &self.tvdb.is_some())
            .field("fanart", &self.fanart.is_some())
            .field("opensubtitles", &self.opensubtitles.is_some())
            .finish()
    }
}

impl ExtraProviders {
    /// Live clients sharing `cache`: TVmaze always; TheTVDB with
    /// `tvdb_key` (and `tvdb_pin`, if the key needs one); Fanart.tv with the build's project key or
    /// `fanart_personal_key`; OpenSubtitles with the build's project key.
    pub fn live(
        cache: Arc<dyn ProviderCache>,
        tvdb_key: Option<&str>,
        tvdb_pin: Option<&str>,
        fanart_personal_key: Option<&str>,
    ) -> Self {
        let tvdb = TvdbClient::live(Default::default(), tvdb_key, cache.clone()).with_pin(tvdb_pin);
        let fanart = FanartClient::live(Default::default(), fanart_personal_key, cache.clone());
        let opensubtitles = OpenSubtitlesClient::live(None, None);
        Self {
            tvmaze: Some(Arc::new(TvMazeClient::live(Default::default(), cache))),
            tvdb: tvdb.has_key().then(|| Arc::new(tvdb)),
            fanart: fanart.key_source().is_some().then(|| Arc::new(fanart)),
            opensubtitles: opensubtitles
                .key_source()
                .is_some()
                .then(|| Arc::new(opensubtitles)),
        }
    }

    /// TVmaze or TheTVDB can resolve dated episodes.
    pub(crate) fn dates(&self) -> bool {
        self.tvmaze.is_some() || self.tvdb.is_some()
    }
}

/// The episode a provider aired on a date.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DatedEpisode {
    pub date: String,
    /// `tvmaze` or `tvdb`.
    pub provider: &'static str,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub runtime_min: Option<u32>,
    /// TheTVDB aired order.
    pub tvdb_numbers: Option<(u32, u32)>,
}

/// What the META-012 providers found for a work.
#[derive(Debug, Clone, Default)]
pub(crate) struct Extras {
    pub dated: Vec<DatedEpisode>,
    /// `(provider, ext_kind, id)`.
    pub ext_ids: Vec<(&'static str, &'static str, String)>,
    pub art: Option<Art>,
}

/// TMDB seasons (not specials) that cover `dates`: the one with the latest
/// first air date on or before each date.
pub(crate) fn seasons_for_dates(details: &Details, dates: &[String]) -> Vec<u32> {
    let mut seasons: Vec<(&str, u32)> = details
        .seasons
        .iter()
        .filter(|s| s.season_number > 0)
        .filter_map(|s| Some((valid_date(s.air_date.as_deref())?, s.season_number)))
        .collect();
    seasons.sort_unstable();
    let mut out: Vec<u32> = dates
        .iter()
        .filter_map(|d| {
            seasons
                .iter()
                .rev()
                .find(|(start, _)| *start <= d.as_str())
                .map(|(_, n)| *n)
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Dated rows of `work` take the numbers of TMDB's episode of their date
/// (the first one, when TMDB lists several that day); the TMDB episode
/// data is then written onto them like onto any numbered row.
pub(crate) fn apply_tmdb_dates(
    tx: &Transaction<'_>,
    work: i64,
    seasons: &[Season],
) -> Result<usize, Error> {
    let mut n = 0;
    for season in seasons {
        for ep in &season.episodes {
            let Some(date) = valid_date(ep.air_date.as_deref()) else {
                continue;
            };
            n += tx.execute(
                "UPDATE episodes SET season_number = ?3, episode_number = ?4
                 WHERE id = (SELECT id FROM episodes
                             WHERE work_id = ?1 AND air_date = ?2 AND episode_type = 'regular'
                               AND season_number IS NULL AND episode_number IS NULL
                               AND absolute_number IS NULL
                             ORDER BY id LIMIT 1)",
                params![work, date, ep.season_number, ep.episode_number],
            )?;
        }
    }
    Ok(n)
}

/// Writes what [`fetch`] found: external ids, dated episode data, Fanart.tv
/// art (old Fanart.tv art of the work is replaced).
pub(crate) fn apply(tx: &Transaction<'_>, work: i64, extras: &Extras) -> Result<(), Error> {
    for (provider, kind, id) in &extras.ext_ids {
        tx.execute(
            "INSERT OR IGNORE INTO external_ids (work_id, provider, ext_kind, ext_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![work, provider, kind, id],
        )?;
    }
    for d in &extras.dated {
        let rows: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM episodes WHERE work_id = ?1 AND air_date = ?2
                   AND episode_type = 'regular'",
            )?;
            stmt.query_map(params![work, d.date], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for id in rows {
            tx.execute(
                "UPDATE episodes SET title = ifnull(?2, title), overview = ifnull(?3, overview),
                     runtime_ms = ifnull(runtime_ms, ?4)
                 WHERE id = ?1",
                params![
                    id,
                    non_empty(d.title.as_deref()),
                    non_empty(d.overview.as_deref()),
                    d.runtime_min
                        .filter(|m| *m > 0)
                        .map(|m| i64::from(m) * 60_000),
                ],
            )?;
            if let Some((season, number)) = d.tvdb_numbers {
                tx.execute(
                    "DELETE FROM episode_mappings WHERE episode_id = ?1 AND scheme = 'tvdb_aired'",
                    [id],
                )?;
                tx.execute(
                    "INSERT INTO episode_mappings (episode_id, scheme, season_number, episode_number,
                                                   source, confidence)
                     VALUES (?1, 'tvdb_aired', ?2, ?3, 'tvdb', 1.0)",
                    params![id, season, number],
                )?;
            }
        }
    }
    if let Some(art) = &extras.art {
        tx.execute(
            "DELETE FROM artwork WHERE work_id = ?1 AND provider = ?2",
            params![work, fanart::PROVIDER],
        )?;
        let lang = |i: &fanart::Image| {
            if i.lang.len() == 2 && i.lang != "00" {
                i.lang.clone()
            } else {
                String::new()
            }
        };
        for (kind, image) in [("logo", &art.logo), ("clearart", &art.clearart)] {
            if let Some(i) = image {
                set_artwork(
                    tx,
                    Some(work),
                    None,
                    kind,
                    fanart::PROVIDER,
                    Some(i.url.clone()),
                    &lang(i),
                    0,
                )?;
            }
        }
    }
    Ok(())
}

/// Normalised form of a title, for "the same title" checks.
fn key(title: &str) -> String {
    TitleKey::new(title).loose
}

/// The work is called `name`.
fn named(work: &WorkInfo, name: &str) -> bool {
    let name = key(name);
    !name.is_empty() && work.titles.iter().any(|t| key(t) == name)
}

/// A TMDB id for a work TMDB did not find by its names: the movie or show
/// most subtitles made for one of its files name. Returns the id, the
/// confidence and what to explain.
pub(crate) fn hash_match(
    client: &OpenSubtitlesClient,
    work: &WorkInfo,
) -> Option<(u64, f64, serde_json::Value)> {
    let movie = work.kind == "movie";
    for hash in work.hashes.iter().take(MAX_HASHES) {
        let found: HashMatch = match client.identify(hash) {
            Ok(Some(m)) => m,
            Ok(None) => continue,
            Err(e) => {
                tracing::debug!(target: T, work = work.id, "OpenSubtitles hash lookup: {e}");
                return None;
            }
        };
        // A movie file names a movie, an episode file a show.
        if found.episode == movie {
            continue;
        }
        let confidence = if found.votes >= 2 && found.against == 0 {
            HASH_SURE
        } else {
            HASH_UNSURE
        };
        let explain = json!({
            "tmdb": found.tmdb_id,
            "votes": found.votes,
            "against": found.against,
            "title": found.title,
            "year": found.year,
        });
        return Some((found.tmdb_id, confidence, explain));
    }
    None
}

/// What TMDB said about a work, as far as the extras need it.
pub(crate) struct TmdbFacts<'a> {
    pub client: &'a TmdbClient,
    pub kind: MediaKind,
    pub id: u64,
    /// Air dates of the episodes of the fetched seasons.
    pub dates: HashSet<String>,
}

/// Asks the META-012 providers about `work` (see the module docs).
pub(crate) fn fetch(
    extra: &ExtraProviders,
    work: &WorkInfo,
    tmdb: Option<&TmdbFacts<'_>>,
    language: &str,
    explanation: &mut serde_json::Value,
) -> Extras {
    let mut out = Extras::default();
    let mut tvdb_id = work
        .tvdb_id
        .as_deref()
        .and_then(|s| s.trim().parse::<u64>().ok());
    let mut imdb_id = work.imdb_id.clone();
    // TMDB's links of a series: TheTVDB id (Fanart.tv, TVmaze, TheTVDB).
    let needs_links = work.kind != "movie"
        && (extra.fanart.is_some() || (!work.dates.is_empty() && extra.dates()));
    if let Some(t) = tmdb
        && t.kind == MediaKind::Tv
        && needs_links
        && let Ok(ids) = t.client.external_ids(MediaKind::Tv, t.id)
    {
        if let Some(id) = ids.tvdb_id.filter(|id| *id > 0) {
            tvdb_id.get_or_insert(id);
        }
        if let Some(id) = ids.imdb_id {
            imdb_id.get_or_insert(id);
        }
    }

    // Daily shows: the dates TMDB does not have.
    let mut open: Vec<&String> = work
        .dates
        .iter()
        .filter(|d| tmdb.is_none_or(|t| !t.dates.contains(*d)))
        .collect();
    if work.kind != "movie" && !open.is_empty() {
        if let Some(tvmaze) = extra.tvmaze.as_deref() {
            let show = tvmaze_show(tvmaze, work, tvdb_id, imdb_id.as_deref());
            if let Some(show) = show {
                out.ext_ids
                    .push((tvmaze::PROVIDER, "", show.id.to_string()));
                if let Some(id) = show.externals.thetvdb {
                    tvdb_id.get_or_insert(id);
                }
                let mut found = Vec::new();
                for date in &open {
                    match tvmaze.episodes_by_date(show.id, date) {
                        Ok(eps) => {
                            let best = eps.iter().find(|e| e.is_regular()).or(eps.first());
                            if let Some(e) = best {
                                out.dated.push(DatedEpisode {
                                    date: (*date).clone(),
                                    provider: tvmaze::PROVIDER,
                                    title: e.name.clone(),
                                    overview: e.summary.as_deref().map(tvmaze::plain_text),
                                    runtime_min: e.runtime,
                                    tvdb_numbers: None,
                                });
                                found.push((*date).clone());
                            }
                        }
                        Err(e) => {
                            tracing::debug!(target: T, work = work.id, "TVmaze episodes: {e}");
                            break;
                        }
                    }
                }
                explanation["tvmaze"] = json!({ "show": show.id, "dates": found });
                open.retain(|d| !found.contains(d));
            }
        }
        if let Some(tvdb) = extra.tvdb.as_deref()
            && !open.is_empty()
        {
            let id = tvdb_id.or_else(|| tvdb_search(tvdb, work));
            if let Some(id) = id {
                tvdb_id.get_or_insert(id);
                let mut found = Vec::new();
                for date in &open {
                    match tvdb.episodes_by_air_date(id, date) {
                        Ok(eps) => {
                            if let Some(e) = eps.first() {
                                out.dated.push(DatedEpisode {
                                    date: (*date).clone(),
                                    provider: pa_metadata::tvdb::PROVIDER,
                                    title: e.name.clone(),
                                    overview: e.overview.clone(),
                                    runtime_min: e.runtime,
                                    tvdb_numbers: e.season_number.zip(e.number),
                                });
                                found.push((*date).clone());
                            }
                        }
                        Err(e) => {
                            tracing::debug!(target: T, work = work.id, "TheTVDB episodes: {e}");
                            break;
                        }
                    }
                }
                explanation["tvdb"] = json!({ "series": id, "dates": found });
            }
        }
    }
    if work.kind != "movie"
        && work.tvdb_id.is_none()
        && let Some(id) = tvdb_id
    {
        out.ext_ids
            .push((pa_metadata::tvdb::PROVIDER, "series", id.to_string()));
    }

    // Fanart.tv art of a matched work.
    if let (Some(fanart), Some(t)) = (extra.fanart.as_deref(), tmdb) {
        let target = match t.kind {
            MediaKind::Movie => Some(Target::Movie(t.id)),
            MediaKind::Tv => tvdb_id.map(Target::Series),
        };
        if let Some(target) = target {
            match fanart.art(target, super::lang_of(language)) {
                Ok(art) if !art.is_empty() => out.art = Some(art),
                Ok(_) => {}
                Err(e) => tracing::debug!(target: T, work = work.id, "Fanart.tv: {e}"),
            }
        }
    }
    out
}

/// The TVmaze show of `work`: by TheTVDB or IMDb id, else the best search
/// hit with the same title.
fn tvmaze_show(
    client: &TvMazeClient,
    work: &WorkInfo,
    tvdb_id: Option<u64>,
    imdb_id: Option<&str>,
) -> Option<tvmaze::Show> {
    let by_id = |lookup| match client.lookup(lookup) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(target: T, work = work.id, "TVmaze lookup: {e}");
            None
        }
    };
    if let Some(s) = tvdb_id.and_then(|id| by_id(Lookup::TheTvdb(id))) {
        return Some(s);
    }
    if let Some(s) = imdb_id.and_then(|id| by_id(Lookup::Imdb(id))) {
        return Some(s);
    }
    for title in work.titles.iter().take(2) {
        match client.search(title) {
            Ok(hits) => {
                if let Some(h) = hits.into_iter().find(|h| named(work, &h.show.name)) {
                    return Some(h.show);
                }
            }
            Err(e) => {
                tracing::debug!(target: T, work = work.id, "TVmaze search: {e}");
                return None;
            }
        }
    }
    None
}

/// TheTVDB series of `work` by a title search: the first result with the
/// same title or alias.
fn tvdb_search(client: &TvdbClient, work: &WorkInfo) -> Option<u64> {
    for title in work.titles.iter().take(2) {
        match client.search_series(title, None) {
            Ok(found) => {
                let hit = found
                    .into_iter()
                    .find(|r| named(work, &r.name) || r.aliases.iter().any(|a| named(work, a)));
                if let Some(id) = hit.and_then(|r| r.id()) {
                    return Some(id);
                }
            }
            Err(e) => {
                tracing::debug!(target: T, work = work.id, "TheTVDB search: {e}");
                return None;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_cover_dates() {
        let d: Details = serde_json::from_str(
            r#"{"id":1,"seasons":[
                {"season_number":0,"episode_count":3,"air_date":"2015-01-01"},
                {"season_number":1,"episode_count":100,"air_date":"2015-09-08"},
                {"season_number":10,"episode_count":120,"air_date":"2024-09-09"},
                {"season_number":11,"episode_count":20,"air_date":"2025-09-08"}
            ]}"#,
        )
        .unwrap();
        let dates = vec![
            "2025-03-14".to_owned(),
            "2025-03-17".to_owned(),
            "2025-10-01".to_owned(),
            "2014-01-01".to_owned(),
        ];
        assert_eq!(seasons_for_dates(&d, &dates), vec![10, 11]);
    }
}
