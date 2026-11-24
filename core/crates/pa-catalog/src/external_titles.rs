// SPDX-License-Identifier: MPL-2.0
//! Library titles by their ids at TMDB, IMDb and TVDB, for trackers such
//! as Trakt (SYNC-011): which title an item is, what is watched and rated
//! here, marking a title watched at a given time, and items of a list of
//! titles (a watchlist shelf). The tracker client itself lives elsewhere
//! (`pa-trakt`); this module only speaks the library's terms.

use pa_library::progress::{self as lib_progress, LocalOnly};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_model::MediaKey;
use pa_sync::library::AliasedSink;

use crate::items::ItemRef;
use crate::progress::{key_of, profile};
use crate::{Catalog, Error, LOCAL_DEVICE_ID};

/// What a title is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TitleKind {
    Movie,
    Show,
    Episode,
}

/// A movie, a show or an episode by external ids. For an episode the ids
/// are those of its show, plus the season and episode numbers (TMDB
/// numbering).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ExternalTitle {
    pub kind: Option<TitleKind>,
    pub tmdb: Option<u64>,
    pub imdb: Option<String>,
    pub tvdb: Option<u64>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl ExternalTitle {
    /// A movie by TMDB id.
    pub fn movie(tmdb: u64) -> Self {
        Self {
            kind: Some(TitleKind::Movie),
            tmdb: Some(tmdb),
            ..Self::default()
        }
    }

    /// A show by TMDB id.
    pub fn show(tmdb: u64) -> Self {
        Self {
            kind: Some(TitleKind::Show),
            tmdb: Some(tmdb),
            ..Self::default()
        }
    }

    /// An episode by the show's TMDB id and TMDB numbering.
    pub fn episode(show_tmdb: u64, season: u32, episode: u32) -> Self {
        Self {
            kind: Some(TitleKind::Episode),
            tmdb: Some(show_tmdb),
            season: Some(season),
            episode: Some(episode),
            ..Self::default()
        }
    }

    /// The progress key of a movie or an episode.
    fn key(&self) -> Option<MediaKey> {
        match self.kind? {
            TitleKind::Movie => Some(MediaKey::TmdbMovie { id: self.tmdb? }),
            TitleKind::Episode => Some(MediaKey::TmdbEpisode {
                show_id: self.tmdb?,
                season: self.season?,
                episode: self.episode?,
            }),
            TitleKind::Show => None,
        }
    }
}

/// The work with TMDB id `tmdb` of `ext_kind` (`movie`, `tv`).
fn work_of(conn: &Connection, ext_kind: &str, tmdb: u64) -> Result<Option<i64>, Error> {
    Ok(conn
        .query_row(
            "SELECT work_id FROM external_ids
             WHERE provider = 'tmdb' AND ext_kind = ?1 AND ext_id = ?2",
            params![ext_kind, tmdb.to_string()],
            |r| r.get(0),
        )
        .optional()?)
}

/// The external ids of `work` into `title`.
fn fill_ids(conn: &Connection, work: i64, title: &mut ExternalTitle) -> Result<(), Error> {
    let mut stmt =
        conn.prepare_cached("SELECT provider, ext_id FROM external_ids WHERE work_id = ?1")?;
    let rows = stmt.query_map([work], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (provider, id) = row?;
        match provider.as_str() {
            "tmdb" => title.tmdb = id.parse().ok().or(title.tmdb),
            "imdb" if id.starts_with("tt") => title.imdb = Some(id),
            "tvdb" => title.tvdb = id.parse().ok(),
            _ => {}
        }
    }
    Ok(())
}

/// The title of a progress key, with every id the library knows.
fn title_of_key(conn: &Connection, key: &MediaKey) -> Result<Option<ExternalTitle>, Error> {
    let (mut title, work) = match key {
        MediaKey::TmdbMovie { id } => (ExternalTitle::movie(*id), work_of(conn, "movie", *id)?),
        MediaKey::TmdbEpisode {
            show_id,
            season,
            episode,
        } => (
            ExternalTitle::episode(*show_id, *season, *episode),
            work_of(conn, "tv", *show_id)?,
        ),
        _ => return Ok(None),
    };
    if let Some(w) = work {
        fill_ids(conn, w, &mut title)?;
    }
    Ok(Some(title))
}

impl Catalog {
    /// The external title of a movie, show or episode; `None` for a track
    /// or an item the metadata did not identify.
    pub fn external_title(&self, item: ItemRef) -> Result<Option<ExternalTitle>, Error> {
        let conn = self.lock();
        crate::profiles::require_visible(&conn, item)?;
        let (work, mut title) = match item {
            ItemRef::Movie(w) => (
                w,
                ExternalTitle {
                    kind: Some(TitleKind::Movie),
                    ..ExternalTitle::default()
                },
            ),
            ItemRef::Show(w) => (
                w,
                ExternalTitle {
                    kind: Some(TitleKind::Show),
                    ..ExternalTitle::default()
                },
            ),
            ItemRef::Episode(ep) => {
                let row: Option<(i64, Option<u32>, Option<u32>)> = conn
                    .query_row(
                        "SELECT work_id, season_number, episode_number FROM episodes WHERE id = ?1",
                        [ep],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((w, Some(season), Some(episode))) = row else {
                    return Ok(None);
                };
                (
                    w,
                    ExternalTitle {
                        kind: Some(TitleKind::Episode),
                        season: Some(season),
                        episode: Some(episode),
                        ..ExternalTitle::default()
                    },
                )
            }
            ItemRef::Track(_) => return Ok(None),
        };
        fill_ids(&conn, work, &mut title)?;
        let known = title.tmdb.is_some() || title.imdb.is_some() || title.tvdb.is_some();
        Ok(known.then_some(title))
    }

    /// Movies and episodes the active profile watched, by TMDB id, with
    /// the time of the last change.
    pub fn watched_titles(&self) -> Result<Vec<(ExternalTitle, i64)>, Error> {
        let conn = self.lock();
        let profile_id = profile(&conn)?;
        let mut stmt = conn.prepare_cached(
            "SELECT media_key, updated_at_ms FROM progress
             WHERE profile_id = ?1 AND watched = 1 AND media_key GLOB 'tmdb:*'",
        )?;
        let rows: Vec<(String, i64)> = stmt
            .query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (text, at) in rows {
            if let Ok(key) = text.parse::<MediaKey>()
                && let Some(title) = title_of_key(&conn, &key)?
            {
                out.push((title, at));
            }
        }
        Ok(out)
    }

    /// Marks a movie or an episode watched at `at_ms` (a tracker's time) if
    /// it is in the library, is not watched yet and nothing newer happened
    /// to it here (a later "unwatched" or a newer position wins). Synced to
    /// paired devices like any mark. Whether it changed.
    pub fn mark_title_watched(&self, title: &ExternalTitle, at_ms: i64) -> Result<bool, Error> {
        let Some(key) = title.key() else {
            return Ok(false);
        };
        let mut conn = self.lock();
        let Some(item) = crate::progress::item_of_key(&conn, &key)? else {
            return Ok(false);
        };
        if crate::profiles::require_visible(&conn, item).is_err() {
            return Ok(false);
        }
        let profile_id = profile(&conn)?;
        if let Some(p) = crate::matching::merged(&conn, profile_id, item)?
            && (p.watched || p.updated_at_ms >= at_ms)
        {
            return Ok(false);
        }
        let Some((primary, _)) = key_of(&conn, item)? else {
            return Ok(false);
        };
        let aliases = crate::matching::item_keys(&conn, item)?
            .map(|k| k.alias_texts())
            .unwrap_or_default();
        let mut local = LocalOnly::new(LOCAL_DEVICE_ID);
        let mut replica = self.sync_lock();
        let mut aliased;
        let sink: &mut dyn lib_progress::ProgressSink = match replica.as_mut() {
            Some(r) => {
                aliased = AliasedSink {
                    replica: r,
                    aliases,
                };
                &mut aliased
            }
            None => &mut local,
        };
        Ok(lib_progress::mark_watched(
            &mut conn, sink, profile_id, &primary, at_ms,
        )?)
    }

    /// Movies and shows the active profile rated: title, stars (1–5) and
    /// when.
    pub fn rated_titles(&self) -> Result<Vec<(ExternalTitle, u8, i64)>, Error> {
        let conn = self.lock();
        let profile_id = profile(&conn)?;
        let mut stmt = conn.prepare_cached(
            "SELECT r.work_id, r.stars, r.updated_at_ms, x.ext_kind FROM user_ratings r
             JOIN external_ids x ON x.work_id = r.work_id AND x.provider = 'tmdb'
             WHERE r.profile_id = ?1 AND x.ext_kind IN ('movie', 'tv')",
        )?;
        let rows: Vec<(i64, u8, i64, String)> = stmt
            .query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (work, stars, at, kind) in rows {
            let mut title = ExternalTitle {
                kind: Some(if kind == "movie" {
                    TitleKind::Movie
                } else {
                    TitleKind::Show
                }),
                ..ExternalTitle::default()
            };
            fill_ids(&conn, work, &mut title)?;
            out.push((title, stars, at));
        }
        Ok(out)
    }

    /// The library movie or show of `title` (by TMDB id), if it is here
    /// and visible to the active profile.
    pub fn item_of_title(&self, title: &ExternalTitle) -> Result<Option<ItemRef>, Error> {
        let (Some(kind), Some(tmdb)) = (title.kind, title.tmdb) else {
            return Ok(None);
        };
        let conn = self.lock();
        let item = match kind {
            TitleKind::Movie => work_of(&conn, "movie", tmdb)?.map(ItemRef::Movie),
            TitleKind::Show => work_of(&conn, "tv", tmdb)?.map(ItemRef::Show),
            TitleKind::Episode => match title.key() {
                Some(key) => crate::progress::item_of_key(&conn, &key)?,
                None => None,
            },
        };
        Ok(item.filter(|i| crate::profiles::require_visible(&conn, *i).is_ok()))
    }

    /// Rates the movie or show of `title` 1–5 stars if it is here. Whether
    /// it changed.
    pub fn rate_title(&self, title: &ExternalTitle, stars: u8) -> Result<bool, Error> {
        match self.item_of_title(title)? {
            Some(item @ (ItemRef::Movie(_) | ItemRef::Show(_))) => {
                self.labels().set_rating(item, Some(stars))
            }
            _ => Ok(false),
        }
    }
}
