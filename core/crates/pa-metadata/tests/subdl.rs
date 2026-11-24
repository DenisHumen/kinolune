// SPDX-License-Identifier: MPL-2.0
//! Subdl client (PLAY-016) against the synthetic cassette and archives of
//! `fixtures/subdl/`. No network: searches come from the cassette, archive
//! bytes from the fixture files, time from a manual clock.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use pa_metadata::clock::ManualClock;
use pa_metadata::http::{BytesResponse, HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_metadata::key::{KeySource, Keys};
use pa_metadata::replay::ReplayTransport;
use pa_metadata::subdl::{
    SubdlClient, SubdlConfig, SubdlError, SubdlQuery, Wanted, extract_subtitle, search_codes,
};

const CASSETTE: &str = include_str!("fixtures/subdl/cassette.jsonl");
const SAMPLE_ASS: &[u8] = include_bytes!("fixtures/subdl/sample.ass");
const MOVIE_ZIP: &[u8] = include_bytes!("fixtures/subdl/movie.zip");
const SEASON_ZIP: &[u8] = include_bytes!("fixtures/subdl/season.zip");
const SINGLE_ZIP: &[u8] = include_bytes!("fixtures/subdl/single.zip");

/// Searches from the cassette; archives by path, as binary bodies.
struct Server {
    replay: ReplayTransport,
    files: HashMap<String, &'static [u8]>,
}

impl HttpTransport for Server {
    fn get(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.replay.get(request)
    }

    fn get_bytes(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
        match self.files.get(&request.path) {
            Some(body) => Ok(BytesResponse {
                status: 200,
                headers: Vec::new(),
                body: body.to_vec(),
            }),
            None => Ok(BytesResponse {
                status: 404,
                headers: Vec::new(),
                body: b"not found".to_vec(),
            }),
        }
    }
}

struct Fixture {
    client: SubdlClient,
    server: Arc<Server>,
    clock: Arc<ManualClock>,
}

fn fixture(keys: Keys) -> Fixture {
    let server = Arc::new(Server {
        replay: ReplayTransport::from_jsonl(CASSETTE).unwrap(),
        files: HashMap::from([
            ("/subtitle/900101-900201.zip".to_owned(), MOVIE_ZIP),
            ("/subtitle/900302-900402.zip".to_owned(), SEASON_ZIP),
            ("/subtitle/900301-900401.zip".to_owned(), SINGLE_ZIP),
        ]),
    });
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let client = SubdlClient::new(
        SubdlConfig::default(),
        keys,
        Arc::clone(&server) as _,
        Arc::clone(&clock) as _,
    );
    Fixture {
        client,
        server,
        clock,
    }
}

fn project_key() -> Keys {
    Keys::none().with_project_key(Some("subdl-project-key"))
}

fn langs(iso: &[&str]) -> Vec<String> {
    iso.iter()
        .flat_map(|c| search_codes(c))
        .map(str::to_owned)
        .collect()
}

fn movie_query() -> SubdlQuery {
    SubdlQuery {
        imdb_id: Some("tt900001".into()),
        tmdb_id: Some(800_001),
        kind: Some("movie".into()),
        languages: langs(&["rus", "eng"]),
        ..SubdlQuery::default()
    }
}

#[test]
fn without_a_key_nothing_is_sent() {
    let f = fixture(Keys::none());
    assert_eq!(f.client.key_source(), None);
    assert_eq!(f.client.search(&movie_query()), Err(SubdlError::NoKey));
    assert!(f.server.replay.requests().is_empty());
}

#[test]
fn movie_search_by_imdb_reads_every_field() {
    let f = fixture(project_key());
    assert_eq!(f.client.key_source(), Some(KeySource::Project));
    let found = f.client.search(&movie_query()).unwrap();
    assert_eq!(found.len(), 3);
    let ru = &found[0];
    assert_eq!(ru.language.as_deref(), Some("RU"));
    assert_eq!(ru.url, "/subtitle/900101-900201.zip");
    assert_eq!(ru.download_count, 120);
    assert!(!ru.is_pack() && !ru.hearing_impaired());
    assert!(found[1].hearing_impaired());
    assert_eq!(found[2].language.as_deref(), Some("BR_PT"));
    // The key went out as `api_key`, but the cassette never saw it.
    let sent = &f.server.replay.requests()[0];
    assert!(
        sent.query
            .iter()
            .any(|(k, v)| k == "api_key" && v == "subdl-project-key")
    );
    assert!(!sent.canonical().contains("subdl-project-key"));
}

#[test]
fn episode_search_waits_out_a_rate_limit_and_filters_by_episode() {
    let f = fixture(project_key());
    let found = f
        .client
        .search(&SubdlQuery {
            imdb_id: Some("tt0900002".into()),
            kind: Some("tv".into()),
            season_number: Some(1),
            episode_number: Some(2),
            languages: langs(&["rus", "eng"]),
            ..SubdlQuery::default()
        })
        .unwrap();
    assert_eq!(f.server.replay.requests().len(), 2, "429, then 200");
    assert!(f.clock.sleeps().contains(&Duration::from_secs(3)));
    let fitting: Vec<&str> = found
        .iter()
        .filter(|s| s.fits_episode(1, 2))
        .map(|s| s.url.as_str())
        .collect();
    assert_eq!(
        fitting,
        ["/subtitle/900301-900401.zip", "/subtitle/900302-900402.zip"]
    );
    assert!(found[1].is_pack());
}

#[test]
fn nothing_found_is_empty_not_an_error() {
    let f = fixture(project_key());
    let found = f
        .client
        .search(&SubdlQuery {
            film_name: Some("Lunar Harbor".into()),
            year: Some(2019),
            kind: Some("movie".into()),
            languages: langs(&["rus", "eng"]),
            ..SubdlQuery::default()
        })
        .unwrap();
    assert!(found.is_empty());
}

#[test]
fn daily_limit_and_refused_key_are_not_retried() {
    let f = fixture(project_key());
    let q = |tmdb| SubdlQuery {
        tmdb_id: Some(tmdb),
        kind: Some("movie".into()),
        languages: langs(&["eng"]),
        ..SubdlQuery::default()
    };
    assert_eq!(f.client.search(&q(800_009)), Err(SubdlError::QuotaExhausted));
    assert_eq!(
        f.client.search(&q(800_010)),
        Err(SubdlError::KeyRejected(403))
    );
    assert_eq!(f.server.replay.requests().len(), 2);
}

#[test]
fn movie_zip_gives_the_ass_byte_for_byte() {
    let f = fixture(project_key());
    let bytes = f.client.fetch("/subtitle/900101-900201.zip").unwrap();
    let got = extract_subtitle(&bytes, "/subtitle/900101-900201.zip", Wanted::default()).unwrap();
    assert_eq!(got.name, "Lunar.Harbor.2019.1080p.BluRay.ass");
    assert_eq!(got.bytes, SAMPLE_ASS);
    assert!(
        String::from_utf8(got.bytes)
            .unwrap()
            .contains("Style: Sign,Georgia")
    );
}

#[test]
fn season_pack_gives_the_episode_and_its_sdh_variant() {
    let want = |episode, sdh| Wanted {
        season: Some(1),
        episode: Some(episode),
        hearing_impaired: sdh,
    };
    let two = extract_subtitle(SEASON_ZIP, "x.zip", want(2, false)).unwrap();
    assert_eq!(two.name, "Quiet.Coast.S01E02.1080p.WEB.srt");
    assert!(String::from_utf8(two.bytes).unwrap().contains("Серия 2"));
    let sdh = extract_subtitle(SEASON_ZIP, "x.zip", want(2, true)).unwrap();
    assert_eq!(sdh.name, "Quiet.Coast.S01E02.1080p.WEB.SDH.srt");
    let three = extract_subtitle(SEASON_ZIP, "x.zip", want(3, false)).unwrap();
    assert!(String::from_utf8(three.bytes).unwrap().contains("Серия 3"));
    assert_eq!(
        extract_subtitle(SEASON_ZIP, "x.zip", want(9, false)),
        Err(SubdlError::NotInArchive)
    );
    let other_season = Wanted {
        season: Some(2),
        ..want(2, false)
    };
    assert_eq!(
        extract_subtitle(SEASON_ZIP, "x.zip", other_season),
        Err(SubdlError::NotInArchive)
    );
}

#[test]
fn single_file_archive_and_bare_files() {
    let one = extract_subtitle(
        SINGLE_ZIP,
        "x.zip",
        Wanted {
            season: Some(1),
            episode: Some(4),
            hearing_impaired: false,
        },
    )
    .unwrap();
    assert_eq!(one.name, "quiet coast ru.srt");
    let bare = extract_subtitle(b"1\n00:00:01,000 --> 00:00:02,000\nHi\n", "/s/x.srt", Wanted::default())
        .unwrap();
    assert_eq!(bare.name, "x.srt");
    assert!(matches!(
        extract_subtitle(b"Rar!\x1a\x07\x00rest", "/s/x.zip", Wanted::default()),
        Err(SubdlError::Decode(_))
    ));
}

#[test]
fn links_outside_subdl_are_refused() {
    let f = fixture(project_key());
    assert!(matches!(
        f.client.fetch("https://evil.example/x.zip"),
        Err(SubdlError::Decode(_))
    ));
    assert!(matches!(
        f.client.fetch("/subtitle/missing.zip"),
        Err(SubdlError::Http { status: 404, .. })
    ));
}
