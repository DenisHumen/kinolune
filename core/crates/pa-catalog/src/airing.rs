// SPDX-License-Identifier: MPL-2.0
//! When the next episode of an ongoing show or anime airs (UI-057).
//!
//! Sources, first answer wins:
//!
//! - **Anime** (an AniList, MAL or Shikimori id): AniList
//!   `nextAiringEpisode` (episode number and the exact moment, so the app
//!   counts down to the hour), else Shikimori `next_episode_at` (ADR-0142
//!   exception), else TMDB.
//! - **Shows**: TMDB `next_episode_to_air` (season, episode and the air
//!   date — TMDB has no time of day) and `status`.
//!
//! The answer is kept in `work_airing` (a cache of the device, never
//! synced). [`Catalog::refresh_airing`] asks when the show page opens;
//! [`Catalog::refresh_airing_due`] once a day in the background. The
//! provider clients answer from their own cache while it is fresh (a day
//! for series that still air) but never with an expired answer in place of
//! a failed request: offline the last value stays and is marked
//! [`ShowAiring::offline`].
//!
//! What the app is shown ([`Catalog::show_airing`], [`Catalog::airing_all`])
//! leaves out a next episode that already aired (the date is in the past:
//! waiting for the next refresh) and one whose file is already in the
//! library (the real episode replaces the card).
//!
//! Times are UTC here; the app shows them in the device's time zone.

use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_metadata::fetch::FetchError;
use pa_metadata::tmdb::TmdbError;

use crate::items::ItemRef;
use crate::metadata::MetadataProviders;
use crate::{Catalog, Error};

const DAY_MS: i64 = 24 * 3600 * 1000;

/// Ongoing shows are asked again after this long (daily refresh).
pub const AIRING_REFRESH_MS: i64 = DAY_MS;

/// Finished shows and shows without data are asked again after this long
/// (a show may be renewed).
pub const ENDED_REFRESH_MS: i64 = 30 * DAY_MS;

/// Where a show is in its run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiringStatus {
    /// New episodes come; [`ShowAiring::next`] has the next one when its
    /// date is known.
    Airing,
    /// Between seasons or on a break, no date yet.
    Hiatus,
    /// Finished or cancelled.
    Ended,
}

impl AiringStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Airing => "airing",
            Self::Hiatus => "hiatus",
            Self::Ended => "ended",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "airing" => Some(Self::Airing),
            "hiatus" => Some(Self::Hiatus),
            "ended" => Some(Self::Ended),
            _ => None,
        }
    }
}

/// The next episode of a show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextEpisode {
    /// Season in TMDB numbering; `None` when the provider numbers the
    /// episodes in one run (AniList, Shikimori): the app puts the card at
    /// the end of the last season.
    pub season: Option<i64>,
    pub episode: i64,
    /// `YYYY-MM-DD` when only the date is known (TMDB).
    pub air_date: Option<String>,
    /// The exact moment, Unix ms (AniList, Shikimori).
    pub air_at_ms: Option<i64>,
}

/// The airing state of a show, as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowAiring {
    pub show: ItemRef,
    pub status: AiringStatus,
    /// The episode still to come; `None` after its date or once its file
    /// is in the library.
    pub next: Option<NextEpisode>,
    /// `tmdb`, `anilist` or `shikimori`.
    pub source: String,
    /// When a provider last answered, Unix ms.
    pub checked_at_ms: i64,
    /// The last refresh failed (offline): this is an older answer.
    pub offline: bool,
}

/// Result of [`Catalog::refresh_airing_due`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AiringReport {
    /// Shows that were asked.
    pub checked: usize,
    /// Of them, shows a provider answered for.
    pub updated: usize,
    /// Of them, shows no provider could be reached for.
    pub failed: usize,
}

/// Provider ids of a show work.
#[derive(Debug, Clone, Default)]
struct Ids {
    anime: bool,
    tmdb: Option<u64>,
    anilist: Option<u64>,
    mal: Option<u64>,
}

/// One provider answer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Answer {
    status: Option<AiringStatus>,
    next: Option<NextEpisode>,
    source: &'static str,
}

/// Why a provider did not answer.
enum Miss {
    /// No id, no client, no key, or the provider does not know the show.
    NoData,
    /// The provider could not be reached (network, rate limit, 5xx).
    Unreachable,
}

fn tmdb_miss(e: &TmdbError) -> Miss {
    match e {
        TmdbError::NotFound(_) | TmdbError::NoKey | TmdbError::Unauthorized => Miss::NoData,
        TmdbError::Decode(_) => Miss::NoData,
        _ => Miss::Unreachable,
    }
}

fn fetch_miss(e: &FetchError) -> Miss {
    match e {
        FetchError::NotFound { .. } | FetchError::Decode(_) => Miss::NoData,
        FetchError::Http { status, .. } if *status < 500 && *status != 429 => Miss::NoData,
        _ => Miss::Unreachable,
    }
}

fn number(n: u32) -> i64 {
    i64::from(n)
}

/// `/tv/{id}` → status and next episode.
fn from_tmdb(d: &pa_metadata::tmdb::Details) -> Answer {
    let next = d
        .next_episode_to_air
        .as_ref()
        .filter(|e| e.episode_number > 0)
        .map(|e| NextEpisode {
            season: Some(number(e.season_number)),
            episode: number(e.episode_number),
            air_date: e.air_date.clone().filter(|s| days_of(s).is_some()),
            air_at_ms: None,
        });
    let status = match d.status.as_deref() {
        Some("Ended" | "Canceled" | "Cancelled") => Some(AiringStatus::Ended),
        _ if next.is_some() => Some(AiringStatus::Airing),
        _ if d.is_ongoing() && d.status.is_some() => Some(AiringStatus::Hiatus),
        _ => None,
    };
    Answer {
        status,
        next: next.filter(|_| status == Some(AiringStatus::Airing)),
        source: "tmdb",
    }
}

/// AniList `Media` → status and next episode.
fn from_anilist(m: &pa_metadata::anilist::Media) -> Answer {
    let next = m
        .next_airing_episode
        .as_ref()
        .filter(|e| e.episode > 0)
        .map(|e| NextEpisode {
            season: None,
            episode: number(e.episode),
            air_date: None,
            air_at_ms: i64::try_from(e.airing_at)
                .ok()
                .and_then(|s| s.checked_mul(1000)),
        });
    let status = match m.status.as_deref() {
        Some("RELEASING" | "NOT_YET_RELEASED") if next.is_some() => Some(AiringStatus::Airing),
        Some("RELEASING") => Some(AiringStatus::Airing),
        Some("HIATUS" | "NOT_YET_RELEASED") => Some(AiringStatus::Hiatus),
        Some("FINISHED" | "CANCELLED") => Some(AiringStatus::Ended),
        _ => None,
    };
    Answer {
        status,
        next: next.filter(|_| status == Some(AiringStatus::Airing)),
        source: "anilist",
    }
}

/// Shikimori `/animes/{id}` → status and next episode.
fn from_shikimori(a: &pa_metadata::shikimori::Anime) -> Answer {
    let at = a.next_episode_at.as_deref().and_then(parse_rfc3339_ms);
    let next = at.map(|at| NextEpisode {
        season: None,
        episode: number(a.base.episodes_aired.unwrap_or(0)) + 1,
        air_date: None,
        air_at_ms: Some(at),
    });
    let status = match a.base.status.as_deref() {
        Some("ongoing") => Some(AiringStatus::Airing),
        Some("anons") if next.is_some() => Some(AiringStatus::Airing),
        Some("anons") => Some(AiringStatus::Hiatus),
        Some("released") => Some(AiringStatus::Ended),
        _ => None,
    };
    Answer {
        status,
        next: next.filter(|_| status == Some(AiringStatus::Airing)),
        source: "shikimori",
    }
}

/// Asks the providers of a show in turn. `Ok(None)`: no provider knows it;
/// `Err(())`: a provider that could know it was unreachable and none
/// answered.
fn ask(providers: &MetadataProviders, ids: &Ids) -> Result<Option<Answer>, ()> {
    let mut unreachable = false;
    let mut note = |m: Miss| {
        if matches!(m, Miss::Unreachable) {
            unreachable = true;
        }
    };
    if ids.anime {
        if let (Some(client), Some(id)) = (&providers.anilist, ids.anilist) {
            match client.airing(id) {
                Ok(m) => {
                    let a = from_anilist(&m);
                    if a.status.is_some() {
                        return Ok(Some(a));
                    }
                }
                Err(e) => note(fetch_miss(&e)),
            }
        }
        if let (Some(client), Some(id)) = (&providers.shikimori, ids.mal) {
            match client.anime_airing(id) {
                Ok(anime) => {
                    let a = from_shikimori(&anime);
                    if a.status.is_some() {
                        return Ok(Some(a));
                    }
                }
                Err(e) => note(fetch_miss(&e)),
            }
        }
    }
    if let (Some(client), Some(id)) = (providers.tmdb(), ids.tmdb) {
        match client.tv_airing(id) {
            Ok(d) => {
                let a = from_tmdb(&d);
                if a.status.is_some() {
                    return Ok(Some(a));
                }
            }
            Err(e) => note(tmdb_miss(&e)),
        }
    }
    if unreachable { Err(()) } else { Ok(None) }
}

fn work_ids(conn: &Connection, work: i64) -> Result<Option<Ids>, Error> {
    let kind: Option<String> = conn
        .query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
        .optional()?;
    let Some(kind) = kind else { return Ok(None) };
    if kind == "movie" {
        return Ok(None);
    }
    let ext = |provider: &str, ext_kind: &str| -> Result<Option<u64>, Error> {
        let v: Option<String> = conn
            .query_row(
                "SELECT ext_id FROM external_ids
                 WHERE work_id = ?1 AND provider = ?2 AND ext_kind = ?3",
                params![work, provider, ext_kind],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|v| v.parse().ok()))
    };
    let anilist = ext("anilist", "")?;
    let mal = match ext("mal", "")? {
        Some(m) => Some(m),
        None => ext("shikimori", "")?,
    };
    Ok(Some(Ids {
        anime: kind == "anime" || anilist.is_some() || mal.is_some(),
        tmdb: ext("tmdb", "tv")?,
        anilist,
        mal,
    }))
}

fn store(conn: &Connection, work: i64, answer: Option<&Answer>, now_ms: i64) -> Result<(), Error> {
    let status = answer
        .and_then(|a| a.status)
        .map_or("unknown", AiringStatus::as_str);
    let next = answer.and_then(|a| a.next.as_ref());
    conn.execute(
        "INSERT INTO work_airing (work_id, status, next_season, next_episode, next_air_date,
                                  next_air_at_ms, source, checked_at_ms, failed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)
         ON CONFLICT (work_id) DO UPDATE SET
             status = ?2, next_season = ?3, next_episode = ?4, next_air_date = ?5,
             next_air_at_ms = ?6, source = ?7, checked_at_ms = ?8, failed_at_ms = NULL",
        params![
            work,
            status,
            next.and_then(|n| n.season),
            next.map(|n| n.episode),
            next.and_then(|n| n.air_date.as_deref()),
            next.and_then(|n| n.air_at_ms),
            answer.map_or("", |a| a.source),
            now_ms,
        ],
    )?;
    Ok(())
}

fn mark_failed(conn: &Connection, work: i64, now_ms: i64) -> Result<(), Error> {
    conn.execute(
        "UPDATE work_airing SET failed_at_ms = ?2 WHERE work_id = ?1",
        params![work, now_ms],
    )?;
    Ok(())
}

/// A stored row before [`visible`] filters it.
struct Row {
    work: i64,
    status: Option<AiringStatus>,
    next: Option<NextEpisode>,
    source: String,
    checked_at_ms: i64,
    failed: bool,
}

const ROW_SQL: &str = "SELECT work_id, status, next_season, next_episode, next_air_date,
        next_air_at_ms, source, checked_at_ms, failed_at_ms IS NOT NULL
 FROM work_airing";

fn row(r: &pa_library::rusqlite::Row<'_>) -> pa_library::rusqlite::Result<Row> {
    let season: Option<i64> = r.get(2)?;
    let episode: Option<i64> = r.get(3)?;
    let air_date: Option<String> = r.get(4)?;
    let air_at_ms: Option<i64> = r.get(5)?;
    Ok(Row {
        work: r.get(0)?,
        status: AiringStatus::parse(&r.get::<_, String>(1)?),
        next: episode.map(|episode| NextEpisode {
            season,
            episode,
            air_date,
            air_at_ms,
        }),
        source: r.get(6)?,
        checked_at_ms: r.get(7)?,
        failed: r.get(8)?,
    })
}

/// The next episode has aired: its moment is past, or (date only) the day
/// is over everywhere on Earth.
fn aired(next: &NextEpisode, now_ms: i64) -> bool {
    if let Some(at) = next.air_at_ms {
        return at <= now_ms;
    }
    match next.air_date.as_deref().and_then(days_of) {
        // The end of the day at UTC−12 (UTC+14 is ahead of it).
        Some(days) => (days + 1) * DAY_MS + 12 * 3600 * 1000 <= now_ms,
        // No date: it stays the next one.
        None => false,
    }
}

/// The file of the next episode is in the library.
fn in_library(conn: &Connection, work: i64, next: &NextEpisode) -> Result<bool, Error> {
    let sql = "SELECT EXISTS (
            SELECT 1 FROM episodes e
            JOIN match_episodes me ON me.episode_id = e.id
            JOIN matches m ON m.id = me.match_id AND m.state <> 'rejected'
            JOIN files f ON f.id = m.file_id AND f.missing_since_ms IS NULL
            WHERE e.work_id = ?1
              AND (
                (?2 IS NOT NULL AND e.season_number = ?2 AND e.episode_number = ?3)
                OR (?2 IS NULL AND (
                    e.absolute_number = ?3
                    OR (e.episode_number = ?3 AND e.season_number = (
                        SELECT max(season_number) FROM episodes
                        WHERE work_id = ?1 AND season_number >= 1)))))
        )";
    Ok(conn.query_row(sql, params![work, next.season, next.episode], |r| r.get(0))?)
}

fn visible(conn: &Connection, r: Row, now_ms: i64) -> Result<Option<ShowAiring>, Error> {
    let Some(status) = r.status else {
        return Ok(None);
    };
    let next = match r.next {
        Some(n) if status == AiringStatus::Airing => {
            if aired(&n, now_ms) || in_library(conn, r.work, &n)? {
                None
            } else {
                Some(n)
            }
        }
        _ => None,
    };
    Ok(Some(ShowAiring {
        show: ItemRef::Show(r.work),
        status,
        next,
        source: r.source,
        checked_at_ms: r.checked_at_ms,
        offline: r.failed,
    }))
}

fn read(conn: &Connection, work: i64, now_ms: i64) -> Result<Option<ShowAiring>, Error> {
    let r = conn
        .query_row(&format!("{ROW_SQL} WHERE work_id = ?1"), [work], row)
        .optional()?;
    match r {
        Some(r) => visible(conn, r, now_ms),
        None => Ok(None),
    }
}

fn show_work(show: ItemRef) -> Result<i64, Error> {
    match show {
        ItemRef::Show(w) => Ok(w),
        other => Err(Error::Invalid(format!("not a show: {other}"))),
    }
}

impl Catalog {
    /// The airing state of `show` as last fetched, at `now_ms`; `None`
    /// without data (never asked, or no provider knows the show).
    pub fn show_airing(&self, show: ItemRef, now_ms: i64) -> Result<Option<ShowAiring>, Error> {
        let work = show_work(show)?;
        let conn = self.lock();
        read(&conn, work, now_ms)
    }

    /// The airing state of every show that has one (labels on the library
    /// posters), at `now_ms`. One query plus one lookup per ongoing show.
    pub fn airing_all(&self, now_ms: i64) -> Result<Vec<ShowAiring>, Error> {
        let conn = self.lock();
        let rows = {
            let mut stmt = conn.prepare(&format!(
                "{ROW_SQL} WHERE status <> 'unknown' AND {}",
                pa_library::work_allowed!("work_id")
            ))?;
            stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?
        };
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if let Some(a) = visible(&conn, r, now_ms)? {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// Asks the providers about `show` now (the show page opened) and
    /// returns what the app shows. Offline the last value stays, marked
    /// [`ShowAiring::offline`]. The catalog is not locked while a provider
    /// is asked.
    pub fn refresh_airing(
        &self,
        providers: &MetadataProviders,
        show: ItemRef,
        now_ms: i64,
    ) -> Result<Option<ShowAiring>, Error> {
        let work = show_work(show)?;
        let ids = {
            let conn = self.lock();
            work_ids(&conn, work)?
        };
        let Some(ids) = ids else { return Ok(None) };
        let answer = ask(providers, &ids);
        let conn = self.lock();
        // The show may be gone (merged, removed) while the provider answered.
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM works WHERE id = ?1)",
            [work],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        match answer {
            Ok(a) => store(&conn, work, a.as_ref(), now_ms)?,
            Err(()) => mark_failed(&conn, work, now_ms)?,
        }
        read(&conn, work, now_ms)
    }

    /// The daily background refresh: asks about every show whose answer is
    /// older than a day ([`AIRING_REFRESH_MS`]; a month for finished shows
    /// and shows without data, [`ENDED_REFRESH_MS`]), or that was never
    /// asked. Stops early when cancelled.
    pub fn refresh_airing_due(
        &self,
        providers: &MetadataProviders,
        now_ms: i64,
        cancel: &crate::CancelToken,
    ) -> Result<AiringReport, Error> {
        let due: Vec<i64> = {
            let conn = self.lock();
            let mut stmt = conn.prepare(
                "SELECT w.id FROM works w
                 LEFT JOIN work_airing a ON a.work_id = w.id
                 WHERE w.kind IN ('show', 'anime')
                   AND EXISTS (SELECT 1 FROM external_ids x WHERE x.work_id = w.id
                               AND ((x.provider = 'tmdb' AND x.ext_kind = 'tv')
                                    OR x.provider IN ('anilist', 'mal', 'shikimori')))
                   AND (a.work_id IS NULL
                        OR (a.status IN ('airing', 'hiatus') AND a.checked_at_ms <= ?1)
                        OR a.checked_at_ms <= ?2
                        OR (a.failed_at_ms IS NOT NULL AND a.failed_at_ms <= ?3))
                 ORDER BY a.checked_at_ms IS NOT NULL, a.checked_at_ms, w.id",
            )?;
            stmt.query_map(
                params![
                    now_ms - AIRING_REFRESH_MS,
                    now_ms - ENDED_REFRESH_MS,
                    now_ms - 3600 * 1000
                ],
                |r| r.get(0),
            )?
            .collect::<Result<_, _>>()?
        };
        let mut report = AiringReport::default();
        for work in due {
            if cancel.is_cancelled() {
                break;
            }
            report.checked += 1;
            self.refresh_airing(providers, ItemRef::Show(work), now_ms)?;
            let failed: bool = self
                .lock()
                .query_row(
                    "SELECT failed_at_ms IS NOT NULL FROM work_airing WHERE work_id = ?1",
                    [work],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if failed {
                report.failed += 1;
            } else {
                report.updated += 1;
            }
        }
        Ok(report)
    }
}

/// Days since 1970-01-01 of `YYYY-MM-DD`.
fn days_of(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { date.get(r)?.parse().ok() };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 (`2026-12-04T18:30:00.000+03:00`, `…Z`) → Unix ms.
fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 20 || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let days = days_of(s.get(0..10)?)?;
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = s.get(19..)?;
    let mut millis = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        let ms: String = frac[..digits]
            .chars()
            .chain("000".chars())
            .take(3)
            .collect();
        millis = ms.parse().ok()?;
        rest = &frac[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        tz if tz.len() == 6 && matches!(&tz[..1], "+" | "-") && &tz[3..4] == ":" => {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            sign * (tz[1..3].parse::<i64>().ok()? * 3600 + tz[4..6].parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    Some((days * 86_400 + hour * 3600 + minute * 60 + second - offset) * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_with_offset() {
        assert_eq!(
            parse_rfc3339_ms("2026-12-04T18:30:00.000+03:00"),
            Some(days_from_civil(2026, 12, 4) * DAY_MS + (15 * 3600 + 30 * 60) * 1000)
        );
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_rfc3339_ms("soon"), None);
    }

    #[test]
    fn date_only_airs_after_the_day_ends_everywhere() {
        let n = NextEpisode {
            season: Some(1),
            episode: 8,
            air_date: Some("2026-12-04".into()),
            air_at_ms: None,
        };
        let day = days_from_civil(2026, 12, 4) * DAY_MS;
        assert!(!aired(&n, day + 20 * 3600 * 1000));
        assert!(!aired(&n, day + DAY_MS + 11 * 3600 * 1000));
        assert!(aired(&n, day + DAY_MS + 12 * 3600 * 1000));
    }
}
