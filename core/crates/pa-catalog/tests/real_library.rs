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

fn media(root: &std::path::Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..120_000u32)
        .map(|i| (i as u8).wrapping_mul(17).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

/// A library recognised by the old parser (numbered episodes filed as
/// movies, site junk in a title) is recognised again after an update:
/// automatic decisions of an older parser version are planned anew, user
/// decisions stay.
#[test]
fn an_older_parser_decision_is_planned_again_on_the_next_scan() {
    use pa_catalog::Catalog;
    use pa_library::rusqlite::{Connection, params};
    use pa_scanner::CancelToken;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(
        &root,
        "Great Teacher Kawada/Season 1/01 The Lesson Begins.m4v",
        1,
    );
    media(
        &root,
        "Great Teacher Kawada/Season 1/02 Enter Hayashida.m4v",
        2,
    );
    media(
        &root,
        "Great Teacher Kawada/Season 1/03 Hide and Seek.m4v",
        3,
    );
    media(
        &root,
        "Сигнал Полиорбита — смотреть аниме онлайн бесплатно на ПримерАниме.mp4",
        4,
    );
    media(&root, "Penance.2007.1080p.mkv", 5);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();

    let episodes = |cat: &Catalog| {
        let shows = cat.shows().unwrap();
        assert_eq!(shows.len(), 1, "{shows:?}");
        assert_eq!(shows[0].title, "Great Teacher Kawada");
        cat.episodes(shows[0].id)
            .unwrap()
            .into_iter()
            .map(|e| (e.season, e.episode, e.title))
            .collect::<Vec<_>>()
    };
    let want = [
        (Some(1), Some(1), "The Lesson Begins".to_owned()),
        (Some(1), Some(2), "Enter Hayashida".to_owned()),
        (Some(1), Some(3), "Hide and Seek".to_owned()),
    ];
    let movie_titles = |cat: &Catalog| {
        let mut m: Vec<String> = cat.movies().unwrap().into_iter().map(|m| m.title).collect();
        m.sort();
        m
    };
    assert_eq!(episodes(&cat), want);
    assert_eq!(movie_titles(&cat), ["Penance", "Сигнал Полиорбита"]);

    // What the old parser left: three movies, a junk title, all of parser
    // version 0.2.0; and the user's own decision on one movie.
    let conn = Connection::open(dir.path().join("library.db")).unwrap();
    let files: Vec<(i64, String)> = conn
        .prepare("SELECT id, path FROM files ORDER BY path")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (id, path) in &files {
        let Some(name) = path
            .trim_start_matches('/')
            .strip_prefix("Great Teacher Kawada/Season 1/")
        else {
            continue;
        };
        conn.execute(
            "INSERT INTO works (kind, title, created_at_ms, updated_at_ms)
             VALUES ('movie', ?1, 0, 0)",
            [name.trim_end_matches(".m4v")],
        )
        .unwrap();
        let work = conn.last_insert_rowid();
        conn.execute(
            "DELETE FROM match_episodes WHERE match_id IN
                 (SELECT id FROM matches WHERE file_id = ?1)",
            [id],
        )
        .unwrap();
        conn.execute(
            "UPDATE matches SET work_id = ?2 WHERE file_id = ?1",
            params![id, work],
        )
        .unwrap();
    }
    for table in ["works SET title", "titles SET text"] {
        let column = table.rsplit(' ').next().unwrap();
        conn.execute(
            &format!(
                "UPDATE {table} = 'Сигнал Полиорбита - смотреть аниме онлайн бесплатно на ПримерАниме'
                 WHERE {column} = 'Сигнал Полиорбита'"
            ),
            [],
        )
        .unwrap();
    }
    conn.execute(
        "DELETE FROM works WHERE kind <> 'movie' AND NOT EXISTS
             (SELECT 1 FROM matches m WHERE m.work_id = works.id)",
        [],
    )
    .unwrap();
    conn.execute("UPDATE parse_results SET parser_version = '0.2.0'", [])
        .unwrap();
    conn.execute(
        "UPDATE matches SET decided_by = 'user', state = 'user_set'
         WHERE file_id = (SELECT id FROM files WHERE path LIKE '%Penance%')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE works SET title = 'Penance (my edit)' WHERE title = 'Penance'",
        [],
    )
    .unwrap();
    assert!(cat.shows().unwrap().is_empty());
    assert_eq!(cat.movies().unwrap().len(), 5, "the old state");

    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    assert_eq!(episodes(&cat), want);
    assert_eq!(
        movie_titles(&cat),
        ["Penance (my edit)", "Сигнал Полиорбита"]
    );
    let old: i64 = conn
        .query_row(
            "SELECT count(*) FROM parse_results WHERE parser_version = '0.2.0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old, 1, "only the user's file keeps its old parse");
}
