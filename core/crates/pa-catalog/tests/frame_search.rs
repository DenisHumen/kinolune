// SPDX-License-Identifier: MPL-2.0
//! Anime recognised by its frames through trace.moe (META-031) end to end,
//! on recorded responses (`fixtures/trace_moe/*.jsonl`, synthetic entries in
//! the shape of real API answers) — no network:
//!
//! - an anime the name, the folder and the databases did not recognise gets
//!   the right title and episode when ≥ 90 % of its frames agree;
//! - frames that do not agree leave it in the review inbox, with the
//!   entries they named as candidates;
//! - without the user's consent nothing is read and no request is sent.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pa_catalog::frame_search::{FrameReader, FrameSource};
use pa_catalog::pa_metadata::anilist::{AniListClient, AniListConfig};
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::{Clock, ManualClock};
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::replay::ReplayTransport;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::pa_metadata::tracemoe::{TraceMoeClient, TraceMoeConfig};
use pa_catalog::{
    Catalog, FrameSearchOptions, ItemRef, MetadataOptions, MetadataProviders, ReviewState,
};
use pa_library::rusqlite::Connection;
use pa_probe::VideoFrame;
use pa_scanner::CancelToken;
use pa_vfs::Vfs;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn ok(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: body.to_owned(),
    }
}

/// TMDB and AniList that know nothing by name; AniList knows the entries
/// of the cassettes by id.
#[derive(Default)]
struct Providers {
    calls: Mutex<Vec<String>>,
}

impl Providers {
    fn respond(&self, r: &HttpRequest, body: Option<&str>) -> HttpResponse {
        let query = r
            .query
            .iter()
            .find(|(k, _)| k == "query")
            .map_or("", |(_, v)| v.as_str());
        self.calls
            .lock()
            .unwrap()
            .push(format!("{} {query} {}", r.path, body.unwrap_or("")));
        let empty = r#"{"page":1,"results":[],"total_pages":0,"total_results":0}"#;
        match r.path.as_str() {
            "/search/tv" | "/search/movie" => ok(empty),
            "" | "/" if body.is_some_and(|b| b.contains("query Media") && b.contains("100002")) => {
                ok(
                    r#"{"data":{"Media":{"id":100002,"format":"TV","status":"FINISHED",
                    "episodes":12,"duration":24,
                    "title":{"romaji":"Hoshi no Haitatsunin","english":"Star Courier",
                             "native":"星の配達人"},
                    "synonyms":[],"startDate":{"year":2021,"month":4,"day":2},
                    "description":"A courier between stars.",
                    "coverImage":{"large":"https://img.example/star-courier.jpg"}}}}"#,
                )
            }
            "" | "/" => ok(r#"{"data":{"Page":{"media":[]}}}"#),
            other => HttpResponse {
                status: 404,
                headers: vec![],
                body: format!(r#"{{"status_message":"no fixture for {other}"}}"#),
            },
        }
    }
}

struct Transport(Arc<Providers>);

impl HttpTransport for Transport {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        Ok(self.0.respond(r, None))
    }

    fn post_json(&self, r: &HttpRequest, body: &str) -> Result<HttpResponse, TransportError> {
        Ok(self.0.respond(r, Some(body)))
    }
}

fn providers(fake: &Arc<Providers>) -> MetadataProviders {
    let transport: Arc<dyn HttpTransport> = Arc::new(Transport(fake.clone()));
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let mut tmdb_config = TmdbConfig::default();
    tmdb_config.retry.max_retries = 0;
    let mut anilist_config = AniListConfig::default();
    anilist_config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            tmdb_config,
            Keys::none().with_user_key(Some("test-key")),
            transport.clone(),
            cache.clone(),
            clock.clone(),
        ))),
        anilist: Some(Arc::new(AniListClient::new(
            anilist_config,
            transport,
            cache,
            clock,
        ))),
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
        extra: Default::default(),
    }
}

/// A trace.moe client answering from a cassette.
fn trace_moe(cassette: &str) -> (Arc<ReplayTransport>, TraceMoeClient) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/trace_moe")
        .join(cassette);
    let text = std::fs::read_to_string(path).unwrap();
    let transport = Arc::new(ReplayTransport::from_jsonl(&text).unwrap());
    let client = TraceMoeClient::new(
        TraceMoeConfig::default(),
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        Arc::new(ManualClock::default()) as Arc<dyn Clock>,
    );
    (transport, client)
}

/// Frames of a 24-minute episode: the first one asked for is black (and
/// must be skipped), the rest are pictures.
#[derive(Default)]
struct Frames {
    opened: AtomicUsize,
    grabbed: Arc<AtomicUsize>,
}

struct Reader {
    grabbed: Arc<AtomicUsize>,
}

impl FrameSource for Frames {
    fn open(&self, _vfs: Arc<dyn Vfs>, _path: &str) -> Result<Box<dyn FrameReader>, String> {
        self.opened.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(Reader {
            grabbed: Arc::clone(&self.grabbed),
        }))
    }
}

impl FrameReader for Reader {
    fn duration_ms(&self) -> Option<u64> {
        Some(24 * 60 * 1000)
    }

    fn frame_at(&mut self, at_ms: u64) -> Result<Option<VideoFrame>, String> {
        let n = self.grabbed.fetch_add(1, Ordering::Relaxed);
        let (w, h) = (64u32, 36u32);
        let rgb = (0..w * h)
            .flat_map(|i| {
                if n == 0 {
                    [0, 0, 0]
                } else {
                    let x = u8::try_from(i % w * 4).unwrap_or(255);
                    [x, 255 - x, u8::try_from(n * 20 % 256).unwrap_or(0)]
                }
            })
            .collect();
        Ok(Some(VideoFrame {
            position_ms: at_ms,
            width: w,
            height: h,
            rgb,
        }))
    }
}

/// A library with one anime file nobody recognises, after a metadata run:
/// the work waits in the review inbox.
fn setup(rel: &str) -> (tempfile::TempDir, Catalog, Arc<Providers>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, rel, 7);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let fake = Arc::new(Providers::default());
    let report = cat
        .fetch_metadata(
            &providers(&fake),
            &MetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(report.not_found, 1, "{report:?}");
    let inbox = cat.review_items(10).unwrap();
    assert_eq!(inbox.len(), 1, "{inbox:?}");
    assert_eq!(inbox[0].kind, "anime");
    assert_eq!(inbox[0].state, ReviewState::Manual);
    (dir, cat, fake)
}

fn consent() -> FrameSearchOptions {
    FrameSearchOptions {
        consent: true,
        ..Default::default()
    }
}

fn db(dir: &tempfile::TempDir) -> Connection {
    Connection::open(dir.path().join("library.db")).unwrap()
}

#[test]
fn agreeing_frames_give_title_and_episode() {
    // A running count across seasons that no database resolves: entry
    // episode 8 in AniList order.
    let (dir, cat, fake) = setup("Anime/[Grp] Hshi Hitts - 20 (1080p).mkv");
    let (transport, client) = trace_moe("agree.jsonl");
    let frames = Frames::default();
    let report = cat
        .search_frames(
            &client,
            &frames,
            &consent(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(report.works, 1, "{report:?}");
    assert_eq!(report.matched, 1, "{report:?}");
    assert_eq!(report.review, 0, "{report:?}");
    // Five agreeing frames decide; the black one was not sent.
    assert_eq!(report.searches, 5, "{report:?}");
    assert!(report.error.is_none(), "{report:?}");
    let sent = transport.requests();
    assert_eq!(sent[0].path, "/me");
    assert_eq!(sent.len(), 6);
    assert!(
        sent[1..]
            .iter()
            .all(|r| r.path == "/search" && r.url().ends_with("/search?anilistInfo="))
    );
    assert!(frames.grabbed.load(Ordering::Relaxed) >= 6);

    // Out of the inbox; the match is automatic, explained by the frames.
    assert!(cat.review_items(10).unwrap().is_empty());
    let conn = db(&dir);
    let (state, confidence, explanation): (String, f64, String) = conn
        .query_row(
            "SELECT state, confidence, explanation_json FROM matches WHERE state <> 'rejected'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, "auto");
    assert!((confidence - 1.0).abs() < 1e-9);
    assert!(explanation.contains("trace_moe"), "{explanation}");
    let anilist: String = conn
        .query_row(
            "SELECT ext_id FROM external_ids WHERE provider = 'anilist'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(anilist, "100002");
    let cached: (String, i64, i64) = conn
        .query_row(
            "SELECT outcome, anilist_id, episode FROM frame_searches",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(cached, ("matched".to_owned(), 100_002, 8));

    // The next metadata run reads AniList by the id, without searching
    // TMDB by the names that failed.
    fake.calls.lock().unwrap().clear();
    let again = cat
        .fetch_metadata(
            &providers(&fake),
            &MetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(again.matched, 1, "{again:?}");
    let calls = fake.calls.lock().unwrap().clone();
    assert!(
        !calls.iter().any(|c| c.starts_with("/search/")),
        "{calls:?}"
    );
    let shows = cat.shows().unwrap();
    assert_eq!(shows.len(), 1, "{shows:?}");
    assert!(
        ["Star Courier", "Hoshi no Haitatsunin"].contains(&shows[0].title.as_str()),
        "{:?}",
        shows[0].title
    );
    let episodes = cat.episodes(shows[0].id).unwrap();
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].episode, Some(8), "{episodes:?}");
    // Still decided: not in the inbox, still automatic.
    assert!(cat.review_items(10).unwrap().is_empty());

    // Never searched twice: the outcome is kept by the file's fingerprint.
    let (transport2, client2) = trace_moe("agree.jsonl");
    let report = cat
        .search_frames(
            &client2,
            &frames,
            &consent(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(report.works, 0, "{report:?}");
    assert!(transport2.requests().is_empty());
}

#[test]
fn disagreeing_frames_go_to_review_with_candidates() {
    let (dir, cat, _fake) = setup("Anime/[Grp] Qwzx Plorb - 03 (1080p).mkv");
    let (transport, client) = trace_moe("disagree.jsonl");
    let frames = Frames::default();
    let report = cat
        .search_frames(
            &client,
            &frames,
            &consent(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(report.matched, 0, "{report:?}");
    assert_eq!(report.review, 1, "{report:?}");
    // One frame for each of two entries and one weak hit: 90 % is out of
    // reach, the search stops early.
    assert_eq!(report.searches, 3, "{report:?}");
    assert_eq!(transport.requests().len(), 4);

    let inbox = cat.review_items(10).unwrap();
    assert_eq!(inbox.len(), 1, "{inbox:?}");
    let item = &inbox[0];
    assert_eq!(item.state, ReviewState::Manual);
    let frames_named: Vec<(&str, &str, &str)> = item
        .candidates
        .iter()
        .filter(|c| c.signals.provider == "trace_moe")
        .map(|c| (c.provider.as_str(), c.ext_id.as_str(), c.title.as_str()))
        .collect();
    assert_eq!(
        frames_named,
        [
            ("anilist", "100002", "Star Courier"),
            ("anilist", "100104", "River Sound"),
        ]
    );
    assert!(
        item.candidates
            .iter()
            .filter(|c| c.provider == "anilist")
            .all(|c| (c.score - 1.0 / 3.0).abs() < 1e-9)
    );
    let outcome: String = db(&dir)
        .query_row("SELECT outcome FROM frame_searches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(outcome, "review");
    // No AniList id was taken.
    let ids: i64 = db(&dir)
        .query_row(
            "SELECT count(*) FROM external_ids WHERE provider = 'anilist'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ids, 0);

    // The user picks one of them: final, and fetched by its id.
    let work = item.work_id;
    let target = cat.review_choose(work, "anilist", "", "100002").unwrap();
    assert!(cat.review_items(10).unwrap().is_empty());
    let candidates = cat.match_candidates(ItemRef::Show(target)).unwrap();
    assert!(
        candidates
            .iter()
            .any(|c| c.provider == "anilist" && c.ext_id == "100002" && c.current),
        "{candidates:?}"
    );
    // A movie id is not an AniList entry.
    assert!(cat.review_choose(target, "anilist", "", "x").is_err());
}

#[test]
fn no_requests_without_consent() {
    let (dir, cat, _fake) = setup("Anime/[Grp] Qwzx Plorb - 03 (1080p).mkv");
    let (transport, client) = trace_moe("agree.jsonl");
    let frames = Frames::default();
    let report = cat
        .search_frames(
            &client,
            &frames,
            &FrameSearchOptions::default(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(report, pa_catalog::FrameSearchReport::default());
    // Nothing asked (not even the quota), no file opened, nothing stored.
    assert!(transport.requests().is_empty());
    assert_eq!(frames.opened.load(Ordering::Relaxed), 0);
    assert_eq!(frames.grabbed.load(Ordering::Relaxed), 0);
    let stored: i64 = db(&dir)
        .query_row("SELECT count(*) FROM frame_searches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, 0);
    assert_eq!(cat.review_items(10).unwrap().len(), 1);
}

#[test]
fn quota_left_is_respected() {
    let (_dir, cat, _fake) = setup("Anime/[Grp] Qwzx Plorb - 03 (1080p).mkv");
    // Four searches left today: not enough for the five frames a decision
    // needs, so nothing is spent.
    let transport = Arc::new(ReplayTransport::default());
    transport.push(
        "/me",
        ok(r#"{"id":"192.0.2.10","priority":0,"concurrency":1,"quota":100,"quotaUsed":96}"#),
    );
    let client = TraceMoeClient::new(
        TraceMoeConfig::default(),
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        Arc::new(ManualClock::default()) as Arc<dyn Clock>,
    );
    let report = cat
        .search_frames(
            &client,
            &Frames::default(),
            &consent(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(report.searches, 0, "{report:?}");
    assert!(report.error.is_some(), "{report:?}");
    assert_eq!(transport.requests().len(), 1);
    assert_eq!(cat.review_items(10).unwrap().len(), 1);
}
