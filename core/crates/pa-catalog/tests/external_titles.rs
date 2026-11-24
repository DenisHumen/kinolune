// SPDX-License-Identifier: MPL-2.0
//! SYNC-011: library titles by TMDB/IMDb/TVDB ids for Trakt — which title
//! an item is, marking a tracker's play watched (idempotent, a newer local
//! state wins), what is watched and rated here.

use std::path::Path;

use pa_catalog::{Catalog, ExternalTitle, ItemKind, ItemRef, TitleKind};
use pa_scanner::CancelToken;

const ARRIVAL: &str = "Movies/Arrival (2016)/Arrival.2016.1080p.mkv";
const DARK: &str = "Shows/Dark/Season 1/Dark.S01E01.1080p.mkv";

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

struct Lib {
    _dir: tempfile::TempDir,
    cat: Catalog,
    arrival: ItemRef,
    dark: ItemRef,
    episode: ItemRef,
}

fn setup() -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, ARRIVAL, 1);
    media(&root, DARK, 2);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let find = |kind: ItemKind, title: &str| {
        let items = match kind {
            ItemKind::Movie => cat.movies().unwrap(),
            _ => cat.shows().unwrap(),
        };
        items.into_iter().find(|i| i.title == title).unwrap().id
    };
    let arrival = find(ItemKind::Movie, "Arrival");
    let dark = find(ItemKind::Show, "Dark");
    let episode = cat.episodes(dark).unwrap()[0].id;
    // What the metadata would have stored (no network in tests).
    let conn = pa_library::open(cat.path()).unwrap();
    let (ItemRef::Movie(m), ItemRef::Show(s)) = (arrival, dark) else {
        unreachable!()
    };
    for (work, provider, kind, id) in [
        (m, "tmdb", "movie", "329865"),
        (m, "imdb", "", "tt2543164"),
        (s, "tmdb", "tv", "70523"),
        (s, "tvdb", "series", "334824"),
    ] {
        conn.execute(
            "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id) VALUES (?1, ?2, ?3, ?4)",
            pa_library::rusqlite::params![work, provider, kind, id],
        )
        .unwrap();
    }
    Lib {
        _dir: dir,
        cat,
        arrival,
        dark,
        episode,
    }
}

#[test]
fn items_know_their_external_titles() {
    let lib = setup();
    let movie = lib.cat.external_title(lib.arrival).unwrap().unwrap();
    assert_eq!(movie.kind, Some(TitleKind::Movie));
    assert_eq!(movie.tmdb, Some(329_865));
    assert_eq!(movie.imdb.as_deref(), Some("tt2543164"));
    let ep = lib.cat.external_title(lib.episode).unwrap().unwrap();
    assert_eq!(ep.kind, Some(TitleKind::Episode));
    assert_eq!((ep.tmdb, ep.tvdb), (Some(70_523), Some(334_824)));
    assert_eq!((ep.season, ep.episode), (Some(1), Some(1)));
    assert_eq!(
        lib.cat.item_of_title(&ExternalTitle::show(70_523)).unwrap(),
        Some(lib.dark)
    );
    assert_eq!(
        lib.cat
            .item_of_title(&ExternalTitle::episode(70_523, 1, 1))
            .unwrap(),
        Some(lib.episode)
    );
    assert_eq!(lib.cat.item_of_title(&ExternalTitle::movie(1)).unwrap(), None);
}

#[test]
fn tracker_plays_are_marked_once_and_newer_local_state_wins() {
    let lib = setup();
    let at = 1_413_046_854_000;
    let movie = ExternalTitle::movie(329_865);
    assert!(lib.cat.mark_title_watched(&movie, at).unwrap());
    assert!(!lib.cat.mark_title_watched(&movie, at).unwrap());
    let p = lib.cat.progress(lib.arrival).unwrap().unwrap();
    assert!(p.watched);
    assert_eq!(p.updated_at_ms, at);
    // Not in the library: nothing.
    assert!(!lib.cat.mark_title_watched(&ExternalTitle::movie(1), at).unwrap());

    // The episode was started here after the tracker's play: kept.
    lib.cat
        .save_progress(lib.episode, 60_000, 3_600_000, at + 10_000, false)
        .unwrap();
    let ep = ExternalTitle::episode(70_523, 1, 1);
    assert!(!lib.cat.mark_title_watched(&ep, at).unwrap());
    assert!(lib.cat.mark_title_watched(&ep, at + 20_000).unwrap());

    let watched = lib.cat.watched_titles().unwrap();
    assert_eq!(watched.len(), 2);
    assert!(watched.iter().any(|(t, when)| t.kind == Some(TitleKind::Movie)
        && t.imdb.as_deref() == Some("tt2543164")
        && *when == at));
    assert!(watched.iter().any(|(t, _)| t.kind == Some(TitleKind::Episode)
        && t.tmdb == Some(70_523)
        && t.season == Some(1)));
}

#[test]
fn ratings_by_title() {
    let lib = setup();
    assert!(lib.cat.rate_title(&ExternalTitle::show(70_523), 4).unwrap());
    assert!(!lib.cat.rate_title(&ExternalTitle::show(70_523), 4).unwrap());
    assert!(!lib.cat.rate_title(&ExternalTitle::movie(1), 4).unwrap());
    lib.cat.labels().set_rating(lib.arrival, Some(5)).unwrap();
    let rated = lib.cat.rated_titles().unwrap();
    assert_eq!(rated.len(), 2);
    assert!(rated
        .iter()
        .any(|(t, s, _)| t.kind == Some(TitleKind::Show) && t.tmdb == Some(70_523) && *s == 4));
    assert!(rated
        .iter()
        .any(|(t, s, _)| t.kind == Some(TitleKind::Movie) && *s == 5));
}
