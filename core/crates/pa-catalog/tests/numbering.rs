// SPDX-License-Identifier: MPL-2.0
//! TMDB numbers of anime files written as running counts or entry titles
//! (META-015): a synthetic TMDB series in three seasons, without and with a
//! synthetic Anime-Lists list (no upstream data), through the metadata run.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{AnimeLists, Catalog, MetadataOptions, MetadataProviders};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;

/// "Kaiju" as AniDB splits it: TMDB seasons 1 (4 episodes) and 2 (6), the
/// second season two AniDB entries (cours of 3).
const LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<anime-list>
  <anime anidbid="9001" tvdbid="500001" defaulttvdbseason="1" tmdbtv="90001" tmdbseason="1"><name>Kaiju</name></anime>
  <anime anidbid="9002" tvdbid="500001" defaulttvdbseason="2" tmdbtv="90001" tmdbseason="2"><name>Kaiju 2nd Season</name></anime>
  <anime anidbid="9003" tvdbid="500001" defaulttvdbseason="2" episodeoffset="3" tmdbtv="90001" tmdbseason="2" tmdboffset="3"><name>Kaiju 2nd Season Part 2</name></anime>
</anime-list>
"#;

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
        headers: vec![("ETag".into(), format!("\"{}\"", body.len()))],
        body: body.to_owned(),
    }
}

struct Fake {
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Fake {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let url = r.url();
        self.calls.lock().unwrap().push(url.clone());
        if url.ends_with("anime-list-master.xml") {
            return Ok(ok(LIST));
        }
        let query = r
            .query
            .iter()
            .find(|(k, _)| k == "query")
            .map_or("", |(_, v)| v.as_str());
        Ok(match r.path.as_str() {
            "/search/tv" if query.starts_with("Kaiju") => ok(r#"{"page":1,"results":[
                {"id":90001,"name":"Kaiju","original_name":"Kaiju","first_air_date":"2020-04-01"}
            ]}"#),
            "/tv/90001" => ok(r#"{"id":90001,"name":"Kaiju","original_name":"Kaiju",
                "original_language":"ja","first_air_date":"2020-04-01","status":"Ended",
                "seasons":[{"season_number":1,"episode_count":4},{"season_number":2,"episode_count":6}]}"#),
            "/tv/90001/season/2" => ok(r#"{"season_number":2,"episodes":[
                {"id":5,"season_number":2,"episode_number":1,"name":"Two-One"},
                {"id":6,"season_number":2,"episode_number":2,"name":"Two-Two"},
                {"id":9,"season_number":2,"episode_number":5,"name":"Two-Five"}]}"#),
            other => HttpResponse {
                status: 404,
                headers: vec![],
                body: format!(r#"{{"status_message":"no fixture for {other}"}}"#),
            },
        })
    }
}

fn providers(fake: &Arc<Fake>, lists: Option<&AnimeLists>) -> MetadataProviders {
    let mut config = TmdbConfig::default();
    config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            config,
            Keys::none().with_user_key(Some("test-key")),
            fake.clone(),
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        ))),
        anilist: None,
        shikimori: None,
        anime_mappings: lists.and_then(|l| l.load().unwrap()).map(Arc::new),
    }
}

/// Fansub releases of "Kaiju": the running count (05, 06 = TMDB S02E01,
/// S02E02) and the second cour under its own title (02 = TMDB S02E05).
fn setup() -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Anime/Kaiju/[Grp] Kaiju - 04 (1080p).mkv", 1);
    media(&root, "Anime/Kaiju/[Grp] Kaiju - 05 (1080p).mkv", 2);
    media(&root, "Anime/Kaiju/[Grp] Kaiju - 06 (1080p).mkv", 3);
    media(
        &root,
        "Anime/Kaiju 2nd Season Part 2/[Grp] Kaiju 2nd Season Part 2 - 02 (1080p).mkv",
        4,
    );
    let cat = Catalog::open(dir.path().join("db/library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    (dir, cat)
}

fn run(cat: &Catalog, p: &MetadataProviders) {
    let report = cat
        .fetch_metadata(
            p,
            &MetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert!(report.matched >= 1, "{report:?}");
}

/// `(file name, season, episode, absolute, title)` of every matched file.
type Row = (
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

fn rows(cat: &Catalog) -> Vec<Row> {
    let conn = Connection::open(cat.path()).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT f.path, e.season_number, e.episode_number, e.absolute_number, e.title
             FROM files f JOIN matches m ON m.file_id = f.id
             JOIN match_episodes me ON me.match_id = m.id JOIN episodes e ON e.id = me.episode_id
             WHERE m.state <> 'rejected' ORDER BY f.path",
        )
        .unwrap();
    stmt.query_map([], |r| {
        let path: String = r.get(0)?;
        Ok((
            path.rsplit('/').next().unwrap().to_owned(),
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn count(cat: &Catalog, sql: &str) -> i64 {
    Connection::open(cat.path())
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

#[test]
fn running_count_lands_in_tmdb_seasons() {
    let (_dir, cat) = setup();
    let fake = Arc::new(Fake {
        calls: Mutex::new(Vec::new()),
    });
    run(&cat, &providers(&fake, None));
    let rows = rows(&cat);
    let of = |name: &str| rows.iter().find(|r| r.0.contains(name)).unwrap();
    let numbers = |name: &str| {
        let r = of(name);
        (r.1, r.2, r.3)
    };
    assert_eq!(
        numbers("Kaiju - 04"),
        (Some(1), Some(4), Some(4)),
        "{rows:?}"
    );
    assert_eq!(
        numbers("Kaiju - 05"),
        (Some(2), Some(1), Some(5)),
        "{rows:?}"
    );
    assert_eq!(
        numbers("Kaiju - 06"),
        (Some(2), Some(2), Some(6)),
        "{rows:?}"
    );
    // Without the list the cour's own count stays as written (season 2 of
    // the title, episode 2); the list resolves it (test below).
    let cour = of("Part 2 - 02");
    assert_eq!((cour.1, cour.2), (Some(2), Some(2)), "{rows:?}");
    // TMDB titles reach the renumbered episodes: season 2 was fetched.
    assert_eq!(of("Kaiju - 05").4.as_deref(), Some("Two-One"), "{rows:?}");
    assert!(
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("/tv/90001/season/2"))
    );
    // The scan's absolute-only rows were replaced, not left behind.
    assert_eq!(
        count(
            &cat,
            "SELECT count(*) FROM episodes WHERE season_number IS NULL"
        ),
        0
    );
}

#[test]
fn entry_titles_follow_the_downloaded_list() {
    let (dir, cat) = setup();
    let fake = Arc::new(Fake {
        calls: Mutex::new(Vec::new()),
    });
    let lists = AnimeLists::new(dir.path().join("cache/anime-mappings")).with_min_entries(1);
    lists.refresh(&*fake, 1_800_000_000, true).unwrap();
    run(&cat, &providers(&fake, Some(&lists)));
    let rows = rows(&cat);
    let last = rows.iter().find(|r| r.0.contains("Part 2")).unwrap();
    assert_eq!(
        (last.1, last.2, last.3),
        (Some(2), Some(5), Some(9)),
        "{rows:?}"
    );
    assert_eq!(last.4.as_deref(), Some("Two-Five"));
    // The cour and the running count are one show on TMDB.
    assert_eq!(cat.shows().unwrap().len(), 1);

    // An older model's fully automatic decision is taken again (the TMDB
    // search runs again) and lands on the same numbers.
    Connection::open(cat.path())
        .unwrap()
        .execute("UPDATE matches SET model_version = 'scorer_v0'", [])
        .unwrap();
    let searches = |f: &Fake| {
        f.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.contains("/search/tv"))
            .count()
    };
    let before = searches(&fake);
    run(&cat, &providers(&fake, Some(&lists)));
    assert!(searches(&fake) > before);
    assert_eq!(rows(&cat), rows);
    assert_eq!(
        count(
            &cat,
            "SELECT count(*) FROM matches WHERE ifnull(model_version, '') <> 'scorer_v1'"
        ),
        0
    );
}

/// `(state, decided_by, model_version)` of the match of the file ending in `name`.
fn decision(cat: &Catalog, name: &str) -> (String, String, Option<String>) {
    Connection::open(cat.path())
        .unwrap()
        .query_row(
            "SELECT m.state, m.decided_by, m.model_version FROM matches m
             JOIN files f ON f.id = m.file_id WHERE f.path LIKE '%' || ?1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[test]
fn newer_model_recomputes_only_automatic_unconfirmed_matches() {
    let (_dir, cat) = setup();
    let before = rows(&cat);
    {
        let conn = Connection::open(cat.path()).unwrap();
        // The user confirmed episode 05; a user folder rule locks the cour.
        conn.execute(
            "UPDATE matches SET state = 'user_confirmed', decided_by = 'user'
             WHERE file_id = (SELECT id FROM files WHERE path LIKE '%Kaiju - 05 (1080p).mkv')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folder_rules (source_id, path_prefix, work_id, created_at_ms)
             SELECT f.source_id, 'Anime/Kaiju 2nd Season Part 2', m.work_id, 0
             FROM files f JOIN matches m ON m.file_id = f.id WHERE f.path LIKE '%Part 2 - 02%'",
            [],
        )
        .unwrap();
    }
    let fake = Arc::new(Fake {
        calls: Mutex::new(Vec::new()),
    });
    let p = providers(&fake, None);
    run(&cat, &p);
    let after = rows(&cat);
    let find =
        |rows: &[Row], name: &str| rows.iter().find(|r| r.0.contains(name)).cloned().unwrap();
    // Locked files keep their numbers and their decision.
    for name in ["Kaiju - 05", "Part 2 - 02"] {
        let (b, a) = (find(&before, name), find(&after, name));
        assert_eq!((b.1, b.2), (a.1, a.2), "{name}: {after:?}");
    }
    assert_eq!(decision(&cat, "Kaiju - 05 (1080p).mkv").0, "user_confirmed");
    assert_ne!(
        decision(&cat, "Part 2 - 02 (1080p).mkv").1,
        "scorer",
        "a rule-locked match was decided again"
    );
    // The automatic one is renumbered and decided by the current model.
    let six = find(&after, "Kaiju - 06");
    assert_eq!((six.1, six.2), (Some(2), Some(2)));
    let d = decision(&cat, "Kaiju - 06 (1080p).mkv");
    assert_eq!(
        (d.1.as_str(), d.2.as_deref()),
        ("scorer", Some("scorer_v1"))
    );

    // A library decided by an older model is decided again on the next
    // run although its metadata is fresh; the user's match is not touched.
    Connection::open(cat.path())
        .unwrap()
        .execute(
            "UPDATE matches SET model_version = 'scorer_v0' WHERE decided_by IN ('scorer', 'user')",
            [],
        )
        .unwrap();
    run(&cat, &p);
    assert_eq!(
        decision(&cat, "Kaiju - 06 (1080p).mkv").2.as_deref(),
        Some("scorer_v1")
    );
    assert_eq!(
        decision(&cat, "Kaiju - 05 (1080p).mkv").2.as_deref(),
        Some("scorer_v0")
    );
}
