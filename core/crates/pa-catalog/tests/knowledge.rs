// SPDX-License-Identifier: MPL-2.0
//! LIB-014: what the library knows goes out to the files — NFO next to the
//! videos and the id in the folder name — and comes back in on a scan.

use std::path::Path;

use pa_catalog::{Catalog, ItemRef};
use pa_scanner::CancelToken;
use pa_vfs::LocalFs;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn item(items: &[pa_catalog::Item], title: &str) -> ItemRef {
    items
        .iter()
        .find(|i| i.title == title)
        .unwrap_or_else(|| panic!("{title} in {items:?}"))
        .id
}

fn work(r: ItemRef) -> i64 {
    match r {
        ItemRef::Movie(w) | ItemRef::Show(w) => w,
        _ => panic!("{r}"),
    }
}

fn add_id(db: &Path, work: i64, provider: &str, kind: &str, id: &str) {
    let conn = pa_library::open(db).unwrap();
    conn.execute(
        "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id) VALUES (?1, ?2, ?3, ?4)",
        pa_library::rusqlite::params![work, provider, kind, id],
    )
    .unwrap();
    conn.execute(
        "UPDATE works SET overview = 'Песчаная планета & пряность' WHERE id = ?1",
        [work],
    )
    .unwrap();
}

fn ids_of(db: &Path, work: i64) -> Vec<(String, String, String)> {
    let conn = pa_library::open(db).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT provider, ext_kind, ext_id FROM external_ids WHERE work_id = ?1 ORDER BY 1",
        )
        .unwrap();
    stmt.query_map([work], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn nfo_export_rename_and_import() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Movies/Dune (2021)/Dune (2021).mkv", 1);
    media(&root, "Movies/Heat 1995.mkv", 2);
    media(&root, "Movies/Alien 1979.mkv", 3);
    media(&root, "TV/Dark/Season 1/Dark S01E01.mkv", 4);
    media(&root, "TV/Dark/Season 1/Dark S01E02.mkv", 5);
    // Import: an id in a folder name decides the work on the scan.
    media(&root, "Movies/Up (2009) {tmdb-14160}/Up.mkv", 6);
    let db = dir.path().join("db/library.db");
    let cat = Catalog::open(&db).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let movies = cat.movies().unwrap();
    let shows = cat.shows().unwrap();
    let (dune, heat, up) = (
        item(&movies, "Dune"),
        item(&movies, "Heat"),
        item(&movies, "Up"),
    );
    let dark = item(&shows, "Dark");
    assert_eq!(
        ids_of(&db, work(up)),
        [("tmdb".into(), "movie".into(), "14160".into())]
    );

    // Without ids there is nothing to write.
    let plan = cat.knowledge_plan(dune).unwrap();
    assert!(plan.ids.is_empty() && plan.rename.is_empty());
    assert!(cat.export_nfo(dune, false).is_err());

    add_id(&db, work(dune), "tmdb", "movie", "438631");
    add_id(&db, work(heat), "imdb", "", "tt0113277");
    add_id(&db, work(dark), "tmdb", "tv", "70523");

    let plan = cat.knowledge_plan(dune).unwrap();
    assert!(!plan.read_only);
    assert_eq!(plan.ids, [("tmdb".to_owned(), "438631".to_owned())]);
    assert_eq!(plan.nfo.len(), 1);
    assert_eq!(plan.nfo[0].path, "/Movies/Dune (2021)/movie.nfo");
    assert!(!plan.nfo[0].exists);
    assert_eq!(plan.rename.len(), 1);
    assert_eq!(plan.rename[0].path, "/Movies/Dune (2021)");
    assert_eq!(plan.rename[0].new_name, "Dune (2021) [tmdbid-438631]");

    // A loose movie: `<video>.nfo`, its name stays.
    let plan = cat.knowledge_plan(heat).unwrap();
    assert_eq!(plan.nfo[0].path, "/Movies/Heat 1995.nfo");
    assert!(plan.rename.is_empty());

    // A show: `tvshow.nfo` above the season folder; an episode is its show.
    let first = cat.episodes(dark).unwrap()[0].id;
    let plan = cat.knowledge_plan(first).unwrap();
    assert_eq!(plan.nfo[0].path, "/TV/Dark/tvshow.nfo");
    assert_eq!(plan.rename[0].new_name, "Dark [tmdbid-70523]");

    // Export: written, then left alone, then replaced (old one to the trash).
    let r = cat.export_nfo(dune, false).unwrap();
    assert_eq!((r.written, r.replaced, r.skipped), (1, 0, 0));
    let nfo = root.join("Movies/Dune (2021)/movie.nfo");
    let text = std::fs::read_to_string(&nfo).unwrap();
    assert!(text.contains("<movie>"), "{text}");
    assert!(
        text.contains("<plot>Песчаная планета &amp; пряность</plot>"),
        "{text}"
    );
    std::fs::write(&nfo, "<movie><title>mine</title></movie>").unwrap();
    assert!(cat.knowledge_plan(dune).unwrap().nfo[0].exists);
    let r = cat.export_nfo(dune, false).unwrap();
    assert_eq!((r.written, r.skipped), (0, 1));
    assert!(std::fs::read_to_string(&nfo).unwrap().contains("mine"));
    let r = cat.export_nfo(dune, true).unwrap();
    assert_eq!((r.written, r.replaced), (1, 1));
    assert!(std::fs::read_to_string(&nfo).unwrap().contains("438631"));
    let vfs = LocalFs::new(&root).unwrap();
    let trash = pa_vfs::files::list_trash(&vfs).unwrap();
    assert_eq!(trash.len(), 1, "{trash:?}");

    for i in [heat, dark] {
        assert_eq!(cat.export_nfo(i, false).unwrap().written, 1);
    }
    assert!(
        std::fs::read_to_string(root.join("Movies/Heat 1995.nfo"))
            .unwrap()
            .contains("tt0113277")
    );
    assert!(
        std::fs::read_to_string(root.join("TV/Dark/tvshow.nfo"))
            .unwrap()
            .contains("<tvshow>")
    );

    // Rename: the folders get the id, the works keep their files.
    assert_eq!(cat.rename_with_ids(dune).unwrap(), 1);
    assert_eq!(cat.rename_with_ids(dark).unwrap(), 1);
    assert!(
        root.join("Movies/Dune (2021) [tmdbid-438631]/Dune (2021).mkv")
            .exists()
    );
    assert!(
        root.join("TV/Dark [tmdbid-70523]/Season 1/Dark S01E01.mkv")
            .exists()
    );
    let movies = cat.movies().unwrap();
    assert_eq!(item(&movies, "Dune"), dune, "{movies:?}");
    assert_eq!(item(&cat.shows().unwrap(), "Dark"), dark);
    // Once named, nothing more to rename.
    assert!(cat.knowledge_plan(dune).unwrap().rename.is_empty());
    assert_eq!(cat.rename_with_ids(dune).unwrap(), 0);

    // A fresh library over the same files learns the ids from them.
    let db2 = dir.path().join("db2/library.db");
    let fresh = Catalog::open(&db2).unwrap();
    fresh
        .add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    fresh.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let movies = fresh.movies().unwrap();
    let shows = fresh.shows().unwrap();
    assert!(ids_of(&db2, work(item(&movies, "Dune"))).contains(&(
        "tmdb".into(),
        "movie".into(),
        "438631".into()
    )));
    assert!(ids_of(&db2, work(item(&movies, "Heat"))).contains(&(
        "imdb".into(),
        String::new(),
        "tt0113277".into()
    )));
    assert!(ids_of(&db2, work(item(&shows, "Dark"))).contains(&(
        "tmdb".into(),
        "tv".into(),
        "70523".into()
    )));
}
