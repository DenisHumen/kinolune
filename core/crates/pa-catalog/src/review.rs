// SPDX-License-Identifier: MPL-2.0
//! The review inbox («На проверку», META-016).
//!
//! The metadata run decides a work by the score of its best provider
//! result ([`crate::metadata`]):
//!
//! - from [`AUTO`](crate::metadata::AUTO) — `auto`: taken silently;
//! - from [`REVIEW`](crate::metadata::REVIEW) up to `AUTO` (an untied result) — `auto_review`: the
//!   result's title and poster are shown with a «проверьте» mark;
//! - below — the manual queue: no provider data, `work_metadata.outcome =
//!   'not_found'` (an LLM fallback, META-020, is optional and off).
//!
//! Both kinds of works are the inbox ([`Catalog::review_items`]), each with
//! the results the search weighed (`work_candidates`, best first, with the
//! signals behind each score — [`MatchSignals`]). The user's answers are
//! final: their matches become `user_confirmed` or `user_set`, which no
//! later scan, parser version or scorer model decides again
//! (`numbering::recomputable!`):
//!
//! - [`Catalog::review_confirm`] — «Верно»: the shown result stays;
//! - [`Catalog::review_choose`] — another candidate, or a result of
//!   [`Catalog::review_search`] (manual search by title and year): the
//!   provider data of the old result is removed and the next metadata run
//!   reads the chosen one by its id (two works that turn out to be one are
//!   merged);
//! - both teach the work's folder a rule with release templates
//!   ([`crate::folder_rules`], META-017): the other files of the folder and
//!   new ones follow the decision without a provider;
//! - [`Catalog::review_not_media`] — «Это не фильм»: the files stay in the
//!   library under their own names and are never sent to a provider again.
//!   Moving them to personal media is UI-025
//!   ([`Catalog::move_to_personal`]).

use pa_library::rusqlite::{Connection, OptionalExtension, Transaction, params};
use pa_metadata::tmdb::{self, MediaKind, TmdbError};

use crate::items::ItemRef;
use crate::metadata::{MatchSignals, MetadataProviders, ScoredResult, search_ranked};
use crate::{Catalog, Error};

/// TMDB poster size of candidates.
pub(crate) const POSTER_SIZE: &str = "w342";

/// Results of a manual search.
pub(crate) const SEARCH_RESULTS: usize = 10;

/// Why a work is in the inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    /// `auto_review`: a result is shown, «проверьте».
    Check,
    /// No result was good enough: the user decides.
    Manual,
}

impl ReviewState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Manual => "manual",
        }
    }
}

/// A provider result offered for a work (the UI's `MatchCandidate`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewCandidate {
    /// `tmdb`, `anilist` for an anime work matched by AniList alone, an
    /// entry its frames named (META-031) or the anime database found (UI-012).
    pub provider: String,
    /// TMDB `movie` / `tv`; `''` for other providers.
    pub ext_kind: String,
    pub ext_id: String,
    /// In the metadata language.
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i64>,
    pub poster_url: Option<String>,
    pub overview: Option<String>,
    /// Ranking score, `0..=1`.
    pub score: f64,
    /// Why it scored so.
    pub signals: MatchSignals,
    /// The work shows this result now.
    pub current: bool,
}

/// A work waiting for the user.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewItem {
    pub work_id: i64,
    /// Library item id (`m<work>`, `s<work>`).
    pub item_id: String,
    /// `movie`, `show` or `anime`.
    pub kind: String,
    pub state: ReviewState,
    /// The title shown now (the provider's for [`ReviewState::Check`]).
    pub title: String,
    /// The title the files are named by.
    pub local_title: String,
    pub year: Option<i64>,
    pub poster_url: Option<String>,
    /// Confidence of the shown result ([`ReviewState::Check`]).
    pub confidence: Option<f64>,
    /// Signals of the shown result.
    pub signals: Option<MatchSignals>,
    /// Video files of the work.
    pub files: u32,
    /// Path of one of them, relative to its source (what the user knows
    /// the item by).
    pub sample_path: String,
    /// Results to choose from, best first; the shown one is
    /// [`current`](ReviewCandidate::current).
    pub candidates: Vec<ReviewCandidate>,
}

/// How many works wait, by state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewCount {
    pub check: u32,
    pub manual: u32,
}

impl ReviewCount {
    #[must_use]
    pub fn total(&self) -> u32 {
        self.check + self.manual
    }
}

/// SQL (on `works AS w`): the work is in the inbox. Visible live files
/// (not personal media, not missing), no user decision, no NFO id, not
/// dismissed; `auto_review` or no provider match.
const IN_INBOX: &str = concat!(
    "NOT EXISTS (SELECT 1 FROM work_reviews wr WHERE wr.work_id = w.id)
     AND EXISTS (SELECT 1 FROM matches m JOIN files f ON f.id = m.file_id
                 WHERE m.work_id = w.id AND m.state <> 'rejected'
                   AND f.missing_since_ms IS NULL AND ",
    pa_library::file_visible!("f"),
    ")
     AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id
                     AND (m.state IN ('user_confirmed', 'user_set')
                          OR m.decided_by IN ('user', 'nfo')))
     AND (EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id AND m.state = 'auto_review')
          OR EXISTS (SELECT 1 FROM work_metadata wm
                     WHERE wm.work_id = w.id AND wm.outcome = 'not_found'))"
);

/// Replaces the stored candidates of `work` with `candidates` (best first,
/// at most [`crate::metadata::CANDIDATES`]).
pub(crate) fn set_candidates(
    tx: &Transaction<'_>,
    work: i64,
    candidates: &[ScoredResult],
) -> Result<(), Error> {
    tx.execute("DELETE FROM work_candidates WHERE work_id = ?1", [work])?;
    let mut insert = tx.prepare_cached(
        "INSERT OR IGNORE INTO work_candidates
             (work_id, rank, provider, ext_kind, ext_id, title, original_title, year,
              poster_url, overview, score, explanation_json)
         VALUES (?1, ?2, 'tmdb', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for (i, c) in candidates
        .iter()
        .take(crate::metadata::CANDIDATES)
        .enumerate()
    {
        let r = &c.result;
        let title = r
            .title
            .as_deref()
            .or(r.original_title.as_deref())
            .unwrap_or_default();
        insert.execute(params![
            work,
            i64::try_from(i + 1).unwrap_or(i64::MAX),
            c.signals.kind,
            r.id.to_string(),
            title,
            r.original_title.as_deref().filter(|o| *o != title),
            r.year().map(i64::from),
            r.poster_path
                .as_deref()
                .map(|p| tmdb::image_url(POSTER_SIZE, p)),
            r.overview.as_deref().filter(|o| !o.trim().is_empty()),
            c.score,
            serde_json::to_string(&c.signals).ok(),
        ])?;
    }
    Ok(())
}

pub(crate) fn candidate_of(c: &ScoredResult) -> ReviewCandidate {
    let r = &c.result;
    let title = r
        .title
        .clone()
        .or_else(|| r.original_title.clone())
        .unwrap_or_default();
    ReviewCandidate {
        provider: "tmdb".to_owned(),
        ext_kind: c.signals.kind.clone(),
        ext_id: r.id.to_string(),
        original_title: r.original_title.clone().filter(|o| *o != title),
        title,
        year: r.year().map(i64::from),
        poster_url: r
            .poster_path
            .as_deref()
            .map(|p| tmdb::image_url(POSTER_SIZE, p)),
        overview: r.overview.clone().filter(|o| !o.trim().is_empty()),
        score: c.score,
        signals: c.signals.clone(),
        current: false,
    }
}

/// The work's id at a provider: `(ext_kind, ext_id)`.
fn external(
    conn: &Connection,
    work: i64,
    provider: &str,
) -> Result<Option<(String, String)>, Error> {
    Ok(conn
        .query_row(
            "SELECT ext_kind, ext_id FROM external_ids WHERE work_id = ?1 AND provider = ?2
             ORDER BY ext_kind LIMIT 1",
            params![work, provider],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// The AniList entries the frames of `work`'s files named (META-031),
/// after `candidates` and without the ones already there.
fn add_frame_candidates(
    conn: &Connection,
    work: i64,
    candidates: &mut Vec<ReviewCandidate>,
    shown: Option<&(String, String)>,
) -> Result<(), Error> {
    for c in crate::frame_search::work_candidates(conn, work)? {
        let id = c.anilist.to_string();
        if candidates
            .iter()
            .any(|x| x.provider == "anilist" && x.ext_id == id)
        {
            continue;
        }
        candidates.push(ReviewCandidate {
            provider: "anilist".to_owned(),
            ext_kind: String::new(),
            current: shown.is_some_and(|(k, i)| k.is_empty() && *i == id),
            ext_id: id,
            original_title: c.native.filter(|n| *n != c.title),
            title: c.title,
            year: None,
            poster_url: None,
            overview: None,
            score: c.share,
            signals: MatchSignals {
                provider: crate::frame_search::PROVIDER.to_owned(),
                title: (c.share * 1000.0).round() / 1000.0,
                tier: "frames".to_owned(),
                file_kind: "anime".to_owned(),
                kind: "tv".to_owned(),
                ..Default::default()
            },
        });
    }
    Ok(())
}

fn poster(conn: &Connection, work: i64) -> Result<Option<String>, Error> {
    Ok(conn
        .query_row(
            "SELECT url FROM artwork WHERE work_id = ?1 AND kind = 'poster'
             ORDER BY priority, id LIMIT 1",
            [work],
            |r| r.get(0),
        )
        .optional()?)
}

fn load_item(conn: &Connection, work: i64) -> Result<Option<ReviewItem>, Error> {
    let row = conn
        .query_row(
            &format!(
                "SELECT w.kind, w.title, w.year, w.original_title, w.overview,
                        EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id
                                AND m.state = 'auto_review'),
                        (SELECT max(m.confidence) FROM matches m WHERE m.work_id = w.id
                                AND m.state = 'auto_review'),
                        (SELECT m.explanation_json FROM matches m WHERE m.work_id = w.id
                                AND m.state = 'auto_review' AND m.explanation_json IS NOT NULL
                                ORDER BY m.id LIMIT 1),
                        (SELECT count(DISTINCT m.file_id) FROM matches m
                                JOIN files f ON f.id = m.file_id
                                WHERE m.work_id = w.id AND m.state <> 'rejected'
                                  AND f.missing_since_ms IS NULL),
                        (SELECT f.path FROM matches m JOIN files f ON f.id = m.file_id
                                WHERE m.work_id = w.id AND m.state <> 'rejected'
                                ORDER BY f.path LIMIT 1),
                        (SELECT t.text FROM titles t WHERE t.work_id = w.id AND t.source = 'local'
                                ORDER BY t.kind = 'main' DESC, t.id LIMIT 1)
                 FROM works w WHERE w.id = ?1 AND {IN_INBOX}"
            ),
            [work],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .optional()?;
    let Some((
        kind,
        title,
        year,
        original_title,
        overview,
        check,
        confidence,
        explanation,
        files,
        sample,
        local,
    )) = row
    else {
        return Ok(None);
    };
    let state = if check {
        ReviewState::Check
    } else {
        ReviewState::Manual
    };
    let signals = explanation
        .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
        .and_then(|v| v.get("signals").cloned())
        .and_then(|v| serde_json::from_value::<MatchSignals>(v).ok());
    let shown_tmdb = external(conn, work, "tmdb")?;
    let mut candidates: Vec<ReviewCandidate> = {
        let mut stmt = conn.prepare_cached(
            "SELECT provider, ext_kind, ext_id, title, original_title, year, poster_url,
                    overview, score, explanation_json
             FROM work_candidates WHERE work_id = ?1 ORDER BY rank",
        )?;
        stmt.query_map([work], |r| {
            Ok(ReviewCandidate {
                provider: r.get(0)?,
                ext_kind: r.get(1)?,
                ext_id: r.get(2)?,
                title: r.get(3)?,
                original_title: r.get(4)?,
                year: r.get(5)?,
                poster_url: r.get(6)?,
                overview: r.get(7)?,
                score: r.get(8)?,
                signals: r
                    .get::<_, Option<String>>(9)?
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
                current: false,
            })
        })?
        .collect::<Result<_, _>>()?
    };
    add_frame_candidates(conn, work, &mut candidates, None)?;
    let poster_url = poster(conn, work)?;
    if state == ReviewState::Check {
        // The shown result: the stored candidate with the work's TMDB id,
        // else (an AniList-only anime, data older than the inbox) the
        // work as it is shown now.
        let shown = shown_tmdb.as_ref().and_then(|(k, id)| {
            candidates
                .iter()
                .position(|c| c.provider == "tmdb" && &c.ext_kind == k && &c.ext_id == id)
        });
        match shown {
            Some(i) => {
                candidates[i].current = true;
                let c = candidates.remove(i);
                candidates.insert(0, c);
            }
            None => {
                let (provider, (ext_kind, ext_id)) = match &shown_tmdb {
                    Some(t) => ("tmdb", t.clone()),
                    None => (
                        "anilist",
                        external(conn, work, "anilist")?.unwrap_or_default(),
                    ),
                };
                candidates.retain(|c| !(c.provider == provider && c.ext_id == ext_id));
                candidates.insert(
                    0,
                    ReviewCandidate {
                        provider: provider.to_owned(),
                        ext_kind,
                        ext_id,
                        title: title.clone(),
                        original_title,
                        year,
                        poster_url: poster_url.clone(),
                        overview,
                        score: confidence.unwrap_or_default(),
                        signals: signals.clone().unwrap_or_default(),
                        current: true,
                    },
                );
            }
        }
    }
    let item_id = if kind == "movie" {
        ItemRef::Movie(work)
    } else {
        ItemRef::Show(work)
    }
    .to_string();
    Ok(Some(ReviewItem {
        work_id: work,
        item_id,
        state,
        local_title: local.unwrap_or_else(|| title.clone()),
        title,
        kind,
        year,
        poster_url,
        confidence: confidence.filter(|_| state == ReviewState::Check),
        signals: signals.filter(|_| state == ReviewState::Check),
        files: u32::try_from(files).unwrap_or(u32::MAX),
        sample_path: sample
            .map(|s| s.trim_start_matches('/').to_owned())
            .unwrap_or_default(),
        candidates,
    }))
}

/// Removes what providers wrote about `work` (ids, titles, artwork,
/// overview, genres, credits, ratings, collection, episode data): the work
/// is shown by its own name again until a metadata run fills it anew.
pub(crate) fn clear_provider_data(tx: &Transaction<'_>, work: i64, now: i64) -> Result<(), Error> {
    let local: Option<String> = tx
        .query_row(
            "SELECT text FROM titles WHERE work_id = ?1 AND source = 'local'
             ORDER BY kind = 'main' DESC, id LIMIT 1",
            [work],
            |r| r.get(0),
        )
        .optional()?;
    for sql in [
        "DELETE FROM external_ids WHERE work_id = ?1",
        "DELETE FROM titles WHERE work_id = ?1 AND source IN ('tmdb', 'anilist', 'shikimori')",
        "DELETE FROM artwork WHERE work_id = ?1
           AND provider IN ('tmdb', 'anilist', 'shikimori', 'fanart')",
        "DELETE FROM artwork WHERE provider = 'tmdb'
           AND episode_id IN (SELECT id FROM episodes WHERE work_id = ?1)",
        "DELETE FROM work_credits WHERE work_id = ?1 AND provider = 'tmdb'",
        "DELETE FROM work_ratings WHERE work_id = ?1 AND provider = 'tmdb'",
        "DELETE FROM episode_mappings WHERE scheme = 'tvdb_aired' AND source = 'tvdb'
           AND episode_id IN (SELECT id FROM episodes WHERE work_id = ?1)",
        // The air date of an unnumbered episode came from its file name
        // (daily shows, META-012): it stays.
        "UPDATE episodes SET title = NULL, overview = NULL, canonical_ref = NULL,
             air_date = CASE WHEN season_number IS NULL AND episode_number IS NULL
                              AND absolute_number IS NULL THEN air_date END
         WHERE work_id = ?1",
    ] {
        tx.execute(sql, [work])?;
    }
    pa_library::views::set_work_genres(tx, work, "tmdb", &[])?;
    pa_library::views::set_work_genres(tx, work, "anilist", &[])?;
    pa_library::profiles::set_certifications(tx, work, "tmdb", &[])?;
    crate::lists::link_tmdb_collection(tx, work, None, now)?;
    if let Some(local) = local {
        tx.execute(
            "UPDATE titles SET kind = 'synonym' WHERE work_id = ?1 AND kind = 'main'",
            [work],
        )?;
        tx.execute(
            "UPDATE titles SET kind = 'main'
             WHERE id = (SELECT id FROM titles WHERE work_id = ?1 AND source = 'local'
                         AND text = ?2 ORDER BY id LIMIT 1)",
            params![work, local],
        )?;
        tx.execute(
            "UPDATE works SET title = ?2, original_title = NULL, original_lang = NULL,
                              overview = NULL, status = NULL, updated_at_ms = ?3
             WHERE id = ?1",
            params![work, local, now],
        )?;
    }
    Ok(())
}

/// Makes every live match of `work` a final user decision.
fn decide(tx: &Transaction<'_>, work: i64, state: &str, now: i64) -> Result<(), Error> {
    tx.execute(
        "UPDATE matches SET state = ?2, decided_by = 'user', updated_at_ms = ?3,
                            confidence = CASE WHEN ?2 = 'user_set' THEN 1.0 ELSE confidence END
         WHERE work_id = ?1 AND state <> 'rejected'",
        params![work, state, now],
    )?;
    tx.execute("DELETE FROM work_candidates WHERE work_id = ?1", [work])?;
    Ok(())
}

impl Catalog {
    /// How many works wait in the inbox.
    pub fn review_count(&self) -> Result<ReviewCount, Error> {
        let conn = self.lock();
        let (check, manual): (i64, i64) = conn.query_row(
            &format!(
                "SELECT
                   ifnull(sum(EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id
                                      AND m.state = 'auto_review')), 0),
                   ifnull(sum(NOT EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id
                                          AND m.state = 'auto_review')), 0)
                 FROM works w WHERE {IN_INBOX}"
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(ReviewCount {
            check: u32::try_from(check).unwrap_or(u32::MAX),
            manual: u32::try_from(manual).unwrap_or(u32::MAX),
        })
    }

    /// The works of the inbox: shown results to check first (least
    /// confident first), then the manual queue by title; at most `limit`.
    pub fn review_items(&self, limit: usize) -> Result<Vec<ReviewItem>, Error> {
        let conn = self.lock();
        let ids: Vec<i64> = {
            let mut stmt = conn.prepare(&format!(
                "SELECT w.id FROM works w WHERE {IN_INBOX}
                 ORDER BY NOT EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id
                                      AND m.state = 'auto_review'),
                          (SELECT max(m.confidence) FROM matches m WHERE m.work_id = w.id
                                  AND m.state = 'auto_review'),
                          w.title COLLATE NOCASE, w.id
                 LIMIT ?1"
            ))?;
            stmt.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(item) = load_item(&conn, id)? {
                out.push(item);
            }
        }
        Ok(out)
    }

    /// The inbox entry of the work of `item` (an episode: its show), if
    /// the work is in the inbox.
    pub fn review_item(&self, item: ItemRef) -> Result<Option<ReviewItem>, Error> {
        let conn = self.lock();
        let work = work_of(&conn, item)?;
        load_item(&conn, work)
    }

    /// The work of `item` and the candidates its last search weighed,
    /// whether or not it is in the inbox (for «Исправить распознавание»).
    pub fn match_candidates(&self, item: ItemRef) -> Result<Vec<ReviewCandidate>, Error> {
        let conn = self.lock();
        let work = work_of(&conn, item)?;
        let shown = external(&conn, work, "tmdb")?;
        let mut stmt = conn.prepare_cached(
            "SELECT provider, ext_kind, ext_id, title, original_title, year, poster_url,
                    overview, score, explanation_json
             FROM work_candidates WHERE work_id = ?1 ORDER BY rank",
        )?;
        let rows = stmt.query_map([work], |r| {
            let (ext_kind, ext_id): (String, String) = (r.get(1)?, r.get(2)?);
            Ok(ReviewCandidate {
                provider: r.get(0)?,
                current: shown
                    .as_ref()
                    .is_some_and(|(k, id)| *k == ext_kind && *id == ext_id),
                ext_kind,
                ext_id,
                title: r.get(3)?,
                original_title: r.get(4)?,
                year: r.get(5)?,
                poster_url: r.get(6)?,
                overview: r.get(7)?,
                score: r.get(8)?,
                signals: r
                    .get::<_, Option<String>>(9)?
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
            })
        })?;
        let mut out: Vec<ReviewCandidate> = rows.collect::<Result<_, _>>()?;
        let anilist = external(&conn, work, "anilist")?;
        add_frame_candidates(&conn, work, &mut out, anilist.as_ref())?;
        Ok(out)
    }

    /// «Верно»: the shown result of `work` stays (`user_confirmed`).
    pub fn review_confirm(&self, work: i64) -> Result<(), Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let shown: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM matches WHERE work_id = ?1 AND state <> 'rejected')
                    AND EXISTS (SELECT 1 FROM work_metadata WHERE work_id = ?1 AND outcome = 'matched')",
            [work],
            |r| r.get(0),
        )?;
        if !shown {
            return Err(Error::Invalid(format!(
                "work {work} shows no provider result to confirm"
            )));
        }
        decide(&tx, work, "user_confirmed", crate::now_ms())?;
        tx.commit()?;
        drop(conn);
        // The folder learns it (META-017).
        self.teach_rules(work, crate::folder_rules::Teach::default())?;
        Ok(())
    }

    /// Another result for `work`: a stored candidate or one of
    /// [`review_search`](Self::review_search) (`provider` `tmdb`,
    /// `ext_kind` `movie` or `tv` as the work's kind), or for an anime an
    /// AniList entry its frames named (`provider` `anilist`, `ext_kind`
    /// `''`, META-031). The old result's
    /// provider data goes; the next metadata run reads the chosen one by
    /// its id. A work that already carries that id absorbs this one.
    /// Returns the work that carries the id now.
    pub fn review_choose(
        &self,
        work: i64,
        provider: &str,
        ext_kind: &str,
        ext_id: &str,
    ) -> Result<i64, Error> {
        if provider != "tmdb" && provider != "anilist" {
            return Err(Error::Invalid(format!("provider {provider:?}")));
        }
        if ext_id.is_empty() || ext_id.parse::<u64>().is_err() {
            return Err(Error::Invalid(format!("{provider} id {ext_id:?}")));
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let kind: String = tx
            .query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("work {work}")))?;
        // An AniList entry (a frame candidate, META-031) is for anime.
        let want = match (provider, kind.as_str()) {
            ("anilist", "anime") => "",
            ("anilist", _) => {
                return Err(Error::Invalid(format!(
                    "a {kind} cannot be matched to an AniList entry"
                )));
            }
            (_, "movie") => "movie",
            _ => "tv",
        };
        if ext_kind != want {
            return Err(Error::Invalid(format!(
                "a {kind} cannot be matched to TMDB {ext_kind}"
            )));
        }
        let now = crate::now_ms();
        clear_provider_data(&tx, work, now)?;
        let mut merged = 0;
        let target =
            crate::metadata::claim_external(&tx, work, provider, ext_kind, ext_id, &mut merged)?;
        decide(&tx, target, "user_set", now)?;
        // Read by id on the next metadata run.
        tx.execute("DELETE FROM work_metadata WHERE work_id = ?1", [target])?;
        tx.commit()?;
        tracing::info!(
            target: "pa_catalog::review",
            work,
            into = target,
            result = %format!("{provider}:{ext_kind}:{ext_id}"),
            "review: user chose a result"
        );
        drop(conn);
        // The fix covers the folder and its new files (META-017).
        self.teach_rules(target, crate::folder_rules::Teach::default())?;
        Ok(target)
    }

    /// «Это не фильм»: `work` is not a movie or series of any provider. Its
    /// files stay in the library under their own names, the provider data
    /// goes, and no provider is asked about it again.
    pub fn review_not_media(&self, work: i64) -> Result<(), Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let exists = tx
            .query_row("SELECT 1 FROM works WHERE id = ?1", [work], |_| Ok(()))
            .optional()?
            .is_some();
        if !exists {
            return Err(Error::NotFound(format!("work {work}")));
        }
        let now = crate::now_ms();
        clear_provider_data(&tx, work, now)?;
        decide(&tx, work, "user_set", now)?;
        tx.execute(
            "INSERT INTO work_reviews (work_id, decision, decided_at_ms) VALUES (?1, 'not_media', ?2)
             ON CONFLICT (work_id) DO UPDATE SET decision = excluded.decision,
                                                 decided_at_ms = excluded.decided_at_ms",
            params![work, now],
        )?;
        tx.execute("DELETE FROM work_metadata WHERE work_id = ?1", [work])?;
        // LIB-025, simplest for the user: what is not a movie or series is
        // the user's own video. Each file gets a kind override (it sticks
        // through rescans and can be undone with «Это не то»), its matches
        // go, and the files appear in «Мои видео».
        let files: Vec<(i64, i64, String)> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT f.id, f.source_id, f.path FROM matches m
                   JOIN files f ON f.id = m.file_id
                  WHERE m.work_id = ?1 AND m.state <> 'rejected'",
            )?;
            stmt.query_map([work], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?
        };
        for (file, source, path) in &files {
            if pa_scanner::media_kind(path) != Some(pa_scanner::MediaKind::Video) {
                continue;
            }
            tx.execute(
                "INSERT INTO folder_content_overrides (source_id, path, kind, set_at_ms)
                 VALUES (?1, ?2, 'own_video', ?3)
                 ON CONFLICT (source_id, path) DO UPDATE SET kind = excluded.kind,
                     set_at_ms = excluded.set_at_ms",
                params![source, path, now],
            )?;
            tx.execute("DELETE FROM matches WHERE file_id = ?1", [file])?;
        }
        tx.commit()?;
        let mut sources: Vec<i64> = files.iter().map(|(_, s, _)| *s).collect();
        sources.sort_unstable();
        sources.dedup();
        for source in sources {
            crate::content_kind::classify(&mut conn, source)?;
        }
        let tx = conn.transaction()?;
        crate::recognize::remove_orphans(&tx)?;
        tx.commit()?;
        pa_library::search::sync(&conn)?;
        Ok(())
    }

    /// Manual search for `work`: TMDB results for `title` (and `year`) of
    /// the work's kind, ranked with what its files say (META-014 pair
    /// features), best first — every result, also below `REVIEW` — and the
    /// AniList entries of the anime database whose titles fit (UI-012;
    /// first for an anime). Without a TMDB key only the latter.
    /// Nothing is written; [`review_choose`](Self::review_choose) takes
    /// one.
    pub fn review_search(
        &self,
        providers: &MetadataProviders,
        work: i64,
        title: &str,
        year: Option<i64>,
    ) -> Result<Vec<ReviewCandidate>, Error> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::Invalid("empty title".into()));
        }
        let mut query = {
            let conn = self.lock();
            crate::metadata::work_query(&conn, work)?
                .ok_or_else(|| Error::NotFound(format!("work {work}")))?
        };
        query.titles = vec![title.to_owned()];
        query.year = year.filter(|y| (1800..=3000).contains(y));
        // UI-012: the anime database answers too (AniDB, AniList, MAL and
        // Shikimori titles, no request).
        let anime = providers
            .anime_titles
            .as_deref()
            .map(|t| crate::review_lookup::anime_search(t, &query))
            .unwrap_or_default();
        let Some(client) = providers.tmdb() else {
            if anime.is_empty() {
                return Err(Error::Provider("TMDB has no key".into()));
            }
            return Ok(anime);
        };
        let mut out = Self::tmdb_search(client, &query, title)?;
        if query.kind == "anime" {
            let mut all = anime;
            all.append(&mut out);
            out = all;
        } else {
            out.extend(anime);
        }
        Ok(out)
    }

    /// TMDB results of [`review_search`](Self::review_search).
    fn tmdb_search(
        client: &tmdb::TmdbClient,
        query: &crate::metadata::WorkQuery,
        title: &str,
    ) -> Result<Vec<ReviewCandidate>, Error> {
        let failure = |e: TmdbError| Error::Provider(format!("TMDB: {e}"));
        let kind = if query.kind == "movie" {
            MediaKind::Movie
        } else {
            MediaKind::Tv
        };
        let search = |y: Option<u32>| {
            match kind {
                MediaKind::Movie => client.search_movie(title, y),
                MediaKind::Tv => client.search_tv(title, y),
            }
            .map(|page| page.results)
            .map_err(failure)
        };
        let y = query.year.and_then(|y| u32::try_from(y).ok());
        let mut page = search(y)?;
        if page.is_empty() && y.is_some() {
            page = search(None)?;
        }
        // Ranked with the work's evidence; the rest of the page follows in
        // TMDB's order, unscored.
        let decision = search_ranked(
            query,
            &mut |_, _, _| Ok::<_, Error>(page.clone()),
            &mut |_, _| Ok(None),
            &mut |_, _| Ok(Vec::new()),
        )?;
        let mut out: Vec<ReviewCandidate> = decision.candidates.iter().map(candidate_of).collect();
        let kind_name = if kind == MediaKind::Movie {
            "movie"
        } else {
            "tv"
        };
        for r in &page {
            if out.len() >= SEARCH_RESULTS {
                break;
            }
            if out.iter().any(|c| c.ext_id == r.id.to_string()) {
                continue;
            }
            out.push(candidate_of(&ScoredResult {
                result: r.clone(),
                score: 0.0,
                signals: MatchSignals {
                    provider: "tmdb".to_owned(),
                    kind: kind_name.to_owned(),
                    file_kind: query.kind.clone(),
                    file_year: query.year,
                    year: r.year().map(i64::from),
                    query: Some(title.to_owned()),
                    ..Default::default()
                },
            }));
        }
        Ok(out)
    }
}

/// The work of an item (an episode: its show).
fn work_of(conn: &Connection, item: ItemRef) -> Result<i64, Error> {
    match item {
        ItemRef::Movie(w) | ItemRef::Show(w) => Ok(w),
        ItemRef::Episode(e) => conn
            .query_row("SELECT work_id FROM episodes WHERE id = ?1", [e], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| Error::NotFound(item.to_string())),
        ItemRef::Track(_) => Err(Error::Invalid(format!("{item} is music"))),
    }
}
