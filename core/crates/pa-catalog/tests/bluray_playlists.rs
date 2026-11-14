// SPDX-License-Identifier: MPL-2.0
//! LIB-021 "done when": a synthetic Blu-ray folder with several playlists
//! plays the clips of its longest playlist, and a library that played the
//! largest stream before keeps its match and progress when the main title
//! moves to the playlist. Synthetic files (bytes, MPLS written with
//! `pa_scanner::mpls`); the player side is in
//! `pa_player/test/edl_libmpv_test.dart`.

use std::path::Path;

use pa_catalog::{Catalog, ItemRef, PlayPart};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;
use pa_scanner::mpls::{PlayItem, Playlist};

const BD: &str = "Movies/Blade Runner (1982)";

fn write(root: &Path, rel: &str, size: usize, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..size)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn item(clip: &str, secs: u32) -> PlayItem {
    PlayItem {
        clip_id: clip.into(),
        in_time: 27_000_000,
        out_time: 27_000_000 + secs * 45_000,
    }
}

fn playlist(root: &Path, name: &str, items: Vec<PlayItem>) {
    let path = root.join(format!("{BD}/BDMV/PLAYLIST/{name}.mpls"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, Playlist { items }.to_bytes()).unwrap();
}

fn stream(clip: &str) -> String {
    format!("{BD}/BDMV/STREAM/{clip}.m2ts")
}

/// A disc whose biggest stream is a bonus documentary and whose film is
/// three clips; the playlists are written by [`good_playlists`].
fn disc(root: &Path) {
    write(root, &format!("{BD}/BDMV/index.bdmv"), 120, 1);
    write(root, &format!("{BD}/BDMV/MovieObject.bdmv"), 120, 2);
    write(root, &format!("{BD}/BDMV/BACKUP/index.bdmv"), 120, 3);
    write(root, &stream("00100"), 200_000, 4); // bonus documentary
    write(root, &stream("00012"), 120_000, 5);
    write(root, &stream("00010"), 90_000, 6);
    write(root, &stream("00011"), 60_000, 7);
    write(root, &stream("00099"), 3_000, 8); // menu loop
}

fn good_playlists(root: &Path) {
    playlist(root, "00000", vec![item("00099", 30)]);
    playlist(root, "00001", vec![item("00100", 3_000)]);
    // The film: the clips in an order that is not their name order.
    playlist(
        root,
        "00002",
        vec![item("00012", 3_000), item("00010", 2_400), item("00011", 1_500)],
    );
    // A copy in BACKUP is never read; a playlist with a missing clip is
    // not usable however long it is.
    let backup = root.join(format!("{BD}/BDMV/BACKUP/PLAYLIST/00009.mpls"));
    std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
    std::fs::write(&backup, Playlist { items: vec![item("00100", 99_000)] }.to_bytes()).unwrap();
    playlist(root, "00003", vec![item("00404", 99_000)]);
}

fn open(dir: &Path, root: &Path) -> Catalog {
    let cat = Catalog::open(dir.join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat
}

fn scan(cat: &Catalog) {
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
}

fn paths(parts: &[PlayPart]) -> Vec<&str> {
    parts.iter().map(|p| p.rel_path.as_str()).collect()
}

fn film() -> [String; 3] {
    [stream("00012"), stream("00010"), stream("00011")]
}

#[test]
fn the_longest_playlist_is_the_main_title() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    disc(&root);
    good_playlists(&root);
    let cat = open(dir.path(), &root);
    scan(&cat);

    let movies = cat.movies().unwrap();
    assert_eq!(movies.len(), 1, "{movies:#?}");
    let movie = &movies[0];
    assert_eq!(movie.title, "Blade Runner");
    let parts = cat.play_parts(movie.source_id, &movie.rel_path).unwrap();
    assert_eq!(paths(&parts), film());
    assert_eq!(cat.versions(movie.id).unwrap().len(), 1);

    // A rescan changes nothing.
    scan(&cat);
    let again = cat.movies().unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].id, movie.id);
    assert_eq!(
        paths(&cat.play_parts(again[0].source_id, &again[0].rel_path).unwrap()),
        film()
    );
}

fn live_matches(db: &Path) -> Vec<(String, String)> {
    let conn = Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT f.path, m.state FROM matches m JOIN files f ON f.id = m.file_id
             WHERE m.state <> 'rejected' ORDER BY f.path",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn a_library_from_before_keeps_its_match_and_progress() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    disc(&root);
    // Before LIB-021 playlists were not read: an unreadable playlist gives
    // the same result, the largest stream.
    write(&root, &format!("{BD}/BDMV/PLAYLIST/00002.mpls"), 300, 9);
    let cat = open(dir.path(), &root);
    scan(&cat);
    let movies = cat.movies().unwrap();
    assert_eq!(movies.len(), 1, "{movies:#?}");
    let before = movies[0].clone();
    assert_eq!(
        paths(&cat.play_parts(before.source_id, &before.rel_path).unwrap()),
        [stream("00100")]
    );
    // The user fixed the match (no TMDB id: progress is under the `file:`
    // key of the stream) and watched half of it.
    cat.fix_match(before.id, Some("Blade Runner: The Final Cut"), Some(1982), None)
        .unwrap();
    assert!(
        cat.save_progress(before.id, 3_000_000, 7_000_000, 1_000, false)
            .unwrap()
    );
    assert!(cat.set_favorite(before.id, true).unwrap());

    // The new analysis reads the playlists: the main title moves to the
    // three clips of the film.
    std::fs::remove_file(root.join(format!("{BD}/BDMV/PLAYLIST/00002.mpls"))).unwrap();
    good_playlists(&root);
    scan(&cat);

    let movies = cat.movies().unwrap();
    assert_eq!(movies.len(), 1, "{movies:#?}");
    let after = &movies[0];
    assert_eq!(after.id, before.id, "same work");
    assert_eq!(after.title, "Blade Runner: The Final Cut");
    assert_eq!(
        paths(&cat.play_parts(after.source_id, &after.rel_path).unwrap()),
        film()
    );
    // The user's decision moved to the clips; the bonus stream lost it.
    let db = dir.path().join("library.db");
    let matches = live_matches(&db);
    let mut want: Vec<(String, String)> = film()
        .iter()
        .map(|p| (format!("/{p}"), "user_set".to_owned()))
        .collect();
    want.sort();
    assert_eq!(matches, want);
    let progress = cat.progress(after.id).unwrap().expect("progress kept");
    assert_eq!(progress.position_ms, 3_000_000);
    assert!(cat.is_favorite(after.id).unwrap(), "favorite kept");
    assert_eq!(cat.versions(after.id).unwrap().len(), 1);

    // And it stays so.
    scan(&cat);
    assert_eq!(live_matches(&db), matches);
    assert_eq!(cat.progress(after.id).unwrap().unwrap().position_ms, 3_000_000);
}
