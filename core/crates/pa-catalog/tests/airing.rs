// SPDX-License-Identifier: MPL-2.0
//! UI-057: when the next episode of an ongoing show or anime airs, on
//! synthetic TMDB, AniList and Shikimori answers (no network, no real
//! provider data): airing, on a break, finished, a date in the past, no
//! data, offline (the last value stays, marked), the file of the next
//! episode arriving, and the daily background refresh.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pa_catalog::pa_metadata::anilist::{AniListClient, AniListConfig};
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::shikimori::{ShikimoriClient, ShikimoriConfig};
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{
    AiringStatus, CancelToken, Catalog, ItemRef, MetadataProviders, NextEpisode, ShowAiring,
};
use pa_library::rusqlite::{Connection, params};

/// 2026-11-26 12:00 UTC.
const NOW_S: u64 = 1_795_694_400;
const NOW: i64 = NOW_S as i64 * 1000;
const HOUR: i64 = 3600 * 1000;
const DAY: i64 = 24 * HOUR;

/// Answers by path (AniList: by the `id` variable); counts requests.
#[derive(Default)]
struct Fake {
    answers: Mutex<HashMap<String, (u16, String)>>,
    calls: Mutex<Vec<String>>,
    offline: Mutex<bool>,
}

impl Fake {
    fn set(&self, key: &str, status: u16, body: &str) {
        self.answers
            .lock()
            .unwrap()
            .insert(key.to_owned(), (status, body.to_owned()));
    }

    fn offline(&self, on: bool) {
        *self.offline.lock().unwrap() = on;
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn respond(&self, key: String) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(key.clone());
        if *self.offline.lock().unwrap() {
            return Err(TransportError("offline".into()));
        }
        let (status, body) = self
            .answers
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or((404, r#"{"status_code":34}"#.to_owned()));
        Ok(HttpResponse {
            status,
            headers: vec![],
            body,
        })
    }
}

struct Transport(Arc<Fake>);

impl HttpTransport for Transport {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.0.respond(r.path.clone())
    }

    fn post_json(&self, _r: &HttpRequest, body: &str) -> Result<HttpResponse, TransportError> {
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let id = v["variables"]["id"].as_u64().unwrap_or(0);
        self.0.respond(format!("anilist {id}"))
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    cat: Catalog,
    conn: Connection,
    fake: Arc<Fake>,
    clock: Arc<ManualClock>,
    providers: MetadataProviders,
    next_file: i64,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let cat = Catalog::open(&path).unwrap();
        let conn = pa_library::open(&path).unwrap();
        conn.execute(
            "INSERT INTO sources (id, kind, name, uri, created_at_ms)
             VALUES (1, 'local', 'NAS', 'file:///srv/nas', 0)",
            [],
        )
        .unwrap();
        let fake = Arc::new(Fake::default());
        let clock = Arc::new(ManualClock::at_unix(NOW_S));
        let transport: Arc<dyn HttpTransport> = Arc::new(Transport(fake.clone()));
        let cache = Arc::new(MemoryCache::default());
        let mut tmdb = TmdbConfig::default();
        tmdb.retry.max_retries = 0;
        let mut anilist = AniListConfig::default();
        anilist.retry.max_retries = 0;
        let mut shikimori = ShikimoriConfig::default();
        shikimori.retry.max_retries = 0;
        let providers = MetadataProviders {
            tmdb: Some(Arc::new(TmdbClient::new(
                tmdb,
                Keys::none().with_user_key(Some("test-key")),
                transport.clone(),
                cache.clone(),
                clock.clone(),
            ))),
            anilist: Some(Arc::new(AniListClient::new(
                anilist,
                transport.clone(),
                cache.clone(),
                clock.clone(),
            ))),
            shikimori: Some(Arc::new(ShikimoriClient::new(
                shikimori,
                transport,
                cache,
                clock.clone(),
            ))),
            ..Default::default()
        };
        Self {
            _dir: dir,
            cat,
            conn,
            fake,
            clock,
            providers,
            next_file: 1,
        }
    }

    fn work(&self, id: i64, kind: &str, ids: &[(&str, &str, &str)]) {
        self.conn
            .execute(
                "INSERT INTO works (id, kind, title, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, 'Show ' || ?1, 0, 0)",
                params![id, kind],
            )
            .unwrap();
        for (provider, ext_kind, ext_id) in ids {
            self.conn
                .execute(
                    "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, provider, ext_kind, ext_id],
                )
                .unwrap();
        }
    }

    /// Episode `season`×`number` of `work` with a file in the library.
    fn episode_file(&mut self, work: i64, season: i64, number: i64) {
        let id = self.next_file;
        self.next_file += 1;
        self.conn
            .execute(
                "INSERT INTO episodes (id, work_id, season_number, episode_number)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, work, season, number],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO files (id, source_id, path, size_bytes, added_at_ms, fingerprint)
                 VALUES (?1, 1, '/Shows/' || ?1 || '.mkv', 1000, 0, printf('%08x', ?1))",
                [id],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO matches (id, file_id, work_id, confidence, state, decided_by,
                                      created_at_ms, updated_at_ms)
                 VALUES (?1, ?1, ?2, 0.9, 'auto', 'rule', 0, 0)",
                params![id, work],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO match_episodes (match_id, position, episode_id) VALUES (?1, 0, ?1)",
                [id],
            )
            .unwrap();
    }

    fn refresh(&self, work: i64, now: i64) -> Option<ShowAiring> {
        self.cat
            .refresh_airing(&self.providers, ItemRef::Show(work), now)
            .unwrap()
    }

    fn shown(&self, work: i64, now: i64) -> Option<ShowAiring> {
        self.cat.show_airing(ItemRef::Show(work), now).unwrap()
    }

    /// Moves the provider clients' clock (their cache freshness).
    fn later(&self, ms: i64) {
        self.clock
            .advance(Duration::from_millis(u64::try_from(ms).unwrap()));
    }
}

fn tmdb_tv(id: u64, status: &str, next: Option<(u32, u32, &str)>) -> String {
    let next = next.map_or("null".to_owned(), |(s, e, d)| {
        format!(
            r#"{{"season_number":{s},"episode_number":{e},"air_date":"{d}","name":"Эпизод {e}"}}"#
        )
    });
    format!(
        r#"{{"id":{id},"name":"Сериал","status":"{status}","next_episode_to_air":{next},
            "last_episode_to_air":{{"season_number":1,"episode_number":7,"air_date":"2026-11-19"}}}}"#
    )
}

fn anilist(id: u64, status: &str, next: Option<(u32, u64)>) -> String {
    let next = next.map_or("null".to_owned(), |(e, at)| {
        format!(r#"{{"episode":{e},"airingAt":{at}}}"#)
    });
    format!(
        r#"{{"data":{{"Media":{{"id":{id},"idMal":{id},"format":"TV","status":"{status}",
            "episodes":12,"nextAiringEpisode":{next}}}}}}}"#
    )
}

#[test]
fn tmdb_show_airing_gives_next_episode_and_date() {
    let mut f = Fixture::new();
    f.work(1, "show", &[("tmdb", "tv", "100")]);
    for e in 1..=7 {
        f.episode_file(1, 1, e);
    }
    f.fake.set(
        "/tv/100",
        200,
        &tmdb_tv(100, "Returning Series", Some((1, 8, "2026-12-03"))),
    );
    let a = f.refresh(1, NOW).expect("airing");
    assert_eq!(a.status, AiringStatus::Airing);
    assert_eq!(a.source, "tmdb");
    assert!(!a.offline);
    assert_eq!(a.checked_at_ms, NOW);
    assert_eq!(
        a.next,
        Some(NextEpisode {
            season: Some(1),
            episode: 8,
            air_date: Some("2026-12-03".into()),
            air_at_ms: None,
        })
    );
    // The same is read back without asking.
    assert_eq!(f.shown(1, NOW), Some(a));
    assert_eq!(f.fake.calls(), vec!["/tv/100"]);
}

#[test]
fn tmdb_break_and_end() {
    let f = Fixture::new();
    f.work(1, "show", &[("tmdb", "tv", "100")]);
    f.work(2, "show", &[("tmdb", "tv", "200")]);
    f.work(3, "show", &[("tmdb", "tv", "300")]);
    f.fake
        .set("/tv/100", 200, &tmdb_tv(100, "Returning Series", None));
    f.fake.set("/tv/200", 200, &tmdb_tv(200, "Ended", None));
    f.fake.set("/tv/300", 200, &tmdb_tv(300, "Canceled", None));
    let hiatus = f.refresh(1, NOW).unwrap();
    assert_eq!(hiatus.status, AiringStatus::Hiatus);
    assert_eq!(hiatus.next, None);
    assert_eq!(f.refresh(2, NOW).unwrap().status, AiringStatus::Ended);
    assert_eq!(f.refresh(3, NOW).unwrap().status, AiringStatus::Ended);
}

#[test]
fn date_in_the_past_is_not_shown_as_next() {
    let f = Fixture::new();
    f.work(1, "show", &[("tmdb", "tv", "100")]);
    f.fake.set(
        "/tv/100",
        200,
        &tmdb_tv(100, "Returning Series", Some((2, 1, "2026-11-20"))),
    );
    let a = f.refresh(1, NOW).unwrap();
    assert_eq!(a.status, AiringStatus::Airing);
    assert_eq!(a.next, None, "aired a week ago: waits for the next answer");

    // A date that passes while the page is open.
    f.fake.set(
        "/tv/100",
        200,
        &tmdb_tv(100, "Returning Series", Some((2, 2, "2026-11-27"))),
    );
    f.later(2 * DAY);
    assert!(f.refresh(1, NOW).unwrap().next.is_some());
    assert!(f.shown(1, NOW + DAY).unwrap().next.is_some(), "the 27th");
    assert_eq!(f.shown(1, NOW + 3 * DAY).unwrap().next, None);
}

#[test]
fn no_data_gives_nothing() {
    let f = Fixture::new();
    // No provider id at all.
    f.work(1, "show", &[]);
    assert_eq!(f.refresh(1, NOW), None);
    // TMDB does not know the id.
    f.work(2, "show", &[("tmdb", "tv", "999")]);
    assert_eq!(f.refresh(2, NOW), None);
    // TMDB answers without a status.
    f.work(3, "show", &[("tmdb", "tv", "300")]);
    f.fake.set("/tv/300", 200, r#"{"id":300,"name":"?"}"#);
    assert_eq!(f.refresh(3, NOW), None);
    // Movies have no episodes.
    f.work(4, "movie", &[("tmdb", "movie", "400")]);
    assert_eq!(f.refresh(4, NOW), None);
    assert!(f.cat.airing_all(NOW).unwrap().is_empty());
    // A show item is required.
    assert!(
        f.cat
            .refresh_airing(&f.providers, ItemRef::Movie(4), NOW)
            .is_err()
    );
}

#[test]
fn offline_keeps_the_last_value_marked() {
    let f = Fixture::new();
    f.work(1, "show", &[("tmdb", "tv", "100")]);
    f.fake.set(
        "/tv/100",
        200,
        &tmdb_tv(100, "Returning Series", Some((1, 8, "2026-12-03"))),
    );
    let first = f.refresh(1, NOW).unwrap();
    assert!(!first.offline);

    // Two days later, offline: the provider's cached answer has expired and
    // is not used in place of the failed request.
    f.later(2 * DAY);
    f.fake.offline(true);
    let later = NOW + 2 * DAY;
    let a = f.refresh(1, later).unwrap();
    assert!(a.offline);
    assert_eq!(a.checked_at_ms, NOW, "the old answer");
    assert_eq!(a.next, first.next);
    assert_eq!(a.status, AiringStatus::Airing);

    // Offline with nothing stored yet: nothing to show.
    f.work(2, "show", &[("tmdb", "tv", "200")]);
    assert_eq!(f.refresh(2, later), None);

    // Back online: fresh again.
    f.fake.offline(false);
    let back = f.refresh(1, later).unwrap();
    assert!(!back.offline);
    assert_eq!(back.checked_at_ms, later);
}

#[test]
fn anime_from_anilist_counts_to_the_hour() {
    let f = Fixture::new();
    f.work(
        1,
        "anime",
        &[
            ("anilist", "", "500"),
            ("mal", "", "500"),
            ("tmdb", "tv", "100"),
        ],
    );
    let at = NOW_S + 2 * 24 * 3600 + 5 * 3600;
    f.fake.set(
        "anilist 500",
        200,
        &anilist(500, "RELEASING", Some((8, at))),
    );
    let a = f.refresh(1, NOW).unwrap();
    assert_eq!(a.source, "anilist");
    assert_eq!(a.status, AiringStatus::Airing);
    assert_eq!(
        a.next,
        Some(NextEpisode {
            season: None,
            episode: 8,
            air_date: None,
            air_at_ms: Some(at as i64 * 1000),
        })
    );
    // Neither Shikimori nor TMDB was asked.
    assert_eq!(f.fake.calls(), vec!["anilist 500"]);

    // An hour after the moment it has aired.
    assert_eq!(f.shown(1, at as i64 * 1000 + HOUR).unwrap().next, None);
}

#[test]
fn anime_states_from_anilist() {
    let f = Fixture::new();
    f.work(1, "anime", &[("anilist", "", "1")]);
    f.work(2, "anime", &[("anilist", "", "2")]);
    f.work(3, "anime", &[("anilist", "", "3")]);
    f.fake.set("anilist 1", 200, &anilist(1, "FINISHED", None));
    f.fake.set("anilist 2", 200, &anilist(2, "HIATUS", None));
    f.fake.set(
        "anilist 3",
        200,
        &anilist(3, "NOT_YET_RELEASED", Some((1, NOW_S + 3600))),
    );
    assert_eq!(f.refresh(1, NOW).unwrap().status, AiringStatus::Ended);
    assert_eq!(f.refresh(2, NOW).unwrap().status, AiringStatus::Hiatus);
    let premiere = f.refresh(3, NOW).unwrap();
    assert_eq!(premiere.status, AiringStatus::Airing);
    assert_eq!(premiere.next.unwrap().episode, 1);
}

#[test]
fn anime_falls_back_to_shikimori_then_tmdb() {
    let f = Fixture::new();
    f.work(
        1,
        "anime",
        &[
            ("anilist", "", "500"),
            ("mal", "", "600"),
            ("tmdb", "tv", "100"),
        ],
    );
    // AniList is down (5xx); Shikimori knows the next episode (ADR-0142).
    f.fake.set("anilist 500", 503, "busy");
    f.fake.set(
        "/animes/600",
        200,
        r#"{"id":600,"name":"Show","russian":"Шоу","kind":"tv","status":"ongoing",
            "episodes":12,"episodes_aired":7,"next_episode_at":"2026-11-28T18:30:00.000+03:00"}"#,
    );
    let a = f.refresh(1, NOW).unwrap();
    assert_eq!(a.source, "shikimori");
    let next = a.next.unwrap();
    assert_eq!(next.episode, 8);
    // 18:30 in Moscow is 15:30 UTC.
    assert_eq!(
        next.air_at_ms,
        Some(NOW + 2 * DAY + 3 * HOUR + 30 * 60 * 1000)
    );

    // Neither anime provider knows it: TMDB.
    f.work(2, "anime", &[("anilist", "", "700"), ("tmdb", "tv", "200")]);
    f.fake.set(
        "/tv/200",
        200,
        &tmdb_tv(200, "Returning Series", Some((3, 4, "2026-12-01"))),
    );
    let t = f.refresh(2, NOW).unwrap();
    assert_eq!(t.source, "tmdb");
    assert_eq!(t.next.unwrap().season, Some(3));
}

#[test]
fn the_file_of_the_next_episode_replaces_the_card() {
    let mut f = Fixture::new();
    f.work(1, "show", &[("tmdb", "tv", "100")]);
    f.episode_file(1, 1, 7);
    f.fake.set(
        "/tv/100",
        200,
        &tmdb_tv(100, "Returning Series", Some((1, 8, "2026-12-03"))),
    );
    assert!(f.refresh(1, NOW).unwrap().next.is_some());
    f.episode_file(1, 1, 8);
    let a = f.shown(1, NOW).unwrap();
    assert_eq!(a.status, AiringStatus::Airing);
    assert_eq!(a.next, None);

    // Anime numbered in one run: the episode number of the last season.
    f.work(2, "anime", &[("anilist", "", "9")]);
    f.episode_file(2, 1, 12);
    f.episode_file(2, 2, 1);
    f.fake.set(
        "anilist 9",
        200,
        &anilist(9, "RELEASING", Some((2, NOW_S + 3600))),
    );
    assert_eq!(f.refresh(2, NOW).unwrap().next.unwrap().episode, 2);
    f.episode_file(2, 2, 2);
    assert_eq!(f.shown(2, NOW).unwrap().next, None);
}

#[test]
fn daily_refresh_asks_only_what_is_due() {
    let f = Fixture::new();
    f.work(1, "show", &[("tmdb", "tv", "100")]);
    f.work(2, "show", &[("tmdb", "tv", "200")]);
    f.work(3, "show", &[]);
    f.fake.set(
        "/tv/100",
        200,
        &tmdb_tv(100, "Returning Series", Some((1, 8, "2026-12-03"))),
    );
    f.fake.set("/tv/200", 200, &tmdb_tv(200, "Ended", None));
    let cancel = CancelToken::new();
    let r = f
        .cat
        .refresh_airing_due(&f.providers, NOW, &cancel)
        .unwrap();
    assert_eq!((r.checked, r.updated, r.failed), (2, 2, 0));
    let all = f.cat.airing_all(NOW).unwrap();
    assert_eq!(all.len(), 2);

    // An hour later nothing is due.
    let r = f
        .cat
        .refresh_airing_due(&f.providers, NOW + HOUR, &cancel)
        .unwrap();
    assert_eq!(r.checked, 0);

    // A day later only the ongoing show; the ended one waits a month.
    f.later(DAY + HOUR);
    let r = f
        .cat
        .refresh_airing_due(&f.providers, NOW + DAY + HOUR, &cancel)
        .unwrap();
    assert_eq!(r.checked, 1);

    // Offline: counted as failed, the value stays.
    f.later(2 * DAY);
    f.fake.offline(true);
    let r = f
        .cat
        .refresh_airing_due(&f.providers, NOW + 3 * DAY + HOUR, &cancel)
        .unwrap();
    assert_eq!((r.checked, r.failed), (1, 1));
    assert!(f.shown(1, NOW + 3 * DAY + HOUR).unwrap().offline);
}
