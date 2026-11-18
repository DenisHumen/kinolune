// SPDX-License-Identifier: MPL-2.0
//! LIB-014 done-when, the Kinolune half: a library over the media folder
//! of `tests/nfo_jellyfin/run.sh` writes `movie.nfo` and `tvshow.nfo` and
//! renames a folder with its id. run.sh then starts Jellyfin without
//! internet on that folder and checks that it reads both. Run with
//! `core/crates/pa-catalog/tests/nfo_jellyfin/run.sh`.
#![cfg(feature = "server-tests")]

use std::path::{Path, PathBuf};

use pa_catalog::{Catalog, ItemRef};
use pa_scanner::CancelToken;

fn work(r: ItemRef) -> i64 {
    match r {
        ItemRef::Movie(w) | ItemRef::Show(w) => w,
        _ => panic!("{r}"),
    }
}

/// What the metadata step would have stored.
fn know(db: &Path, work: i64, title: &str, overview: &str, ids: &[(&str, &str, &str)]) {
    let conn = pa_library::open(db).unwrap();
    conn.execute(
        "UPDATE works SET title = ?2, overview = ?3, year = coalesce(year, 2020) WHERE id = ?1",
        pa_library::rusqlite::params![work, title, overview],
    )
    .unwrap();
    for (provider, kind, id) in ids {
        conn.execute(
            "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id) VALUES (?1, ?2, ?3, ?4)",
            pa_library::rusqlite::params![work, provider, kind, id],
        )
        .unwrap();
    }
}

#[test]
fn export_for_jellyfin() {
    let media = PathBuf::from(
        std::env::var("PA_NFO_MEDIA").expect("PA_NFO_MEDIA: use tests/nfo_jellyfin/run.sh"),
    );
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Catalog::open(&db).unwrap();
    cat.add_source(None, "local", "Media", media.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let movies = cat.movies().unwrap();
    let shows = cat.shows().unwrap();
    let find = |items: &[pa_catalog::Item], t: &str| {
        items
            .iter()
            .find(|i| i.title.contains(t))
            .unwrap_or_else(|| panic!("{t} in {items:?}"))
            .id
    };
    let film = find(&movies, "Export Film");
    let tagged = find(&movies, "Tagged Film");
    let show = find(&shows, "Export Show");
    know(
        &db,
        work(film),
        "Экспортный фильм",
        "Описание из Kinolune & NFO",
        &[("tmdb", "movie", "438631"), ("imdb", "", "tt1160419")],
    );
    know(
        &db,
        work(tagged),
        "Tagged Film",
        "",
        &[("tmdb", "movie", "550")],
    );
    know(
        &db,
        work(show),
        "Экспортный сериал",
        "Сериал из Kinolune",
        &[("tmdb", "tv", "70523"), ("tvdb", "series", "334824")],
    );

    assert_eq!(cat.export_nfo(film, false).unwrap().written, 1);
    assert_eq!(cat.export_nfo(show, false).unwrap().written, 1);
    assert_eq!(cat.rename_with_ids(tagged).unwrap(), 1);
    for p in [
        "Movies/Kinolune Export Film (2020)/movie.nfo",
        "Shows/Kinolune Export Show/tvshow.nfo",
        "Movies/Kinolune Tagged Film (2019) [tmdbid-550]",
    ] {
        assert!(media.join(p).exists(), "{p}");
    }
}
