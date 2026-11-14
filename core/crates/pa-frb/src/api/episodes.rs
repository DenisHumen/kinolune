// SPDX-License-Identifier: MPL-2.0
//! Episodes of a show in the profile's episode order (UI-010) for Dart. The
//! calls are [`CoreLibrary::show_episodes`](crate::api::library::CoreLibrary::show_episodes)
//! and [`CoreLibrary::set_episode_order`](crate::api::library::CoreLibrary::set_episode_order).

use pa_catalog as c;

use crate::api::library::{CoreItem, CoreProgress};

/// How a show numbers its episodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEpisodeOrder {
    Tmdb,
    /// One run of regular episodes; specials stay in season 0.
    Absolute,
    /// AniDB entries as seasons (from anime mappings).
    Anidb,
}

impl From<c::EpisodeOrder> for CoreEpisodeOrder {
    fn from(o: c::EpisodeOrder) -> Self {
        match o {
            c::EpisodeOrder::Tmdb => Self::Tmdb,
            c::EpisodeOrder::Absolute => Self::Absolute,
            c::EpisodeOrder::Anidb => Self::Anidb,
        }
    }
}

impl From<CoreEpisodeOrder> for c::EpisodeOrder {
    fn from(o: CoreEpisodeOrder) -> Self {
        match o {
            CoreEpisodeOrder::Tmdb => Self::Tmdb,
            CoreEpisodeOrder::Absolute => Self::Absolute,
            CoreEpisodeOrder::Anidb => Self::Anidb,
        }
    }
}

/// An episode numbered in the chosen order.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreShowEpisode {
    /// `season` and `episode` in the chosen order.
    pub item: CoreItem,
    /// Numbers in TMDB order.
    pub tmdb_season: Option<i32>,
    pub tmdb_episode: Option<i32>,
    pub progress: Option<CoreProgress>,
}

/// Episodes of a show for the Show Detail screen.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreShowEpisodes {
    pub order: CoreEpisodeOrder,
    /// Orders the core has data for; TMDB first.
    pub available: Vec<CoreEpisodeOrder>,
    pub episodes: Vec<CoreShowEpisode>,
}

fn small(v: Option<i64>) -> Option<i32> {
    v.and_then(|v| i32::try_from(v).ok())
}

impl From<c::ShowEpisodes> for CoreShowEpisodes {
    fn from(s: c::ShowEpisodes) -> Self {
        Self {
            order: s.order.into(),
            available: s.available.into_iter().map(Into::into).collect(),
            episodes: s
                .episodes
                .into_iter()
                .map(|e| CoreShowEpisode {
                    tmdb_season: small(e.tmdb_season),
                    tmdb_episode: small(e.tmdb_episode),
                    progress: e.progress.map(Into::into),
                    item: e.item.into(),
                })
                .collect(),
        }
    }
}
