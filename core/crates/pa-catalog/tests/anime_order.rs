// SPDX-License-Identifier: MPL-2.0
//! AniDB order of real anime (META-029): a synthetic Anime-Lists list (no
//! upstream data) downloaded into the device cache, the metadata run
//! writing `episode_mappings`, and the Show Detail order rebuilt from them.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_anime_mappings::update::Outcome;
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{
    AnimeLists, Catalog, EpisodeOrder, ItemRef, MetadataOptions, MetadataProviders, ShowEpisodes,
};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;

/// Two AniDB entries on the two TMDB seasons of "Kaiju" (TMDB 90001, TVDB
/// 500001). TMDB numbers season 2 on from season 1 (S02E04…), so entry 2
/// has `tmdboffset` 3; the AniDB special S1 is TVDB special 1 (mapping
/// list). Another anime shares nothing with the show.
const LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<anime-list>
  <anime anidbid="9001" tvdbid="500001" defaulttvdbseason="1" tmdbtv="90001" tmdbseason="1" tmdboffset="0">
    <name>Kaiju</name>
    <mapping-list>
      <mapping anidbseason="0" tvdbseason="0">;1-1;</mapping>
    </mapping-list>
  </anime>
  <anime anidbid="9002" tvdbid="500001" defaulttvdbseason="2" episodeoffset="3" tmdbtv="90001" tmdbseason="2" tmdboffset="3">
    <name>Kaiju 2nd Season</name>
  </anime>
  <anime anidbid="9100" tvdbid="500002" defaulttvdbseason="1" tmdbtv="90002" tmdbseason="1">
    <name>Someone Else</name>
  </anime>
</anime-list>
"#;

/// The same franchise after an upstream fix: entry 2 starts at S02E03.
const LIST_FIXED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<anime-list>
  <anime anidbid="9001" tvdbid="500001" defaulttvdbseason="1" tmdbtv="90001" tmdbseason="1">
    <name>Kaiju</name>
  </anime>
  <anime anidbid="9002" tvdbid="500001" defaulttvdbseason="2" episodeoffset="2" tmdbtv="90001" tmdbseason="2" tmdboffset="2">
    <name>Kaiju 2nd Season</name>
  </anime>
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

/// Synthetic TMDB for the show and the upstream Anime-Lists file.
struct Fake {
    list: Mutex<Option<&'static str>>,
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Fake {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let url = r.url();
        self.calls.lock().unwrap().push(url.clone());
        if url.ends_with("anime-list-master.xml") {
            return match *self.list.lock().unwrap() {
                Some(list) => Ok(ok(list)),
                None => Err(TransportError("offline".into())),
            };
        }
        let query = r
            .query
            .iter()
            .find(|(k, _)| k == "query")
            .map_or("", |(_, v)| v.as_str());
        Ok(match r.path.as_str() {
            "/search/tv" if query == "Kaiju" => ok(r#"{"page":1,"results":[
                {"id":90001,"name":"Kaiju","original_name":"Kaiju","first_air_date":"2020-04-01"}
            ]}"#),
            "/tv/90001" => ok(r#"{"id":90001,"name":"Kaiju","original_name":"Kaiju",
                "original_language":"ja","first_air_date":"2020-04-01","status":"Ended",
                "seasons":[{"season_number":1,"episode_count":3},{"season_number":2,"episode_count":2}]}"#),
            other => HttpResponse {
                status: 404,
                headers: vec![],
                body: format!(r#"{{"status_message":"no fixture for {other}"}}"#),
            },
        })
    }
}

fn fake(list: Option<&'static str>) -> Arc<Fake> {
    Arc::new(Fake {
        list: Mutex::new(list),
        calls: Mutex::new(Vec::new()),
    })
}

fn providers(fake: &Arc<Fake>, lists: &AnimeLists) -> MetadataProviders {
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
        anime_mappings: lists.load().unwrap().map(Arc::new),
        anime_titles: None,
        extra: Default::default(),
    }
}

/// "Kaiju": TMDB season 1 (3 episodes), season 2 numbered on (S02E04,
/// S02E05) and a special.
fn setup() -> (tempfile::TempDir, Catalog, ItemRef) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Shows/Kaiju/Season 1/Kaiju.S01E01.mkv", 1);
    media(&root, "Shows/Kaiju/Season 1/Kaiju.S01E02.mkv", 2);
    media(&root, "Shows/Kaiju/Season 1/Kaiju.S01E03.mkv", 3);
    media(&root, "Shows/Kaiju/Season 2/Kaiju.S02E04.mkv", 4);
    media(&root, "Shows/Kaiju/Season 2/Kaiju.S02E05.mkv", 5);
    media(&root, "Shows/Kaiju/Specials/Kaiju.S00E01.mkv", 6);
    let cat = Catalog::open(dir.path().join("db/library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let shows = cat.shows().unwrap();
    assert_eq!(shows.len(), 1, "{shows:?}");
    (dir, cat, shows[0].id)
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
    assert_eq!(report.matched, 1, "{report:?}");
}

/// (season, episode) in the order, then in TMDB order.
type Numbers = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);

fn numbers(eps: &ShowEpisodes) -> Vec<Numbers> {
    eps.episodes
        .iter()
        .map(|e| (e.item.season, e.item.episode, e.tmdb_season, e.tmdb_episode))
        .collect()
}

/// `(tmdb season, tmdb episode, anidb season, anidb episode, source, confidence)`.
type Row = (i64, i64, Option<i64>, i64, String, f64);

fn rows(cat: &Catalog) -> Vec<Row> {
    let conn = Connection::open(cat.path()).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT e.season_number, e.episode_number, m.season_number, m.episode_number,
                    m.source, m.confidence
             FROM episode_mappings m JOIN episodes e ON e.id = m.episode_id
             WHERE m.scheme = 'anidb' AND m.scheme_ref = ''
             ORDER BY e.season_number, e.episode_number",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

#[test]
fn two_tmdb_seasons_get_an_anidb_order_of_two_entries() {
    let (dir, cat, show) = setup();
    let fake = fake(Some(LIST));
    let lists = AnimeLists::new(dir.path().join("cache/anime-mappings")).with_min_entries(1);
    assert!(lists.load().unwrap().is_none(), "nothing downloaded yet");
    assert_eq!(
        lists.refresh(fake.as_ref(), 1_800_000_000, false).unwrap(),
        Outcome::Updated { entries: 3 }
    );
    run(&cat, &providers(&fake, &lists));

    let source = || "anime_lists".to_owned();
    assert_eq!(
        rows(&cat),
        vec![
            // AniDB special S1 through the TVDB id the franchise shares.
            (0, 1, Some(0), 1, source(), 0.6),
            (1, 1, Some(1), 1, source(), 1.0),
            (1, 2, Some(1), 2, source(), 1.0),
            (1, 3, Some(1), 3, source(), 1.0),
            (2, 4, Some(2), 1, source(), 1.0),
            (2, 5, Some(2), 2, source(), 1.0),
        ]
    );

    let tmdb = cat.show_episodes(show, None).unwrap();
    assert_eq!(tmdb.order, EpisodeOrder::Tmdb);
    assert_eq!(
        tmdb.available,
        vec![
            EpisodeOrder::Tmdb,
            EpisodeOrder::Absolute,
            EpisodeOrder::Anidb
        ]
    );
    let anidb = cat.show_episodes(show, Some(EpisodeOrder::Anidb)).unwrap();
    assert_eq!(anidb.order, EpisodeOrder::Anidb);
    assert_eq!(
        numbers(&anidb),
        vec![
            (Some(0), Some(1), Some(0), Some(1)),
            (Some(1), Some(1), Some(1), Some(1)),
            (Some(1), Some(2), Some(1), Some(2)),
            (Some(1), Some(3), Some(1), Some(3)),
            (Some(2), Some(1), Some(2), Some(4)),
            (Some(2), Some(2), Some(2), Some(5)),
        ]
    );
    let entries: std::collections::BTreeSet<_> = anidb
        .episodes
        .iter()
        .filter_map(|e| e.item.season.filter(|s| *s >= 1))
        .collect();
    assert_eq!(entries.len(), 2, "two AniDB entries");

    // The stored choice survives and the order stays AniDB.
    cat.set_episode_order(show, EpisodeOrder::Anidb).unwrap();
    assert_eq!(
        cat.show_episodes(show, None).unwrap().order,
        EpisodeOrder::Anidb
    );
}

#[test]
fn offline_uses_the_last_list_and_a_new_list_replaces_the_rows() {
    let (dir, cat, show) = setup();
    let lists = AnimeLists::new(dir.path().join("cache/anime-mappings")).with_min_entries(1);

    // Never downloaded and offline: no AniDB order, nothing written.
    let offline = fake(None);
    assert!(
        lists
            .refresh(offline.as_ref(), 1_800_000_000, false)
            .is_err()
    );
    assert!(lists.load().unwrap().is_none());
    run(&cat, &providers(&offline, &lists));
    assert!(rows(&cat).is_empty());
    let eps = cat.show_episodes(show, Some(EpisodeOrder::Anidb)).unwrap();
    assert_eq!(eps.order, EpisodeOrder::Tmdb, "AniDB not offered");
    assert!(!eps.available.contains(&EpisodeOrder::Anidb));

    // Downloaded once, then offline: the last list still applies.
    let online = fake(Some(LIST));
    lists
        .refresh(online.as_ref(), 1_800_000_000, false)
        .unwrap();
    assert!(
        lists
            .refresh(offline.as_ref(), 1_900_000_000, false)
            .is_err()
    );
    let index = lists.load().unwrap().expect("the last downloaded list");
    assert_eq!(cat.apply_anime_mappings(&index).unwrap(), 6);
    assert!(
        cat.show_episodes(show, None)
            .unwrap()
            .available
            .contains(&EpisodeOrder::Anidb)
    );

    // Checked recently: no request.
    let calls = online.calls.lock().unwrap().len();
    assert_eq!(
        lists
            .refresh(online.as_ref(), 1_800_000_100, false)
            .unwrap(),
        Outcome::Skipped
    );
    assert_eq!(online.calls.lock().unwrap().len(), calls);

    // An upstream fix: rows of the old list are replaced, not added to.
    let fixed = fake(Some(LIST_FIXED));
    lists.refresh(fixed.as_ref(), 1_900_000_000, true).unwrap();
    let index = lists.load().unwrap().unwrap();
    cat.apply_anime_mappings(&index).unwrap();
    let r: Vec<_> = rows(&cat)
        .into_iter()
        .map(|(s, e, a, n, _, _)| (s, e, a, n))
        .collect();
    assert_eq!(
        r,
        vec![
            (1, 1, Some(1), 1),
            (1, 2, Some(1), 2),
            (1, 3, Some(1), 3),
            (2, 4, Some(2), 2),
            (2, 5, Some(2), 3),
        ],
        "no special in the fixed list, entry 2 starts at S02E03"
    );

    // Rows of other sources are left alone.
    let conn = Connection::open(cat.path()).unwrap();
    let s1e1: i64 = conn
        .query_row(
            "SELECT id FROM episodes WHERE season_number = 1 AND episode_number = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO episode_mappings (episode_id, scheme, season_number, episode_number, source)
         VALUES (?1, 'anidb', 3, 7, 'user')",
        [s1e1],
    )
    .unwrap();
    cat.apply_anime_mappings(&index).unwrap();
    let user: i64 = conn
        .query_row(
            "SELECT count(*) FROM episode_mappings WHERE source = 'user'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(user, 1);
}
