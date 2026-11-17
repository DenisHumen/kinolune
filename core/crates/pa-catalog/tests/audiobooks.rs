// SPDX-License-Identifier: MPL-2.0
//! MUS-018 on a real folder: the classifier marks audiobook tracks, and
//! `Catalog::audiobooks` makes books of them — a folder of parts and an
//! m4b with chapters both become books with chapters, author and narrator.
//! Where the listener stopped is saved per book under its key and, with
//! sync on, travels to the other device as a progress op.

use std::path::Path;

use pa_catalog::{Audiobook, Catalog};
use pa_library::rusqlite::{Connection, params};
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

const PICNIC: &str = "Аудиокниги/Стругацкие - Пикник на обочине (читает Кирилл Радциг)";
const SOLARIS: &str = "Аудиокниги/Лем/Солярис.m4b";
const SONG: &str = "Music/Band/2019 - Album/01 - Song.mp3";

struct Lib {
    dir: tempfile::TempDir,
    cat: Catalog,
}

impl Lib {
    fn db(&self) -> Connection {
        Connection::open(self.dir.path().join("library.db")).unwrap()
    }

    fn scan(&self) {
        self.cat
            .scan(None, &CancelToken::new(), &mut |_| {})
            .unwrap();
    }
}

/// What the probe writes for an m4b with three chapters (`pa_probe::store`).
const SOLARIS_TAGS: &str = r#"{"tags":{},"chapters":3,
    "chapter_list":[[0,600000,"Пришелец"],[600000,1500000,"Соляристы"],[1500000,2400000,null]],
    "book":{"narrator":"Владимир Самойлов","series":"Фантастика Лема","series_part":"2"}}"#;

fn setup() -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    // Parts in a disc folder and out of string order: 2 before 10.
    media(&root, &format!("{PICNIC}/CD1/01.mp3"), 1);
    media(&root, &format!("{PICNIC}/CD1/02.mp3"), 2);
    media(&root, &format!("{PICNIC}/CD2/10.mp3"), 3);
    media(&root, SOLARIS, 4);
    media(&root, SONG, 5);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    let lib = Lib { dir, cat };
    lib.scan();
    // Lengths and chapters as the container probe gives them (the fake
    // files have none).
    let db = lib.db();
    for (path, ms) in [
        ("%/CD1/01.mp3", 1_800_000),
        ("%/CD1/02.mp3", 1_200_000),
        ("%/CD2/10.mp3", 900_000),
        ("%Солярис.m4b", 2_400_000),
    ] {
        db.execute(
            "UPDATE music_tracks SET duration_ms = ?2
             WHERE file_id IN (SELECT id FROM files WHERE path LIKE ?1)",
            params![path, ms],
        )
        .unwrap();
    }
    db.execute(
        "UPDATE files SET embedded_tags_json = ?1 WHERE path LIKE '%Солярис.m4b'",
        [SOLARIS_TAGS],
    )
    .unwrap();
    lib
}

fn find<'a>(books: &'a [Audiobook], title: &str) -> &'a Audiobook {
    books
        .iter()
        .find(|b| b.title == title)
        .unwrap_or_else(|| panic!("no {title:?} in {books:#?}"))
}

#[test]
fn a_folder_of_parts_and_an_m4b_with_chapters_become_books() {
    let lib = setup();
    let books = lib.cat.audiobooks().unwrap();
    assert_eq!(books.len(), 2, "{books:#?}");

    // A folder of parts: one chapter per part, in play order.
    let picnic = find(&books, "Пикник на обочине");
    assert_eq!(picnic.author.as_deref(), Some("Стругацкие"));
    assert_eq!(picnic.narrator.as_deref(), Some("Кирилл Радциг"));
    assert_eq!(picnic.rel_path, PICNIC);
    let parts: Vec<&str> = picnic
        .parts
        .iter()
        .map(|p| p.rel_path.rsplit('/').next().unwrap())
        .collect();
    assert_eq!(parts, ["01.mp3", "02.mp3", "10.mp3"]);
    assert_eq!(picnic.chapters.len(), 3);
    let starts: Vec<i64> = picnic.chapters.iter().map(|c| c.book_start_ms).collect();
    assert_eq!(starts, [0, 1_800_000, 3_000_000]);
    assert_eq!(picnic.chapters[2].part, 2);
    assert_eq!(picnic.chapters[2].start_ms, 0);
    assert_eq!(picnic.duration_ms, 3_900_000);
    assert!(picnic.key.as_deref().is_some_and(|k| k.starts_with("file:")));

    // An m4b: one part, its own chapters, tags for narrator and series.
    let solaris = find(&books, "Солярис");
    assert_eq!(solaris.parts.len(), 1);
    assert_eq!(solaris.rel_path, SOLARIS);
    assert_eq!(solaris.narrator.as_deref(), Some("Владимир Самойлов"));
    assert_eq!(solaris.series.as_deref(), Some("Фантастика Лема"));
    assert_eq!(solaris.series_number, Some(2.0));
    let chapters: Vec<(Option<&str>, i64, i64)> = solaris
        .chapters
        .iter()
        .map(|c| (c.title.as_deref(), c.book_start_ms, c.duration_ms))
        .collect();
    assert_eq!(
        chapters,
        [
            (Some("Пришелец"), 0, 600_000),
            (Some("Соляристы"), 600_000, 900_000),
            (None, 1_500_000, 900_000),
        ]
    );
    assert_eq!(solaris.duration_ms, 2_400_000);

    // The song stays music.
    assert!(books.iter().all(|b| b.parts.iter().all(|p| p.rel_path != SONG)));
}

#[test]
fn two_m4b_books_in_one_folder_stay_two_books() {
    let lib = setup();
    let root = lib.dir.path().join("media");
    media(&root, "Аудиокниги/Лем/Непобедимый.m4b", 6);
    lib.scan();
    let db = lib.db();
    db.execute(
        "UPDATE files SET embedded_tags_json = ?1 WHERE path LIKE '%.m4b'",
        [SOLARIS_TAGS],
    )
    .unwrap();
    let books = lib.cat.audiobooks().unwrap();
    find(&books, "Солярис");
    find(&books, "Непобедимый");
    assert_eq!(books.len(), 3, "{books:#?}");
}

#[test]
fn the_place_in_a_book_is_saved_and_resumed() {
    let lib = setup();
    let books = lib.cat.audiobooks().unwrap();
    let picnic = find(&books, "Пикник на обочине");
    assert_eq!(picnic.progress, None);
    let key = picnic.key.clone().unwrap();

    // In the second part: 1 800 000 + 300 000.
    lib.cat
        .save_book_progress(&key, 2_100_000, 3_900_000, 1_763_000_000_000, false)
        .unwrap();
    let p = lib.cat.book_progress(&key).unwrap().expect("saved");
    assert_eq!(p.position_ms, 2_100_000);
    assert_eq!(p.duration_ms, 3_900_000);
    assert!(!p.finished);
    let books = lib.cat.audiobooks().unwrap();
    assert_eq!(
        find(&books, "Пикник на обочине")
            .progress
            .as_ref()
            .map(|p| p.position_ms),
        Some(2_100_000)
    );
    // Each book has its own place.
    assert_eq!(find(&books, "Солярис").progress, None);

    // A later report wins; the end of the book marks it listened to.
    lib.cat
        .save_book_progress(&key, 3_850_000, 3_900_000, 1_763_000_100_000, false)
        .unwrap();
    assert!(lib.cat.book_progress(&key).unwrap().unwrap().finished);

    // Book places are not «Продолжить просмотр» of films and shows.
    assert!(lib.cat.continue_watching(10).unwrap().is_empty());
    // Only `file:` keys are book keys.
    assert!(
        lib.cat
            .save_book_progress("tmdb:movie:603", 1, 2, 3, false)
            .is_err()
    );
}

#[test]
fn the_place_in_a_book_syncs_to_the_other_device() {
    let a = setup();
    let b = setup();
    a.cat.enable_sync("dev-a").unwrap();
    b.cat.enable_sync("dev-b").unwrap();
    let key = find(&a.cat.audiobooks().unwrap(), "Солярис")
        .key
        .clone()
        .unwrap();
    // The same file on both devices: the same book key.
    assert_eq!(
        find(&b.cat.audiobooks().unwrap(), "Солярис").key.as_deref(),
        Some(key.as_str())
    );

    a.cat
        .save_book_progress(&key, 700_000, 2_400_000, 1_763_000_000_000, false)
        .unwrap();
    let fingerprint = key.trim_start_matches("file:");
    let ops: Vec<_> = a
        .cat
        .pending_ops(64)
        .unwrap()
        .into_iter()
        .filter(|op| format!("{op:?}").contains(fingerprint))
        .collect();
    assert!(!ops.is_empty());
    for op in &ops {
        b.cat.apply_remote(op).unwrap();
    }
    let p = b.cat.book_progress(&key).unwrap().expect("synced");
    assert_eq!(p.position_ms, 700_000);
    assert_eq!(
        find(&b.cat.audiobooks().unwrap(), "Солярис")
            .progress
            .as_ref()
            .map(|p| p.position_ms),
        Some(700_000)
    );
}
