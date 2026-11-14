// SPDX-License-Identifier: MPL-2.0
//! Episodes of a show in the order the profile chose (UI-010, the
//! `DisplayPreference` of 03 §4.2): TMDB, absolute or AniDB numbering.
//!
//! The order only renumbers what the Show Detail screen shows; files,
//! matches and progress keys stay as they are.
//!
//! - **TMDB** (default): `episodes.season_number` / `episode_number`.
//! - **Absolute**: regular episodes in one run, `episodes.absolute_number`
//!   when the provider gave it, else counted over all regular episodes the
//!   catalog knows of the work (also the ones without a file), in TMDB
//!   order. Specials stay in season 0. Stored as the `tvdb_absolute` scheme.
//! - **AniDB**: `episode_mappings` rows of the `anidb` scheme. AniDB has no
//!   seasons: `season_number` of a row is the AniDB entry of the franchise
//!   (1, 2…), NULL meaning 1, and 0 holds the AniDB specials. Episodes
//!   without a row keep their TMDB numbers. Offered only when at least one
//!   present episode has a row. The metadata run fills them from the
//!   Anime-Lists list downloaded on the device ([`crate::anime_order`],
//!   META-029).
//!
//! The choice is stored per profile and work in `display_preferences`.

use std::collections::HashMap;

use pa_library::progress;
use pa_library::rusqlite::{Connection, OptionalExtension, params};

use crate::items::{self, Item, ItemRef};
use crate::progress::{ItemProgress, PROFILE, key_of};
use crate::{Catalog, Error};

/// How a show numbers its episodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EpisodeOrder {
    Tmdb,
    Absolute,
    Anidb,
}

impl EpisodeOrder {
    /// The `display_preferences.scheme` value.
    #[must_use]
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Tmdb => "tmdb",
            Self::Absolute => "tvdb_absolute",
            Self::Anidb => "anidb",
        }
    }

    fn from_scheme(s: &str) -> Option<Self> {
        match s {
            "tmdb" => Some(Self::Tmdb),
            "tvdb_absolute" => Some(Self::Absolute),
            "anidb" => Some(Self::Anidb),
            _ => None,
        }
    }
}

/// An episode with its numbers in the chosen order.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowEpisode {
    /// `season` and `episode` are in the chosen order.
    pub item: Item,
    /// Numbers in TMDB order.
    pub tmdb_season: Option<i64>,
    pub tmdb_episode: Option<i64>,
    pub progress: Option<ItemProgress>,
}

/// Episodes of a show for the Show Detail screen.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowEpisodes {
    /// The order the episodes are in.
    pub order: EpisodeOrder,
    /// Orders the catalog has data for; TMDB first.
    pub available: Vec<EpisodeOrder>,
    /// By season, then episode; specials (season 0) first.
    pub episodes: Vec<ShowEpisode>,
}

fn work_of(show: ItemRef) -> Result<i64, Error> {
    match show {
        ItemRef::Show(w) => Ok(w),
        other => Err(Error::Invalid(format!("not a show: {other}"))),
    }
}

fn episode_id(item: &Item) -> i64 {
    match item.id {
        ItemRef::Episode(e) => e,
        ItemRef::Movie(i) | ItemRef::Show(i) | ItemRef::Track(i) => i,
    }
}

/// Absolute numbers of the regular episodes of `work`.
fn absolute_numbers(conn: &Connection, work: i64) -> Result<HashMap<i64, i64>, Error> {
    let mut stmt = conn.prepare(
        "SELECT id, absolute_number FROM episodes
         WHERE work_id = ?1 AND season_number >= 1 AND episode_type = 'regular'
           AND episode_number IS NOT NULL
         ORDER BY season_number, episode_number, id",
    )?;
    let rows = stmt.query_map([work], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
    })?;
    let mut out = HashMap::new();
    let mut n = 0;
    for row in rows {
        let (id, absolute) = row?;
        n += 1;
        out.insert(id, absolute.unwrap_or(n));
    }
    Ok(out)
}

/// AniDB numbers (entry, episode) of the episodes of `work` that have them.
fn anidb_numbers(conn: &Connection, work: i64) -> Result<HashMap<i64, (i64, i64)>, Error> {
    let mut stmt = conn.prepare(
        "SELECT em.episode_id, ifnull(em.season_number, 1), em.episode_number
         FROM episode_mappings em JOIN episodes e ON e.id = em.episode_id
         WHERE e.work_id = ?1 AND em.scheme = 'anidb'
         ORDER BY em.episode_id, em.confidence DESC, em.part, em.id",
    )?;
    let rows = stmt.query_map([work], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?),
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, numbers) = row?;
        out.entry(id).or_insert(numbers);
    }
    Ok(out)
}

fn stored_order(conn: &Connection, work: i64) -> Result<Option<EpisodeOrder>, Error> {
    let scheme: Option<String> = conn
        .query_row(
            "SELECT scheme FROM display_preferences WHERE profile_id = ?1 AND work_id = ?2",
            params![PROFILE, work],
            |r| r.get(0),
        )
        .optional()?;
    Ok(scheme.as_deref().and_then(EpisodeOrder::from_scheme))
}

fn episode_progress(conn: &Connection, item: ItemRef) -> Result<Option<ItemProgress>, Error> {
    let Some((key, _)) = key_of(conn, item)? else {
        return Ok(None);
    };
    Ok(progress::get(conn, PROFILE, &key)?.map(|p| ItemProgress {
        item,
        position_ms: i64::try_from(p.position_ms).unwrap_or(i64::MAX),
        duration_ms: p
            .duration_ms
            .map_or(0, |d| i64::try_from(d).unwrap_or(i64::MAX)),
        updated_at_ms: p.updated_at_ms,
        watched: p.watched,
    }))
}

pub(crate) fn show_episodes(
    conn: &Connection,
    work: i64,
    order: Option<EpisodeOrder>,
) -> Result<ShowEpisodes, Error> {
    let list = items::episodes(conn, work)?;
    let anidb = anidb_numbers(conn, work)?;
    let mut available = vec![EpisodeOrder::Tmdb];
    if list.iter().any(|e| e.season.is_some_and(|s| s >= 1)) {
        available.push(EpisodeOrder::Absolute);
    }
    if list.iter().any(|e| anidb.contains_key(&episode_id(e))) {
        available.push(EpisodeOrder::Anidb);
    }
    let wanted = match order {
        Some(o) => o,
        None => stored_order(conn, work)?.unwrap_or(EpisodeOrder::Tmdb),
    };
    let order = if available.contains(&wanted) {
        wanted
    } else {
        EpisodeOrder::Tmdb
    };
    let absolute = if order == EpisodeOrder::Absolute {
        absolute_numbers(conn, work)?
    } else {
        HashMap::new()
    };

    let mut episodes = Vec::with_capacity(list.len());
    for item in list {
        let id = episode_id(&item);
        let (tmdb_season, tmdb_episode) = (item.season, item.episode);
        let (season, episode) = match order {
            EpisodeOrder::Tmdb => (tmdb_season, tmdb_episode),
            EpisodeOrder::Absolute => match (tmdb_season, absolute.get(&id)) {
                (Some(0), _) | (_, None) => (tmdb_season, tmdb_episode),
                (_, Some(n)) => (Some(1), Some(*n)),
            },
            EpisodeOrder::Anidb => match anidb.get(&id) {
                Some((s, e)) => (Some(*s), Some(*e)),
                None => (tmdb_season, tmdb_episode),
            },
        };
        let progress = episode_progress(conn, item.id)?;
        episodes.push(ShowEpisode {
            item: Item {
                season,
                episode,
                ..item
            },
            tmdb_season,
            tmdb_episode,
            progress,
        });
    }
    episodes.sort_by(|a, b| {
        (a.item.season.unwrap_or(1), a.item.episode)
            .cmp(&(b.item.season.unwrap_or(1), b.item.episode))
            .then_with(|| a.item.rel_path.cmp(&b.item.rel_path))
    });
    Ok(ShowEpisodes {
        order,
        available,
        episodes,
    })
}

impl Catalog {
    /// Episodes of a show in `order`, or in the order stored for it
    /// ([`set_episode_order`](Self::set_episode_order)) when `None`. An
    /// order the catalog has no data for falls back to TMDB.
    pub fn show_episodes(
        &self,
        show: ItemRef,
        order: Option<EpisodeOrder>,
    ) -> Result<ShowEpisodes, Error> {
        let work = work_of(show)?;
        show_episodes(&self.lock(), work, order)
    }

    /// Stores the episode order of a show for the profile.
    pub fn set_episode_order(&self, show: ItemRef, order: EpisodeOrder) -> Result<(), Error> {
        let work = work_of(show)?;
        let conn = self.lock();
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM works WHERE id = ?1)",
            [work],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::Invalid(format!("no show {show}")));
        }
        conn.execute(
            "INSERT INTO display_preferences (profile_id, work_id, scheme, scheme_ref)
             VALUES (?1, ?2, ?3, '')
             ON CONFLICT (profile_id, work_id) DO UPDATE SET scheme = ?3, scheme_ref = ''",
            params![PROFILE, work, order.scheme()],
        )?;
        Ok(())
    }
}
