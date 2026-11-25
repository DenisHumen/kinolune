// SPDX-License-Identifier: MPL-2.0
//! META-036: the review reason carries the similarity of the titles alone
//! and the adjustments (year, remake, type) apart from the score.
//!
//! BUG-023: `Movies/Plan 9 from Outer Space (1959).mkv` showed «название
//! совпало на 75 %» for an exact original title — 75 % was the score after
//! the year and remake penalties, not the comparison of the titles. The
//! TMDB response below is synthetic, shaped like a `ru-RU` search page.

use pa_catalog::metadata::{MatchSignals, WorkQuery, search_ranked};
use pa_metadata::tmdb::SearchResult;
use pa_recognition::parse::parse;

fn plan9() -> SearchResult {
    serde_json::from_value(serde_json::json!({
        "id": 10513,
        "media_type": "movie",
        "title": "План 9 из открытого космоса",
        "original_title": "Plan 9 from Outer Space",
        "original_language": "en",
        "release_date": "1957-07-01",
    }))
    .unwrap()
}

/// The signals of the Plan 9 candidate for the movie file `path`.
fn signals(path: &str) -> MatchSignals {
    let p = parse(path);
    let b = &p.best().parsed;
    let work = WorkQuery {
        kind: "movie".into(),
        year: b.year.map(i64::from),
        titles: b.title.iter().chain(&b.alt_titles).cloned().collect(),
        ..Default::default()
    };
    let d = search_ranked::<()>(
        &work,
        &mut |_, _, _| Ok(vec![plan9()]),
        &mut |_, _| Ok(None),
        &mut |_, _| Ok(vec!["Plan 9 from Outer Space".to_owned()]),
    )
    .unwrap();
    let c = d
        .candidates
        .into_iter()
        .find(|c| c.result.id == 10513)
        .expect("Plan 9 is a candidate");
    c.signals
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.0015
}

/// The parts add up to the score shown before (the title index clamps at
/// 1, so only below it).
fn adds_up(s: &MatchSignals) {
    let sum = s.title_similarity.unwrap()
        + s.year_adjust.unwrap()
        + s.kind_adjust.unwrap()
        + s.remake_adjust.unwrap_or(0.0);
    assert!(close(sum, s.title), "{s:?}");
}

#[test]
fn bug_023_exact_title_two_years_off() {
    let s = signals("Movies/Plan 9 from Outer Space (1959).mkv");
    assert_eq!(s.matched_title.as_deref(), Some("Plan 9 from Outer Space"));
    // The titles are the same: 100 %, whatever the score.
    assert_eq!(s.title_similarity, Some(1.0), "{s:?}");
    // Two years apart: −8 % in the title index.
    assert_eq!((s.file_year, s.year), (Some(1959), Some(1957)));
    assert_eq!(s.year_adjust, Some(-0.08), "{s:?}");
    // A movie file against a movie: +2 %.
    assert_eq!(s.kind_adjust, Some(0.02), "{s:?}");
    assert!(s.title < 1.0, "{s:?}");
    adds_up(&s);
}

#[test]
fn remake_penalty_is_its_own_part() {
    // Three years off: another film for all the title says (×0.8).
    let s = signals("Movies/Plan 9 from Outer Space (1960).mkv");
    assert_eq!(s.title_similarity, Some(1.0), "{s:?}");
    assert_eq!(s.year_adjust, Some(-0.08), "{s:?}");
    let remake = s.remake_adjust.expect("remake penalty");
    assert!(close(remake, -0.188), "{s:?}");
    adds_up(&s);
}

#[test]
fn same_year_has_no_penalty() {
    let s = signals("Movies/Plan 9 from Outer Space (1957).mkv");
    assert_eq!(s.title_similarity, Some(1.0));
    assert_eq!(s.year_adjust, Some(0.04));
    assert_eq!(s.remake_adjust, None);
}

#[test]
fn old_rows_without_the_split_still_read() {
    // `explanation_json` stored before META-036.
    let s: MatchSignals = serde_json::from_value(serde_json::json!({
        "provider": "tmdb",
        "title": 0.752,
        "tier": "exact",
        "file_year": 1959,
        "year": 1957,
    }))
    .unwrap();
    assert!(close(s.title, 0.752));
    assert_eq!(s.title_similarity, None);
    assert_eq!(s.year_adjust, None);
    assert_eq!(s.remake_adjust, None);
    assert_eq!(s.kind_adjust, None);
    // And new rows do not write empty parts.
    let v = serde_json::to_value(&s).unwrap();
    assert!(v.get("title_similarity").is_none());
}
