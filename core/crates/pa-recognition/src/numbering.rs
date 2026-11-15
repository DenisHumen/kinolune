// SPDX-License-Identifier: MPL-2.0
//! Episode numbering (META-015): one canonical episode space and its
//! projections into the orders providers and releasers use — stage \[6\] of
//! research 03 §4.2.
//!
//! - **Canonical space.** Anime episodes live in AniDB: an AniDB entry (one
//!   per season, cour or OVA) and an episode of an AniDB type — regular,
//!   special (`S1`), credit (`C1`, NCOP/NCED), trailer, parody, other.
//!   Everything else lives in TMDB: series, season, episode
//!   ([`Canonical`]).
//! - **Series layout.** A [`Series`] is one TMDB/TVDB series with the AniDB
//!   entries it is split into, in broadcast order, and the rules of each
//!   entry: where its regular episodes sit in TMDB, TVDB aired and DVD
//!   order (a season and an offset, or the absolute order), explicit pairs
//!   for what the offsets do not cover (Anime-Lists `mapping-list`, TheXEM),
//!   the regular episode counts of TMDB seasons and the TMDB episode groups.
//!   [`SeriesBook::from_mappings`] builds them from the Anime-Lists list the
//!   device downloaded (`pa-anime-mappings`, META-008): entries sharing a
//!   TVDB or TMDB series form one series.
//! - **Reading.** What a releaser wrote ([`Written`]: an absolute number, a
//!   season and episode, a number inside a named entry, a special) is
//!   resolved into the canonical episode ([`Series::resolve`]); a number
//!   past the end of an entry runs on into the next entries.
//! - **Projections.** A canonical episode is projected into every order
//!   ([`Series::project`] → [`Projections`]): AniDB, TMDB, TVDB aired, TVDB
//!   DVD, TVDB absolute (the running count of regular episodes across the
//!   entries), TMDB episode groups and scene numbering (TheXEM).
//! - **Types.** `.5` episodes, recaps, specials and creditless OP/ED go by
//!   AniDB types ([`classify`]): `.5` and `SP`/`OVA` markers are AniDB
//!   specials, NCOP/NCED are credits (extras, never a regular episode), a
//!   recap keeps its regular number (AniDB lists most recaps as regular
//!   episodes). Specials reach TMDB/TVDB only through explicit pairs, never
//!   "season 0, same number".
//! - **Display.** The order a series is shown in is chosen per series
//!   ([`DisplayOrder`], [`Projections::display`]).
//!
//! Sans-IO and deterministic like the rest of the crate.

use std::collections::{BTreeMap, HashMap};

use pa_anime_mappings::anime_lists::{AnimeListEntry, SeasonRef};
use pa_anime_mappings::{IdRef, MappingIndex};
use serde::{Deserialize, Serialize};

use crate::parse::{EpisodeKind, ParsedName};
use crate::title::normalize::TitleKey;

/// AniDB episode type.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AnidbType {
    /// Regular episode (`1`).
    #[default]
    Regular,
    /// Special (`S1`): OVA episodes of a TV entry, `.5` episodes, recaps
    /// AniDB keeps apart.
    Special,
    /// Credit (`C1`): creditless opening or ending.
    Credit,
    /// Trailer, promo (`T1`).
    Trailer,
    /// Parody (`P1`).
    Parody,
    /// Other (`O1`).
    Other,
}

impl AnidbType {
    /// The corpus / `episodes.episode_type` name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::Special => "special",
            Self::Credit => "credit",
            Self::Trailer => "trailer",
            Self::Parody => "parody",
            Self::Other => "other",
        }
    }

    /// The prefix AniDB writes before the number (`S1`, `C2`).
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Regular => "",
            Self::Special => "S",
            Self::Credit => "C",
            Self::Trailer => "T",
            Self::Parody => "P",
            Self::Other => "O",
        }
    }
}

/// A season and an episode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct SeasonEpisode {
    /// Season; 0 holds specials.
    pub season: u32,
    /// Episode inside the season.
    pub episode: u32,
}

impl SeasonEpisode {
    /// `season`x`episode`.
    #[must_use]
    pub const fn new(season: u32, episode: u32) -> Self {
        Self { season, episode }
    }
}

/// Where the regular episodes of an entry sit in an order: episode `n` of
/// the entry is episode `n + offset` of `season`, or absolute `n + offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// A numbered season (0 = specials).
    Season {
        /// Season.
        season: u32,
        /// Episodes of the season before this entry.
        offset: i32,
    },
    /// The order's absolute numbering.
    Absolute {
        /// Episodes before this entry.
        offset: i32,
    },
}

/// A numbering order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scheme {
    /// TMDB seasons.
    Tmdb,
    /// TVDB aired order.
    TvdbAired,
    /// TVDB DVD order.
    TvdbDvd,
}

/// An explicit position for one AniDB episode in one order (Anime-Lists
/// `mapping-list`, TheXEM): wins over the entry's placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    /// Order.
    pub scheme: Scheme,
    /// AniDB episode type.
    pub kind: AnidbType,
    /// AniDB episode number.
    pub number: u32,
    /// Episodes it maps onto; empty: none in that order.
    pub to: Vec<SeasonEpisode>,
}

/// One AniDB entry of a series.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// AniDB id.
    pub anidb: u32,
    /// Titles (Anime-Lists name, anime-offline-database titles).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub titles: Vec<String>,
    /// Number of regular episodes, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episodes: Option<u32>,
    /// Placement in TMDB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmdb: Option<Placement>,
    /// Placement in TVDB aired order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb: Option<Placement>,
    /// Placement in TVDB DVD order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb_dvd: Option<Placement>,
    /// Explicit pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<Pair>,
}

impl Entry {
    /// The entry's regular episodes sit in season 0 (an OVA listed among
    /// the specials of the series): it is not part of the running count.
    #[must_use]
    pub fn special_only(&self) -> bool {
        let zero = |p: &Option<Placement>| matches!(p, Some(Placement::Season { season: 0, .. }));
        zero(&self.tvdb) || (self.tvdb.is_none() && zero(&self.tmdb))
    }

    fn placement(&self, scheme: Scheme) -> Option<Placement> {
        match scheme {
            Scheme::Tmdb => self.tmdb,
            Scheme::TvdbAired => self.tvdb,
            Scheme::TvdbDvd => self.tvdb_dvd,
        }
    }

    fn pair(&self, scheme: Scheme, kind: AnidbType, number: u32) -> Option<&Pair> {
        self.pairs
            .iter()
            .find(|p| p.scheme == scheme && p.kind == kind && p.number == number)
    }
}

/// One TMDB episode group (alternative order: absolute, DVD, story arcs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeGroup {
    /// TMDB group id (24 hex digits).
    pub id: String,
    /// Group name, for the order picker.
    #[serde(default)]
    pub name: String,
    /// Group seasons in order (`order` 0, 1…; 0 is usually specials): the
    /// TMDB episodes of each, in group order.
    pub seasons: Vec<Vec<SeasonEpisode>>,
}

/// One TheXEM link between TVDB aired order and scene numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct XemLink {
    /// TVDB aired position.
    pub tvdb: SeasonEpisode,
    /// Scene position (how releases number it).
    pub scene: SeasonEpisode,
}

/// A canonical episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Canonical {
    /// Anime: AniDB entry, type and number.
    Anidb {
        /// AniDB id of the entry.
        anidb: u32,
        /// AniDB episode type.
        kind: AnidbType,
        /// Number inside the entry and type.
        number: u32,
    },
    /// Everything else: the TMDB season and episode of the work.
    Tmdb(SeasonEpisode),
}

/// What a releaser wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// A running count across the series (`One Piece - 1085`).
    Absolute(u32),
    /// A season and episode (`S03E15`, `Season 3/… - 15`), TMDB order
    /// first, then TVDB aired order.
    Season(SeasonEpisode),
    /// A number counted from the start of the entry at this position of
    /// [`Series::entries`] (`Mugen Ressha-hen - 12` runs on into the next
    /// entry).
    Entry {
        /// Entry position.
        entry: usize,
        /// Number.
        number: u32,
    },
    /// A special of the entry at this position.
    Special {
        /// Entry position.
        entry: usize,
        /// AniDB type.
        kind: AnidbType,
        /// Number.
        number: u32,
    },
}

/// A canonical episode in every order the series has data for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projections {
    /// The canonical episode.
    pub canonical: Canonical,
    /// Position of the AniDB entry in the series (0-based), anime only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<usize>,
    /// TMDB season and episode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmdb: Option<SeasonEpisode>,
    /// TVDB aired order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb_aired: Option<SeasonEpisode>,
    /// TVDB DVD order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb_dvd: Option<SeasonEpisode>,
    /// TVDB absolute number (regular episodes only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb_absolute: Option<u32>,
    /// `(group id, group season, episode)` per TMDB episode group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tmdb_groups: Vec<(String, SeasonEpisode)>,
    /// Scene numbering (TheXEM).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<SeasonEpisode>,
}

/// The order a series is displayed in, chosen per series.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayOrder {
    /// TMDB seasons (default).
    Tmdb,
    /// TVDB aired order.
    TvdbAired,
    /// TVDB DVD order.
    TvdbDvd,
    /// One running count (season 1; specials in season 0).
    Absolute,
    /// AniDB: every entry its own season (1, 2…), specials in season 0.
    Anidb,
    /// A TMDB episode group by id.
    TmdbGroup(String),
    /// Scene numbering (TheXEM).
    Scene,
}

impl Projections {
    /// `(season, episode)` in `order`, `None` without data for it.
    #[must_use]
    pub fn display(&self, order: &DisplayOrder) -> Option<SeasonEpisode> {
        match order {
            DisplayOrder::Tmdb => self.tmdb,
            DisplayOrder::TvdbAired => self.tvdb_aired,
            DisplayOrder::TvdbDvd => self.tvdb_dvd,
            DisplayOrder::Absolute => match self.canonical {
                Canonical::Anidb {
                    kind: AnidbType::Regular,
                    ..
                }
                | Canonical::Tmdb(_) => self.tvdb_absolute.map(|a| SeasonEpisode::new(1, a)),
                Canonical::Anidb { .. } => self.tmdb.filter(|p| p.season == 0),
            },
            DisplayOrder::Anidb => match self.canonical {
                Canonical::Anidb {
                    kind: AnidbType::Regular,
                    number,
                    ..
                } => {
                    let season = u32::try_from(self.entry? + 1).ok()?;
                    Some(SeasonEpisode::new(season, number))
                }
                Canonical::Anidb {
                    kind: AnidbType::Special,
                    number,
                    ..
                } => Some(SeasonEpisode::new(0, number)),
                _ => None,
            },
            DisplayOrder::TmdbGroup(id) => self
                .tmdb_groups
                .iter()
                .find(|(g, _)| g == id)
                .map(|(_, p)| *p),
            DisplayOrder::Scene => self.scene,
        }
    }
}

/// One series: TMDB/TVDB ids, its AniDB entries and the provider data the
/// projections need.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Series {
    /// TMDB TV id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmdb: Option<u32>,
    /// TVDB series id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb: Option<u32>,
    /// AniDB entries in broadcast order; empty for a non-anime series.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// Regular episodes per TMDB season (TMDB details).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tmdb_seasons: BTreeMap<u32, u32>,
    /// TMDB episode groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<EpisodeGroup>,
    /// TheXEM links.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub xem: Vec<XemLink>,
}

fn shift(n: u32, offset: i32) -> Option<u32> {
    u32::try_from(i64::from(n) + i64::from(offset))
        .ok()
        .filter(|&v| v > 0)
}

impl Series {
    /// Position of the AniDB entry `anidb`.
    #[must_use]
    pub fn entry(&self, anidb: u32) -> Option<usize> {
        self.entries.iter().position(|e| e.anidb == anidb)
    }

    /// Positions of the entries in the running count, in order.
    fn chain(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.entries.len()).filter(|&i| !self.entries[i].special_only())
    }

    /// Regular episodes before entry `i` in the running count: the TVDB
    /// absolute offset when the list states it, else the sum of the
    /// earlier entries' counts (all must be known).
    #[must_use]
    pub fn offset(&self, i: usize) -> Option<u32> {
        let e = self.entries.get(i)?;
        if e.special_only() {
            return None;
        }
        if let Some(Placement::Absolute { offset }) = e.tvdb {
            return u32::try_from(offset).ok();
        }
        self.chain()
            .take_while(|&j| j != i)
            .map(|j| self.entries[j].episodes.filter(|&n| n > 0))
            .sum()
    }

    /// The entry a title names: the loose key of `title` equals the key of
    /// one of the entry's titles. With several, the one claiming the most
    /// titles; `None` when no entry or a tie.
    #[must_use]
    pub fn entry_named(&self, title: &str) -> Option<usize> {
        let want = TitleKey::new(title).loose;
        if want.is_empty() {
            return None;
        }
        let hits: Vec<(usize, usize)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                (
                    i,
                    e.titles
                        .iter()
                        .filter(|t| TitleKey::new(t).loose == want)
                        .count(),
                )
            })
            .filter(|&(_, n)| n > 0)
            .collect();
        let best = hits.iter().map(|&(_, n)| n).max()?;
        let mut top = hits.iter().filter(|&&(_, n)| n == best);
        let first = top.next()?.0;
        top.next().is_none().then_some(first)
    }

    /// Canonical episode of what a releaser wrote; `None` when the series
    /// has no place for it.
    #[must_use]
    pub fn resolve(&self, w: Written) -> Option<Canonical> {
        let anidb = |i: usize, number| Canonical::Anidb {
            anidb: self.entries[i].anidb,
            kind: AnidbType::Regular,
            number,
        };
        match w {
            Written::Absolute(n) if n > 0 => {
                if self.entries.is_empty() {
                    return self.tmdb_of_absolute(n).map(Canonical::Tmdb);
                }
                // The entry with the largest offset below `n` that still
                // holds it (an entry with an unknown count holds the rest).
                self.chain()
                    .filter_map(|i| Some((self.offset(i)?, i)))
                    .filter(|&(off, i)| {
                        n > off
                            && self.entries[i]
                                .episodes
                                .filter(|&c| c > 0)
                                .is_none_or(|c| n <= off + c)
                    })
                    .max_by_key(|&(off, i)| (off, std::cmp::Reverse(i)))
                    .map(|(off, i)| anidb(i, n - off))
            }
            Written::Absolute(_) => None,
            Written::Season(p) => {
                if self.entries.is_empty() {
                    let fits = self
                        .tmdb_seasons
                        .get(&p.season)
                        .is_none_or(|&c| p.episode <= c);
                    return (p.episode > 0 && fits).then_some(Canonical::Tmdb(p));
                }
                [Scheme::Tmdb, Scheme::TvdbAired]
                    .into_iter()
                    .find_map(|s| self.at_season(s, p))
            }
            Written::Entry { entry, number } => {
                let e = self.entries.get(entry)?;
                match e.episodes.filter(|&c| c > 0) {
                    Some(c) if number > c => {
                        // Runs on into the next entries of the running count.
                        let off = self.offset(entry)?;
                        self.resolve(Written::Absolute(off + number))
                    }
                    _ if number > 0 => Some(anidb(entry, number)),
                    _ => None,
                }
            }
            Written::Special {
                entry,
                kind,
                number,
            } => {
                let e = self.entries.get(entry)?;
                (number > 0).then_some(Canonical::Anidb {
                    anidb: e.anidb,
                    kind,
                    number,
                })
            }
        }
    }

    /// AniDB episode at `p` of `scheme`: explicit pairs first, then the
    /// entry with the largest offset in that season that covers it.
    fn at_season(&self, scheme: Scheme, p: SeasonEpisode) -> Option<Canonical> {
        for e in &self.entries {
            if let Some(pair) = e
                .pairs
                .iter()
                .find(|x| x.scheme == scheme && x.to.contains(&p))
            {
                return Some(Canonical::Anidb {
                    anidb: e.anidb,
                    kind: pair.kind,
                    number: pair.number,
                });
            }
        }
        let mut best: Option<(i32, usize, u32)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            let Some(Placement::Season { season, offset }) = e.placement(scheme) else {
                continue;
            };
            if season != p.season || season == 0 {
                continue;
            }
            let Some(n) = shift(p.episode, -offset) else {
                continue;
            };
            if e.episodes.is_some_and(|c| c > 0 && n > c) {
                continue;
            }
            if best.is_none_or(|(o, _, _)| offset > o) {
                best = Some((offset, i, n));
            }
        }
        best.map(|(_, i, n)| Canonical::Anidb {
            anidb: self.entries[i].anidb,
            kind: AnidbType::Regular,
            number: n,
        })
    }

    /// TMDB position of a running count over the TMDB seasons (1, 2…).
    fn tmdb_of_absolute(&self, n: u32) -> Option<SeasonEpisode> {
        let mut left = n;
        for (&s, &c) in self.tmdb_seasons.range(1..) {
            if c == 0 {
                return None;
            }
            if left <= c {
                return Some(SeasonEpisode::new(s, left));
            }
            left -= c;
        }
        None
    }

    /// Running count of a TMDB position over the TMDB seasons.
    fn absolute_of_tmdb(&self, p: SeasonEpisode) -> Option<u32> {
        if p.season == 0 {
            return None;
        }
        let before: Option<u32> = (1..p.season)
            .map(|s| self.tmdb_seasons.get(&s).copied())
            .sum();
        let count = self.tmdb_seasons.get(&p.season)?;
        (p.episode <= *count).then_some(before? + p.episode)
    }

    fn position(&self, i: usize, scheme: Scheme, kind: AnidbType, n: u32) -> Option<SeasonEpisode> {
        let e = &self.entries[i];
        if let Some(pair) = e.pair(scheme, kind, n) {
            return pair.to.first().copied();
        }
        if kind != AnidbType::Regular {
            return None;
        }
        match e.placement(scheme)? {
            Placement::Season { season, offset } => {
                shift(n, offset).map(|ep| SeasonEpisode::new(season, ep))
            }
            Placement::Absolute { offset } => match scheme {
                Scheme::Tmdb => shift(n, offset).and_then(|a| self.tmdb_of_absolute(a)),
                Scheme::TvdbAired | Scheme::TvdbDvd => None,
            },
        }
    }

    /// The canonical episode in every order the series has data for.
    #[must_use]
    pub fn project(&self, c: Canonical) -> Projections {
        let mut out = Projections {
            canonical: c,
            entry: None,
            tmdb: None,
            tvdb_aired: None,
            tvdb_dvd: None,
            tvdb_absolute: None,
            tmdb_groups: Vec::new(),
            scene: None,
        };
        match c {
            Canonical::Anidb {
                anidb,
                kind,
                number,
            } => {
                let Some(i) = self.entry(anidb) else {
                    return out;
                };
                out.entry = Some(i);
                out.tmdb = self.position(i, Scheme::Tmdb, kind, number);
                out.tvdb_aired = self.position(i, Scheme::TvdbAired, kind, number);
                out.tvdb_dvd = self.position(i, Scheme::TvdbDvd, kind, number);
                if kind == AnidbType::Regular {
                    out.tvdb_absolute = self.offset(i).map(|o| o + number);
                }
            }
            Canonical::Tmdb(p) => {
                out.tmdb = Some(p);
                out.tvdb_absolute = self.absolute_of_tmdb(p);
            }
        }
        if let Some(t) = out.tmdb {
            for g in &self.groups {
                for (order, eps) in g.seasons.iter().enumerate() {
                    if let Some(k) = eps.iter().position(|x| *x == t)
                        && let (Ok(s), Ok(e)) = (u32::try_from(order), u32::try_from(k + 1))
                    {
                        out.tmdb_groups.push((g.id.clone(), SeasonEpisode::new(s, e)));
                    }
                }
            }
        }
        if let Some(t) = out.tvdb_aired {
            out.scene = self.xem.iter().find(|x| x.tvdb == t).map(|x| x.scene);
        }
        out
    }

    /// Resolves and projects the file name of `path` (see
    /// [`read_parsed`](Self::read_parsed)).
    #[must_use]
    pub fn read(&self, path: &str) -> Vec<Projections> {
        self.read_parsed(&crate::parse::parse(path).best().parsed, None)
    }

    /// Canonical episodes of a parsed name and their projections, one per
    /// episode of the file:
    ///
    /// - `.5`, `SP`, `OVA`/`Special` markers: AniDB specials of the entry
    ///   the title names (or `entry`); NCOP/NCED and other extras: none;
    /// - an explicit season (`S03E15`, `3x15`, `Season 3/`): TMDB order,
    ///   then TVDB aired order;
    /// - a bare number (`- 38`): counted from the entry the title names
    ///   (`Shingeki no Kyojin Season 3 Part 2 - 05`), or `entry`, running on
    ///   into the next entries; under the series title, the running count;
    ///   a title qualifier (`Mob Psycho 100 III - 05`) is a TMDB season.
    #[must_use]
    pub fn read_parsed(&self, p: &ParsedName, entry: Option<usize>) -> Vec<Projections> {
        let class = classify(p);
        if class == Class::Extra {
            return Vec::new();
        }
        let named = entry.or_else(|| p.title.as_deref().and_then(|t| self.entry_named(t)));
        let title_season = p.title.as_deref().map(TitleKey::new).and_then(|k| k.season);
        let numbers: Vec<_> = if p.absolute.is_empty() {
            p.episodes.clone()
        } else {
            p.absolute.clone()
        };
        let mut out = Vec::new();
        for n in &numbers {
            let written = match class {
                Class::Special(kind) => {
                    let Some(e) = named.or((self.entries.len() == 1).then_some(0)) else {
                        continue;
                    };
                    // `13.5` is the fifth special after 13 for nobody: the
                    // number of the special is unknown unless written whole.
                    if n.fraction.is_some() {
                        continue;
                    }
                    Written::Special {
                        entry: e,
                        kind,
                        number: n.number,
                    }
                }
                _ if n.fraction.is_some() => continue,
                _ => match (p.season, named) {
                    // The season of an entry title (`… Season 3 Part 2 - 05`)
                    // is the entry's, not a TMDB season of the number.
                    (Some(s), _) if p.absolute.is_empty() && !(named.is_some() && title_season == Some(s)) => {
                        Written::Season(SeasonEpisode::new(s, n.number))
                    }
                    (_, Some(e)) => Written::Entry {
                        entry: e,
                        number: n.number,
                    },
                    _ => match p.title.as_deref().map(TitleKey::new).and_then(|k| k.season) {
                        Some(s) if s > 1 && p.absolute.is_empty() => {
                            Written::Season(SeasonEpisode::new(s, n.number))
                        }
                        _ => Written::Absolute(n.number),
                    },
                },
            };
            if let Some(c) = self.resolve(written) {
                out.push(self.project(c));
            }
        }
        out
    }

    /// Builds the series of entries sharing `tmdb` or `tvdb` from the
    /// Anime-Lists layer of `index`; `None` when no entry carries them.
    #[must_use]
    pub fn from_mappings(index: &MappingIndex, tmdb: Option<u32>, tvdb: Option<u32>) -> Option<Self> {
        let mut found: Vec<&pa_anime_mappings::Record> = Vec::new();
        let records = tmdb
            .into_iter()
            .flat_map(|id| index.get(IdRef::TmdbTv(id)))
            .chain(tvdb.into_iter().flat_map(|id| index.get(IdRef::Tvdb(id))));
        for r in records {
            if r.ids.anidb.is_some()
                && r.anime_list.is_some()
                && !found.iter().any(|f| f.ids.anidb == r.ids.anidb)
            {
                found.push(r);
            }
        }
        if found.is_empty() {
            return None;
        }
        // One id per series: the given one, else the one most entries share.
        let common = |ids: Vec<u32>| {
            let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
            for id in ids {
                *counts.entry(id).or_default() += 1;
            }
            counts
                .into_iter()
                .max_by_key(|&(id, n)| (n, std::cmp::Reverse(id)))
                .map(|(id, _)| id)
        };
        let lists = || found.iter().filter_map(|r| r.anime_list.as_ref());
        let tmdb = tmdb.or_else(|| common(lists().filter_map(|al| al.tmdb_tv).collect()));
        let tvdb = tvdb.or_else(|| common(lists().filter_map(|al| al.tvdb).collect()));
        let mut entries: Vec<Entry> = found
            .iter()
            .filter_map(|r| {
                let (aid, al) = (r.ids.anidb?, r.anime_list.as_ref()?);
                let mut titles: Vec<String> = Vec::new();
                for t in r.title.iter().chain(std::iter::once(&al.name)) {
                    if !t.is_empty() && !titles.contains(t) {
                        titles.push(t.clone());
                    }
                }
                let mut e = entry_of(aid, titles, r.episodes, al);
                // Placements of another TMDB / TVDB series do not apply.
                if al.tmdb_tv != tmdb {
                    e.tmdb = None;
                }
                if al.tvdb != tvdb {
                    e.tvdb = None;
                    e.pairs.clear();
                }
                Some(e)
            })
            .collect();
        entries.sort_by_key(order_key);
        Some(Self {
            tmdb,
            tvdb,
            entries,
            ..Default::default()
        })
    }

    /// Fills unknown episode counts of entries from their TMDB placements
    /// and [`tmdb_seasons`](Self::tmdb_seasons): an entry runs up to the
    /// next entry of its season, or to the season's end. Without
    /// anime-offline-database (the app's list has no counts) this is what
    /// makes the running count work.
    pub fn infer_counts(&mut self) {
        let placed: Vec<(usize, u32, i32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e.tmdb {
                Some(Placement::Season { season, offset }) if season > 0 => {
                    Some((i, season, offset))
                }
                _ => None,
            })
            .collect();
        for &(i, season, offset) in &placed {
            if self.entries[i].episodes.is_some_and(|n| n > 0) {
                continue;
            }
            let next = placed
                .iter()
                .filter(|&&(j, s, o)| j != i && s == season && o > offset)
                .map(|&(_, _, o)| o)
                .min();
            let end = next.or_else(|| {
                self.tmdb_seasons
                    .get(&season)
                    .and_then(|&c| i32::try_from(c).ok())
            });
            if let Some(n) = end.and_then(|end| u32::try_from(end - offset).ok()) {
                self.entries[i].episodes = (n > 0).then_some(n);
            }
        }
    }

    /// Completes the entry with this AniDB id from another source (the
    /// local index, anime-offline-database): an unknown episode count and
    /// more titles.
    pub fn fill<'a>(
        &mut self,
        anidb: u32,
        episodes: Option<u32>,
        titles: impl IntoIterator<Item = &'a String>,
    ) {
        if let Some(i) = self.entry(anidb) {
            let e = &mut self.entries[i];
            if e.episodes.is_none_or(|n| n == 0) {
                e.episodes = episodes.filter(|&n| n > 0);
            }
            for t in titles {
                if !e.titles.contains(t) {
                    e.titles.push(t.clone());
                }
            }
        }
    }
}

/// Sort key of an entry inside its series: TVDB position (absolute offset,
/// or season and offset), then TMDB, then the AniDB id; entries only in
/// season 0 last.
fn order_key(e: &Entry) -> (bool, u32, i64, u32) {
    let key = |p: Placement| match p {
        Placement::Absolute { offset } => (1, i64::from(offset)),
        Placement::Season { season, offset } => (season, i64::from(offset)),
    };
    let (s, o) = e
        .tvdb
        .filter(|p| !matches!(p, Placement::Season { season: 0, .. }))
        .or(e.tmdb)
        .map_or((u32::MAX, 0), key);
    (e.special_only(), s, o, e.anidb)
}

fn placement(season: Option<SeasonRef>, offset: i32) -> Option<Placement> {
    match season? {
        SeasonRef::Season(season) => Some(Placement::Season { season, offset }),
        SeasonRef::Absolute => Some(Placement::Absolute { offset }),
    }
}

/// An [`Entry`] from an Anime-Lists `<anime>`: default placements and the
/// mapping list as explicit TVDB pairs (ranges expanded).
fn entry_of(aid: u32, titles: Vec<String>, episodes: Option<u32>, al: &AnimeListEntry) -> Entry {
    let mut pairs: Vec<Pair> = Vec::new();
    for m in &al.mappings {
        let kind = if m.anidb_season == 0 {
            AnidbType::Special
        } else {
            AnidbType::Regular
        };
        for (a, to) in &m.explicit {
            pairs.push(Pair {
                scheme: Scheme::TvdbAired,
                kind,
                number: *a,
                to: to
                    .iter()
                    .map(|&e| SeasonEpisode::new(m.tvdb_season, e))
                    .collect(),
            });
        }
        if let (Some(start), Some(end)) = (m.start, m.end) {
            for a in start..=end.min(start.saturating_add(2000)) {
                if pairs
                    .iter()
                    .any(|p| p.scheme == Scheme::TvdbAired && p.kind == kind && p.number == a)
                {
                    continue;
                }
                pairs.push(Pair {
                    scheme: Scheme::TvdbAired,
                    kind,
                    number: a,
                    to: shift(a, m.offset)
                        .map(|e| SeasonEpisode::new(m.tvdb_season, e))
                        .into_iter()
                        .collect(),
                });
            }
        }
    }
    Entry {
        anidb: aid,
        titles,
        episodes,
        tmdb: al
            .tmdb_tv
            .and_then(|_| placement(al.tmdb_season, al.tmdb_offset)),
        tvdb: al
            .tvdb
            .and_then(|_| placement(al.default_tvdb_season, al.episode_offset)),
        tvdb_dvd: None,
        pairs,
    }
}

/// Every series of a mapping index, by AniDB id.
#[derive(Debug, Clone, Default)]
pub struct SeriesBook {
    series: Vec<Series>,
    by_anidb: HashMap<u32, usize>,
}

impl SeriesBook {
    /// Groups the Anime-Lists entries of `index` into series: entries
    /// sharing a TVDB series (else a TMDB TV series) are one series.
    #[must_use]
    pub fn from_mappings(index: &MappingIndex) -> Self {
        let mut book = Self::default();
        for r in index.records() {
            let (Some(aid), Some(al)) = (r.ids.anidb, r.anime_list.as_ref()) else {
                continue;
            };
            if book.by_anidb.contains_key(&aid) {
                continue;
            }
            let (tvdb, tmdb) = (al.tvdb, al.tmdb_tv);
            if tvdb.is_none() && tmdb.is_none() {
                continue;
            }
            // TVDB groups first: it is the id Anime-Lists is keyed on.
            let Some(s) = Series::from_mappings(index, tmdb.filter(|_| tvdb.is_none()), tvdb)
                .or_else(|| Series::from_mappings(index, tmdb, None))
            else {
                continue;
            };
            let pos = book.series.len();
            for e in &s.entries {
                book.by_anidb.entry(e.anidb).or_insert(pos);
            }
            book.series.push(s);
        }
        book
    }

    /// Number of series.
    #[must_use]
    pub fn len(&self) -> usize {
        self.series.len()
    }

    /// `true` without series.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// The series an AniDB entry belongs to.
    #[must_use]
    pub fn of_anidb(&self, anidb: u32) -> Option<&Series> {
        self.by_anidb.get(&anidb).map(|&i| &self.series[i])
    }

    /// Completes the entry with this AniDB id ([`Series::fill`]).
    pub fn fill<'a>(
        &mut self,
        anidb: u32,
        episodes: Option<u32>,
        titles: impl IntoIterator<Item = &'a String>,
    ) {
        if let Some(&i) = self.by_anidb.get(&anidb) {
            self.series[i].fill(anidb, episodes, titles);
        }
    }

    /// Completes every entry from the anime records of a title index
    /// (counts and titles of anime-offline-database).
    pub fn fill_from_index(&mut self, index: &crate::title::TitleIndex) {
        for r in index.records() {
            if let Some(a) = r.ids.get("anidb").and_then(|v| v.parse().ok()) {
                self.fill(a, r.episodes, &r.titles);
            }
        }
    }

    /// The series with this TMDB TV id.
    #[must_use]
    pub fn of_tmdb(&self, tmdb: u32) -> Option<&Series> {
        self.series.iter().find(|s| s.tmdb == Some(tmdb))
    }
}

/// What kind of episode a parsed name is, by AniDB types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A regular episode (a recap included: AniDB numbers most recaps as
    /// regular episodes).
    Regular,
    /// An AniDB special or another non-regular type.
    Special(AnidbType),
    /// Not an episode: NCOP/NCED, trailers and other extras.
    Extra,
}

/// AniDB type of a parsed name: `.5` episodes and `SP`, `OVA`, `ONA`,
/// `Special` markers are specials, NCOP/NCED are credits and therefore
/// extras (never a regular episode), a recap stays regular.
#[must_use]
pub fn classify(p: &ParsedName) -> Class {
    if p.extra.is_some() {
        return Class::Extra;
    }
    match p.episode_kind {
        Some(EpisodeKind::Ncop | EpisodeKind::Nced) => Class::Extra,
        Some(EpisodeKind::Special | EpisodeKind::Sp) => Class::Special(AnidbType::Special),
        // `Title OVA - 02` is usually an OVA entry of its own (the title
        // names it); a bare `OVA` marker after a TV title is a special.
        Some(EpisodeKind::Ova | EpisodeKind::Ona)
            if !p
                .title
                .as_deref()
                .is_some_and(|t| t.to_lowercase().contains("ova") || t.to_lowercase().contains("ona")) =>
        {
            Class::Special(AnidbType::Special)
        }
        _ if p
            .episodes
            .iter()
            .chain(&p.absolute)
            .any(|n| n.fraction.is_some()) =>
        {
            Class::Special(AnidbType::Special)
        }
        _ => Class::Regular,
    }
}

/// AniDB type of the corpus / `episodes.episode_type` name.
#[must_use]
pub fn anidb_type(name: &str) -> Option<AnidbType> {
    Some(match name {
        "regular" | "recap" => AnidbType::Regular,
        "special" => AnidbType::Special,
        "credit" => AnidbType::Credit,
        "trailer" => AnidbType::Trailer,
        "parody" => AnidbType::Parody,
        "other" => AnidbType::Other,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn entry(anidb: u32, title: &str, episodes: u32, tmdb: (u32, i32)) -> Entry {
        Entry {
            anidb,
            titles: vec![title.to_owned()],
            episodes: Some(episodes),
            tmdb: Some(Placement::Season {
                season: tmdb.0,
                offset: tmdb.1,
            }),
            ..Default::default()
        }
    }

    /// Attack on Titan as AniDB splits it: four entries, the fourth the
    /// second cour of TMDB season 3.
    fn aot() -> Series {
        Series {
            tmdb: Some(1429),
            entries: vec![
                entry(9541, "Shingeki no Kyojin", 25, (1, 0)),
                entry(10944, "Shingeki no Kyojin Season 2", 12, (2, 0)),
                entry(13241, "Shingeki no Kyojin Season 3", 12, (3, 0)),
                entry(14444, "Shingeki no Kyojin Season 3 Part 2", 10, (3, 12)),
            ],
            ..Default::default()
        }
    }

    fn anidb(c: &Projections) -> (u32, u32) {
        match c.canonical {
            Canonical::Anidb { anidb, number, .. } => (anidb, number),
            Canonical::Tmdb(_) => panic!("not anime"),
        }
    }

    #[test]
    fn absolute_numbers_split_into_entries_and_project_to_tmdb() {
        let s = aot();
        let read = |n| s.project(s.resolve(Written::Absolute(n)).unwrap());
        // `Attack on Titan - 38…47`: season 3, which AniDB starts at 1.
        let p = read(38);
        assert_eq!(anidb(&p), (13241, 1));
        assert_eq!(p.tmdb, Some(SeasonEpisode::new(3, 1)));
        assert_eq!(p.tvdb_absolute, Some(38));
        let p = read(47);
        assert_eq!(anidb(&p), (13241, 10));
        assert_eq!(p.tmdb, Some(SeasonEpisode::new(3, 10)));
        let p = read(50);
        assert_eq!(anidb(&p), (14444, 1));
        assert_eq!(p.tmdb, Some(SeasonEpisode::new(3, 13)));
        assert_eq!(s.resolve(Written::Absolute(60)), None);
    }

    #[test]
    fn season_numbers_find_the_cour() {
        let s = aot();
        let c = s
            .resolve(Written::Season(SeasonEpisode::new(3, 15)))
            .unwrap();
        assert_eq!(
            c,
            Canonical::Anidb {
                anidb: 14444,
                kind: AnidbType::Regular,
                number: 3
            }
        );
        assert_eq!(s.project(c).tvdb_absolute, Some(52));
        assert!(
            s.resolve(Written::Season(SeasonEpisode::new(3, 23)))
                .is_none()
        );
    }

    #[test]
    fn names_resolve_through_the_entry_they_name() {
        let s = aot();
        let read = |path: &str| s.read(path);
        let p = read("Anime/AoT/[Grp] Shingeki no Kyojin Season 3 Part 2 - 05 (1080p).mkv");
        assert_eq!(anidb(&p[0]), (14444, 5));
        assert_eq!(p[0].tmdb, Some(SeasonEpisode::new(3, 17)));
        // A continued count of the previous cour runs on into the next one.
        let p = read("Anime/AoT/[Grp] Shingeki no Kyojin Season 3 - 15 (1080p).mkv");
        assert_eq!(anidb(&p[0]), (14444, 3));
        // The series title with a bare number is the running count.
        let p = read("Anime/Attack on Titan/[Grp] Shingeki no Kyojin - 38 (1080p).mkv");
        assert_eq!(anidb(&p[0]), (13241, 1));
        // Scene SxxEyy is TMDB order.
        let p = read("TV/Attack on Titan/Season 03/Attack.on.Titan.S03E15.1080p.WEB.mkv");
        assert_eq!(anidb(&p[0]), (14444, 3));
        assert_eq!(p[0].tmdb, Some(SeasonEpisode::new(3, 15)));
    }

    #[test]
    fn types_follow_anidb() {
        let s = aot();
        let c = |path: &str| classify(&parse(path).best().parsed);
        assert_eq!(c("Anime/X/[G] Shingeki no Kyojin - NCOP1 [BD].mkv"), Class::Extra);
        assert_eq!(c("Anime/X/[G] Shingeki no Kyojin - NCED (BD 1080p).mkv"), Class::Extra);
        assert_eq!(
            c("Anime/X/[G] Shingeki no Kyojin - 13.5 [1080p].mkv"),
            Class::Special(AnidbType::Special)
        );
        assert_eq!(c("Anime/X/[G] Shingeki no Kyojin - 05 [1080p].mkv"), Class::Regular);
        // A `.5` is a special with no known number: no guess in season 0.
        assert!(s.read("Anime/X/[G] Shingeki no Kyojin - 13.5 [1080p].mkv").is_empty());
        assert!(s.read("Anime/X/[G] Shingeki no Kyojin - NCOP1 [BD].mkv").is_empty());
        // A special with a number goes to the AniDB specials, and to TMDB
        // only through an explicit pair.
        let mut s2 = aot();
        s2.entries[0].pairs.push(Pair {
            scheme: Scheme::Tmdb,
            kind: AnidbType::Special,
            number: 1,
            to: vec![SeasonEpisode::new(0, 4)],
        });
        let c = s2
            .resolve(Written::Special {
                entry: 0,
                kind: AnidbType::Special,
                number: 1,
            })
            .unwrap();
        assert_eq!(s2.project(c).tmdb, Some(SeasonEpisode::new(0, 4)));
        let c = s2
            .resolve(Written::Special {
                entry: 0,
                kind: AnidbType::Special,
                number: 2,
            })
            .unwrap();
        assert_eq!(s2.project(c).tmdb, None);
        assert_eq!(s2.project(c).tvdb_absolute, None);
    }

    #[test]
    fn display_order_per_series() {
        let mut s = aot();
        s.groups.push(EpisodeGroup {
            id: "g1".into(),
            name: "Absolute".into(),
            seasons: vec![
                Vec::new(),
                (1..=25)
                    .map(|e| SeasonEpisode::new(1, e))
                    .chain((1..=12).map(|e| SeasonEpisode::new(2, e)))
                    .chain((1..=22).map(|e| SeasonEpisode::new(3, e)))
                    .collect(),
            ],
        });
        let p = s.project(s.resolve(Written::Absolute(50)).unwrap());
        assert_eq!(p.display(&DisplayOrder::Tmdb), Some(SeasonEpisode::new(3, 13)));
        assert_eq!(p.display(&DisplayOrder::Anidb), Some(SeasonEpisode::new(4, 1)));
        assert_eq!(
            p.display(&DisplayOrder::Absolute),
            Some(SeasonEpisode::new(1, 50))
        );
        assert_eq!(
            p.display(&DisplayOrder::TmdbGroup("g1".into())),
            Some(SeasonEpisode::new(1, 50))
        );
        assert_eq!(p.display(&DisplayOrder::TvdbDvd), None);
    }

    #[test]
    fn western_series_are_canonical_in_tmdb() {
        let s = Series {
            tmdb: Some(1399),
            tmdb_seasons: [(1, 10), (2, 10)].into(),
            xem: vec![XemLink {
                tvdb: SeasonEpisode::new(2, 1),
                scene: SeasonEpisode::new(2, 1),
            }],
            ..Default::default()
        };
        let p = s.project(
            s.resolve(Written::Season(SeasonEpisode::new(2, 3)))
                .unwrap(),
        );
        assert_eq!(p.canonical, Canonical::Tmdb(SeasonEpisode::new(2, 3)));
        assert_eq!(p.tvdb_absolute, Some(13));
        assert_eq!(
            s.resolve(Written::Absolute(13)),
            Some(Canonical::Tmdb(SeasonEpisode::new(2, 3)))
        );
        assert_eq!(s.resolve(Written::Season(SeasonEpisode::new(2, 11))), None);
    }

    #[test]
    fn series_from_anime_lists() {
        let xml = r#"<anime-list>
  <anime anidbid="9001" tvdbid="100001" defaulttvdbseason="1" episodeoffset="" tmdbtv="200001" tmdbseason="1" tmdboffset="">
    <name>Example Saga</name>
    <mapping-list><mapping anidbseason="0" tvdbseason="0">;1-3;</mapping></mapping-list>
  </anime>
  <anime anidbid="9002" tvdbid="100001" defaulttvdbseason="1" episodeoffset="12" tmdbtv="200001" tmdbseason="2" tmdboffset="">
    <name>Example Saga Part 2</name>
  </anime>
  <anime anidbid="9005" tvdbid="100001" defaulttvdbseason="0" episodeoffset="5" tmdbtv="" tmdbseason="" tmdboffset="">
    <name>Example Saga OVA</name>
  </anime>
  <anime anidbid="9006" tvdbid="100006" defaulttvdbseason="a" episodeoffset="" tmdbtv="" tmdbseason="" tmdboffset="">
    <name>Long Runner</name>
  </anime>
  <anime anidbid="9007" tvdbid="100006" defaulttvdbseason="a" episodeoffset="500" tmdbtv="" tmdbseason="" tmdboffset="">
    <name>Long Runner Final Arc</name>
  </anime>
</anime-list>"#;
        let mut index = MappingIndex::new();
        index.add_anime_lists(pa_anime_mappings::anime_lists::parse(xml).unwrap());
        let book = SeriesBook::from_mappings(&index);
        assert_eq!(book.len(), 2);
        let mut s = book.of_anidb(9002).unwrap().clone();
        assert_eq!(
            s.entries.iter().map(|e| e.anidb).collect::<Vec<_>>(),
            [9001, 9002, 9005]
        );
        assert_eq!((s.tmdb, s.tvdb), (Some(200001), Some(100001)));
        // Episode counts come from anime-offline-database.
        s.entries[0].episodes = Some(12);
        s.entries[1].episodes = Some(12);
        // Part 2 is TVDB season 1 from 13, TMDB season 2 from 1.
        let p = s.project(s.resolve(Written::Season(SeasonEpisode::new(1, 14))).unwrap());
        assert_eq!(anidb(&p), (9002, 2));
        assert_eq!(p.tmdb, Some(SeasonEpisode::new(2, 2)));
        assert_eq!(p.tvdb_aired, Some(SeasonEpisode::new(1, 14)));
        // The special goes where the mapping list says.
        let c = s
            .resolve(Written::Special {
                entry: 0,
                kind: AnidbType::Special,
                number: 1,
            })
            .unwrap();
        assert_eq!(s.project(c).tvdb_aired, Some(SeasonEpisode::new(0, 3)));
        // The OVA sits in season 0: outside the running count.
        assert_eq!(s.offset(2), None);
        let long = book.of_anidb(9006).unwrap();
        let p = long.project(long.resolve(Written::Absolute(512)).unwrap());
        assert_eq!(anidb(&p), (9007, 12));
        assert_eq!(p.tvdb_absolute, Some(512));
        assert_eq!(long.entry_named("long runner final arc"), Some(1));
    }
}
