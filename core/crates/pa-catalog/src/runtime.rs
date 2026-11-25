// SPDX-License-Identifier: MPL-2.0
//! Total runtime and time left of a show and of each of its seasons
//! (UI-056): «3 сезона · 30 серий · всего 21 ч 40 мин · осталось 12 ч 5 мин».
//!
//! One query reads every episode of the show that has a present file in the
//! library, with its duration and its progress; the sums are added up here.
//!
//! - **Duration** of an episode: what its file plays (`play_duration!`, all
//!   parts of a split episode, BUG-021), from the file the episode list
//!   shows ([`crate::items`] `EPISODE_SQL`, the lowest file id). A file not
//!   probed yet falls back to the runtime the metadata gave the episode,
//!   and the sum is marked estimated («≈»); an episode with neither adds
//!   nothing and marks it estimated too.
//! - **Left**: unwatched episodes whole, the rest of a started one (LIB-011
//!   thresholds: below [`Thresholds::in_progress_min`] the episode counts as
//!   not started, from [`Thresholds::watched_after`] on as watched),
//!   nothing of a watched one. The progress is the row the Show Detail
//!   screen reads for the episode ([`crate::episode_order`]): the key of
//!   [`crate::progress::key_of`], `tmdb:episode:` or `file:`.
//! - **Seasons** are those of the chosen episode order, like the season tabs;
//!   specials (season 0) count only in their own season, never in the
//!   total of the show.

use pa_library::progress::Thresholds;
use pa_library::rusqlite::{Connection, params};

use crate::episode_order::EpisodeOrder;
use crate::progress::profile;
use crate::{Catalog, Error, ItemRef};

/// Runtime of a set of episodes: a season or a whole show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeTotal {
    /// Episodes in the library.
    pub episodes: u32,
    /// Of them, watched.
    pub watched: u32,
    /// Sum of the episode durations.
    pub total_ms: i64,
    /// What is left to watch.
    pub remaining_ms: i64,
    /// A duration in [`total_ms`](Self::total_ms) is the metadata runtime of
    /// a file not probed yet, or unknown: shown with «≈».
    pub estimated: bool,
    /// The same for an episode not watched yet, in
    /// [`remaining_ms`](Self::remaining_ms).
    pub remaining_estimated: bool,
}

impl RuntimeTotal {
    /// Every episode is watched («просмотрено» instead of the time left).
    #[must_use]
    pub fn all_watched(&self) -> bool {
        self.episodes > 0 && self.watched == self.episodes
    }

    fn add(&mut self, e: &EpisodeRuntime) {
        self.episodes += 1;
        self.total_ms = self.total_ms.saturating_add(e.duration_ms);
        self.estimated |= !e.exact;
        if e.watched {
            self.watched += 1;
        } else {
            self.remaining_ms = self.remaining_ms.saturating_add(e.remaining_ms);
            self.remaining_estimated |= !e.exact;
        }
    }
}

/// Runtime of one season tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonRuntime {
    /// Season number in the episode order; 0 = specials.
    pub season: i64,
    pub total: RuntimeTotal,
}

/// Runtime of a show: the regular seasons together, and each season.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShowRuntime {
    /// All regular seasons; specials are not in it.
    pub show: RuntimeTotal,
    /// Regular seasons ascending, then specials (0), like the season tabs.
    pub seasons: Vec<SeasonRuntime>,
}

impl ShowRuntime {
    /// The total of one season, if the show has episodes in it.
    #[must_use]
    pub fn season(&self, season: i64) -> Option<&RuntimeTotal> {
        self.seasons
            .iter()
            .find(|s| s.season == season)
            .map(|s| &s.total)
    }
}

/// One episode as the sums see it.
struct EpisodeRuntime {
    duration_ms: i64,
    /// The duration is the probed one.
    exact: bool,
    watched: bool,
    remaining_ms: i64,
}

/// Season of the episode `e` in an episode order, as the season tabs number
/// it ([`crate::episode_order`]).
fn season_sql(order: EpisodeOrder) -> &'static str {
    match order {
        EpisodeOrder::Tmdb => "ifnull(e.season_number, 1)",
        // Regular episodes in one run; specials and episodes without a
        // number keep their season.
        EpisodeOrder::Absolute => {
            "CASE WHEN e.season_number >= 1 AND e.episode_type = 'regular'
                       AND e.episode_number IS NOT NULL THEN 1
                  ELSE ifnull(e.season_number, 1) END"
        }
        // The AniDB entry of the best mapping, else the TMDB season.
        EpisodeOrder::Anidb => {
            "ifnull((SELECT ifnull(em.season_number, 1) FROM episode_mappings em
                      WHERE em.episode_id = e.id AND em.scheme = 'anidb'
                      ORDER BY em.confidence DESC, em.part, em.id LIMIT 1),
                    ifnull(e.season_number, 1))"
        }
    }
}

/// Every episode of `?1` with a present file, one row each: season, the
/// duration of the file the list shows, the metadata runtime and the
/// progress row of the episode on the profile `?2`. The progress key is
/// that of `progress::key_of`: `tmdb:episode:<show>:<season>:<episode>`
/// when the show has a TMDB id and the episode both numbers, else `file:`
/// with the first valid fingerprint of its files.
const EPISODES_SQL: &str = concat!(
    "
SELECT {season}, ",
    crate::items::play_duration!(),
    ", e.runtime_ms, p.position_ms, p.duration_ms, p.watched, min(f.id)
FROM episodes e
JOIN match_episodes me ON me.episode_id = e.id
JOIN matches m ON m.id = me.match_id AND m.state <> 'rejected'
JOIN files f ON f.id = m.file_id AND f.missing_since_ms IS NULL
LEFT JOIN external_ids tv ON tv.work_id = e.work_id AND tv.provider = 'tmdb'
     AND tv.ext_kind = 'tv' AND tv.ext_id GLOB '[1-9]*' AND tv.ext_id NOT GLOB '*[^0-9]*'
LEFT JOIN progress p ON p.profile_id = ?2 AND p.media_key =
     CASE WHEN tv.ext_id IS NOT NULL AND e.season_number IS NOT NULL
               AND e.episode_number IS NOT NULL
          THEN 'tmdb:episode:' || tv.ext_id || ':' || e.season_number || ':' || e.episode_number
          ELSE 'file:' || (SELECT kf.fingerprint FROM match_episodes kme
                            JOIN matches km ON km.id = kme.match_id AND km.state <> 'rejected'
                            JOIN files kf ON kf.id = km.file_id
                           WHERE kme.episode_id = e.id AND kf.fingerprint <> ''
                             AND kf.fingerprint NOT GLOB '*[^0-9a-f]*'
                           ORDER BY kf.missing_since_ms IS NOT NULL, kf.id LIMIT 1)
     END
WHERE e.work_id = ?1
  AND ",
    pa_library::file_visible!("f"),
    "
  AND ",
    pa_library::work_allowed!("e.work_id"),
    "
GROUP BY e.id"
);

/// Time left of an episode of `duration` (> 0) at `position` of a file the
/// player saw as `seen` long (0 if unknown).
fn left_of(duration: i64, position: i64, seen: i64, th: &Thresholds) -> i64 {
    if position <= 0 {
        return duration;
    }
    let base = if seen > 0 { seen } else { duration };
    #[allow(clippy::cast_precision_loss)]
    let done = position as f64 / base as f64;
    if done < th.in_progress_min {
        duration
    } else if done >= th.watched_after {
        0
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let left = (duration as f64 * (1.0 - done)).round() as i64;
        left.clamp(0, duration)
    }
}

/// Runtime of the show `work` with its seasons in `order` (one query).
pub(crate) fn show_runtime(
    conn: &Connection,
    work: i64,
    order: EpisodeOrder,
) -> Result<ShowRuntime, Error> {
    let th = Thresholds::default();
    let sql = EPISODES_SQL.replacen("{season}", season_sql(order), 1);
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params![work, profile(conn)?], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, Option<bool>>(5)?,
        ))
    })?;
    let mut seasons: Vec<SeasonRuntime> = Vec::new();
    let mut show = RuntimeTotal::default();
    for row in rows {
        let (season, probed, runtime, position, seen, watched) = row?;
        let probed = probed.filter(|d| *d > 0);
        let (duration_ms, exact) = match (probed, runtime.filter(|d| *d > 0)) {
            (Some(d), _) => (d, true),
            (None, Some(d)) => (d, false),
            (None, None) => (0, false),
        };
        let watched = watched.unwrap_or(false);
        let ep = EpisodeRuntime {
            duration_ms,
            exact,
            watched,
            remaining_ms: if watched || duration_ms == 0 {
                0
            } else {
                left_of(
                    duration_ms,
                    position.unwrap_or(0),
                    seen.unwrap_or(0),
                    &th,
                )
            },
        };
        match seasons.iter_mut().find(|s| s.season == season) {
            Some(s) => s.total.add(&ep),
            None => {
                let mut total = RuntimeTotal::default();
                total.add(&ep);
                seasons.push(SeasonRuntime { season, total });
            }
        }
        if season != 0 {
            show.add(&ep);
        }
    }
    // Like the season tabs: regular seasons ascending, specials last.
    seasons.sort_by_key(|s| (s.season == 0, s.season));
    Ok(ShowRuntime { show, seasons })
}

impl Catalog {
    /// Total runtime and time left of a show and of each of its seasons in
    /// `order` (UI-056). [`Catalog::show_episodes`] carries the same for the
    /// order it applies.
    pub fn show_runtime(&self, show: ItemRef, order: EpisodeOrder) -> Result<ShowRuntime, Error> {
        let ItemRef::Show(work) = show else {
            return Err(Error::Invalid(format!("not a show: {show}")));
        };
        let conn = self.lock();
        crate::profiles::require_visible(&conn, show)?;
        show_runtime(&conn, work, order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_follows_the_thresholds() {
        let th = Thresholds::default();
        let min = 60_000;
        // Not started, a stray second, half way, at the end.
        assert_eq!(left_of(40 * min, 0, 0, &th), 40 * min);
        assert_eq!(left_of(40 * min, 1_000, 40 * min, &th), 40 * min);
        assert_eq!(left_of(40 * min, 10 * min, 40 * min, &th), 30 * min);
        assert_eq!(left_of(40 * min, 39 * min, 40 * min, &th), 0);
        // Another version of the episode: the share carries over.
        assert_eq!(left_of(40 * min, 25 * min, 50 * min, &th), 20 * min);
        // The player never said how long it is: this file's duration.
        assert_eq!(left_of(40 * min, 10 * min, 0, &th), 30 * min);
    }
}
