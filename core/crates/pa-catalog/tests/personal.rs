// SPDX-License-Identifier: MPL-2.0
//! UI-025 end to end on real folders: personal media is never recognized
//! and never reaches a provider, a locked personal library and its videos
//! are unreachable through every call of the catalog and never show up in
//! the shared rows, and an item moves into a personal library and back.
//! UI-033: playback positions of a personal library are read and written
//! only while it is unlocked, and never reach the shared "Continue
//! watching", the sync outbox or another profile.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{
    Catalog, ContentType, Error, ItemKind, ItemRef, LibraryFilter, LibraryQuery, LibrarySort, Lock,
    LockKind, MetadataOptions, MetadataProviders, PersonalFilter, PersonalSort, ProfileSettings,
};
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

const DUNE: &str = "Movies/Dune (2021)/Dune.2021.2160p.mkv";
const ARRIVAL: &str = "Movies/Arrival (2016)/Arrival.2016.1080p.mkv";
const DARK1: &str = "Shows/Dark/Season 1/Dark.S01E01.1080p.mkv";
/// Named like a movie on purpose: only the content type keeps it from
/// being recognized.
const HEAT: &str = "Heat.1995.1080p.mkv";
const BIRTHDAY: &str = "2024/Birthday/clip_0042.mp4";
/// A folder of the main source given to a personal library.
const BEACH: &str = "Home videos/Beach (2019)/Beach.2019.mkv";

/// Records every provider request and answers "nothing found".
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Recorder {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let q = r
            .query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        self.calls.lock().unwrap().push(format!("{} {q}", r.path));
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: r#"{"page":1,"results":[],"total_pages":0,"total_results":0}"#.into(),
        })
    }

    fn post_json(&self, r: &HttpRequest, body: &str) -> Result<HttpResponse, TransportError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{} {body}", r.path));
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: r#"{"data":{"Page":{"media":[]}}}"#.into(),
        })
    }
}

fn tmdb(rec: &Arc<Recorder>) -> MetadataProviders {
    let transport: Arc<dyn HttpTransport> = rec.clone();
    let mut config = TmdbConfig::default();
    config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            config,
            Keys::none().with_user_key(Some("test-key")),
            transport,
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        ))),
        anilist: None,
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
        extra: Default::default(),
    }
}

struct Lib {
    _dir: tempfile::TempDir,
    cat: Catalog,
    main: i64,
    home: i64,
    /// PIN 2468.
    family: i64,
}

impl Lib {
    fn scan(&self) {
        self.cat
            .scan(None, &CancelToken::new(), &mut |_| {})
            .unwrap();
    }

    fn movie_titles(&self) -> Vec<String> {
        let mut t: Vec<String> = self
            .cat
            .movies()
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect();
        t.sort();
        t
    }

    fn movie(&self, title: &str) -> ItemRef {
        self.cat
            .movies()
            .unwrap()
            .into_iter()
            .find(|m| m.title == title)
            .unwrap_or_else(|| panic!("no {title}"))
            .id
    }

    fn query_titles(&self, kind: ItemKind) -> Vec<String> {
        let q = LibraryQuery {
            kind,
            filter: LibraryFilter::default(),
            sort: LibrarySort::default(),
        };
        let mut t: Vec<String> = self
            .cat
            .query(&q, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.title)
            .collect();
        t.sort();
        t
    }

    fn personal_titles(&self, id: i64) -> Vec<String> {
        let mut t: Vec<String> = self
            .cat
            .personal_items(id, &PersonalFilter::default(), PersonalSort::Title)
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect();
        t.sort();
        t
    }
}

/// A main source with movies, a show and a home-video folder, and a
/// `personal` source of home videos in the PIN-locked library "Семья".
fn setup() -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    let home = dir.path().join("home");
    for (i, rel) in [DUNE, ARRIVAL, DARK1, BEACH].iter().enumerate() {
        media(&root, rel, u8::try_from(i).unwrap() + 1);
    }
    media(&home, HEAT, 20);
    media(&home, BIRTHDAY, 21);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let main = cat
        .add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap()
        .id;
    let family = cat
        .create_personal_library("Семья", &Lock::Pin("2468"))
        .unwrap()
        .id;
    let home = cat
        .add_source(None, "local", "Home", home.to_str().unwrap(), None)
        .unwrap()
        .id;
    cat.set_source_content(home, ContentType::Personal, Some(family))
        .unwrap();
    let lib = Lib {
        _dir: dir,
        cat,
        main,
        home,
        family,
    };
    lib.scan();
    lib
}

#[test]
fn personal_files_are_not_recognized_and_never_reach_tmdb() {
    let lib = setup();
    assert_eq!(
        lib.cat.source_content(lib.home).unwrap(),
        ContentType::Personal
    );
    assert_eq!(
        lib.cat.source_content(lib.main).unwrap(),
        ContentType::Video
    );
    // Only the main source makes works: no "Heat".
    assert_eq!(
        lib.movie_titles(),
        ["Arrival", "Beach", "Dune"],
        "the home source makes no items"
    );
    let conn = pa_library::open(lib.cat.path()).unwrap();
    let recognized: i64 = conn
        .query_row(
            "SELECT count(*) FROM files f WHERE f.source_id = ?1
               AND (EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.id)
                    OR EXISTS (SELECT 1 FROM parse_results p WHERE p.file_id = f.id))",
            [lib.home],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recognized, 0, "no parse, no match for personal files");

    // A folder of the main source given to a library before its first
    // scan is not recognized either.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("m");
    media(&root, BEACH, 5);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let src = cat
        .add_source(None, "local", "M", root.to_str().unwrap(), None)
        .unwrap()
        .id;
    let open = cat.create_personal_library("Дом", &Lock::None).unwrap().id;
    cat.add_personal_folder(open, src, "Home videos").unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    assert!(cat.movies().unwrap().is_empty());
    assert_eq!(
        cat.personal_items(open, &PersonalFilter::default(), PersonalSort::Added)
            .unwrap()
            .len(),
        1
    );

    // The metadata run looks up the main works only.
    let rec = Arc::new(Recorder::default());
    let report = lib
        .cat
        .fetch_metadata(
            &tmdb(&rec),
            &MetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    let calls = rec.calls.lock().unwrap().clone();
    assert!(
        calls.iter().any(|c| c.contains("Dune")),
        "the run did ask TMDB: {calls:?} {report:?}"
    );
    for word in ["Heat", "clip", "Birthday", "1995"] {
        assert!(
            calls.iter().all(|c| !c.contains(word)),
            "{word} went to TMDB: {calls:?}"
        );
    }

    // The videos are in the library, by file name.
    lib.cat.unlock_personal_library(lib.family, "2468").unwrap();
    assert_eq!(
        lib.personal_titles(lib.family),
        ["Heat.1995.1080p", "clip_0042"]
    );
}

#[test]
fn a_locked_library_is_unreachable_and_never_in_shared_rows() {
    let lib = setup();
    let lib_id = lib.family;
    fn denied<T>(r: Result<T, Error>) -> bool {
        matches!(r, Err(Error::Denied(_)))
    }

    // Listed by name only.
    let infos = lib.cat.personal_libraries().unwrap();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].name, "Семья");
    assert_eq!(infos[0].lock, LockKind::Pin);
    // Created unlocked in this process; lock it as a restart would.
    assert!(!infos[0].locked);
    lib.cat.lock_personal_libraries();
    let info = lib.cat.personal_library(lib_id).unwrap();
    assert!(info.locked);
    assert_eq!(info.item_count, None);
    assert!(info.folders.is_empty());

    // Every call about its contents is refused.
    assert!(denied(lib.cat.personal_items(
        lib_id,
        &PersonalFilter::default(),
        PersonalSort::Added
    )));
    assert!(denied(lib.cat.personal_play_parts(lib_id, 1)));
    assert!(denied(lib.cat.personal_labels(lib_id).tags()));
    assert!(denied(lib.cat.personal_labels(lib_id).categories()));
    assert!(denied(lib.cat.personal_labels(lib_id).create_category("x")));
    assert!(denied(lib.cat.rename_personal_library(lib_id, "x")));
    assert!(denied(lib.cat.set_personal_lock(lib_id, &Lock::None)));
    assert!(denied(lib.cat.delete_personal_library(lib_id)));
    assert!(denied(
        lib.cat.add_personal_folder(lib_id, lib.main, "Movies")
    ));
    assert!(denied(lib.cat.move_to_personal(lib.movie("Dune"), lib_id)));
    assert!(denied(lib.cat.set_source_content(
        lib.home,
        ContentType::Video,
        None
    )));
    // The stream proxy will not open its files.
    assert!(!lib.cat.may_open(lib.home, HEAT).unwrap());
    assert!(!lib.cat.may_open(lib.home, BIRTHDAY).unwrap());
    assert!(lib.cat.may_open(lib.main, DUNE).unwrap());

    // Nothing of it in the main library, whatever the call.
    for kind in [ItemKind::Movie, ItemKind::Show] {
        assert!(
            lib.query_titles(kind).iter().all(|t| !t.contains("Heat")),
            "{kind:?}"
        );
    }
    assert!(lib.cat.search("Heat", 50).unwrap().is_empty());
    assert!(lib.cat.search("clip", 50).unwrap().is_empty());
    assert!(
        lib.cat
            .recently_added(ItemKind::Movie, 50)
            .unwrap()
            .iter()
            .all(|i| i.source_id != lib.home)
    );
    assert!(lib.cat.play_parts(lib.home, HEAT).unwrap().is_empty());
    let browse = lib
        .cat
        .browse(lib.home, "", LibrarySort::default(), true)
        .unwrap();
    assert!(browse.is_empty(), "{browse:?}");

    // A wrong PIN does not open it, and counts.
    let v = lib.cat.unlock_personal_library(lib_id, "1111").unwrap();
    assert!(!v.ok);
    assert_eq!(v.attempts.failures, 1);
    assert!(denied(lib.cat.personal_items(
        lib_id,
        &PersonalFilter::default(),
        PersonalSort::Added
    )));
    // The right one does.
    assert!(lib.cat.unlock_personal_library(lib_id, "2468").unwrap().ok);
    let items = lib
        .cat
        .personal_items(lib_id, &PersonalFilter::default(), PersonalSort::Added)
        .unwrap();
    assert_eq!(items.len(), 2);
    assert!(lib.cat.may_open(lib.home, HEAT).unwrap());
    let heat = items.iter().find(|i| i.rel_path == HEAT).unwrap();
    let parts = lib.cat.personal_play_parts(lib_id, heat.file_id).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].rel_path, HEAT);
    // Still not in the main library while unlocked.
    assert!(lib.cat.search("Heat", 50).unwrap().is_empty());
    assert_eq!(lib.movie_titles(), ["Arrival", "Beach", "Dune"]);
    assert_eq!(
        lib.cat.personal_library(lib_id).unwrap().item_count,
        Some(2)
    );

    // Locked again: closed again.
    lib.cat.lock_personal_library(lib_id);
    assert!(!lib.cat.may_open(lib.home, HEAT).unwrap());
    assert!(denied(lib.cat.personal_play_parts(lib_id, heat.file_id)));

    // Another profile does not even see it.
    lib.cat.unlock_personal_library(lib_id, "2468").unwrap();
    let guest = lib
        .cat
        .create_profile(
            &ProfileSettings {
                name: "Гость".into(),
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
    lib.cat.switch_profile(guest.id, None, None).unwrap();
    assert!(lib.cat.personal_libraries().unwrap().is_empty());
    assert!(matches!(
        lib.cat
            .personal_items(lib_id, &PersonalFilter::default(), PersonalSort::Added),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        lib.cat.unlock_personal_library(lib_id, "2468"),
        Err(Error::NotFound(_))
    ));
    assert!(!lib.cat.may_open(lib.home, HEAT).unwrap());
    // Switching back finds it locked.
    lib.cat
        .switch_profile(pa_library::profiles::DEFAULT_PROFILE, None, None)
        .unwrap();
    assert!(lib.cat.personal_library(lib_id).unwrap().locked);
}

#[test]
fn a_password_lock() {
    let lib = setup();
    let id = lib
        .cat
        .create_personal_library("Архив", &Lock::Password("correct horse"))
        .unwrap()
        .id;
    assert!(matches!(
        lib.cat.create_personal_library("x", &Lock::Password("123")),
        Err(Error::Invalid(_))
    ));
    lib.cat.lock_personal_libraries();
    assert!(
        !lib.cat
            .unlock_personal_library(id, "Correct horse")
            .unwrap()
            .ok
    );
    assert!(
        lib.cat
            .unlock_personal_library(id, "correct horse")
            .unwrap()
            .ok
    );
    lib.cat.set_personal_lock(id, &Lock::Pin("1357")).unwrap();
    lib.cat.lock_personal_libraries();
    assert_eq!(lib.cat.personal_library(id).unwrap().lock, LockKind::Pin);
    assert!(
        !lib.cat
            .unlock_personal_library(id, "correct horse")
            .unwrap()
            .ok
    );
    assert!(lib.cat.unlock_personal_library(id, "1357").unwrap().ok);
    lib.cat.set_personal_lock(id, &Lock::None).unwrap();
    lib.cat.lock_personal_libraries();
    assert!(!lib.cat.personal_library(id).unwrap().locked);
    lib.cat.delete_personal_library(id).unwrap();
    assert!(matches!(
        lib.cat.personal_library(id),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn an_item_moves_into_a_personal_library_and_back() {
    let lib = setup();
    let id = lib.family;
    let dune = lib.movie("Dune");
    let before = lib.cat.item(dune).unwrap().unwrap();
    lib.cat
        .save_progress(dune, 600_000, 9_000_000, 1, false)
        .unwrap();
    assert!(
        lib.cat
            .continue_watching(10)
            .unwrap()
            .iter()
            .any(|c| c.item.id == dune)
    );

    assert_eq!(lib.cat.move_to_personal(dune, id).unwrap(), 1);
    // Gone from every shared row and lookup.
    assert_eq!(lib.movie_titles(), ["Arrival", "Beach"]);
    assert!(
        !lib.query_titles(ItemKind::Movie)
            .contains(&"Dune".to_owned())
    );
    assert!(lib.cat.search("Dune", 50).unwrap().is_empty());
    assert!(lib.cat.item(dune).unwrap().is_none());
    assert!(
        lib.cat
            .continue_watching(10)
            .unwrap()
            .iter()
            .all(|c| c.item.id != dune)
    );
    assert!(
        lib.cat
            .recently_added(ItemKind::Movie, 10)
            .unwrap()
            .iter()
            .all(|i| i.id != dune)
    );
    assert!(matches!(lib.cat.progress(dune), Err(Error::NotFound(_))));
    // Its metadata is not refreshed any more.
    let rec = Arc::new(Recorder::default());
    lib.cat
        .fetch_metadata(
            &tmdb(&rec),
            &MetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert!(
        rec.calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| !c.contains("Dune")),
        "{:?}",
        rec.calls.lock().unwrap()
    );

    // In the library, under its title, with labels of the library.
    let items = lib
        .cat
        .personal_items(id, &PersonalFilter::default(), PersonalSort::Added)
        .unwrap();
    let moved = items.iter().find(|i| i.moved).expect("the moved movie");
    assert_eq!(moved.title, "Dune");
    assert_eq!(moved.item, Some(dune));
    let labels = lib.cat.personal_labels(id);
    labels.set_rating(dune, Some(5)).unwrap();
    labels.add_tag(dune, "семья").unwrap();
    let cat = labels.create_category("Лето").unwrap();
    let clip = items.iter().find(|i| i.rel_path == BIRTHDAY).unwrap();
    let clip_key = clip.key.clone().expect("fingerprinted");
    labels
        .set_in_category(cat.id, clip_key.clone(), true)
        .unwrap();
    labels.set_rating(clip_key, Some(3)).unwrap();
    // Filters and sorts.
    let by_rating = lib
        .cat
        .personal_items(id, &PersonalFilter::default(), PersonalSort::Rating)
        .unwrap();
    assert_eq!(
        by_rating
            .iter()
            .map(|i| i.title.as_str())
            .collect::<Vec<_>>(),
        ["Dune", "clip_0042", "Heat.1995.1080p"]
    );
    let only = |f: PersonalFilter| -> Vec<String> {
        lib.cat
            .personal_items(id, &f, PersonalSort::Title)
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect()
    };
    assert_eq!(
        only(PersonalFilter {
            tags: vec!["СЕМЬЯ".into()],
            ..Default::default()
        }),
        ["Dune"]
    );
    assert_eq!(
        only(PersonalFilter {
            categories: vec![cat.id],
            ..Default::default()
        }),
        ["clip_0042"]
    );
    assert_eq!(
        only(PersonalFilter {
            ratings: vec![0],
            ..Default::default()
        }),
        ["Heat.1995.1080p"]
    );
    assert_eq!(
        only(PersonalFilter {
            text: Some("heat".into()),
            ..Default::default()
        }),
        ["Heat.1995.1080p"]
    );
    // The library's names stay in the library.
    assert!(lib.cat.labels().tags().unwrap().is_empty());
    assert!(lib.cat.labels().categories().unwrap().is_empty());
    assert_eq!(labels.tags().unwrap()[0].name, "семья");
    assert_eq!(labels.tags().unwrap()[0].item_count, 1);

    // A file of a personal folder cannot "move back".
    assert!(matches!(
        lib.cat.move_to_main(id, clip.file_id),
        Err(Error::Invalid(_))
    ));
    // Back to the main library, as it was.
    assert_eq!(lib.cat.move_to_main(id, moved.file_id).unwrap(), 1);
    assert_eq!(lib.movie_titles(), ["Arrival", "Beach", "Dune"]);
    let after = lib.cat.item(dune).unwrap().expect("Dune is back");
    assert_eq!(after.id, before.id);
    assert_eq!(after.title, before.title);
    assert_eq!(
        lib.cat.progress(dune).unwrap().unwrap().position_ms,
        600_000
    );
    assert!(
        lib.cat
            .continue_watching(10)
            .unwrap()
            .iter()
            .any(|c| c.item.id == dune)
    );
    assert_eq!(lib.personal_titles(id), ["Heat.1995.1080p", "clip_0042"]);

    // A whole show moves with its episodes.
    let dark = lib.cat.shows().unwrap()[0].id;
    assert_eq!(lib.cat.move_to_personal(dark, id).unwrap(), 1);
    assert!(lib.cat.shows().unwrap().is_empty());
    assert!(
        lib.personal_titles(id)
            .iter()
            .any(|t| t.starts_with("Dark · S01E01"))
    );
    // Moving into a locked library is refused.
    lib.cat.lock_personal_libraries();
    assert!(matches!(
        lib.cat.move_to_personal(lib.movie("Arrival"), id),
        Err(Error::Denied(_))
    ));
}

#[test]
fn a_folder_of_a_main_source_becomes_personal_and_back() {
    let lib = setup();
    let open = lib
        .cat
        .create_personal_library("Дом", &Lock::None)
        .unwrap()
        .id;
    lib.cat
        .add_personal_folder(open, lib.main, "Home videos")
        .unwrap();
    // Recognized before: it keeps its match but leaves the main library.
    assert_eq!(lib.movie_titles(), ["Arrival", "Dune"]);
    assert_eq!(lib.personal_titles(open), ["Beach"]);
    let info = lib.cat.personal_library(open).unwrap();
    assert_eq!(info.folders.len(), 1);
    assert_eq!(info.folders[0].path, "/Home videos");
    // A library without a lock stays open after "lock all".
    lib.cat.lock_personal_libraries();
    assert_eq!(lib.personal_titles(open), ["Beach"]);
    // In one library at a time.
    lib.cat.unlock_personal_library(lib.family, "2468").unwrap();
    assert!(matches!(
        lib.cat
            .add_personal_folder(lib.family, lib.main, "/Home videos/"),
        Err(Error::Invalid(_))
    ));
    assert!(
        lib.cat
            .remove_personal_folder(open, lib.main, "Home videos")
            .unwrap()
    );
    assert_eq!(lib.movie_titles(), ["Arrival", "Beach", "Dune"]);
    assert!(lib.personal_titles(open).is_empty());
}

#[test]
fn positions_are_kept_only_inside_an_unlocked_library() {
    let lib = setup();
    let id = lib.family;
    fn denied<T>(r: Result<T, Error>) -> bool {
        matches!(r, Err(Error::Denied(_)))
    }
    lib.cat.enable_sync("phone").unwrap();
    let ops_before = lib.cat.pending_ops(1_000).unwrap().len();
    let items = lib
        .cat
        .personal_items(id, &PersonalFilter::default(), PersonalSort::Added)
        .unwrap();
    let heat = items.iter().find(|i| i.rel_path == HEAT).unwrap().clone();
    let clip = items
        .iter()
        .find(|i| i.rel_path == BIRTHDAY)
        .unwrap()
        .clone();
    assert!(heat.progress.is_none());
    assert!(lib.cat.personal_continue(id, 10).unwrap().is_empty());

    // Saved and read back in the library, by its `file:` key.
    assert!(
        lib.cat
            .save_personal_progress(id, heat.file_id, 600_000, 6_000_000, 1_000, false)
            .unwrap()
    );
    assert!(
        lib.cat
            .save_personal_progress(id, clip.file_id, 30_000, 120_000, 2_000, false)
            .unwrap()
    );
    let row = lib.cat.personal_continue(id, 10).unwrap();
    assert_eq!(
        row.iter().map(|i| i.file_id).collect::<Vec<_>>(),
        [clip.file_id, heat.file_id],
        "latest first"
    );
    let p = row[1].progress.unwrap();
    assert_eq!((p.position_ms, p.duration_ms), (600_000, Some(6_000_000)));
    assert!(!p.watched);
    let listed = lib
        .cat
        .personal_items(id, &PersonalFilter::default(), PersonalSort::Added)
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|i| i.file_id == heat.file_id)
            .unwrap()
            .progress
            .map(|p| p.position_ms),
        Some(600_000)
    );
    let conn = pa_library::open(lib.cat.path()).unwrap();
    let key: String = conn
        .query_row(
            "SELECT media_key FROM personal_progress WHERE library_id = ?1 AND position_ms = 600000",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(key, heat.key.clone().unwrap().to_string());
    assert!(key.starts_with("file:"), "{key}");

    // Nowhere outside the library: not the shared "Continue watching"
    // (Watch Next and Top Shelf read it), not the progress of the main
    // library, not one op for other devices.
    assert!(lib.cat.continue_watching(50).unwrap().is_empty());
    let shared: i64 = conn
        .query_row("SELECT count(*) FROM progress", [], |r| r.get(0))
        .unwrap();
    assert_eq!(shared, 0);
    let ops = lib.cat.pending_ops(1_000).unwrap();
    assert_eq!(ops.len(), ops_before, "{ops:?}");
    let fingerprint = heat.key.clone().unwrap().to_string();
    let fingerprint = fingerprint.trim_start_matches("file:");
    for op in &ops {
        assert!(!pa_sync::outbox::encode(op).unwrap().contains(fingerprint));
    }

    // Past 95 % it is watched and leaves the row; clearing forgets it.
    lib.cat
        .save_personal_progress(id, heat.file_id, 5_900_000, 6_000_000, 3_000, false)
        .unwrap();
    let row = lib.cat.personal_continue(id, 10).unwrap();
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].file_id, clip.file_id);
    assert!(lib.cat.clear_personal_progress(id, clip.file_id).unwrap());
    assert!(lib.cat.personal_continue(id, 10).unwrap().is_empty());
    lib.cat
        .save_personal_progress(id, clip.file_id, 40_000, 120_000, 4_000, false)
        .unwrap();

    // A file outside the library has no position here.
    let dune_file: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE source_id = ?1 AND path LIKE '%Dune%'",
            [lib.main],
            |r| r.get(0),
        )
        .unwrap();
    assert!(matches!(
        lib.cat
            .save_personal_progress(id, dune_file, 1, 2, 5_000, false),
        Err(Error::NotFound(_))
    ));

    // Locked: neither saved nor read.
    lib.cat.lock_personal_libraries();
    assert!(denied(lib.cat.save_personal_progress(
        id,
        clip.file_id,
        50_000,
        120_000,
        6_000,
        false
    )));
    assert!(denied(lib.cat.personal_continue(id, 10)));
    assert!(denied(lib.cat.clear_personal_progress(id, clip.file_id)));
    assert!(lib.cat.continue_watching(50).unwrap().is_empty());
    // Unlocked again: as it was.
    assert!(lib.cat.unlock_personal_library(id, "2468").unwrap().ok);
    let row = lib.cat.personal_continue(id, 10).unwrap();
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].progress.unwrap().position_ms, 40_000);

    // A moved movie keeps two positions apart: the main one stays as it
    // was, the one watched in the library stays in the library.
    let dune = lib.movie("Dune");
    lib.cat
        .save_progress(dune, 600_000, 9_000_000, 7_000, false)
        .unwrap();
    let ops_main = lib.cat.pending_ops(1_000).unwrap().len();
    lib.cat.move_to_personal(dune, id).unwrap();
    assert!(lib.cat.continue_watching(50).unwrap().is_empty());
    let moved = lib
        .cat
        .personal_items(id, &PersonalFilter::default(), PersonalSort::Added)
        .unwrap()
        .into_iter()
        .find(|i| i.moved)
        .unwrap();
    lib.cat
        .save_personal_progress(id, moved.file_id, 3_000_000, 9_000_000, 8_000, false)
        .unwrap();
    assert_eq!(lib.cat.pending_ops(1_000).unwrap().len(), ops_main);
    assert!(lib.cat.continue_watching(50).unwrap().is_empty());
    lib.cat.move_to_main(id, moved.file_id).unwrap();
    assert_eq!(
        lib.cat.progress(dune).unwrap().unwrap().position_ms,
        600_000
    );

    // Another profile has none of it.
    let guest = lib
        .cat
        .create_profile(
            &ProfileSettings {
                name: "Гость".into(),
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
    lib.cat.switch_profile(guest.id, None, None).unwrap();
    assert!(matches!(
        lib.cat.personal_continue(id, 10),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        lib.cat
            .save_personal_progress(id, clip.file_id, 1, 2, 9_000, false),
        Err(Error::NotFound(_))
    ));
    assert!(lib.cat.continue_watching(50).unwrap().is_empty());
    lib.cat
        .switch_profile(pa_library::profiles::DEFAULT_PROFILE, None, None)
        .unwrap();

    // Deleting the library deletes its positions.
    lib.cat.unlock_personal_library(id, "2468").unwrap();
    lib.cat.delete_personal_library(id).unwrap();
    let left: i64 = conn
        .query_row("SELECT count(*) FROM personal_progress", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}
