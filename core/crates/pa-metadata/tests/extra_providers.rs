// SPDX-License-Identifier: MPL-2.0
//! META-012 clients against recorded (synthetic) responses: TVmaze daily
//! shows by date, TheTVDB with the user's key, Fanart.tv art and the
//! OpenSubtitles moviehash → id lookup. No network: responses come from the
//! cassettes in `fixtures/`, time from a manual clock.

use std::sync::Arc;
use std::time::Duration;

use pa_metadata::cache::MemoryCache;
use pa_metadata::clock::{Clock, ManualClock};
use pa_metadata::fanart::{FanartClient, FanartConfig, Target};
use pa_metadata::fetch::FetchError;
use pa_metadata::key::{KeySource, Keys};
use pa_metadata::opensubtitles::{OpenSubtitlesClient, OpenSubtitlesConfig};
use pa_metadata::replay::ReplayTransport;
use pa_metadata::tvdb::{TvdbClient, TvdbConfig, TvdbError};
use pa_metadata::tvmaze::{Lookup, TvMazeClient, TvMazeConfig, plain_text};

const TVMAZE: &str = include_str!("fixtures/tvmaze/cassette.jsonl");
const TVDB: &str = include_str!("fixtures/tvdb/cassette.jsonl");
const FANART: &str = include_str!("fixtures/fanart/cassette.jsonl");
const IDENTIFY: &str = include_str!("fixtures/opensubtitles/identify.jsonl");
const T0: u64 = 1_800_000_000;

fn tvmaze() -> (TvMazeClient, Arc<ReplayTransport>, Arc<ManualClock>) {
    let t = Arc::new(ReplayTransport::from_jsonl(TVMAZE).unwrap());
    let clock = Arc::new(ManualClock::at_unix(T0));
    let c = TvMazeClient::new(
        TvMazeConfig::default(),
        t.clone(),
        Arc::new(MemoryCache::default()),
        clock.clone(),
    );
    (c, t, clock)
}

fn tvdb(key: Option<&str>) -> (TvdbClient, Arc<ReplayTransport>) {
    let t = Arc::new(ReplayTransport::from_jsonl(TVDB).unwrap());
    let c = TvdbClient::new(
        TvdbConfig::default(),
        key,
        t.clone(),
        Arc::new(MemoryCache::default()),
        Arc::new(ManualClock::at_unix(T0)),
    );
    (c, t)
}

#[test]
fn tvmaze_finds_a_daily_show_episode_by_its_air_date() {
    let (c, t, _) = tvmaze();
    let hits = c.search("The Late Desk").unwrap();
    assert_eq!(hits.len(), 2);
    let show = &hits[0].show;
    assert_eq!((show.id, show.year()), (90001, Some(2015)));
    assert_eq!(show.externals.thetvdb, Some(390001));
    assert_eq!(
        plain_text(show.summary.as_deref().unwrap()),
        "A nightly news satire with guests."
    );

    let eps = c.episodes_by_date(90001, "2025-03-14").unwrap();
    assert_eq!(eps.len(), 1);
    let e = &eps[0];
    assert!(e.is_regular());
    assert_eq!((e.season, e.number), (Some(2025), Some(31)));
    assert_eq!(e.airdate.as_deref(), Some("2025-03-14"));
    assert_eq!(e.name.as_deref(), Some("Guest One; Musical Guest"));

    // No episode that day (TVmaze answers 404): empty, not an error.
    assert!(c.episodes_by_date(90001, "2025-03-15").unwrap().is_empty());
    // Not a date: nothing is sent.
    let before = t.requests().len();
    assert!(c.episodes_by_date(90001, "2025-3-15").unwrap().is_empty());
    assert_eq!(t.requests().len(), before);

    // Answers are cached: asking again sends nothing.
    c.episodes_by_date(90001, "2025-03-14").unwrap();
    assert_eq!(t.requests().len(), before);
}

#[test]
fn tvmaze_lookup_by_tvdb_and_imdb_ids() {
    let (c, _, _) = tvmaze();
    let s = c.lookup(Lookup::TheTvdb(390001)).unwrap().unwrap();
    assert_eq!(s.id, 90001);
    let s = c.lookup(Lookup::Imdb("tt9000001")).unwrap().unwrap();
    assert_eq!(s.name, "The Late Desk");
    assert_eq!(c.lookup(Lookup::TheTvdb(399999)).unwrap(), None);
    // Not an IMDb id: nothing asked.
    assert_eq!(c.lookup(Lookup::Imdb("9000001")).unwrap(), None);
}

#[test]
fn tvmaze_waits_out_a_429() {
    let (c, t, clock) = tvmaze();
    let start = clock.monotonic();
    assert!(c.search("Busy Hour").unwrap().is_empty());
    assert_eq!(t.requests().len(), 2);
    assert!(clock.monotonic() - start >= Duration::from_secs(2));
}

#[test]
fn tvdb_logs_in_with_the_users_key_and_finds_episodes_by_air_date() {
    let (c, t) = tvdb(Some("test-key"));
    let found = c.search_series("The Late Desk", None).unwrap();
    assert_eq!(found[0].id(), Some(390001));
    assert_eq!(found[0].year(), Some(2015));

    let s = c.series(390001).unwrap().unwrap();
    assert_eq!(s.tmdb_id(), Some(190001));
    assert_eq!(c.series(390404).unwrap(), None);

    let eps = c.episodes_by_air_date(390001, "2025-03-17").unwrap();
    assert_eq!(eps.len(), 1);
    assert_eq!((eps[0].season_number, eps[0].number), (Some(10), Some(32)));
    assert_eq!(eps[0].name.as_deref(), Some("Guest Two"));

    let requests = t.requests();
    // One login for all of them; the token goes as a bearer header and
    // never into the cache key.
    assert_eq!(requests.iter().filter(|r| r.path == "/login").count(), 1);
    let search = requests.iter().find(|r| r.path == "/search").unwrap();
    assert!(
        search
            .headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer synthetic-token")
    );
    assert!(!search.canonical().contains("synthetic-token"));
}

#[test]
fn tvdb_logs_in_again_after_a_401() {
    let (c, t) = tvdb(Some("test-key"));
    let s = c.series(390002).unwrap().unwrap();
    assert_eq!(s.name.as_deref(), Some("Other Desk"));
    assert_eq!(
        t.requests().iter().filter(|r| r.path == "/login").count(),
        2
    );
}

#[test]
fn tvdb_sends_the_subscriber_pin_with_the_key() {
    let t = Arc::new(ReplayTransport::from_jsonl(TVDB).unwrap());
    let c = TvdbClient::new(
        TvdbConfig::default(),
        Some("subscriber-key"),
        t.clone(),
        Arc::new(MemoryCache::default()),
        Arc::new(ManualClock::at_unix(T0)),
    )
    .with_pin(Some("1234"));
    assert_eq!(c.search_series("The Late Desk", None).unwrap().len(), 1);
}

#[test]
fn tvdb_without_a_key_or_with_a_wrong_one() {
    let (c, t) = tvdb(None);
    assert_eq!(c.search_series("x", None), Err(TvdbError::NoKey));
    assert!(t.requests().is_empty());
    let (c, _) = tvdb(Some("wrong-key"));
    assert!(matches!(
        c.search_series("The Late Desk", None),
        Err(TvdbError::KeyRejected(m)) if m.contains("not valid")
    ));
}

#[test]
fn fanart_picks_logo_and_clearart() {
    let t = Arc::new(ReplayTransport::from_jsonl(FANART).unwrap());
    let c = FanartClient::new(
        FanartConfig::default(),
        Keys::none().with_project_key(Some("project-key")),
        t.clone(),
        Arc::new(MemoryCache::default()),
        Arc::new(ManualClock::at_unix(T0)),
    );
    assert_eq!(c.key_source(), Some(KeySource::Project));
    // A Russian logo exists in HD: it wins over a more liked English one.
    let art = c.art(Target::Movie(190002), "ru").unwrap();
    assert_eq!(art.logo.unwrap().id, "700002");
    // No HD clear art: the SD one.
    assert_eq!(art.clearart.unwrap().id, "700004");
    // English interface: the English HD logo.
    let art = c.art(Target::Movie(190002), "en").unwrap();
    assert_eq!(art.logo.unwrap().id, "700001");

    let art = c.art(Target::Series(390001), "ru").unwrap();
    assert_eq!(art.logo.unwrap().id, "800001");
    assert_eq!(art.clearart.unwrap().id, "800003");

    assert!(c.art(Target::Movie(190404), "ru").unwrap().is_empty());
    assert!(
        t.requests()
            .iter()
            .all(|r| !r.canonical().contains("project-key"))
    );
}

#[test]
fn fanart_server_errors_are_errors() {
    let t = Arc::new(ReplayTransport::default());
    t.push(
        "/movies/1",
        pa_metadata::http::HttpResponse {
            status: 403,
            headers: vec![],
            body: r#"{"status":"error","error message":"Invalid API key"}"#.into(),
        },
    );
    let c = FanartClient::new(
        FanartConfig::default(),
        Keys::none().with_project_key(Some("k")),
        t,
        Arc::new(MemoryCache::default()),
        Arc::new(ManualClock::at_unix(T0)),
    );
    assert!(matches!(
        c.art(Target::Movie(1), "en"),
        Err(FetchError::Http { status: 403, .. })
    ));
}

#[test]
fn opensubtitles_moviehash_names_the_episode_or_movie() {
    let t = Arc::new(ReplayTransport::from_jsonl(IDENTIFY).unwrap());
    let c = OpenSubtitlesClient::new(
        OpenSubtitlesConfig::default(),
        Keys::none().with_project_key(Some("project-key-1")),
        None,
        t.clone(),
        Arc::new(ManualClock::at_unix(T0)),
    );
    let m = c.identify("0123456789ABCDEF").unwrap().unwrap();
    assert!(m.episode);
    assert_eq!((m.tmdb_id, m.season, m.number), (190200, Some(2), Some(3)));
    assert_eq!(m.title.as_deref(), Some("Paper Lanterns"));
    // Two subtitles agree, one names another episode; the one not made
    // for this hash does not count.
    assert_eq!((m.votes, m.against), (2, 1));

    let m = c.identify("fedcba9876543210").unwrap().unwrap();
    assert!(!m.episode);
    assert_eq!((m.tmdb_id, m.year, m.votes), (190002, Some(2020), 1));

    assert_eq!(c.identify("1111111111111111").unwrap(), None);
    // Not a hash: nothing is sent.
    let before = t.requests().len();
    assert_eq!(c.identify("not-a-hash").unwrap(), None);
    assert_eq!(t.requests().len(), before);
}

#[test]
fn opensubtitles_identify_needs_a_key() {
    let c = OpenSubtitlesClient::new(
        OpenSubtitlesConfig::default(),
        Keys::none(),
        None,
        Arc::new(ReplayTransport::from_jsonl(IDENTIFY).unwrap()),
        Arc::new(ManualClock::at_unix(T0)),
    );
    assert!(c.identify("0123456789abcdef").is_err());
}
