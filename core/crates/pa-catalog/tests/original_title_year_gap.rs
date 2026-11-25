// SPDX-License-Identifier: MPL-2.0
//! BUG-023: a film named exactly like its original title, dated a year or
//! two off its premiere, is decided automatically.
//!
//! `Movies/Plan 9 from Outer Space (1959).mkv` went to the review inbox in
//! a Russian UI: TMDB answers with the Russian title and the original one,
//! the file's name is the original title, but TMDB dates the film by its
//! 1957 premiere (wide release 1959). The responses below are synthetic,
//! shaped like the real ones of a metadata run in `ru-RU` or `uk-UA`.

use std::collections::HashMap;

use pa_catalog::metadata::{AUTO, SearchMatch, WorkQuery, match_search_titled};
use pa_metadata::tmdb::SearchResult;
use pa_recognition::parse::parse;

/// A TMDB movie in the metadata language: id, localized title, original
/// title, original language, year, titles from translations and
/// alternative titles.
struct Film {
    id: u64,
    title: &'static str,
    original: &'static str,
    lang: &'static str,
    year: u32,
    other: &'static [&'static str],
}

impl Film {
    fn result(&self) -> SearchResult {
        serde_json::from_value(serde_json::json!({
            "id": self.id,
            "media_type": "movie",
            "title": self.title,
            "original_title": self.original,
            "original_language": self.lang,
            "release_date": format!("{}-07-01", self.year),
        }))
        .unwrap()
    }
}

const PLAN_9_RU: Film = Film {
    id: 10513,
    title: "План 9 из открытого космоса",
    original: "Plan 9 from Outer Space",
    lang: "en",
    year: 1957,
    other: &["Plan 9 from Outer Space", "Grave Robbers from Outer Space"],
};

const PLAN_9_UK: Film = Film {
    title: "План 9 з відкритого космосу",
    ..PLAN_9_RU
};

/// An unrelated film the search also returns.
const DECOY: Film = Film {
    id: 1,
    title: "Космос",
    original: "Outer Space",
    lang: "en",
    year: 1999,
    other: &["Outer Space"],
};

/// The work a library would query for the movie file `path`.
fn query(path: &str) -> WorkQuery {
    let p = parse(path);
    let b = &p.best().parsed;
    WorkQuery {
        kind: "movie".into(),
        year: b.year.map(i64::from),
        titles: b.title.iter().chain(&b.alt_titles).cloned().collect(),
        ..Default::default()
    }
}

/// The match of `work` on a search page of `films`, translations and
/// alternative titles answered from the films.
fn search(work: &WorkQuery, films: &[&Film]) -> Option<SearchMatch> {
    let page: Vec<SearchResult> = films.iter().map(|f| f.result()).collect();
    let other: HashMap<u64, Vec<String>> = films
        .iter()
        .map(|f| (f.id, f.other.iter().map(|s| (*s).to_owned()).collect()))
        .collect();
    match_search_titled::<()>(
        work,
        &mut |_, _, _| Ok(page.clone()),
        &mut |_, _| Ok(None),
        &mut |_, id| Ok(other.get(&id).cloned().unwrap_or_default()),
    )
    .unwrap()
}

#[test]
fn exact_original_title_two_years_off_is_auto() {
    let work = query("Movies/Plan 9 from Outer Space (1959).mkv");
    assert_eq!(work.year, Some(1959));
    for film in [&PLAN_9_RU, &PLAN_9_UK] {
        let m = search(&work, &[film, &DECOY]).expect("a match");
        assert_eq!(m.result.id, film.id, "{m:?}");
        assert!(m.score >= AUTO, "{}: {m:?}", film.title);
        // Compared with the original title, not with the translation.
        assert_eq!(
            m.signals.matched_title.as_deref(),
            Some("Plan 9 from Outer Space")
        );
        assert_eq!(m.tier, "exact");
        // The display title stays in the metadata language.
        assert_eq!(m.result.title.as_deref(), Some(film.title));
    }
    // A year apart was already enough.
    let m = search(&query("Movies/Plan 9 from Outer Space (1958).mkv"), &[&PLAN_9_RU]).unwrap();
    assert!(m.score >= AUTO, "{m:?}");
}

#[test]
fn romaji_title_two_years_off_is_auto() {
    // The romaji title is only among the alternative titles (META-033);
    // the original title is Japanese.
    let film = Film {
        id: 23_000,
        title: "Яйцо ангела",
        original: "天使のたまご",
        lang: "ja",
        year: 1985,
        other: &["Angel's Egg", "Tenshi no Tamago"],
    };
    for path in [
        "Movies/Tenshi no Tamago (1987).mkv",
        "Movies/Angel's Egg (1987).mkv",
    ] {
        let m = search(&query(path), &[&film, &DECOY]).expect("a match");
        assert_eq!(m.result.id, film.id, "{path}: {m:?}");
        assert!(m.score >= AUTO, "{path}: {m:?}");
    }
}

#[test]
fn a_year_gap_still_asks_when_the_title_is_not_enough() {
    // A remake with the same title on the page: the year tells which one,
    // and a gap of two years tells nothing.
    let dune = |id, year| Film {
        id,
        title: "Дюна",
        original: "Dune",
        lang: "en",
        year,
        other: &["Dune"],
    };
    let (old, new) = (dune(841, 1984), dune(438_631, 2021));
    let m = search(&query("Movies/Dune (1986).mkv"), &[&old, &new]);
    assert!(m.as_ref().is_none_or(|m| m.score < AUTO), "{m:?}");
    // Three years off: another film for all the title says.
    let m = search(&query("Movies/Plan 9 from Outer Space (1960).mkv"), &[&PLAN_9_RU]);
    assert!(m.as_ref().is_none_or(|m| m.score < AUTO), "{m:?}");
    // Only part of the title: not the same film.
    let m = search(&query("Movies/Plan 9 (1959).mkv"), &[&PLAN_9_RU]);
    assert!(m.as_ref().is_none_or(|m| m.score < AUTO), "{m:?}");
}
