// SPDX-License-Identifier: MPL-2.0
//! META-017 end to end: a fix teaches the folder a rule with release
//! templates. After it a new file in `Anime/Frieren/` gets the same series,
//! episode mapping and offset with no network request — the providers of
//! that run fail on any call. Synthetic provider responses (no real TMDB
//! data); sync between two libraries through `folder_rule.set` ops.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_metadata::cache::{MemoryCache, ProviderCache};
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::pa_sync::OpBody;
use pa_catalog::{Catalog, ItemRef, MetadataOptions, MetadataProviders};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;

const FOLDER: &str = "Anime/Frieren";

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

/// A fansub file of the folder: the running count `n` (TMDB season 1 has
/// four episodes, so 05 is S02E01).
fn frieren(n: u32) -> String {
    format!("{FOLDER}/[Grp] Sousou no Frieren - {n:02} (1080p) [AAAA{n:04}].mkv")
}

fn ok(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: body.to_owned(),
    }
}

/// Synthetic TMDB: "Frieren" (id 900017) in two seasons. With `search` its
/// title search finds it, else nothing (the manual queue).
struct Fake {
    search: bool,
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Fake {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(r.path.clone());
        let query = r
            .query
            .iter()
            .find(|(k, _)| k == "query")
            .map_or("", |(_, v)| v.as_str());
        Ok(match r.path.as_str() {
            "/search/tv" if self.search && query.starts_with("Sousou no Frieren") => {
                ok(r#"{"page":1,"results":[
                {"id":900017,"name":"Frieren","original_name":"Sousou no Frieren",
                 "first_air_date":"2023-09-29","original_language":"ja"}]}"#)
            }
            "/search/tv" | "/search/movie" => ok(r#"{"page":1,"results":[]}"#),
            "/tv/900017" => ok(
                r#"{"id":900017,"name":"Frieren","original_name":"Sousou no Frieren",
                "original_language":"ja","first_air_date":"2023-09-29","status":"Returning Series",
                "seasons":[{"season_number":1,"episode_count":4},{"season_number":2,"episode_count":6}]}"#,
            ),
            "/tv/900017/translations" => ok(r#"{"id":900017,"translations":[]}"#),
            "/tv/900017/credits" => ok(r#"{"id":900017,"cast":[],"crew":[]}"#),
            "/tv/900017/content_ratings" => ok(r#"{"id":900017,"results":[]}"#),
            "/tv/900017/season/2" => ok(r#"{"season_number":2,"episodes":[
                {"id":21,"season_number":2,"episode_number":1,"name":"Two-One"},
                {"id":22,"season_number":2,"episode_number":2,"name":"Two-Two"},
                {"id":23,"season_number":2,"episode_number":3,"name":"Two-Three"}]}"#),
            other => HttpResponse {
                status: 404,
                headers: vec![],
                body: format!(r#"{{"status_message":"no fixture for {other}"}}"#),
            },
        })
    }
}

/// Fails on any call, and remembers it.
#[derive(Default)]
struct Offline {
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Offline {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(r.url());
        Err(TransportError("offline".into()))
    }

    fn post_json(&self, r: &HttpRequest, _body: &str) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(r.url());
        Err(TransportError("offline".into()))
    }
}

fn providers(
    transport: Arc<dyn HttpTransport>,
    cache: Arc<dyn ProviderCache>,
) -> MetadataProviders {
    let mut config = TmdbConfig::default();
    config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            config,
            Keys::none().with_user_key(Some("test-key")),
            transport,
            cache,
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        ))),
        anilist: None,
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
        extra: Default::default(),
    }
}

fn run(cat: &Catalog, p: &MetadataProviders) {
    cat.fetch_metadata(
        p,
        &MetadataOptions::default(),
        &CancelToken::new(),
        &mut |_, _| {},
    )
    .unwrap();
}

fn scan(cat: &Catalog) {
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
}

struct Lib {
    dir: tempfile::TempDir,
    cat: Catalog,
}

impl Lib {
    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("media")
    }

    fn db(&self) -> Connection {
        Connection::open(self.cat.path()).unwrap()
    }

    /// `(work, season, episode, absolute, title, state, decided_by)` of
    /// the file named `- n`.
    fn file(&self, n: u32) -> FileRow {
        let path = format!("/{}", frieren(n));
        self.db()
            .query_row(
                "SELECT m.work_id, e.season_number, e.episode_number, e.absolute_number, e.title,
                        m.state, m.decided_by
                 FROM files f JOIN matches m ON m.file_id = f.id AND m.state <> 'rejected'
                 JOIN match_episodes me ON me.match_id = m.id
                 JOIN episodes e ON e.id = me.episode_id
                 WHERE f.path = ?1",
                [&path],
                |r| {
                    Ok(FileRow {
                        work: r.get(0)?,
                        season: r.get(1)?,
                        episode: r.get(2)?,
                        absolute: r.get(3)?,
                        title: r.get(4)?,
                        state: r.get(5)?,
                        decided_by: r.get(6)?,
                    })
                },
            )
            .unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn work_of_path(&self, rel: &str) -> i64 {
        self.db()
            .query_row(
                "SELECT m.work_id FROM files f JOIN matches m ON m.file_id = f.id
                 WHERE f.path = ?1 AND m.state <> 'rejected'",
                [format!("/{rel}")],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FileRow {
    work: i64,
    season: Option<i64>,
    episode: Option<i64>,
    absolute: Option<i64>,
    title: Option<String>,
    state: String,
    decided_by: String,
}

const DANDADAN: &str = "Anime/Frieren/[Grp] Dandadan - 07 (1080p) [BBBB0007].mkv";

fn library(files: &[String], scan_now: bool) -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    for (i, f) in files.iter().enumerate() {
        media(&root, f, u8::try_from(i).unwrap() + 1);
    }
    std::fs::create_dir_all(&root).unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    if scan_now {
        scan(&cat);
    }
    Lib { dir, cat }
}

/// The library, its metadata run and the user's fix: «Верно» on the
/// series TMDB found, whose running count the run laid out in TMDB
/// seasons.
fn fixed_by_confirm() -> (Lib, Arc<dyn ProviderCache>) {
    let lib = library(&[frieren(5), frieren(6), DANDADAN.to_owned()], true);
    let cache: Arc<dyn ProviderCache> = Arc::new(MemoryCache::default());
    let fake = Arc::new(Fake {
        search: true,
        calls: Mutex::new(Vec::new()),
    });
    run(&lib.cat, &providers(fake, cache.clone()));
    let before = lib.file(5);
    assert_eq!(
        (before.season, before.episode, before.absolute),
        (Some(2), Some(1), Some(5)),
        "the metadata run renumbered: {before:?}"
    );
    assert_ne!(lib.work_of_path(DANDADAN), before.work);
    lib.cat.review_confirm(before.work).unwrap();
    (lib, cache)
}

#[test]
fn a_new_file_follows_the_fix_without_the_network() {
    let (lib, cache) = fixed_by_confirm();
    let work = lib.file(5).work;
    let rules = lib.cat.folder_rules(ItemRef::Show(work)).unwrap();
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert_eq!(rules[0].folder, FOLDER);
    assert_eq!(rules[0].scheme.as_deref(), Some("tmdb"));
    assert_eq!((rules[0].season, rules[0].offset), (Some(2), -4));
    assert_eq!(
        rules[0].templates[0].pattern,
        "grp sousou no frieren {n} 1080p {crc}"
    );
    // The other show in the folder is not the rule's.
    let dandadan = lib.work_of_path(DANDADAN);
    assert_ne!(dandadan, work);

    // A new episode arrives; the providers are offline now.
    media(&lib.root(), &frieren(7), 7);
    scan(&lib.cat);
    let new = lib.file(7);
    assert_eq!(new.work, work);
    assert_eq!(
        (new.season, new.episode, new.absolute),
        (Some(2), Some(3), Some(7)),
        "{new:?}"
    );
    assert_eq!(
        (new.state.as_str(), new.decided_by.as_str()),
        ("user_set", "rule")
    );
    assert_eq!(lib.work_of_path(DANDADAN), dandadan);

    // The metadata run refreshes the work from the device's provider cache:
    // the new episode gets its title and nothing goes to the network.
    let offline = Arc::new(Offline::default());
    run(&lib.cat, &providers(offline.clone(), cache));
    assert_eq!(offline.calls.lock().unwrap().clone(), Vec::<String>::new());
    let new = lib.file(7);
    assert_eq!(new.title.as_deref(), Some("Two-Three"));
    assert_eq!((new.season, new.episode), (Some(2), Some(3)));
    // Older episodes keep theirs.
    assert_eq!(lib.file(5).title.as_deref(), Some("Two-One"));
}

#[test]
fn a_chosen_result_and_an_offset_spread_to_the_folder() {
    // TMDB's search does not find it: the manual queue.
    let lib = library(&[frieren(5), frieren(6)], true);
    let cache: Arc<dyn ProviderCache> = Arc::new(MemoryCache::default());
    let fake = Arc::new(Fake {
        search: false,
        calls: Mutex::new(Vec::new()),
    });
    let p = providers(fake, cache.clone());
    run(&lib.cat, &p);
    let work = lib.file(5).work;
    let target = lib.cat.review_choose(work, "tmdb", "tv", "900017").unwrap();
    // The user tells the numbering: 05 is the first episode of season 2.
    lib.cat
        .set_numbering(ItemRef::Show(target), Some(2), -4, false)
        .unwrap();
    for (n, e) in [(5, 1), (6, 2)] {
        let f = lib.file(n);
        assert_eq!(
            (f.work, f.season, f.episode),
            (target, Some(2), Some(e)),
            "{f:?}"
        );
    }
    run(&lib.cat, &p);
    assert_eq!(lib.file(6).title.as_deref(), Some("Two-Two"));

    media(&lib.root(), &frieren(7), 7);
    scan(&lib.cat);
    let offline = Arc::new(Offline::default());
    run(&lib.cat, &providers(offline.clone(), cache));
    assert!(offline.calls.lock().unwrap().is_empty());
    let new = lib.file(7);
    assert_eq!(
        (new.work, new.season, new.episode, new.title.as_deref()),
        (target, Some(2), Some(3), Some("Two-Three")),
        "{new:?}"
    );

    // Without a provider cache at all the mapping is the same: recognition
    // never asks; the failed refresh changes nothing.
    media(&lib.root(), &frieren(8), 8);
    scan(&lib.cat);
    let offline = Arc::new(Offline::default());
    run(
        &lib.cat,
        &providers(offline, Arc::new(MemoryCache::default())),
    );
    let f = lib.file(8);
    assert_eq!((f.work, f.season, f.episode), (target, Some(2), Some(4)));
    assert_eq!(lib.file(7).title.as_deref(), Some("Two-Three"));
}

#[test]
fn forgetting_the_rule_brings_back_plain_recognition() {
    let (lib, _cache) = fixed_by_confirm();
    let work = lib.file(5).work;
    assert_eq!(lib.cat.forget_folder_rules(ItemRef::Show(work)).unwrap(), 1);
    assert!(
        lib.cat
            .folder_rules(ItemRef::Show(work))
            .unwrap()
            .is_empty()
    );
    // Decisions made stay.
    assert_eq!(lib.file(5).episode, Some(1));
    media(&lib.root(), &frieren(7), 7);
    scan(&lib.cat);
    let new = lib.file(7);
    assert_eq!(
        new.decided_by, "rule",
        "the parser's rule, not a folder rule"
    );
    assert_eq!(new.state, "auto");
    assert_ne!((new.season, new.episode), (Some(2), Some(3)));
}

#[test]
fn the_type_is_taught_too() {
    let lib = library(
        &["Movies/Pilot/Pilot Story (2019) 1080p.mkv".to_owned()],
        true,
    );
    let work = lib.work_of_path("Movies/Pilot/Pilot Story (2019) 1080p.mkv");
    lib.cat.set_work_kind(ItemRef::Movie(work), "show").unwrap();
    let kind: String = lib
        .db()
        .query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
        .unwrap();
    assert_eq!(kind, "show");
    assert_eq!(lib.cat.work_kind(ItemRef::Show(work)).unwrap(), "show");
    let rules = lib.cat.folder_rules(ItemRef::Show(work)).unwrap();
    assert_eq!(rules[0].kind.as_deref(), Some("show"));
    // Another file of the folder, read as a movie, joins the series.
    media(&lib.root(), "Movies/Pilot/Pilot Story (2019) 720p.mkv", 9);
    scan(&lib.cat);
    assert_eq!(
        lib.work_of_path("Movies/Pilot/Pilot Story (2019) 720p.mkv"),
        work
    );
}

fn synced(lib: &Lib, device: &str) {
    lib.db()
        .execute(
            "UPDATE sources SET sync_id = 'src-family-1', sync_mode = 1",
            [],
        )
        .unwrap();
    lib.cat.enable_sync(device).unwrap();
}

#[test]
fn rules_travel_by_folder_hash_and_wait_for_the_folder() {
    let (a, _cache) = fixed_by_confirm();
    synced(&a, "dev-a");
    let ops: Vec<_> = a
        .cat
        .pending_ops(100)
        .unwrap()
        .into_iter()
        .filter(|op| matches!(op.body, OpBody::FolderRule { .. }))
        .collect();
    assert_eq!(ops.len(), 1);
    let OpBody::FolderRule { rule_key, rule } = &ops[0].body else {
        unreachable!()
    };
    // Never a path.
    assert!(rule_key.starts_with("src-family-1:"));
    let text = format!("{rule_key} {}", rule.as_deref().unwrap());
    assert!(!text.contains("Anime/"), "{text}");
    assert!(text.contains("900017"), "{text}");

    // B has the source but has not scanned the folder yet: the rule waits.
    let b = library(&[frieren(5), frieren(6), frieren(7)], false);
    synced(&b, "dev-b");
    assert!(!b.cat.apply_remote(&ops[0]).unwrap());
    // The scan finds the folder: every file follows the rule at once, the
    // work named by its TMDB id — no provider was asked on B.
    scan(&b.cat);
    let five = b.file(5);
    let seven = b.file(7);
    assert_eq!(five.work, seven.work);
    assert_eq!((five.season, five.episode), (Some(2), Some(1)));
    assert_eq!(
        (seven.season, seven.episode, seven.absolute),
        (Some(2), Some(3), Some(7))
    );
    assert_eq!(seven.decided_by, "rule");
    let tmdb: String = b
        .db()
        .query_row(
            "SELECT ext_id FROM external_ids WHERE work_id = ?1 AND provider = 'tmdb'",
            [seven.work],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tmdb, "900017");
    // Applying it caused no echo.
    assert!(
        b.cat
            .pending_ops(100)
            .unwrap()
            .iter()
            .all(|op| !matches!(op.body, OpBody::FolderRule { .. }))
    );
    // An older op changes nothing; a removal on A removes it on B.
    assert!(!b.cat.apply_remote(&ops[0]).unwrap());
    for op in &ops {
        a.cat.ack_op(op.op_id).unwrap();
    }
    a.cat
        .forget_folder_rules(ItemRef::Show(a.file(5).work))
        .unwrap();
    let removal: Vec<_> = a
        .cat
        .pending_ops(100)
        .unwrap()
        .into_iter()
        .filter(|op| matches!(&op.body, OpBody::FolderRule { rule: None, .. }))
        .collect();
    assert_eq!(removal.len(), 1);
    assert!(b.cat.apply_remote(&removal[0]).unwrap());
    assert!(
        b.cat
            .folder_rules(ItemRef::Show(seven.work))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_rule_received_for_scanned_files_moves_them() {
    let (a, _cache) = fixed_by_confirm();
    synced(&a, "dev-a");
    let op = a
        .cat
        .pending_ops(100)
        .unwrap()
        .into_iter()
        .find(|op| matches!(op.body, OpBody::FolderRule { .. }))
        .unwrap();
    // B scanned the same files and recognised them by name only.
    let b = library(&[frieren(5), frieren(6)], true);
    let before = b.file(5);
    assert_eq!(before.state, "auto");
    synced(&b, "dev-b");
    assert!(b.cat.apply_remote(&op).unwrap());
    let after = b.file(5);
    assert_eq!((after.season, after.episode), (Some(2), Some(1)));
    assert_eq!(after.state, "user_set");
    // The old work had no files left.
    let left: i64 = b
        .db()
        .query_row(
            "SELECT count(*) FROM works WHERE id = ?1",
            [before.work],
            |r| r.get(0),
        )
        .unwrap();
    assert!(left == 0 || after.work == before.work);
}
