// SPDX-License-Identifier: MPL-2.0
//! «Исправить матч» (UI-012, ADR-0128): a pasted id or page link, the
//! anime database in the manual search, and one call that applies a
//! result to the whole folder with its rule.
//!
//! - [`parse_pasted`] reads what the user pasted: a page link of TMDB,
//!   TheTVDB, IMDb, AniDB, MyAnimeList, Shikimori or AniList, a prefixed
//!   id (`tvdb 81189`, `[tmdbid-603]`, `mal:5114`), an IMDb `tt…` id or
//!   bare digits (a TMDB id of the work's kind).
//! - [`Catalog::review_lookup`] turns it into results the work can take —
//!   TMDB entries (TheTVDB and IMDb through TMDB `/find`) and, for anime
//!   ids, the AniList entry and TMDB series the anime database and the
//!   Anime-Lists list name (Shikimori ids are MAL ids). Nothing is written.
//! - [`anime_search`] adds the anime database to the manual search by
//!   title ([`Catalog::review_search`]): no request, AniDB / AniList / MAL
//!   / Shikimori titles in one index.
//! - [`Catalog::review_apply`] takes a result for the work in one step: a
//!   result of another kind switches the work's type first (a series for
//!   a «movie», AniList for an «anime»), then
//!   [`Catalog::review_choose`] decides every file of the work and teaches
//!   the folder its rule; the [`FixReport`] says how many files follow it
//!   and which folders got a rule.

use pa_anime_mappings::{AnimeIds, IdRef};
use pa_library::rusqlite::OptionalExtension;
use pa_metadata::fetch::FetchError;
use pa_metadata::tmdb::{self, ExternalSource, MediaKind, TmdbError};

use crate::items::ItemRef;
use crate::metadata::{MatchSignals, MetadataProviders, WorkQuery, title_query};
use crate::review::{POSTER_SIZE, ReviewCandidate, SEARCH_RESULTS};
use crate::{AnimeTitles, Catalog, Error};

/// An id the user pasted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PastedId {
    /// TMDB; `kind` when the link or prefix says it.
    Tmdb { kind: Option<MediaKind>, id: u64 },
    Tvdb(u64),
    /// `tt0133093`.
    Imdb(String),
    AniDb(u32),
    AniList(u32),
    Mal(u32),
    /// Shikimori ids are MAL ids.
    Shikimori(u32),
}

impl PastedId {
    /// Where the id is from: `tmdb`, `tvdb`, `imdb`, `anidb`, `anilist`,
    /// `mal`, `shikimori`.
    #[must_use]
    pub fn source(&self) -> &'static str {
        match self {
            Self::Tmdb { .. } => "tmdb",
            Self::Tvdb(_) => "tvdb",
            Self::Imdb(_) => "imdb",
            Self::AniDb(_) => "anidb",
            Self::AniList(_) => "anilist",
            Self::Mal(_) => "mal",
            Self::Shikimori(_) => "shikimori",
        }
    }
}

/// What a paste resolved to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IdLookup {
    /// [`PastedId::source`]; `None`: the text is not an id or link we read.
    pub source: Option<String>,
    /// Results the work can take, those of its own kind first; empty: the
    /// providers do not know the id.
    pub candidates: Vec<ReviewCandidate>,
}

/// What a fix did ([`Catalog::review_apply`], [`Catalog::fix_report`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixReport {
    /// The work that carries the result now.
    pub work_id: i64,
    /// `movie`, `show` or `anime` after the fix.
    pub kind: String,
    /// Video files of the work, all decided by the user or the rule.
    pub files: u32,
    /// Folders (relative to their source) whose rule follows the work:
    /// their new files join it without a provider.
    pub folders: Vec<String>,
}

/// Reads a pasted id or link. `None`: not one we know (a TheTVDB link
/// with a name instead of the number among them: its number is needed).
#[must_use]
pub fn parse_pasted(text: &str) -> Option<PastedId> {
    let t = text.trim().trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '[' | ']' | '{' | '}' | '(' | ')')
    });
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(found) = from_link(&lower) {
        return found;
    }
    from_words(&lower)
}

/// Leading digits of a path segment (`603-the-matrix` → 603).
fn leading<T: std::str::FromStr>(seg: &str) -> Option<T> {
    let end = seg.find(|c: char| !c.is_ascii_digit()).unwrap_or(seg.len());
    if end == 0 {
        return None;
    }
    seg[..end].parse().ok()
}

/// The number after `key` in the path: `/anime/5114/…`, `/animes/z5114-…`.
fn after<T: std::str::FromStr>(segs: &[&str], key: &str) -> Option<T> {
    let i = segs.iter().position(|s| *s == key)?;
    let seg = segs.get(i + 1)?;
    leading(seg.trim_start_matches(|c: char| c.is_ascii_alphabetic()))
}

/// A query parameter.
fn param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// `Some(found)` when `s` is a link of a known site.
fn from_link(s: &str) -> Option<Option<PastedId>> {
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let rest = rest
        .strip_prefix("www.")
        .or_else(|| rest.strip_prefix("m."))
        .unwrap_or(rest);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(':').next().unwrap_or(host);
    let path = path.split('#').next().unwrap_or(path);
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let found = match host {
        "themoviedb.org" | "tmdb.org" => segs.iter().enumerate().find_map(|(i, s)| {
            let kind = match *s {
                "movie" => MediaKind::Movie,
                "tv" => MediaKind::Tv,
                _ => return None,
            };
            Some(PastedId::Tmdb {
                kind: Some(kind),
                id: leading(segs.get(i + 1)?)?,
            })
        }),
        "thetvdb.com" | "tvdb.com" => param(query, "id")
            .and_then(leading::<u64>)
            .or_else(|| {
                // `/dereferrer/series/81189`; `/series/<name>` has no number.
                ["series", "movies", "movie"].iter().find_map(|k| {
                    let i = segs.iter().position(|s| s == k)?;
                    let seg = segs.get(i + 1)?;
                    seg.chars()
                        .all(|c| c.is_ascii_digit())
                        .then(|| seg.parse().ok())?
                })
            })
            .map(PastedId::Tvdb),
        "imdb.com" => segs
            .iter()
            .position(|s| *s == "title")
            .and_then(|i| segs.get(i + 1))
            .and_then(|s| imdb(s))
            .map(PastedId::Imdb),
        "anidb.net" => after::<u32>(&segs, "anime")
            .or_else(|| param(query, "aid").and_then(leading::<u32>))
            .or_else(|| {
                segs.first()
                    .and_then(|s| s.strip_prefix('a'))
                    .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
                    .and_then(|s| s.parse().ok())
            })
            .map(PastedId::AniDb),
        "myanimelist.net" => after(&segs, "anime").map(PastedId::Mal),
        "anilist.co" => after(&segs, "anime").map(PastedId::AniList),
        h if h.starts_with("shikimori.") || h == "shiki.one" => {
            after(&segs, "animes").map(PastedId::Shikimori)
        }
        _ => return None,
    };
    Some(found)
}

/// `tt0133093`.
fn imdb(s: &str) -> Option<String> {
    let digits = s.strip_prefix("tt")?;
    (digits.len() >= 5 && digits.chars().all(|c| c.is_ascii_digit())).then(|| s.to_owned())
}

/// `tvdb 81189`, `[tmdbid-603]`, `tmdb movie 603`, `mal:5114`, `tt0133093`,
/// `603`.
fn from_words(s: &str) -> Option<PastedId> {
    let words: Vec<&str> = s
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    match words.as_slice() {
        [one] if one.chars().all(|c| c.is_ascii_digit()) => Some(PastedId::Tmdb {
            kind: None,
            id: one.parse().ok()?,
        }),
        [one] => imdb(one).map(PastedId::Imdb),
        [name, rest @ ..] => {
            let last = rest.last()?;
            let kind = rest.first().and_then(|k| match *k {
                "movie" | "film" => Some(MediaKind::Movie),
                "tv" | "series" | "show" => Some(MediaKind::Tv),
                _ => None,
            });
            if rest.len() > 2 || (rest.len() == 2 && kind.is_none()) {
                return None;
            }
            let n = || last.parse::<u32>().ok();
            match name.trim_end_matches("id") {
                "tmdb" | "themoviedb" => Some(PastedId::Tmdb {
                    kind,
                    id: last.parse().ok()?,
                }),
                "tvdb" | "thetvdb" => last.parse().ok().map(PastedId::Tvdb),
                "imdb" => imdb(last).map(PastedId::Imdb),
                "anidb" => n().map(PastedId::AniDb),
                "anilist" => n().map(PastedId::AniList),
                "mal" | "myanimelist" => n().map(PastedId::Mal),
                "shikimori" | "shiki" => n().map(PastedId::Shikimori),
                _ => None,
            }
        }
        [] => None,
    }
}

fn kind_name(k: MediaKind) -> &'static str {
    match k {
        MediaKind::Movie => "movie",
        MediaKind::Tv => "tv",
    }
}

/// A candidate found by its id.
fn by_id(
    provider: &str,
    ext_kind: &str,
    ext_id: String,
    title: String,
    source: &str,
    file_kind: &str,
) -> ReviewCandidate {
    ReviewCandidate {
        provider: provider.to_owned(),
        ext_kind: ext_kind.to_owned(),
        ext_id,
        title,
        original_title: None,
        year: None,
        poster_url: None,
        overview: None,
        score: 1.0,
        signals: MatchSignals {
            provider: source.to_owned(),
            title: 1.0,
            tier: "id".to_owned(),
            file_kind: file_kind.to_owned(),
            kind: if ext_kind.is_empty() { "tv" } else { ext_kind }.to_owned(),
            ..Default::default()
        },
        current: false,
    }
}

/// A TMDB entry as a candidate; with the client its title, year, poster
/// and overview, else `fallback` for the title.
fn tmdb_candidate(
    client: Option<&tmdb::TmdbClient>,
    kind: MediaKind,
    id: u64,
    fallback: Option<&str>,
    source: &str,
    file_kind: &str,
) -> Result<Option<ReviewCandidate>, Error> {
    let mut c = by_id(
        "tmdb",
        kind_name(kind),
        id.to_string(),
        fallback.map_or_else(|| format!("TMDB {id}"), str::to_owned),
        source,
        file_kind,
    );
    let Some(client) = client else {
        return Ok(Some(c));
    };
    let d = match client.details(kind, id) {
        Ok(d) => d,
        Err(TmdbError::NotFound(_)) => return Ok(fallback.map(|_| c)),
        Err(e) => return Err(Error::Provider(format!("TMDB: {e}"))),
    };
    if let Some(t) = d.title.clone().or_else(|| d.original_title.clone()) {
        c.title = t;
    }
    c.original_title = d.original_title.clone().filter(|o| *o != c.title);
    c.year = tmdb::year_of(d.release_date.as_deref()).map(i64::from);
    c.signals.year = c.year;
    c.poster_url = d
        .poster_path
        .as_deref()
        .map(|p| tmdb::image_url(POSTER_SIZE, p));
    c.overview = d.overview.clone().filter(|o| !o.trim().is_empty());
    Ok(Some(c))
}

/// Results of the anime database for a manual search (no request): the
/// AniList entries whose titles fit, best first. For works that are not
/// anime only close titles count.
pub(crate) fn anime_search(titles: &AnimeTitles, query: &WorkQuery) -> Vec<ReviewCandidate> {
    let floor = if query.kind == "anime" { 0.6 } else { 0.85 };
    let mut out = Vec::new();
    for c in titles.index().search(&title_query(query), 5) {
        let score = f64::from(c.score);
        if score < floor {
            continue;
        }
        let Some(e) = titles.entry_at(c.record) else {
            continue;
        };
        let Some(anilist) = e.ids.anilist else {
            continue;
        };
        let movie = e.kind == "MOVIE";
        out.push(ReviewCandidate {
            provider: "anilist".to_owned(),
            ext_kind: String::new(),
            ext_id: anilist.to_string(),
            title: e.title.clone(),
            original_title: None,
            year: e.year.map(i64::from),
            poster_url: None,
            overview: None,
            score: (score * 1000.0).round() / 1000.0,
            signals: MatchSignals {
                provider: "anime_offline_database".to_owned(),
                title: (score * 1000.0).round() / 1000.0,
                tier: format!("{:?}", c.tier).to_lowercase(),
                matched_title: titles.index().record(c.record).titles.get(c.title).cloned(),
                file_year: query.year,
                year: e.year.map(i64::from),
                file_kind: query.kind.clone(),
                kind: if movie { "movie" } else { "tv" }.to_owned(),
                ..Default::default()
            },
            current: false,
        });
    }
    out
}

impl Catalog {
    /// The results a pasted id or link names, for `work` (nothing is
    /// written; [`review_apply`](Self::review_apply) takes one).
    pub fn review_lookup(
        &self,
        providers: &MetadataProviders,
        work: i64,
        text: &str,
    ) -> Result<IdLookup, Error> {
        let kind: String = {
            let conn = self.lock();
            conn.query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
                .optional()?
                .ok_or_else(|| Error::NotFound(format!("work {work}")))?
        };
        let Some(id) = parse_pasted(text) else {
            return Ok(IdLookup::default());
        };
        let source = id.source();
        let tmdb = providers.tmdb();
        let need_tmdb = || tmdb.ok_or_else(|| Error::Provider("TMDB has no key".into()));
        let mut out: Vec<ReviewCandidate> = Vec::new();
        match &id {
            PastedId::Tmdb { kind: k, id: n } => {
                let client = need_tmdb()?;
                let kinds = match k {
                    Some(k) => vec![*k],
                    // Bare digits: the work's kind first, the other one if
                    // TMDB has no such entry of it.
                    None if kind == "movie" => vec![MediaKind::Movie, MediaKind::Tv],
                    None => vec![MediaKind::Tv, MediaKind::Movie],
                };
                for k in kinds {
                    if let Some(c) = tmdb_candidate(Some(client), k, *n, None, source, &kind)? {
                        out.push(c);
                        break;
                    }
                }
            }
            PastedId::Tvdb(_) | PastedId::Imdb(_) => {
                let client = need_tmdb()?;
                let (ext, src) = match &id {
                    PastedId::Tvdb(n) => (n.to_string(), ExternalSource::Tvdb),
                    PastedId::Imdb(s) => (s.clone(), ExternalSource::Imdb),
                    _ => unreachable!("matched above"),
                };
                let found = client
                    .find(&ext, src)
                    .map_err(|e| Error::Provider(format!("TMDB: {e}")))?;
                for (k, results) in [
                    (MediaKind::Tv, &found.tv_results),
                    (MediaKind::Movie, &found.movie_results),
                ] {
                    for r in results.iter().take(SEARCH_RESULTS) {
                        let mut c = by_id(
                            "tmdb",
                            kind_name(k),
                            r.id.to_string(),
                            r.title
                                .clone()
                                .or_else(|| r.original_title.clone())
                                .unwrap_or_else(|| format!("TMDB {}", r.id)),
                            source,
                            &kind,
                        );
                        c.original_title = r.original_title.clone().filter(|o| *o != c.title);
                        c.year = r.year().map(i64::from);
                        c.signals.year = c.year;
                        c.poster_url = r
                            .poster_path
                            .as_deref()
                            .map(|p| tmdb::image_url(POSTER_SIZE, p));
                        c.overview = r.overview.clone().filter(|o| !o.trim().is_empty());
                        out.push(c);
                    }
                }
            }
            PastedId::AniDb(_) | PastedId::AniList(_) | PastedId::Mal(_) | PastedId::Shikimori(_) => {
                anime_lookup(providers, &id, &kind, &mut out)?;
            }
        }
        // Results of the work's own kind first (the others switch its type
        // when taken).
        let fits = |c: &ReviewCandidate| match c.provider.as_str() {
            "anilist" => kind == "anime",
            _ => (c.ext_kind == "movie") == (kind == "movie"),
        };
        out.sort_by_key(|c| !fits(c));
        Ok(IdLookup {
            source: Some(source.to_owned()),
            candidates: out,
        })
    }

    /// Takes a result for `work` in one step: a result of another kind
    /// switches the work's type first ([`set_work_kind`](Self::set_work_kind)),
    /// then [`review_choose`](Self::review_choose) decides every file of
    /// the work and teaches its folder the rule. Returns what follows the
    /// fix now.
    pub fn review_apply(
        &self,
        work: i64,
        provider: &str,
        ext_kind: &str,
        ext_id: &str,
    ) -> Result<FixReport, Error> {
        let kind: String = {
            let conn = self.lock();
            conn.query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
                .optional()?
                .ok_or_else(|| Error::NotFound(format!("work {work}")))?
        };
        let want = match (provider, ext_kind) {
            ("anilist", _) => "anime",
            ("tmdb", "movie") => "movie",
            ("tmdb", "tv") if kind == "movie" => "show",
            _ => kind.as_str(),
        };
        if want != kind {
            let item = if kind == "movie" {
                ItemRef::Movie(work)
            } else {
                ItemRef::Show(work)
            };
            self.set_work_kind(item, want)?;
        }
        let target = self.review_choose(work, provider, ext_kind, ext_id)?;
        self.fix_report(target)
    }

    /// What follows `work`'s decision now: its files and the folders whose
    /// rule names it.
    pub fn fix_report(&self, work: i64) -> Result<FixReport, Error> {
        let conn = self.lock();
        let kind: String = conn
            .query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("work {work}")))?;
        let files: u32 = conn.query_row(
            "SELECT count(DISTINCT m.file_id) FROM matches m JOIN files f ON f.id = m.file_id
             WHERE m.work_id = ?1 AND m.state <> 'rejected' AND f.missing_since_ms IS NULL",
            [work],
            |r| r.get(0),
        )?;
        let mut stmt = conn.prepare(
            "SELECT trim(path_prefix, '/') FROM folder_rules WHERE work_id = ?1 ORDER BY id",
        )?;
        let folders = stmt
            .query_map([work], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(FixReport {
            work_id: work,
            kind,
            files,
            folders,
        })
    }
}

/// An anime id: the anime database and the Anime-Lists list name the
/// AniList entry and the TMDB series or film; AniList answers (title,
/// cover) when it can.
fn anime_lookup(
    providers: &MetadataProviders,
    id: &PastedId,
    kind: &str,
    out: &mut Vec<ReviewCandidate>,
) -> Result<(), Error> {
    let source = id.source();
    let key = match *id {
        PastedId::AniDb(n) => IdRef::AniDb(n),
        PastedId::AniList(n) => IdRef::AniList(n),
        PastedId::Mal(n) | PastedId::Shikimori(n) => IdRef::Mal(n),
        _ => return Ok(()),
    };
    let mut ids = AnimeIds::default();
    match key {
        IdRef::AniDb(n) => ids.anidb = Some(n),
        IdRef::AniList(n) => ids.anilist = Some(n),
        IdRef::Mal(n) => ids.mal = Some(n),
        _ => {}
    }
    let mut title: Option<String> = None;
    let mut year: Option<i64> = None;
    if let Some(e) = providers.anime_titles.as_deref().and_then(|t| t.find(key)) {
        ids.fill_from(&e.ids);
        title = Some(e.title.clone());
        year = e.year.map(i64::from);
    }
    if let Some(m) = providers.anime_mappings.as_deref() {
        // Every id of the entry may lead to the record with the TMDB ids.
        let record = m
            .find(key)
            .or_else(|| ids.anidb.and_then(|a| m.find(IdRef::AniDb(a))));
        if let Some(r) = record {
            ids.fill_from(&r.ids);
            if title.is_none() {
                title.clone_from(&r.title);
            }
            if let Some(l) = &r.anime_list {
                ids.tmdb_tv = ids.tmdb_tv.or(l.tmdb_tv);
                ids.tmdb_movie = ids.tmdb_movie.or_else(|| l.tmdb_movie.first().copied());
            }
        }
    }
    let mut cover: Option<String> = None;
    let mut overview: Option<String> = None;
    if let Some(al) = providers.anilist.as_deref() {
        let media = match (ids.anilist, ids.mal) {
            (Some(a), _) => Some(al.media(u64::from(a))),
            (None, Some(m)) => Some(al.media_by_mal(u64::from(m))),
            _ => None,
        };
        match media {
            Some(Ok(m)) => {
                ids.anilist = u32::try_from(m.id).ok().or(ids.anilist);
                let t = &m.title;
                if let Some(x) = t.romaji.clone().or_else(|| t.english.clone()) {
                    title = Some(x);
                }
                year = year.or(m.year().map(i64::from));
                cover = m.cover_image.as_ref().and_then(|c| c.large.clone());
                overview = m
                    .description
                    .as_deref()
                    .map(pa_metadata::shikimori::strip_markup)
                    .filter(|d| !d.trim().is_empty());
            }
            Some(Err(FetchError::NotFound { .. })) | None => {}
            Some(Err(e)) => {
                // Offline: the local databases still name the entry.
                tracing::info!(target: "pa_catalog::review", error = %e, "AniList lookup failed");
            }
        }
    }
    let tmdb = providers.tmdb();
    let tmdb_found = match (ids.tmdb_tv, ids.tmdb_movie) {
        (Some(tv), _) => Some((MediaKind::Tv, u64::from(tv))),
        (None, Some(m)) => Some((MediaKind::Movie, u64::from(m))),
        _ => None,
    };
    if let Some((k, n)) = tmdb_found
        && let Some(c) = tmdb_candidate(tmdb, k, n, title.as_deref(), source, kind)?
    {
        out.push(c);
    }
    if let Some(a) = ids.anilist {
        let mut c = by_id(
            "anilist",
            "",
            a.to_string(),
            title.clone().unwrap_or_else(|| format!("AniList {a}")),
            source,
            kind,
        );
        c.year = year;
        c.signals.year = year;
        c.poster_url = cover;
        c.overview = overview;
        out.push(c);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmdb(kind: Option<MediaKind>, id: u64) -> Option<PastedId> {
        Some(PastedId::Tmdb { kind, id })
    }

    #[test]
    fn links_of_every_site() {
        let cases = [
            (
                "https://www.themoviedb.org/movie/603-the-matrix?language=ru",
                tmdb(Some(MediaKind::Movie), 603),
            ),
            (
                "themoviedb.org/tv/1399-game-of-thrones/season/1",
                tmdb(Some(MediaKind::Tv), 1399),
            ),
            (
                "https://thetvdb.com/?tab=series&id=81189",
                Some(PastedId::Tvdb(81189)),
            ),
            (
                "https://thetvdb.com/dereferrer/series/424536",
                Some(PastedId::Tvdb(424536)),
            ),
            ("https://thetvdb.com/series/frieren", None),
            (
                "https://www.imdb.com/title/tt0133093/?ref_=nv",
                Some(PastedId::Imdb("tt0133093".into())),
            ),
            ("https://anidb.net/anime/17617", Some(PastedId::AniDb(17617))),
            ("https://anidb.net/a69", Some(PastedId::AniDb(69))),
            (
                "https://anidb.net/perl-bin/animedb.pl?show=anime&aid=23",
                Some(PastedId::AniDb(23)),
            ),
            (
                "https://myanimelist.net/anime/52991/Sousou_no_Frieren",
                Some(PastedId::Mal(52991)),
            ),
            (
                "https://shikimori.one/animes/z52991-sousou-no-frieren",
                Some(PastedId::Shikimori(52991)),
            ),
            (
                "shikimori.me/animes/5114-fullmetal-alchemist",
                Some(PastedId::Shikimori(5114)),
            ),
            (
                "https://anilist.co/anime/154587/Sousou-no-Frieren/",
                Some(PastedId::AniList(154587)),
            ),
            ("https://example.com/anime/1", None),
        ];
        for (text, want) in cases {
            assert_eq!(parse_pasted(text), want, "{text}");
        }
    }

    #[test]
    fn prefixed_and_bare_ids() {
        let cases = [
            ("603", tmdb(None, 603)),
            ("  tmdb 603 ", tmdb(None, 603)),
            ("[tmdbid-603]", tmdb(None, 603)),
            ("{tmdb-1399}", tmdb(None, 1399)),
            ("tmdb movie 603", tmdb(Some(MediaKind::Movie), 603)),
            ("TMDB:tv:1399", tmdb(Some(MediaKind::Tv), 1399)),
            ("tvdb 81189", Some(PastedId::Tvdb(81189))),
            ("tvdbid-81189", Some(PastedId::Tvdb(81189))),
            ("tt0133093", Some(PastedId::Imdb("tt0133093".into()))),
            ("imdb tt0133093", Some(PastedId::Imdb("tt0133093".into()))),
            ("anidb:69", Some(PastedId::AniDb(69))),
            ("MAL 5114", Some(PastedId::Mal(5114))),
            ("shiki 5114", Some(PastedId::Shikimori(5114))),
            ("anilist 154587", Some(PastedId::AniList(154587))),
            ("Sousou no Frieren", None),
            ("", None),
            ("tmdb", None),
        ];
        for (text, want) in cases {
            assert_eq!(parse_pasted(text), want, "{text}");
        }
    }
}
