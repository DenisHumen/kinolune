// SPDX-License-Identifier: MPL-2.0
//! Episode orders of a show (UI-010): TMDB, absolute and AniDB numbering on
//! a scanned folder, the stored display preference and episode progress.

use std::path::Path;

use pa_catalog::{Catalog, EpisodeOrder, ItemRef, ShowEpisodes};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

/// An anime split into two TMDB seasons (two AniDB entries) and a special.
fn setup() -> (tempfile::TempDir, Catalog, ItemRef) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Shows/Kaiju/Season 1/Kaiju.S01E01.mkv", 1);
    media(&root, "Shows/Kaiju/Season 1/Kaiju.S01E02.mkv", 2);
    media(&root, "Shows/Kaiju/Season 1/Kaiju.S01E03.mkv", 3);
    media(&root, "Shows/Kaiju/Season 2/Kaiju.S02E01.mkv", 4);
    media(&root, "Shows/Kaiju/Season 2/Kaiju.S02E02.mkv", 5);
    media(&root, "Shows/Kaiju/Specials/Kaiju.S00E01.mkv", 6);
    let cat = Catalog::open(dir.path().join("db/library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let shows = cat.shows().unwrap();
    assert_eq!(shows.len(), 1, "{shows:?}");
    (dir, cat, shows[0].id)
}

/// (season, episode) in the order, then in TMDB order.
type Numbers = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);

fn numbers(eps: &ShowEpisodes) -> Vec<Numbers> {
    eps.episodes
        .iter()
        .map(|e| (e.item.season, e.item.episode, e.tmdb_season, e.tmdb_episode))
        .collect()
}

fn episode_rows(cat: &Catalog) -> Connection {
    Connection::open(cat.path()).unwrap()
}

/// Id of the episode row with TMDB numbers `s`x`e`.
fn episode_id(conn: &Connection, s: i64, e: i64) -> i64 {
    conn.query_row(
        "SELECT id FROM episodes WHERE season_number = ?1 AND episode_number = ?2",
        [s, e],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn tmdb_order_by_default_and_absolute_counts_regular_episodes() {
    let (_dir, cat, show) = setup();
    let tmdb = cat.show_episodes(show, None).unwrap();
    assert_eq!(tmdb.order, EpisodeOrder::Tmdb);
    assert_eq!(
        tmdb.available,
        vec![EpisodeOrder::Tmdb, EpisodeOrder::Absolute],
        "no AniDB mappings yet"
    );
    assert_eq!(
        numbers(&tmdb),
        vec![
            (Some(0), Some(1), Some(0), Some(1)),
            (Some(1), Some(1), Some(1), Some(1)),
            (Some(1), Some(2), Some(1), Some(2)),
            (Some(1), Some(3), Some(1), Some(3)),
            (Some(2), Some(1), Some(2), Some(1)),
            (Some(2), Some(2), Some(2), Some(2)),
        ]
    );

    let abs = cat
        .show_episodes(show, Some(EpisodeOrder::Absolute))
        .unwrap();
    assert_eq!(abs.order, EpisodeOrder::Absolute);
    let n: Vec<_> = abs
        .episodes
        .iter()
        .map(|e| (e.item.season, e.item.episode))
        .collect();
    assert_eq!(
        n,
        vec![
            (Some(0), Some(1)),
            (Some(1), Some(1)),
            (Some(1), Some(2)),
            (Some(1), Some(3)),
            (Some(1), Some(4)),
            (Some(1), Some(5)),
        ]
    );

    // A provider absolute number wins over counting.
    let conn = episode_rows(&cat);
    let s2e1 = episode_id(&conn, 2, 1);
    conn.execute(
        "UPDATE episodes SET absolute_number = 13 WHERE id = ?1",
        [s2e1],
    )
    .unwrap();
    let abs = cat
        .show_episodes(show, Some(EpisodeOrder::Absolute))
        .unwrap();
    assert!(
        abs.episodes
            .iter()
            .any(|e| e.tmdb_season == Some(2) && e.item.episode == Some(13))
    );
}

#[test]
fn switching_from_tmdb_to_anidb_rebuilds_the_episodes() {
    let (_dir, cat, show) = setup();
    // AniDB (as Anime-Lists maps it): TMDB S1 is entry 1, S1E3 and S2 are
    // entry 2 (episodes 1–3), the TMDB special is AniDB special S1.
    let conn = episode_rows(&cat);
    for (s, e, anidb_season, anidb_episode) in [
        (1, 1, 1, 1),
        (1, 2, 1, 2),
        (1, 3, 2, 1),
        (2, 1, 2, 2),
        (2, 2, 2, 3),
        (0, 1, 0, 1),
    ] {
        let id = episode_id(&conn, s, e);
        conn.execute(
            "INSERT INTO episode_mappings (episode_id, scheme, season_number, episode_number, source)
             VALUES (?1, 'anidb', ?2, ?3, 'anime_lists')",
            [id, anidb_season, anidb_episode],
        )
        .unwrap();
    }

    let tmdb = cat.show_episodes(show, None).unwrap();
    assert_eq!(
        tmdb.available,
        vec![
            EpisodeOrder::Tmdb,
            EpisodeOrder::Absolute,
            EpisodeOrder::Anidb
        ]
    );
    let seasons = |e: &ShowEpisodes, s: i64| {
        e.episodes
            .iter()
            .filter(|x| x.item.season == Some(s))
            .count()
    };
    assert_eq!((seasons(&tmdb, 1), seasons(&tmdb, 2)), (3, 2));

    cat.set_episode_order(show, EpisodeOrder::Anidb).unwrap();
    let anidb = cat.show_episodes(show, None).unwrap();
    assert_eq!(anidb.order, EpisodeOrder::Anidb, "stored preference");
    assert_eq!((seasons(&anidb, 1), seasons(&anidb, 2)), (2, 3));
    let s1e3 = anidb
        .episodes
        .iter()
        .find(|e| e.tmdb_season == Some(1) && e.tmdb_episode == Some(3))
        .unwrap();
    assert_eq!((s1e3.item.season, s1e3.item.episode), (Some(2), Some(1)));
    // Same items, other numbers.
    let mut a: Vec<_> = tmdb.episodes.iter().map(|e| e.item.id).collect();
    let mut b: Vec<_> = anidb.episodes.iter().map(|e| e.item.id).collect();
    a.sort_by_key(ToString::to_string);
    b.sort_by_key(ToString::to_string);
    assert_eq!(a, b);

    // Back to TMDB.
    cat.set_episode_order(show, EpisodeOrder::Tmdb).unwrap();
    assert_eq!(
        cat.show_episodes(show, None).unwrap().order,
        EpisodeOrder::Tmdb
    );
}

#[test]
fn unavailable_order_falls_back_to_tmdb_and_progress_is_attached() {
    let (_dir, cat, show) = setup();
    cat.set_episode_order(show, EpisodeOrder::Anidb).unwrap();
    let eps = cat.show_episodes(show, None).unwrap();
    assert_eq!(eps.order, EpisodeOrder::Tmdb, "no AniDB data");

    let first = eps.episodes[1].item.id;
    assert!(
        cat.save_progress(first, 2_990_000, 3_000_000, 1_000, false)
            .unwrap()
    );
    let second = eps.episodes[2].item.id;
    assert!(
        cat.save_progress(second, 600_000, 3_000_000, 2_000, false)
            .unwrap()
    );
    let eps = cat.show_episodes(show, None).unwrap();
    let p1 = eps.episodes[1].progress.as_ref().unwrap();
    assert!(p1.watched);
    let p2 = eps.episodes[2].progress.as_ref().unwrap();
    assert_eq!((p2.position_ms, p2.watched), (600_000, false));
    assert!(eps.episodes[3].progress.is_none());

    assert!(cat.set_episode_order(first, EpisodeOrder::Tmdb).is_err());
    assert!(cat.show_episodes(first, None).is_err());
}
