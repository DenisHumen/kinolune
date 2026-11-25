// SPDX-License-Identifier: MPL-2.0
//! UI-056: total runtime and time left of a show and of each season — the
//! sum of the episodes in the library, a started episode by what is left of
//! it, a watched one not at all, a file not probed yet by the metadata
//! runtime («≈»), a split episode by all its parts, specials only in their
//! own season. Bulk watched marks (UI-049) change the time left at once,
//! and a show of 1000 episodes is summed in one query within the bound.

use std::time::{Duration, Instant};

use pa_catalog::{Catalog, EpisodeOrder, ItemRef, RuntimeTotal};
use pa_library::rusqlite::{Connection, params};

const MIN: i64 = 60_000;

/// A library database with a writable side connection for fixtures.
struct Fixture {
    _dir: tempfile::TempDir,
    cat: Catalog,
    conn: Connection,
    next_file: i64,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let cat = Catalog::open(&path).unwrap();
        let conn = pa_library::open(&path).unwrap();
        conn.execute(
            "INSERT INTO sources (id, kind, name, uri, created_at_ms)
             VALUES (1, 'local', 'NAS', 'file:///srv/nas', 0)",
            [],
        )
        .unwrap();
        Self {
            _dir: dir,
            cat,
            conn,
            next_file: 1,
        }
    }

    fn show(&self, id: i64, title: &str, tmdb: Option<i64>) {
        self.conn
            .execute(
                "INSERT INTO works (id, kind, title, created_at_ms, updated_at_ms)
                 VALUES (?1, 'show', ?2, 0, 0)",
                params![id, title],
            )
            .unwrap();
        if let Some(tmdb) = tmdb {
            self.conn
                .execute(
                    "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id)
                     VALUES (?1, 'tmdb', 'tv', ?2)",
                    params![id, tmdb.to_string()],
                )
                .unwrap();
        }
    }

    /// An episode with the runtime the metadata gave it (ms).
    fn episode(&self, id: i64, work: i64, season: i64, number: i64, runtime: Option<i64>) {
        self.conn
            .execute(
                "INSERT INTO episodes (id, work_id, season_number, episode_number, runtime_ms,
                                       episode_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, CASE WHEN ?3 = 0 THEN 'special' ELSE 'regular' END)",
                params![id, work, season, number, runtime],
            )
            .unwrap();
    }

    /// A file of `episode` lasting `duration` (None: not probed yet).
    fn file(&mut self, work: i64, episode: i64, duration: Option<i64>) -> i64 {
        let id = self.next_file;
        self.next_file += 1;
        self.conn
            .execute(
                "INSERT INTO files (id, source_id, path, size_bytes, added_at_ms, duration_ms,
                                    fingerprint, probed_at_ms)
                 VALUES (?1, 1, '/Shows/' || ?1 || '.mkv', 1000, 0, ?2, printf('%08x', ?1),
                         CASE WHEN ?2 IS NULL THEN NULL ELSE 1 END)",
                params![id, duration],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO matches (id, file_id, work_id, confidence, state, decided_by,
                                      created_at_ms, updated_at_ms)
                 VALUES (?1, ?1, ?2, 0.9, 'auto', 'rule', 0, 0)",
                params![id, work],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO match_episodes (match_id, position, episode_id) VALUES (?1, 0, ?2)",
                params![id, episode],
            )
            .unwrap();
        id
    }

    /// The files are the parts of one split episode (LIB-020).
    fn parts(&self, group: i64, files: &[i64]) {
        self.conn
            .execute(
                "INSERT INTO file_groups (id, source_id, root_path, kind)
                 VALUES (?1, 1, '/Shows/parts-' || ?1, 'loose')",
                [group],
            )
            .unwrap();
        for (n, f) in files.iter().enumerate() {
            self.conn
                .execute(
                    "INSERT INTO file_group_members (group_id, file_id, role, part_no)
                     VALUES (?1, ?2, 'part', ?3)",
                    params![group, f, i64::try_from(n).unwrap() + 1],
                )
                .unwrap();
        }
    }
}

fn ep(id: i64) -> ItemRef {
    ItemRef::Episode(id)
}

/// "Dark" (TMDB 70523, progress under `tmdb:episode:` keys):
///
/// | ep  | S·E   | file             | metadata | progress        |
/// |-----|-------|------------------|----------|-----------------|
/// | 101 | 1·1   | 50 min           |          | watched         |
/// | 102 | 1·2   | 50 min           |          | 10 of 50 min    |
/// | 103 | 1·3   | 50 min           |          | 30 s (< 2 %)    |
/// | 104 | 1·4   | not probed       | 45 min   |                 |
/// | 105 | 2·1   | 60 min           |          | watched         |
/// | 106 | 2·2   | not probed       | —        |                 |
/// | 107 | 2·3   | 2 parts × 30 min |          |                 |
/// | 108 | 0·1   | 20 min           |          |                 |
/// | 109 | 2·4   | (no file)        | 50 min   |                 |
/// | 110 | 2·5   | missing          | 50 min   |                 |
fn dark(fx: &mut Fixture) -> ItemRef {
    fx.show(1, "Dark", Some(70523));
    for (id, s, e, runtime) in [
        (101, 1, 1, None),
        (102, 1, 2, None),
        (103, 1, 3, None),
        (104, 1, 4, Some(45 * MIN)),
        (105, 2, 1, None),
        (106, 2, 2, None),
        (107, 2, 3, None),
        (108, 0, 1, None),
        (109, 2, 4, Some(50 * MIN)),
        (110, 2, 5, Some(50 * MIN)),
    ] {
        fx.episode(id, 1, s, e, runtime);
    }
    for id in [101, 102, 103] {
        fx.file(1, id, Some(50 * MIN));
    }
    fx.file(1, 104, None);
    fx.file(1, 105, Some(60 * MIN));
    fx.file(1, 106, None);
    let a = fx.file(1, 107, Some(30 * MIN));
    let b = fx.file(1, 107, Some(30 * MIN));
    fx.parts(1, &[a, b]);
    fx.file(1, 108, Some(20 * MIN));
    let gone = fx.file(1, 110, Some(50 * MIN));
    fx.conn
        .execute(
            "UPDATE files SET missing_since_ms = 1 WHERE id = ?1",
            [gone],
        )
        .unwrap();

    let cat = &fx.cat;
    cat.save_progress(ep(101), 0, 50 * MIN, 1_000, true)
        .unwrap();
    cat.save_progress(ep(102), 10 * MIN, 50 * MIN, 1_000, false)
        .unwrap();
    cat.save_progress(ep(103), 30_000, 50 * MIN, 1_000, false)
        .unwrap();
    cat.save_progress(ep(105), 0, 60 * MIN, 1_000, true)
        .unwrap();
    ItemRef::Show(1)
}

#[test]
fn sums_seasons_and_the_show() {
    let mut fx = Fixture::new();
    let show = dark(&mut fx);
    let r = fx.cat.show_runtime(show, EpisodeOrder::Tmdb).unwrap();

    // Tabs: regular seasons ascending, specials last.
    let seasons: Vec<i64> = r.seasons.iter().map(|s| s.season).collect();
    assert_eq!(seasons, [1, 2, 0]);

    // Season 1: 3 × 50 + ≈45; left: 40 of the started one, the one barely
    // started whole, the unprobed one by its metadata runtime.
    assert_eq!(
        *r.season(1).unwrap(),
        RuntimeTotal {
            episodes: 4,
            watched: 1,
            total_ms: 195 * MIN,
            remaining_ms: 135 * MIN,
            estimated: true,
            remaining_estimated: true,
        }
    );
    // Season 2: 60 watched, one without any duration (≈, adds nothing),
    // the split episode by both parts; no file and a missing file are not
    // in the library.
    assert_eq!(
        *r.season(2).unwrap(),
        RuntimeTotal {
            episodes: 3,
            watched: 1,
            total_ms: 120 * MIN,
            remaining_ms: 60 * MIN,
            estimated: true,
            remaining_estimated: true,
        }
    );
    // Specials: in their own season only.
    assert_eq!(
        *r.season(0).unwrap(),
        RuntimeTotal {
            episodes: 1,
            watched: 0,
            total_ms: 20 * MIN,
            remaining_ms: 20 * MIN,
            estimated: false,
            remaining_estimated: false,
        }
    );
    assert_eq!(
        r.show,
        RuntimeTotal {
            episodes: 7,
            watched: 2,
            total_ms: 315 * MIN,
            remaining_ms: 195 * MIN,
            estimated: true,
            remaining_estimated: true,
        }
    );
    assert!(!r.show.all_watched());

    // The Show Detail screen gets the same with its episodes.
    let eps = fx.cat.show_episodes(show, None).unwrap();
    assert_eq!(eps.runtime, r);
}

#[test]
fn a_season_marked_watched_leaves_nothing() {
    let mut fx = Fixture::new();
    let show = dark(&mut fx);
    let season1 = [ep(101), ep(102), ep(103), ep(104)];
    fx.cat.set_watched_many(&season1, true, 2_000).unwrap();

    let r = fx.cat.show_runtime(show, EpisodeOrder::Tmdb).unwrap();
    let s1 = r.season(1).unwrap();
    assert_eq!((s1.watched, s1.remaining_ms), (4, 0));
    assert!(s1.all_watched());
    // The total stays; the show has season 2 left.
    assert_eq!(s1.total_ms, 195 * MIN);
    assert_eq!(r.show.remaining_ms, 60 * MIN);

    // Season 2 and the specials too: the show is watched.
    fx.cat
        .set_watched_many(&[ep(105), ep(106), ep(107)], true, 3_000)
        .unwrap();
    let r = fx.cat.show_runtime(show, EpisodeOrder::Tmdb).unwrap();
    assert!(r.show.all_watched());
    assert_eq!(r.show.remaining_ms, 0);
    // Specials are not part of «просмотрено» of the show.
    assert!(!r.season(0).unwrap().all_watched());

    // Unmarked: everything left again.
    fx.cat.set_watched_many(&season1, false, 4_000).unwrap();
    let r = fx.cat.show_runtime(show, EpisodeOrder::Tmdb).unwrap();
    assert_eq!(r.season(1).unwrap().remaining_ms, 195 * MIN);
}

#[test]
fn a_show_without_tmdb_reads_progress_by_file() {
    let mut fx = Fixture::new();
    fx.show(2, "Home Videos", None);
    fx.episode(201, 2, 1, 1, None);
    fx.episode(202, 2, 1, 2, None);
    fx.file(2, 201, Some(30 * MIN));
    fx.file(2, 202, Some(30 * MIN));
    fx.cat
        .save_progress(ep(201), 15 * MIN, 30 * MIN, 1_000, false)
        .unwrap();
    let r = fx
        .cat
        .show_runtime(ItemRef::Show(2), EpisodeOrder::Tmdb)
        .unwrap();
    assert_eq!(r.show.total_ms, 60 * MIN);
    assert_eq!(r.show.remaining_ms, 45 * MIN);
    assert!(!r.show.estimated);
}

#[test]
fn seasons_follow_the_episode_order() {
    let mut fx = Fixture::new();
    let show = dark(&mut fx);
    // Absolute: one run of the regular episodes, specials apart.
    let r = fx.cat.show_runtime(show, EpisodeOrder::Absolute).unwrap();
    let seasons: Vec<i64> = r.seasons.iter().map(|s| s.season).collect();
    assert_eq!(seasons, [1, 0]);
    assert_eq!(*r.season(1).unwrap(), r.show);
}

#[test]
fn a_thousand_episodes_in_one_query() {
    let mut fx = Fixture::new();
    fx.show(3, "One Piece", Some(37854));
    fx.conn.execute_batch("BEGIN").unwrap();
    for n in 0..1000 {
        let id = 1000 + n;
        fx.episode(id, 3, 1 + n / 100, 1 + n % 100, Some(24 * MIN));
        fx.file(3, id, (n % 10 != 0).then_some(24 * MIN));
    }
    // Every third episode watched, a few started.
    for n in (0..1000).step_by(3) {
        fx.conn
            .execute(
                "INSERT INTO progress (profile_id, media_key, source_type, position_ms, duration_ms,
                                       watched, updated_at_ms, device_id)
                 VALUES (1, ?1, 'episode', ?2, ?3, ?4, 1, 'test')",
                params![
                    format!("tmdb:episode:37854:{}:{}", 1 + n / 100, 1 + n % 100),
                    if n % 2 == 0 { 24 * MIN } else { 12 * MIN },
                    24 * MIN,
                    n % 2 == 0
                ],
            )
            .unwrap();
    }
    fx.conn.execute_batch("COMMIT").unwrap();

    let show = ItemRef::Show(3);
    let r = fx.cat.show_runtime(show, EpisodeOrder::Tmdb).unwrap();
    assert_eq!(r.show.episodes, 1000);
    assert_eq!(r.seasons.len(), 10);
    assert_eq!(r.show.total_ms, 1000 * 24 * MIN);
    assert_eq!(r.show.watched, 167);
    // 167 watched, 167 at half.
    assert_eq!(
        r.show.remaining_ms,
        (1000 - 167) * 24 * MIN - 167 * 12 * MIN
    );

    // The bound of the task is for release builds; debug builds get slack.
    let bound = if cfg!(debug_assertions) {
        Duration::from_millis(500)
    } else {
        Duration::from_millis(50)
    };
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        fx.cat.show_runtime(show, EpisodeOrder::Tmdb).unwrap();
        best = best.min(t.elapsed());
    }
    println!("runtime of 1000 episodes: {best:?}");
    assert!(best <= bound, "{best:?} > {bound:?}");
}
