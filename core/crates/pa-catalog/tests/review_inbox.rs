// SPDX-License-Identifier: MPL-2.0
//! The review inbox (META-016) end to end: a scanned folder, synthetic
//! provider responses (no network, no real TMDB data), the thresholds'
//! states, the candidates with their signals, and the user's answers —
//! which no later scan or metadata run decides again.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::metadata::{AUTO, REVIEW};
use pa_catalog::pa_metadata::anilist::{AniListClient, AniListConfig};
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{Catalog, MetadataOptions, MetadataProviders, ReviewState};
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

/// Answers by path and `query`; records every request.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
}

fn ok(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: body.to_owned(),
    }
}

impl Fake {
    fn respond(&self, r: &HttpRequest) -> HttpResponse {
        let query = r
            .query
            .iter()
            .find(|(k, _)| k == "query")
            .map_or("", |(_, v)| v.as_str());
        self.calls
            .lock()
            .unwrap()
            .push(format!("{} {query}", r.path));
        let empty = r#"{"page":1,"results":[],"total_pages":0,"total_results":0}"#;
        match (r.path.as_str(), query) {
            ("/search/movie", "Inception") => ok(r#"{"page":1,"results":[
                {"id":27205,"title":"Начало","original_title":"Inception",
                 "release_date":"2010-07-15","poster_path":"/inception.jpg",
                 "overview":"Сны."}]}"#),
            ("/movie/27205", _) => ok(
                r#"{"id":27205,"title":"Начало","original_title":"Inception",
                "original_language":"en","release_date":"2010-07-15","runtime":148,
                "poster_path":"/inception.jpg"}"#,
            ),
            ("/search/movie", "Steins Gate Load Region") => ok(r#"{"page":1,"results":[
                {"id":212162,"title":"Врата Штейна: Нагрузочная область дежавю",
                 "original_title":"劇場版 STEINS;GATE 負荷領域のデジャヴ",
                 "release_date":"2013-04-20","poster_path":"/sg.jpg"}]}"#),
            ("/movie/212162/translations", _) => ok(r#"{"id":212162,"translations":[
                {"iso_3166_1":"US","iso_639_1":"en",
                 "data":{"title":"Steins;Gate: The Movie − Load Region of Déjà Vu"}}]}"#),
            ("/movie/212162", _) => ok(r#"{"id":212162,
                "title":"Врата Штейна: Нагрузочная область дежавю",
                "original_title":"劇場版 STEINS;GATE 負荷領域のデジャヴ","original_language":"ja",
                "release_date":"2013-04-20","runtime":90,"poster_path":"/sg.jpg"}"#),
            ("/search/tv", "Fairy Tail OVA" | "Fairy Tail") => ok(r#"{"page":1,"results":[
                {"id":46261,"name":"Хвост феи","original_name":"FAIRY TAIL",
                 "first_air_date":"2009-10-12","poster_path":"/ft.jpg",
                 "original_language":"ja"}]}"#),
            ("/tv/46261/translations", _) => ok(r#"{"id":46261,"translations":[
                {"iso_3166_1":"US","iso_639_1":"en","data":{"name":"Fairy Tail"}}]}"#),
            ("/tv/46261", _) => ok(
                r#"{"id":46261,"name":"Хвост феи","original_name":"FAIRY TAIL",
                "original_language":"ja","first_air_date":"2009-10-12","poster_path":"/ft.jpg",
                "seasons":[{"season_number":1,"episode_count":48}]}"#,
            ),
            ("/search/movie" | "/search/tv", _) => ok(empty),
            // AniList: nothing.
            ("" | "/", _) => ok(r#"{"data":{"Page":{"media":[]}}}"#),
            (other, _) => HttpResponse {
                status: 404,
                headers: vec![],
                body: format!(r#"{{"status_message":"no fixture for {other}"}}"#),
            },
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

struct Transport(Arc<Fake>);

impl HttpTransport for Transport {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        Ok(self.0.respond(r))
    }

    fn post_json(&self, r: &HttpRequest, _body: &str) -> Result<HttpResponse, TransportError> {
        Ok(self.0.respond(r))
    }
}

fn providers(fake: &Arc<Fake>) -> MetadataProviders {
    let transport: Arc<dyn HttpTransport> = Arc::new(Transport(fake.clone()));
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let mut tmdb = TmdbConfig::default();
    tmdb.retry.max_retries = 0;
    let mut anilist = AniListConfig::default();
    anilist.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            tmdb,
            Keys::none().with_user_key(Some("test-key")),
            transport.clone(),
            cache.clone(),
            clock.clone(),
        ))),
        anilist: Some(Arc::new(AniListClient::new(
            anilist, transport, cache, clock,
        ))),
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
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

fn setup() -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Movies/Inception (2010)/Inception.2010.1080p.mkv", 1);
    media(&root, "Movies/Steins Gate Load Region (2013).mkv", 2);
    media(&root, "Movies/Zzyzx Qwerty 2003.mkv", 3);
    media(&root, "Movies/Qwerty Vortex 2001.mkv", 4);
    media(
        &root,
        "Anime/Fairy Tail OVA/[Doki] Fairy Tail OVA - 01 (720p) [33B1F140].mkv",
        5,
    );
    media(
        &root,
        "Anime/Fairy Tail OVA/[Doki] Fairy Tail OVA - 02 (720p) [707F44E4].mkv",
        6,
    );
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    (dir, cat)
}

fn db(dir: &tempfile::TempDir) -> Connection {
    Connection::open(dir.path().join("library.db")).unwrap()
}

/// `state` of every live match of `work`.
fn states(conn: &Connection, work: i64) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT state FROM matches WHERE work_id = ?1 AND state <> 'rejected'")
        .unwrap();
    stmt.query_map([work], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn work_by_local_title(conn: &Connection, title: &str) -> i64 {
    conn.query_row(
        "SELECT work_id FROM titles WHERE text = ?1 ORDER BY id LIMIT 1",
        [title],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn thresholds_states_and_candidates() {
    let (dir, cat) = setup();
    let fake = Arc::new(Fake::default());
    let p = providers(&fake);
    run(&cat, &p);
    let conn = db(&dir);

    // ≥ AUTO: silent.
    let inception = work_by_local_title(&conn, "Inception");
    assert_eq!(states(&conn, inception), ["auto"]);
    // Below AUTO, from REVIEW: shown and «проверьте».
    let ova = work_by_local_title(&conn, "Fairy Tail OVA");
    assert_eq!(states(&conn, ova), ["auto_review"]);
    let explanation: String = conn
        .query_row(
            "SELECT explanation_json FROM matches WHERE work_id = ?1 LIMIT 1",
            [ova],
            |r| r.get(0),
        )
        .unwrap();
    let e: serde_json::Value = serde_json::from_str(&explanation).unwrap();
    assert_eq!(e["signals"]["provider"], "tmdb");
    assert_eq!(e["signals"]["lost_marker"], "ova");
    assert_eq!(
        e["signals"]["matched_title"]
            .as_str()
            .map(str::to_lowercase),
        Some("fairy tail".to_owned())
    );
    // Below REVIEW: nothing shown, the manual queue.
    let steins = work_by_local_title(&conn, "Steins Gate Load Region");
    let shown: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM external_ids WHERE work_id = ?1",
            [steins],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(shown, None);

    let count = cat.review_count().unwrap();
    assert_eq!((count.check, count.manual), (1, 3), "{count:?}");
    let items = cat.review_items(100).unwrap();
    assert_eq!(items.len(), 4);
    // Shown results first.
    let first = &items[0];
    assert_eq!(first.work_id, ova);
    assert_eq!(first.state, ReviewState::Check);
    assert_eq!(first.item_id, format!("s{ova}"));
    assert_eq!(first.title, "Хвост феи");
    assert_eq!(first.local_title, "Fairy Tail OVA");
    assert_eq!(first.files, 2);
    let c = first.confidence.unwrap();
    assert!((REVIEW..AUTO).contains(&c), "{c}");
    assert!(first.candidates[0].current);
    assert_eq!(first.candidates[0].ext_id, "46261");
    assert_eq!(
        first.candidates[0].poster_url.as_deref(),
        Some("https://image.tmdb.org/t/p/w342/ft.jpg")
    );
    assert_eq!(
        first.signals.as_ref().unwrap().lost_marker.as_deref(),
        Some("ova")
    );

    let manual = items.iter().find(|i| i.work_id == steins).unwrap();
    assert_eq!(manual.state, ReviewState::Manual);
    assert_eq!(manual.confidence, None);
    let best = &manual.candidates[0];
    assert_eq!(
        (best.ext_kind.as_str(), best.ext_id.as_str()),
        ("movie", "212162")
    );
    assert!(best.score < REVIEW, "{best:?}");
    assert!(!best.current);
    assert_eq!(best.signals.year, Some(2013));
    assert_eq!(best.signals.file_year, Some(2013));
    // A work no search found is in the queue without candidates.
    let zzyzx = work_by_local_title(&conn, "Zzyzx Qwerty");
    let z = items.iter().find(|i| i.work_id == zzyzx).unwrap();
    assert!(z.candidates.is_empty());
    assert_eq!(z.sample_path, "Movies/Zzyzx Qwerty 2003.mkv");
}

#[test]
fn user_answers_are_final() {
    let (dir, cat) = setup();
    let fake = Arc::new(Fake::default());
    let p = providers(&fake);
    run(&cat, &p);
    let conn = db(&dir);
    let ova = work_by_local_title(&conn, "Fairy Tail OVA");
    let steins = work_by_local_title(&conn, "Steins Gate Load Region");
    let zzyzx = work_by_local_title(&conn, "Zzyzx Qwerty");
    let qwerty = work_by_local_title(&conn, "Qwerty Vortex");
    let inception = work_by_local_title(&conn, "Inception");

    // «Верно».
    cat.review_confirm(ova).unwrap();
    assert_eq!(states(&conn, ova), ["user_confirmed"]);
    // Nothing shown, nothing to confirm.
    assert!(cat.review_confirm(steins).is_err());

    // Another candidate: read by id on the next run.
    assert_eq!(
        cat.review_choose(steins, "tmdb", "movie", "212162")
            .unwrap(),
        steins
    );
    assert!(
        cat.review_choose(steins, "tmdb", "tv", "1").is_err(),
        "kind"
    );
    assert_eq!(states(&conn, steins), ["user_set"]);

    // Manual search: ranked with the work's files, nothing written.
    let found = cat
        .review_search(&p, qwerty, "Inception", Some(2010))
        .unwrap();
    assert_eq!(found[0].ext_id, "27205");
    assert_eq!(found[0].title, "Начало");
    assert!(found[0].score > 0.0);
    // Choosing a result another work carries merges the two.
    assert_eq!(
        cat.review_choose(qwerty, "tmdb", "movie", "27205").unwrap(),
        inception
    );
    assert_eq!(states(&conn, inception), ["user_set"]);

    // «Это не фильм».
    cat.review_not_media(zzyzx).unwrap();
    // LIB-025: what is not a movie goes to «Мои видео», unmatched.
    assert!(states(&conn, zzyzx).is_empty());
    assert!(
        cat.own_videos()
            .unwrap()
            .iter()
            .any(|v| v.rel_path.contains("Zzyzx"))
    );

    assert_eq!(cat.review_count().unwrap().total(), 0);
    assert!(cat.review_items(10).unwrap().is_empty());

    // The next run reads the chosen result and never asks about the rest.
    fake.calls.lock().unwrap().clear();
    run(&cat, &p);
    let calls = fake.calls();
    assert!(
        calls.iter().any(|c| c.starts_with("/movie/212162 ")),
        "{calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|c| c.contains("Zzyzx") || c.starts_with("/search")),
        "{calls:?}"
    );
    let title: String = conn
        .query_row("SELECT title FROM works WHERE id = ?1", [steins], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(title, "Врата Штейна: Нагрузочная область дежавю");

    // A full refresh and a rescan decide nothing again.
    cat.reset_metadata(true).unwrap();
    fake.calls.lock().unwrap().clear();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    run(&cat, &p);
    let calls = fake.calls();
    assert!(!calls.iter().any(|c| c.starts_with("/search")), "{calls:?}");
    assert_eq!(states(&conn, ova), ["user_confirmed"]);
    assert_eq!(states(&conn, steins), ["user_set"]);
    assert_eq!(states(&conn, zzyzx), ["user_set"]);
    let title: String = conn
        .query_row("SELECT title FROM works WHERE id = ?1", [zzyzx], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(title, "Zzyzx Qwerty");
    assert_eq!(cat.review_count().unwrap().total(), 0);
}

#[test]
fn not_media_moves_the_files_to_own_videos() {
    let (dir, cat) = setup();
    let fake = Arc::new(Fake::default());
    run(&cat, &providers(&fake));
    let conn = db(&dir);
    let ova = work_by_local_title(&conn, "Fairy Tail OVA");
    cat.review_not_media(ova).unwrap();
    // LIB-025, simplest for the user: no provider data, no show left; the
    // files are the user's own videos, and they stay that way after a scan.
    assert!(
        !cat.shows()
            .unwrap()
            .iter()
            .any(|s| s.title.contains("Fairy Tail"))
    );
    let own = |cat: &Catalog| {
        cat.own_videos()
            .unwrap()
            .iter()
            .filter(|v| v.rel_path.contains("Fairy Tail OVA"))
            .count()
    };
    assert_eq!(own(&cat), 2);
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    run(&cat, &providers(&fake));
    assert_eq!(own(&cat), 2);
    // The detail page's «Исправить распознавание» sees the inbox entry of
    // a work in the inbox and the candidates of any work.
    let steins = work_by_local_title(&conn, "Steins Gate Load Region");
    let item = pa_catalog::ItemRef::Movie(steins);
    assert!(cat.review_item(item).unwrap().is_some());
    assert_eq!(cat.match_candidates(item).unwrap()[0].ext_id, "212162");
}

#[test]
fn an_untitled_clip_nobody_knows_goes_to_own_videos() {
    // LIB-025, simplest for the user: a video without any release marks
    // that the provider does not know is a clip of the user's own, not a
    // review task. A film with a year that is not found stays in review.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Video/Birthday at the lake.mkv", 1);
    media(&root, "Movies/Zzyzx Qwerty 2003.mkv", 2);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let fake = Arc::new(Fake::default());
    run(&cat, &providers(&fake));
    let own: Vec<String> = cat
        .own_videos()
        .unwrap()
        .into_iter()
        .map(|v| v.rel_path)
        .collect();
    assert!(own.iter().any(|p| p.contains("Birthday")), "{own:?}");
    assert!(!own.iter().any(|p| p.contains("Zzyzx")), "{own:?}");
    // It stays there after the next scan and run.
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    run(&cat, &providers(&fake));
    assert!(
        cat.own_videos()
            .unwrap()
            .iter()
            .any(|v| v.rel_path.contains("Birthday"))
    );
}
