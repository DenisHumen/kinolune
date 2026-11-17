// SPDX-License-Identifier: MPL-2.0
//! OpenSubtitles client (PLAY-008) against the synthetic cassette
//! `fixtures/opensubtitles/cassette.jsonl`. No network: every response
//! comes from the cassette, time from a manual clock.

use std::sync::Arc;
use std::time::Duration;

use pa_metadata::clock::ManualClock;
use pa_metadata::key::{KeySource, Keys};
use pa_metadata::opensubtitles::{
    Account, OpenSubtitlesClient, OpenSubtitlesConfig, OpenSubtitlesError, SearchQuery,
    search_codes,
};
use pa_metadata::replay::ReplayTransport;

const CASSETTE: &str = include_str!("fixtures/opensubtitles/cassette.jsonl");
const SAMPLE_ASS: &str = include_str!("fixtures/opensubtitles/sample.ass");
const HASH: &str = "1a2b3c4d5e6f7081";

struct Fixture {
    client: OpenSubtitlesClient,
    transport: Arc<ReplayTransport>,
    clock: Arc<ManualClock>,
}

fn fixture(keys: Keys, account: Option<Account>) -> Fixture {
    let transport = Arc::new(ReplayTransport::from_jsonl(CASSETTE).unwrap());
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let client = OpenSubtitlesClient::new(
        OpenSubtitlesConfig::default(),
        keys,
        account,
        Arc::clone(&transport) as _,
        Arc::clone(&clock) as _,
    );
    Fixture {
        client,
        transport,
        clock,
    }
}

fn project_key() -> Keys {
    Keys::none().with_project_key(Some("project-key-1"))
}

fn account(password: &str) -> Option<Account> {
    Account::new("test-user", password)
}

fn langs(codes: &[&str]) -> Vec<String> {
    codes
        .iter()
        .flat_map(|c| search_codes(c))
        .map(str::to_owned)
        .collect()
}

#[test]
fn search_by_hash_and_imdb_id() {
    let f = fixture(project_key(), None);
    let hits = f
        .client
        .search(&SearchQuery {
            moviehash: Some(HASH.into()),
            imdb_id: Some("tt0900001".into()),
            languages: langs(&["rus", "eng", "por"]),
            kind: Some("movie".into()),
            ..SearchQuery::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 6);
    let ass = hits.iter().find(|h| h.subtitle_id == "9002").unwrap();
    assert!(ass.moviehash_match);
    assert_eq!(ass.language.as_deref(), Some("ru"));
    assert_eq!(ass.download_count, 300);
    assert_eq!(ass.single_file().unwrap().file_id, 7002);
    let ai = hits.iter().find(|h| h.subtitle_id == "9006").unwrap();
    assert!(ai.ai_translated && !ai.moviehash_match);
    let cds = hits.iter().find(|h| h.subtitle_id == "9004").unwrap();
    assert!(cds.single_file().is_none(), "two CDs are not one file");

    // Every request carries the key and the app's User-Agent; the key is
    // never part of the recorded request.
    let sent = f.transport.requests();
    let headers = &sent[0].headers;
    assert!(headers.contains(&("Api-Key".into(), "project-key-1".into())));
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.starts_with("Kinolune v"))
    );
    assert!(!sent[0].canonical().contains("project-key-1"));
}

#[test]
fn episode_search_by_show_season_and_number() {
    let f = fixture(project_key(), None);
    let hits = f
        .client
        .search(&SearchQuery {
            moviehash: Some("00000000000000ff".into()),
            parent_tmdb_id: Some(800_002),
            season_number: Some(1),
            episode_number: Some(2),
            languages: langs(&["ru", "en"]),
            kind: Some("episode".into()),
            ..SearchQuery::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 2);
    let details = hits[0].feature_details.as_ref().unwrap();
    assert_eq!((details.season_number, details.episode_number), (Some(1), Some(2)));
}

#[test]
fn no_key_sends_nothing() {
    let f = fixture(Keys::none(), None);
    assert_eq!(f.client.key_source(), None);
    let err = f
        .client
        .search(&SearchQuery {
            moviehash: Some(HASH.into()),
            ..SearchQuery::default()
        })
        .unwrap_err();
    assert_eq!(err, OpenSubtitlesError::NoKey);
    assert!(f.transport.requests().is_empty());
}

#[test]
fn user_key_wins_over_project_key() {
    let f = fixture(project_key().with_user_key(Some("user-key-1")), None);
    assert_eq!(f.client.key_source(), Some(KeySource::User));
    f.client
        .search(&SearchQuery {
            moviehash: Some(HASH.into()),
            languages: langs(&["en", "ru"]),
            ..SearchQuery::default()
        })
        .unwrap();
    let sent = f.transport.requests();
    assert!(
        sent[0]
            .headers
            .contains(&("Api-Key".into(), "user-key-1".into()))
    );
}

#[test]
fn throttled_search_waits_and_retries() {
    let f = fixture(project_key(), None);
    let hits = f
        .client
        .search(&SearchQuery {
            moviehash: Some("ffffffffffffffff".into()),
            ..SearchQuery::default()
        })
        .unwrap();
    assert!(hits.is_empty());
    assert_eq!(f.transport.requests().len(), 2);
    assert!(f.clock.sleeps().contains(&Duration::from_secs(1)));
}

#[test]
fn login_quota_and_download_of_an_ass_file() {
    let f = fixture(project_key(), account("test-password"));
    let info = f.client.user_info().unwrap().unwrap();
    assert_eq!(info.allowed_downloads, Some(20));
    assert_eq!(info.remaining_downloads, Some(19));
    let quota = f.client.quota().unwrap();
    assert!(quota.logged_in);
    assert_eq!((quota.remaining, quota.allowed), (Some(19), Some(20)));

    let link = f.client.download(7002).unwrap();
    assert_eq!(link.remaining, Some(19));
    assert_eq!(
        link.file_name.as_deref(),
        Some("Lunar.Harbor.2019.1080p.WEB-DL.ass")
    );
    let quota = f.client.quota().unwrap();
    assert_eq!(quota.remaining, Some(19));
    assert_eq!(quota.allowed, Some(20));
    assert_eq!(
        quota.reset_time_utc.as_deref(),
        Some("2026-11-18T20:00:00.000Z")
    );

    // One login for both calls; the token goes to the API, not to the CDN.
    let sent = f.transport.requests();
    let paths: Vec<&str> = sent.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["/login", "/infos/user", "/download"]);
    let bearer = ("Authorization".to_owned(), "Bearer synthetic-token-not-a-secret".to_owned());
    assert!(sent[1].headers.contains(&bearer));
    assert!(sent[2].headers.contains(&bearer));

    let bytes = f.client.fetch(&link).unwrap();
    let cdn = f.transport.requests().pop().unwrap();
    assert_eq!(cdn.base, "https://www.opensubtitles.com");
    assert!(
        cdn.headers
            .iter()
            .all(|(k, _)| k != "Authorization" && k != "Api-Key")
    );
    assert_eq!(bytes, SAMPLE_ASS.as_bytes(), "the file comes as it is");
}

#[test]
fn downloaded_ass_keeps_its_styles() {
    let f = fixture(project_key(), None);
    let link = f.client.download(7002).unwrap();
    let text = String::from_utf8(f.client.fetch(&link).unwrap()).unwrap();
    assert!(text.starts_with("[Script Info]"));
    assert!(text.contains("[V4+ Styles]"));
    assert!(text.contains("Style: Sign,Georgia,48,&H0000D7FF"));
    assert!(text.contains(r"{\pos(960,120)\fad(200,200)}"));
    // Without an account the quota is the address's.
    assert!(!f.client.quota().unwrap().logged_in);
}

#[test]
fn quota_used_up_is_not_retried() {
    let f = fixture(project_key(), account("test-password"));
    let err = f.client.download(7003).unwrap_err();
    assert_eq!(
        err,
        OpenSubtitlesError::QuotaExhausted {
            reset_time_utc: Some("2026-11-18T03:00:00.000Z".into())
        }
    );
    let downloads = f
        .transport
        .requests()
        .iter()
        .filter(|r| r.path == "/download")
        .count();
    assert_eq!(downloads, 1);
}

#[test]
fn wrong_password_is_a_login_error() {
    let f = fixture(project_key(), account("wrong"));
    let err = f.client.user_info().unwrap_err();
    assert!(matches!(err, OpenSubtitlesError::Login(_)), "{err:?}");
}

#[test]
fn without_an_account_there_is_no_user_info() {
    let f = fixture(project_key(), None);
    assert_eq!(f.client.user_info().unwrap(), None);
    assert!(f.transport.requests().is_empty());
}
