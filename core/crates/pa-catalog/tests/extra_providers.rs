// SPDX-License-Identifier: MPL-2.0
//! META-012 end to end: daily shows (A11) matched by their air dates
//! (TMDB seasons, then TVmaze and TheTVDB for dates TMDB lacks), Fanart.tv
//! clear logo and clear art, and a movie found by the OSHash of its file
//! through OpenSubtitles. Synthetic responses, no network.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::fanart::{FanartClient, FanartConfig};
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::opensubtitles::{OpenSubtitlesClient, OpenSubtitlesConfig};
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::pa_metadata::tvdb::{TvdbClient, TvdbConfig};
use pa_catalog::pa_metadata::tvmaze::{TvMazeClient, TvMazeConfig};
use pa_catalog::{Catalog, ExtraProviders, MetadataOptions, MetadataProviders};
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

fn ok(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: body.to_owned(),
    }
}

fn not_found() -> HttpResponse {
    HttpResponse {
        status: 404,
        headers: vec![],
        body: r#"{"status_message":"no fixture"}"#.to_owned(),
    }
}

fn param<'a>(r: &'a HttpRequest, key: &str) -> Option<&'a str> {
    r.query
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Every provider by host and path; records `host path?query`.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn called(&self, prefix: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }

    fn respond(&self, r: &HttpRequest) -> HttpResponse {
        let host = r.base.split('/').nth(2).unwrap_or("");
        self.calls
            .lock()
            .unwrap()
            .push(format!("{host} {}", r.canonical()));
        let q = param(r, "query").unwrap_or("");
        match (host, r.path.as_str()) {
            ("api.themoviedb.org", "/search/tv") if q == "The Late Desk" => {
                ok(r#"{"page":1,"results":[
                {"id":190001,"name":"The Late Desk","original_name":"The Late Desk",
                 "first_air_date":"2015-09-08","popularity":30}]}"#)
            }
            ("api.themoviedb.org", "/search/tv" | "/search/movie") => {
                ok(r#"{"page":1,"results":[],"total_pages":0,"total_results":0}"#)
            }
            ("api.themoviedb.org", "/tv/190001") => ok(r#"{"id":190001,"name":"The Late Desk",
                "original_name":"The Late Desk","original_language":"en","first_air_date":"2015-09-08",
                "status":"Returning Series","episode_run_time":[30],
                "seasons":[{"season_number":0,"episode_count":2,"air_date":"2015-01-01"},
                           {"season_number":1,"episode_count":150,"air_date":"2015-09-08"},
                           {"season_number":10,"episode_count":150,"air_date":"2024-09-09"}]}"#),
            ("api.themoviedb.org", "/tv/190001/season/10") => ok(r#"{"id":5010,"season_number":10,
                "episodes":[
                {"id":9031,"season_number":10,"episode_number":31,"name":"Guest One",
                 "overview":"First guest.","air_date":"2025-03-14","runtime":29,"still_path":"/s31.jpg"},
                {"id":9032,"season_number":10,"episode_number":32,"name":"Guest Two",
                 "air_date":"2025-03-17"}]}"#),
            ("api.themoviedb.org", "/tv/190001/external_ids") => {
                ok(r#"{"id":190001,"imdb_id":"tt9000001","tvdb_id":390001}"#)
            }
            ("api.themoviedb.org", "/movie/190002") => {
                ok(r#"{"id":190002,"title":"Paper Lanterns",
                "original_title":"Paper Lanterns","original_language":"en","release_date":"2020-05-01",
                "status":"Released","runtime":101}"#)
            }
            ("api.tvmaze.com", "/lookup/shows") if param(r, "thetvdb") == Some("390001") => ok(
                r#"{"id":90001,"name":"The Late Desk","premiered":"2015-09-08",
                   "externals":{"thetvdb":390001,"imdb":"tt9000001"}}"#,
            ),
            ("api.tvmaze.com", "/search/shows") if param(r, "q") == Some("The Late Desk") => ok(
                r#"[{"score":0.9,"show":{"id":90001,"name":"The Late Desk","premiered":"2015-09-08",
                   "externals":{"thetvdb":390001,"imdb":"tt9000001"}}}]"#,
            ),
            ("api.tvmaze.com", "/shows/90001/episodesbydate") => match param(r, "date") {
                Some("2025-03-14") => ok(r#"[{"id":1901,"name":"TVmaze One","season":2025,
                    "number":31,"type":"regular","airdate":"2025-03-14","runtime":30,
                    "summary":"<p>From TVmaze.</p>"}]"#),
                Some("2025-03-18") => ok(r#"[{"id":1903,"name":"Guest Three","season":2025,
                    "number":33,"type":"regular","airdate":"2025-03-18","runtime":30,
                    "summary":"<p>Third <b>guest</b>.</p>"}]"#),
                _ => not_found(),
            },
            ("api4.thetvdb.com", "/series/390001/episodes/default")
                if param(r, "airDate") == Some("2025-03-19") =>
            {
                ok(
                    r#"{"status":"success","data":{"episodes":[{"id":8034,"name":"Guest Four",
                    "aired":"2025-03-19","runtime":31,"overview":"Fourth.","seasonNumber":10,
                    "number":34}]}}"#,
                )
            }
            ("api4.thetvdb.com", "/series/390001/episodes/default") => {
                ok(r#"{"status":"success","data":{"episodes":[]}}"#)
            }
            ("webservice.fanart.tv", "/tv/390001") => ok(r#"{"name":"The Late Desk",
                "hdtvlogo":[{"id":"1","url":"https://assets.fanart.tv/tv/logo-en.png","lang":"en","likes":"2"}],
                "hdclearart":[{"id":"2","url":"https://assets.fanart.tv/tv/art.png","lang":"00","likes":"1"}]}"#),
            ("api.opensubtitles.com", "/subtitles") => ok(r#"{"total_count":2,"data":[
                {"id":"1","attributes":{"subtitle_id":"1","moviehash_match":true,
                 "feature_details":{"feature_type":"Movie","title":"Paper Lanterns","year":2020,"tmdb_id":190002}}},
                {"id":"2","attributes":{"subtitle_id":"2","moviehash_match":true,
                 "feature_details":{"feature_type":"Movie","title":"Paper Lanterns","year":2020,"tmdb_id":190002}}}
            ]}"#),
            _ => not_found(),
        }
    }
}

struct Transport(Arc<Fake>);

impl HttpTransport for Transport {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        Ok(self.0.respond(r))
    }

    fn post_json(&self, r: &HttpRequest, _body: &str) -> Result<HttpResponse, TransportError> {
        Ok(if r.path == "/login" {
            self.0
                .calls
                .lock()
                .unwrap()
                .push("api4.thetvdb.com /login".into());
            ok(r#"{"status":"success","data":{"token":"t"}}"#)
        } else {
            not_found()
        })
    }
}

struct Options {
    tmdb: bool,
    tvdb: bool,
}

fn providers(fake: &Arc<Fake>, o: &Options) -> MetadataProviders {
    let t: Arc<dyn HttpTransport> = Arc::new(Transport(fake.clone()));
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let mut tmdb = TmdbConfig::default();
    tmdb.retry.max_retries = 0;
    let tmdb_keys = if o.tmdb {
        Keys::none().with_user_key(Some("test-key"))
    } else {
        Keys::none()
    };
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            tmdb,
            tmdb_keys,
            t.clone(),
            cache.clone(),
            clock.clone(),
        ))),
        anilist: None,
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
        extra: ExtraProviders {
            tvmaze: Some(Arc::new(TvMazeClient::new(
                TvMazeConfig::default(),
                t.clone(),
                cache.clone(),
                clock.clone(),
            ))),
            tvdb: o.tvdb.then(|| {
                Arc::new(TvdbClient::new(
                    TvdbConfig::default(),
                    Some("user-key"),
                    t.clone(),
                    cache.clone(),
                    clock.clone(),
                ))
            }),
            fanart: Some(Arc::new(FanartClient::new(
                FanartConfig::default(),
                Keys::none().with_project_key(Some("project-key")),
                t.clone(),
                cache.clone(),
                clock.clone(),
            ))),
            opensubtitles: Some(Arc::new(OpenSubtitlesClient::new(
                OpenSubtitlesConfig::default(),
                Keys::none().with_project_key(Some("project-key")),
                None,
                t,
                clock,
            ))),
        },
    }
}

const DAYS: [&str; 4] = ["2025.03.14", "2025.03.17", "2025.03.18", "2025.03.19"];

fn setup() -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    for (i, d) in DAYS.iter().enumerate() {
        media(
            &root,
            &format!("Shows/The Late Desk/The.Late.Desk.{d}.Guest.Name.1080p.WEB.h264-GRP.mkv"),
            u8::try_from(i).unwrap() + 1,
        );
    }
    media(&root, "Movies/Qvxzt Wobbly.mkv", 9);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    (dir, cat)
}

fn run(cat: &Catalog, p: &MetadataProviders) -> pa_catalog::MetadataReport {
    cat.fetch_metadata(
        p,
        &MetadataOptions::default(),
        &CancelToken::new(),
        &mut |_, _| {},
    )
    .unwrap()
}

/// `(air_date, season, episode, title)` of the show's episodes.
type Row = (String, Option<i64>, Option<i64>, Option<String>);

fn episodes(dir: &tempfile::TempDir) -> Vec<Row> {
    let db = Connection::open(dir.path().join("library.db")).unwrap();
    let mut stmt = db
        .prepare(
            "SELECT e.air_date, e.season_number, e.episode_number, e.title FROM episodes e
             JOIN works w ON w.id = e.work_id WHERE w.kind = 'show' ORDER BY e.air_date",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn row(date: &str, s: Option<i64>, e: Option<i64>, title: &str) -> Row {
    (date.to_owned(), s, e, Some(title.to_owned()))
}

#[test]
fn a_daily_show_is_matched_by_air_date() {
    let (dir, cat) = setup();
    // One row per date, not one row for all unnumbered files.
    assert_eq!(episodes(&dir).len(), 4, "{:?}", episodes(&dir));
    let fake = Arc::new(Fake::default());
    let report = run(
        &cat,
        &providers(
            &fake,
            &Options {
                tmdb: true,
                tvdb: true,
            },
        ),
    );
    assert!(report.error.is_none(), "{report:?}");
    assert_eq!(report.matched, 2, "{report:?}");

    assert_eq!(
        episodes(&dir),
        [
            // TMDB's season 10 aired these two: numbers and titles.
            row("2025-03-14", Some(10), Some(31), "Guest One"),
            row("2025-03-17", Some(10), Some(32), "Guest Two"),
            // TMDB lacks the date: TVmaze's episode, no TMDB numbers.
            row("2025-03-18", None, None, "Guest Three"),
            // Neither TMDB nor TVmaze: TheTVDB with the user's key.
            row("2025-03-19", None, None, "Guest Four"),
        ]
    );
    // Only the season covering the dates was asked; TVmaze only for the
    // dates TMDB lacks.
    assert_eq!(fake.called("api.themoviedb.org /tv/190001/season/"), 1);
    assert_eq!(fake.called("api.tvmaze.com /shows/90001/episodesbydate"), 2);

    let db = Connection::open(dir.path().join("library.db")).unwrap();
    let overview: String = db
        .query_row(
            "SELECT overview FROM episodes WHERE air_date = '2025-03-18'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(overview, "Third guest.");
    let tvdb: (i64, i64) = db
        .query_row(
            "SELECT m.season_number, m.episode_number FROM episode_mappings m
             JOIN episodes e ON e.id = m.episode_id
             WHERE e.air_date = '2025-03-19' AND m.scheme = 'tvdb_aired'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(tvdb, (10, 34));
    let ids: Vec<(String, String)> = db
        .prepare(
            "SELECT provider, ext_id FROM external_ids x JOIN works w ON w.id = x.work_id
             WHERE w.kind = 'show' ORDER BY provider",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        ids,
        [
            ("tmdb".to_owned(), "190001".to_owned()),
            ("tvdb".to_owned(), "390001".to_owned()),
            ("tvmaze".to_owned(), "90001".to_owned()),
        ]
    );

    // Fanart.tv: the clear logo and the clear art of the series.
    let show = cat
        .shows()
        .unwrap()
        .into_iter()
        .find(|s| s.title == "The Late Desk")
        .unwrap();
    let d = cat.details(show.id).unwrap();
    assert_eq!(
        d.logo_url.as_deref(),
        Some("https://assets.fanart.tv/tv/logo-en.png")
    );
    assert_eq!(
        d.clearart_url.as_deref(),
        Some("https://assets.fanart.tv/tv/art.png")
    );

    // A rescan finds the dated rows again by their dates: no duplicates.
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    assert_eq!(episodes(&dir).len(), 4);
    let shows: i64 = db
        .query_row("SELECT count(*) FROM works WHERE kind = 'show'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(shows, 1);
}

#[test]
fn without_a_tmdb_key_tvmaze_names_the_daily_episodes() {
    let (dir, cat) = setup();
    let fake = Arc::new(Fake::default());
    let report = run(
        &cat,
        &providers(
            &fake,
            &Options {
                tmdb: false,
                tvdb: false,
            },
        ),
    );
    assert!(report.error.is_none(), "{report:?}");
    // The show is not matched (TMDB is not asked), the movie not asked.
    assert_eq!((report.works, report.matched), (1, 0), "{report:?}");
    assert_eq!(fake.called("api.themoviedb.org"), 0);
    assert_eq!(fake.called("api4.thetvdb.com"), 0);
    let rows = episodes(&dir);
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(
        rows.iter().all(|r| r.1.is_none() && r.2.is_none()),
        "{rows:?}"
    );
    assert_eq!(rows[0], row("2025-03-14", None, None, "TVmaze One"));
    assert_eq!(rows[2], row("2025-03-18", None, None, "Guest Three"));
    // TVmaze has nothing for these days: the names stay as the files say.
    assert_ne!(rows[1].3.as_deref(), Some("Guest Two"));
    let db = Connection::open(dir.path().join("library.db")).unwrap();
    let tvmaze: String = db
        .query_row(
            "SELECT ext_id FROM external_ids WHERE provider = 'tvmaze'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tvmaze, "90001");
}

#[test]
fn a_movie_tmdb_cannot_find_is_found_by_its_file_hash() {
    let (dir, cat) = setup();
    let fake = Arc::new(Fake::default());
    run(
        &cat,
        &providers(
            &fake,
            &Options {
                tmdb: true,
                tvdb: false,
            },
        ),
    );
    let movie = cat
        .movies()
        .unwrap()
        .into_iter()
        .find(|m| m.tmdb_id == Some(190_002))
        .expect("matched by hash");
    assert_eq!(movie.title, "Paper Lanterns");
    let db = Connection::open(dir.path().join("library.db")).unwrap();
    let (state, confidence, explanation): (String, f64, String) = db
        .query_row(
            "SELECT m.state, m.confidence, m.explanation_json FROM matches m
             JOIN files f ON f.id = m.file_id WHERE f.path LIKE '%Qvxzt%'",
            params![],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    // Two subtitles made for this very file agree: automatic.
    assert_eq!(state, "auto");
    assert!(confidence > 0.92, "{confidence}");
    assert!(explanation.contains("opensubtitles"), "{explanation}");
    // Only the hash went to OpenSubtitles.
    let calls = fake.calls.lock().unwrap().clone();
    let os: Vec<_> = calls
        .iter()
        .filter(|c| c.starts_with("api.opensubtitles.com"))
        .collect();
    assert_eq!(os.len(), 1, "{calls:?}");
    assert!(os[0].starts_with("api.opensubtitles.com /subtitles?moviehash="));
    assert!(!os[0].contains("Qvxzt"));
}
