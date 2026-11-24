// SPDX-License-Identifier: MPL-2.0
//! Titles as Trakt names them: ids of a movie, show or episode.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Ids of a movie or show. Trakt matches by any of them; we always have
/// the TMDB id, IMDb and TVDB when the metadata had them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ids {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trakt: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmdb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb: Option<u64>,
}

impl Ids {
    /// Ids with just the TMDB id.
    pub fn tmdb(id: u64) -> Self {
        Self {
            tmdb: Some(id),
            ..Self::default()
        }
    }

    /// Whether Trakt can find the title by these ids.
    pub fn is_empty(&self) -> bool {
        self.trakt.is_none()
            && self.tmdb.is_none()
            && self.tvdb.is_none()
            && self.imdb.as_deref().is_none_or(str::is_empty)
    }
}

/// A movie, a show or an episode of a show.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Movie(Ids),
    Show(Ids),
    /// An episode by its show and Trakt numbering (TMDB numbering in the
    /// library: the same for nearly every show).
    Episode {
        show: Ids,
        season: u32,
        number: u32,
    },
}

impl Target {
    /// The identity used to pair a Trakt entry with a library one: the TMDB
    /// id (`m:603`, `s:1399`, `e:1399:1:2`). `None` without a TMDB id.
    pub fn key(&self) -> Option<String> {
        Some(match self {
            Self::Movie(ids) => format!("m:{}", ids.tmdb?),
            Self::Show(ids) => format!("s:{}", ids.tmdb?),
            Self::Episode {
                show,
                season,
                number,
            } => format!("e:{}:{season}:{number}", show.tmdb?),
        })
    }

    /// The scrobble body without `progress`.
    pub(crate) fn scrobble_json(&self) -> Value {
        match self {
            Self::Movie(ids) => json!({ "movie": { "ids": ids } }),
            Self::Show(ids) => json!({ "show": { "ids": ids } }),
            Self::Episode {
                show,
                season,
                number,
            } => json!({
                "show": { "ids": show },
                "episode": { "season": season, "number": number },
            }),
        }
    }
}

/// A title watched at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub target: Target,
    /// Unix ms.
    pub watched_at_ms: i64,
}

/// A rating of a movie or show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rated {
    pub target: Target,
    /// Trakt's 1–10.
    pub rating: u8,
    /// Unix ms.
    pub rated_at_ms: i64,
}

/// A watchlist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// A movie or show.
    pub target: Target,
    /// Unix ms.
    pub listed_at_ms: i64,
}

/// Our 1–5 stars (LIB-022) as Trakt's 1–10: twice as many.
pub fn stars_to_trakt(stars: u8) -> u8 {
    stars.clamp(1, 5) * 2
}

/// Trakt's 1–10 as our 1–5 stars, halves rounded up (7 → 4, 1 → 1).
pub fn stars_from_trakt(rating: u8) -> u8 {
    rating.clamp(1, 10).div_ceil(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_convert_both_ways() {
        for s in 1..=5 {
            assert_eq!(stars_from_trakt(stars_to_trakt(s)), s);
        }
        assert_eq!(stars_from_trakt(1), 1);
        assert_eq!(stars_from_trakt(7), 4);
        assert_eq!(stars_from_trakt(10), 5);
    }

    #[test]
    fn keys_need_tmdb() {
        assert_eq!(
            Target::Movie(Ids::tmdb(603)).key().as_deref(),
            Some("m:603")
        );
        let ep = Target::Episode {
            show: Ids::tmdb(1399),
            season: 1,
            number: 2,
        };
        assert_eq!(ep.key().as_deref(), Some("e:1399:1:2"));
        assert_eq!(Target::Show(Ids::default()).key(), None);
    }
}
