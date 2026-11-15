// SPDX-License-Identifier: MPL-2.0
//! META-032: a real library, anonymised (`fixtures/real-library.tsv`: invented
//! titles, the real folders, separators, brackets, numbering, Cyrillic and
//! site junk), through the folder analysis, the planner and the work
//! grouping — every video must land in the right kind, work, season and
//! episode. The summary line counts what used to go wrong: folders that are
//! one series, episodes filed as movies, junk left in titles.

use std::collections::{BTreeMap, BTreeSet};

use pa_catalog::recognize::{group_works, plan_files};
use pa_recognition::title::TitleKey;
use pa_scanner::structure::{ListedFile, StructureOptions, analyze};

const FIXTURE: &str = include_str!("fixtures/real-library.tsv");

struct Expected {
    path: String,
    series: bool,
    title: String,
    /// Year of a movie, season of an episode.
    number: Option<i64>,
    episode: Option<i64>,
    episode_title: Option<String>,
}

fn expected() -> Vec<Expected> {
    FIXTURE
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty() && !l.ends_with('/'))
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            let num = |i: usize| c.get(i).and_then(|v| v.parse().ok());
            Expected {
                path: format!("/{}", c[0]),
                series: c[1] == "series",
                title: c[2].to_owned(),
                number: num(3),
                episode: num(4),
                episode_title: c.get(5).map(|s| (*s).to_owned()),
            }
        })
        .collect()
}

fn same_title(a: &str, b: &str) -> bool {
    TitleKey::new(a).loose == TitleKey::new(b).loose
}

/// Release and site junk that must not survive in a work title.
fn junk(title: &str) -> bool {
    let lower = title.to_lowercase();
    [
        "смотреть",
        "бесплатно",
        "онлайн",
        ".org",
        ".info",
        "remux",
        "bdrip",
        "dvdrip",
        "[",
        "]",
        "_",
    ]
    .iter()
    .any(|j| lower.contains(j))
        || lower.ends_with(" tv")
        || lower.ends_with(" ova")
}

fn top_folder(path: &str) -> Option<&str> {
    let rest = path.strip_prefix('/')?;
    let (top, _) = rest.split_once('/')?;
    Some(top)
}

#[test]
fn an_anonymised_real_library_lands_in_the_right_works() {
    let want = expected();
    assert!(want.len() > 1000, "fixture rows: {}", want.len());
    let files: Vec<ListedFile> = want
        .iter()
        .map(|e| ListedFile::new(e.path.clone(), 700_000_000))
        .collect();
    let groups = analyze(&files, &StructureOptions::default());
    let paths: Vec<&str> = want.iter().map(|e| e.path.as_str()).collect();
    let plans = plan_files(&paths, &groups);
    let (works, of) = group_works(&plans);

    let mut errors: Vec<String> = Vec::new();
    let mut misfiled_movies = 0;
    let mut wrong_numbers = 0;
    let mut wrong_titles = 0;
    // Series folder → the works its files landed in.
    let mut folder_works: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for (i, e) in want.iter().enumerate() {
        let (Some(plan), Some(w)) = (&plans[i], of[i]) else {
            errors.push(format!("{}: no plan", e.path));
            continue;
        };
        let work = &works[w];
        if e.series {
            if let Some(top) = top_folder(&e.path) {
                folder_works.entry(top).or_default().insert(w);
            }
            if !work.is_series() {
                misfiled_movies += 1;
                errors.push(format!("{}: a movie `{}`", e.path, work.title));
                continue;
            }
            let numbers = plan
                .episodes
                .iter()
                .map(|n| (n.season, n.episode))
                .collect::<Vec<_>>();
            if numbers != [(e.number, e.episode)] {
                wrong_numbers += 1;
                errors.push(format!("{}: episodes {numbers:?}", e.path));
            }
            if let Some(t) = &e.episode_title
                && plan.episode_title.as_deref() != Some(t.as_str())
            {
                errors.push(format!(
                    "{}: episode title {:?}",
                    e.path, plan.episode_title
                ));
            }
        } else {
            if work.is_series() {
                errors.push(format!("{}: a series `{}`", e.path, work.title));
                continue;
            }
            if work.year != e.number {
                errors.push(format!("{}: year {:?}", e.path, work.year));
            }
        }
        if !same_title(&work.title, &e.title) {
            wrong_titles += 1;
            errors.push(format!(
                "{}: title `{}`, want `{}`",
                e.path, work.title, e.title
            ));
        }
    }
    let junk_titles: Vec<&str> = works
        .iter()
        .map(|w| w.title.as_str())
        .filter(|t| junk(t))
        .collect();
    for t in &junk_titles {
        errors.push(format!("junk in title `{t}`"));
    }
    let series_folders = folder_works.len();
    let one_series = folder_works.values().filter(|w| w.len() == 1).count();
    // Two folders of one show (`Show/`, `Show season 2/`) share a work.
    let distinct_series: BTreeSet<String> = want
        .iter()
        .filter(|e| e.series)
        .map(|e| TitleKey::new(&e.title).loose)
        .collect();
    let series_works = works.iter().filter(|w| w.is_series()).count();
    let movie_works = works.iter().filter(|w| !w.is_series()).count();
    let movies = want.iter().filter(|e| !e.series).count();
    println!(
        "META-032 real library: {} videos; series folders as one series {one_series}/{series_folders}; \
         series works {series_works} (want {}); movie works {movie_works} (want {movies}); \
         episodes filed as movies {misfiled_movies}; wrong season/episode {wrong_numbers}; \
         wrong titles {wrong_titles}; titles with junk {}",
        want.len(),
        distinct_series.len(),
        junk_titles.len(),
    );
    if series_works != distinct_series.len() {
        errors.push(format!(
            "{series_works} series works, want {}",
            distinct_series.len()
        ));
    }
    if movie_works != movies {
        errors.push(format!("{movie_works} movie works, want {movies}"));
    }
    assert!(
        errors.is_empty(),
        "{} problems:\n{}",
        errors.len(),
        errors
            .iter()
            .take(60)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
