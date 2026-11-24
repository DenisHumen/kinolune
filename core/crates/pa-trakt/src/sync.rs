// SPDX-License-Identifier: MPL-2.0
//! Two-way sync of watched marks and ratings between Trakt and the
//! library. Titles pair by TMDB id ([`Target::key`]).
//!
//! - **Watched:** a play on Trakt the library lacks is marked watched here
//!   with Trakt's time, if the title is in the library and the library has
//!   nothing newer about it ([`Library::mark_watched`] decides); a title
//!   watched here that Trakt lacks is added to Trakt's history with our
//!   time. Both sides only grow, so a second run changes nothing
//!   (idempotent). "Unwatched" travels only as an explicit action (the
//!   item menu → [`crate::Session::remove_history`]): a missing entry
//!   cannot tell "removed" from "never synced".
//! - **Ratings** (movies and shows): a rating on one side only is copied
//!   to the other (Trakt 1–10 ↔ our 1–5 stars); when both differ, the
//!   newer one wins. 7/10 and 4 stars count as the same.

use std::collections::HashMap;

use crate::model::{Rated, Target, Watched, stars_from_trakt, stars_to_trakt};
use crate::{Result, Session};

/// A rating of the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRating {
    /// A movie or show.
    pub target: Target,
    /// 1–5.
    pub stars: u8,
    /// Unix ms.
    pub rated_at_ms: i64,
}

/// What the sync needs of the library.
pub trait Library {
    /// Titles watched here (movies and episodes with a TMDB id) and when.
    fn watched(&self) -> Result<Vec<Watched>>;
    /// Marks `w` watched at its time if the title is in the library, is
    /// not watched yet and has no newer local state. Whether it changed.
    fn mark_watched(&self, w: &Watched) -> Result<bool>;
    /// Ratings of movies and shows here.
    fn ratings(&self) -> Result<Vec<LocalRating>>;
    /// Rates a movie or show `stars` if it is in the library. Whether it
    /// changed.
    fn set_rating(&self, target: &Target, stars: u8) -> Result<bool>;
}

/// What a watched sync does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchedPlan {
    /// Trakt plays to mark here.
    pub import: Vec<Watched>,
    /// Local marks to add to Trakt.
    pub export: Vec<Watched>,
}

/// The watched sync of `remote` and `local`.
pub fn plan_watched(remote: &[Watched], local: &[Watched]) -> WatchedPlan {
    let remote_keys: HashMap<String, &Watched> = remote
        .iter()
        .filter_map(|w| Some((w.target.key()?, w)))
        .collect();
    let local_keys: HashMap<String, &Watched> = local
        .iter()
        .filter_map(|w| Some((w.target.key()?, w)))
        .collect();
    let mut plan = WatchedPlan::default();
    for w in remote {
        if let Some(k) = w.target.key()
            && !local_keys.contains_key(&k)
        {
            plan.import.push(w.clone());
        }
    }
    for w in local {
        if let Some(k) = w.target.key()
            && !remote_keys.contains_key(&k)
        {
            plan.export.push(w.clone());
        }
    }
    plan
}

/// What a ratings sync does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RatingsPlan {
    /// Ratings to set here: target and stars.
    pub import: Vec<(Target, u8)>,
    /// Ratings to send to Trakt.
    pub export: Vec<Rated>,
}

/// The ratings sync of `remote` and `local`.
pub fn plan_ratings(remote: &[Rated], local: &[LocalRating]) -> RatingsPlan {
    let remote_keys: HashMap<String, &Rated> = remote
        .iter()
        .filter_map(|r| Some((r.target.key()?, r)))
        .collect();
    let mut plan = RatingsPlan::default();
    let mut seen = std::collections::HashSet::new();
    for l in local {
        let Some(k) = l.target.key() else { continue };
        seen.insert(k.clone());
        match remote_keys.get(&k) {
            Some(r) if stars_from_trakt(r.rating) == l.stars => {}
            Some(r) if r.rated_at_ms > l.rated_at_ms => {
                plan.import.push((l.target.clone(), stars_from_trakt(r.rating)));
            }
            _ => plan.export.push(Rated {
                target: l.target.clone(),
                rating: stars_to_trakt(l.stars),
                rated_at_ms: l.rated_at_ms,
            }),
        }
    }
    for r in remote {
        if let Some(k) = r.target.key()
            && !seen.contains(&k)
        {
            plan.import
                .push((r.target.clone(), stars_from_trakt(r.rating)));
        }
    }
    plan
}

/// What a sync changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Titles marked watched here.
    pub watched_imported: u32,
    /// Titles added to Trakt's history.
    pub watched_exported: u32,
    /// Ratings set here.
    pub ratings_imported: u32,
    /// Ratings sent to Trakt.
    pub ratings_exported: u32,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Syncs watched marks and ratings both ways.
pub fn run(session: &Session, library: &dyn Library) -> Result<Report> {
    let mut report = Report::default();
    let plan = plan_watched(&session.watched()?, &library.watched()?);
    for w in &plan.import {
        if library.mark_watched(w)? {
            report.watched_imported += 1;
        }
    }
    session.add_history(&plan.export)?;
    report.watched_exported = count(plan.export.len());

    let plan = plan_ratings(&session.ratings()?, &library.ratings()?);
    for (target, stars) in &plan.import {
        if library.set_rating(target, *stars)? {
            report.ratings_imported += 1;
        }
    }
    session.add_ratings(&plan.export)?;
    report.ratings_exported = count(plan.export.len());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ids;

    fn movie(id: u64, at: i64) -> Watched {
        Watched {
            target: Target::Movie(Ids::tmdb(id)),
            watched_at_ms: at,
        }
    }

    #[test]
    fn watched_goes_both_ways_once() {
        let remote = vec![movie(1, 10), movie(2, 20)];
        let local = vec![movie(2, 25), movie(3, 30)];
        let plan = plan_watched(&remote, &local);
        assert_eq!(plan.import, vec![movie(1, 10)]);
        assert_eq!(plan.export, vec![movie(3, 30)]);
        // After applying, both sides hold all three: nothing more to do.
        let all = vec![movie(1, 10), movie(2, 20), movie(3, 30)];
        assert_eq!(plan_watched(&all, &all), WatchedPlan::default());
    }

    #[test]
    fn titles_without_tmdb_are_skipped() {
        let remote = vec![Watched {
            target: Target::Movie(Ids {
                imdb: Some("tt1".into()),
                ..Ids::default()
            }),
            watched_at_ms: 1,
        }];
        assert_eq!(plan_watched(&remote, &[]), WatchedPlan::default());
    }

    #[test]
    fn newer_rating_wins_and_equal_ones_stay() {
        let t = |id| Target::Movie(Ids::tmdb(id));
        let remote = vec![
            Rated { target: t(1), rating: 7, rated_at_ms: 100 },
            Rated { target: t(2), rating: 10, rated_at_ms: 100 },
            Rated { target: t(3), rating: 2, rated_at_ms: 50 },
            Rated { target: t(4), rating: 6, rated_at_ms: 10 },
        ];
        let local = vec![
            LocalRating { target: t(1), stars: 4, rated_at_ms: 5 },
            LocalRating { target: t(2), stars: 3, rated_at_ms: 50 },
            LocalRating { target: t(3), stars: 5, rated_at_ms: 60 },
            LocalRating { target: t(5), stars: 1, rated_at_ms: 70 },
        ];
        let plan = plan_ratings(&remote, &local);
        assert_eq!(plan.import, vec![(t(2), 5), (t(4), 3)]);
        assert_eq!(
            plan.export,
            vec![
                Rated { target: t(3), rating: 10, rated_at_ms: 60 },
                Rated { target: t(5), rating: 2, rated_at_ms: 70 },
            ]
        );
    }
}
