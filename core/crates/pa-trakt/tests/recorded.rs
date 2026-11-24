// SPDX-License-Identifier: MPL-2.0
//! The Trakt client on recorded answers (SYNC-011): no live network.

use std::sync::{Arc, Mutex};

use pa_secrets::{MemoryStore, SecretKey, SecretStore, SecretString};
use pa_trakt::sync::{self, Library, LocalRating};
use pa_trakt::testing::{Exchange, Recorded};
use pa_trakt::{
    Client, Credentials, Endpoints, Error, Ids, Method, Poll, Rated, ScrobbleAction, Session,
    Target, Tokens, Watched, parse_ms,
};
use serde_json::Value;

const NOW: i64 = 1_700_000_100_000;

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).expect("fixture")
}

fn get(path: &str, fixture_name: &str) -> Exchange {
    Exchange::new(Method::Get, path, 200, &fixture(fixture_name))
}

fn post(path: &str, status: u16) -> Exchange {
    Exchange::new(Method::Post, path, status, "{}")
}

fn session(
    exchanges: Vec<Exchange>,
    tokens: Option<Tokens>,
    secret: Option<&str>,
) -> (Session, Arc<Recorded>, Arc<MemoryStore>) {
    let transport = Arc::new(Recorded::new(exchanges));
    let store = Arc::new(MemoryStore::new());
    if let Some(t) = tokens {
        let key = SecretKey::trakt_tokens("main").unwrap();
        store
            .set(&key, &SecretString::new(serde_json::to_string(&t).unwrap()))
            .unwrap();
    }
    let client = Client::new(
        Credentials::new("test-client-id", secret).unwrap(),
        transport.clone(),
        Endpoints::default(),
    )
    .with_sleep(|_| {});
    let s = Session::open(client, store.clone())
        .unwrap()
        .with_clock(|| NOW);
    (s, transport, store)
}

fn signed_in() -> Option<Tokens> {
    Some(Tokens {
        access_token: "test-access-1".into(),
        refresh_token: "test-refresh-1".into(),
        expires_at_ms: NOW + 6 * 24 * 3600 * 1000,
        username: Some("kinolune-test".into()),
    })
}

fn body(t: &Recorded, i: usize) -> Value {
    serde_json::from_str(t.sent()[i].body.as_deref().unwrap()).unwrap()
}

fn header(t: &Recorded, i: usize, name: &str) -> Option<String> {
    t.sent()[i]
        .headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
}

#[test]
fn device_sign_in_stores_tokens_and_user() {
    let (s, t, store) = session(
        vec![
            Exchange::new(
                Method::Post,
                "/oauth/device/code",
                200,
                &fixture("device_code.json"),
            ),
            Exchange::new(Method::Post, "/oauth/device/token", 400, ""),
            Exchange::new(Method::Post, "/oauth/device/token", 429, ""),
            Exchange::new(
                Method::Post,
                "/oauth/device/token",
                200,
                &fixture("token.json"),
            ),
            get("/users/settings", "users_settings.json"),
        ],
        None,
        None,
    );
    assert!(!s.is_signed_in());
    let code = s.begin().unwrap();
    assert_eq!(code.user_code, "5055CC52");
    assert_eq!(code.interval, 5);
    assert_eq!(code.qr_url(), "https://trakt.tv/activate/5055CC52");
    assert_eq!(s.poll(&code).unwrap(), Poll::Pending);
    assert_eq!(s.poll(&code).unwrap(), Poll::SlowDown);
    let Poll::Approved(tokens) = s.poll(&code).unwrap() else {
        panic!("not approved");
    };
    assert_eq!(tokens.username.as_deref(), Some("kinolune-test"));
    assert_eq!(tokens.expires_at_ms, (1_700_000_000 + 604_800) * 1000);
    assert_eq!(s.username().as_deref(), Some("kinolune-test"));
    assert!(t.is_done());
    // OAuth goes to auth.trakt.tv with the client id and, without a
    // configured secret, no client_secret.
    assert!(t.sent()[1].url.starts_with("https://auth.trakt.tv/"));
    let poll = body(&t, 1);
    assert_eq!(poll["client_id"], "test-client-id");
    assert!(poll.get("client_secret").is_none());
    assert_eq!(
        header(&t, 4, "authorization").as_deref(),
        Some("Bearer test-access-1")
    );
    assert_eq!(header(&t, 4, "trakt-api-version").as_deref(), Some("2"));
    assert_eq!(
        header(&t, 4, "trakt-api-key").as_deref(),
        Some("test-client-id")
    );
    // In the secret store, under a key without the user name.
    let stored = store
        .get(&SecretKey::trakt_tokens("main").unwrap())
        .unwrap()
        .unwrap();
    assert!(stored.expose().contains("test-refresh-1"));
}

#[test]
fn a_configured_secret_goes_with_the_token_exchange() {
    let (s, t, _) = session(
        vec![Exchange::new(Method::Post, "/oauth/device/token", 418, "")],
        None,
        Some("test-secret"),
    );
    let code = serde_json::from_str(&fixture("device_code.json")).unwrap();
    assert_eq!(s.poll(&code).unwrap(), Poll::Denied);
    assert_eq!(body(&t, 0)["client_secret"], "test-secret");
}

#[test]
fn an_expiring_token_is_refreshed_first_and_rotated() {
    let mut tokens = signed_in().unwrap();
    tokens.expires_at_ms = NOW + 3600 * 1000;
    let (s, t, store) = session(
        vec![
            Exchange::new(
                Method::Post,
                "/oauth/token",
                200,
                &fixture("token_refreshed.json"),
            ),
            get("/sync/watchlist/movies", "watchlist_movies.json"),
            get("/sync/watchlist/shows", "watchlist_shows.json"),
        ],
        Some(tokens),
        None,
    );
    let list = s.watchlist().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        list[0].target,
        Target::Movie(Ids {
            trakt: Some(1),
            imdb: Some("tt1104001".into()),
            tmdb: Some(20526),
            tvdb: None,
        })
    );
    assert!(matches!(&list[1].target, Target::Show(ids) if ids.tmdb == Some(1402)));
    assert_eq!(
        list[0].listed_at_ms,
        parse_ms("2014-09-01T09:10:11.000Z").unwrap()
    );
    let refresh = body(&t, 0);
    assert_eq!(refresh["grant_type"], "refresh_token");
    assert_eq!(refresh["refresh_token"], "test-refresh-1");
    assert_eq!(
        header(&t, 1, "authorization").as_deref(),
        Some("Bearer test-access-2")
    );
    // The single-use refresh token was replaced at once; the name kept.
    let stored = store
        .get(&SecretKey::trakt_tokens("main").unwrap())
        .unwrap()
        .unwrap();
    assert!(stored.expose().contains("test-refresh-2"));
    assert_eq!(s.username().as_deref(), Some("kinolune-test"));
}

#[test]
fn a_refused_token_is_refreshed_once_and_a_dead_sign_in_signs_out() {
    let (s, _, _) = session(
        vec![
            Exchange::new(Method::Get, "/sync/watchlist/movies", 401, ""),
            Exchange::new(
                Method::Post,
                "/oauth/token",
                200,
                &fixture("token_refreshed.json"),
            ),
            get("/sync/watchlist/movies", "watchlist_movies.json"),
            get("/sync/watchlist/shows", "watchlist_shows.json"),
        ],
        signed_in(),
        None,
    );
    assert_eq!(s.watchlist().unwrap().len(), 2);

    let (s, _, store) = session(
        vec![
            Exchange::new(Method::Get, "/sync/ratings/movies", 401, ""),
            Exchange::new(
                Method::Post,
                "/oauth/token",
                400,
                r#"{"error":"invalid_grant","error_description":"session not found"}"#,
            ),
        ],
        signed_in(),
        None,
    );
    assert_eq!(s.ratings(), Err(Error::SignedOut));
    assert!(!s.is_signed_in());
    assert!(store.keys().is_empty());
}

#[test]
fn scrobbles_episodes_by_show_ids_and_accepts_duplicates() {
    let (s, t, _) = session(
        vec![
            post("/scrobble/start", 201),
            Exchange::new(
                Method::Post,
                "/scrobble/stop",
                409,
                r#"{"watched_at":"2014-10-15T22:21:29.000Z"}"#,
            ),
            post("/scrobble/pause", 429).retry_after(2),
            post("/scrobble/pause", 201),
        ],
        signed_in(),
        None,
    );
    let ep = Target::Episode {
        show: Ids {
            tmdb: Some(1396),
            tvdb: Some(81189),
            imdb: Some("tt0903747".into()),
            trakt: None,
        },
        season: 1,
        number: 2,
    };
    s.scrobble(ScrobbleAction::Start, &ep, 1.256).unwrap();
    s.scrobble(ScrobbleAction::Stop, &ep, 92.0).unwrap();
    s.scrobble(ScrobbleAction::Pause, &ep, 40.0).unwrap();
    assert!(t.is_done());
    let start = body(&t, 0);
    assert_eq!(start["show"]["ids"]["tmdb"], 1396);
    assert_eq!(start["show"]["ids"]["tvdb"], 81189);
    assert_eq!(start["episode"]["season"], 1);
    assert_eq!(start["episode"]["number"], 2);
    assert_eq!(start["progress"], 1.26);
    assert!(start["show"]["ids"].get("trakt").is_none());
}

#[test]
fn a_long_rate_limit_is_reported() {
    let (s, _, _) = session(
        vec![post("/scrobble/start", 429).retry_after(120)],
        signed_in(),
        None,
    );
    let r = s.scrobble(ScrobbleAction::Start, &Target::Movie(Ids::tmdb(1)), 1.0);
    assert_eq!(
        r,
        Err(Error::RateLimited {
            retry_after_secs: 120
        })
    );
}

/// A library in memory.
#[derive(Default)]
struct FakeLibrary {
    /// Titles in the library (target keys).
    have: Vec<String>,
    watched: Mutex<Vec<Watched>>,
    ratings: Mutex<Vec<LocalRating>>,
}

impl Library for FakeLibrary {
    fn watched(&self) -> pa_trakt::Result<Vec<Watched>> {
        Ok(self.watched.lock().unwrap().clone())
    }

    fn mark_watched(&self, w: &Watched) -> pa_trakt::Result<bool> {
        let key = w.target.key().unwrap();
        let mut list = self.watched.lock().unwrap();
        if !self.have.contains(&key) || list.iter().any(|x| x.target.key() == Some(key.clone())) {
            return Ok(false);
        }
        list.push(w.clone());
        Ok(true)
    }

    fn ratings(&self) -> pa_trakt::Result<Vec<LocalRating>> {
        Ok(self.ratings.lock().unwrap().clone())
    }

    fn set_rating(&self, target: &Target, stars: u8) -> pa_trakt::Result<bool> {
        let key = target.key().unwrap();
        if !self.have.contains(&key) {
            return Ok(false);
        }
        let mut list = self.ratings.lock().unwrap();
        list.retain(|r| r.target.key() != Some(key.clone()));
        list.push(LocalRating {
            target: target.clone(),
            stars,
            rated_at_ms: NOW,
        });
        Ok(true)
    }
}

#[test]
fn history_and_ratings_sync_both_ways() {
    let (s, t, _) = session(
        vec![
            get("/sync/watched/movies", "watched_movies.json"),
            get("/sync/watched/shows", "watched_shows.json"),
            post("/sync/history", 201),
            get("/sync/ratings/movies", "ratings_movies.json"),
            Exchange::new(Method::Get, "/sync/ratings/shows", 200, "[]"),
            post("/sync/ratings", 201),
        ],
        signed_in(),
        None,
    );
    let lib = FakeLibrary {
        // Batman Begins and S01E01 are here; The Dark Knight and S01E02
        // are not.
        have: vec!["m:272".into(), "e:1396:1:1".into(), "m:603".into()],
        watched: Mutex::new(vec![Watched {
            target: Target::Movie(Ids::tmdb(603)),
            watched_at_ms: 1_600_000_000_000,
        }]),
        ratings: Mutex::new(vec![LocalRating {
            target: Target::Movie(Ids::tmdb(603)),
            stars: 4,
            rated_at_ms: 1_600_000_000_000,
        }]),
    };
    let report = sync::run(&s, &lib).unwrap();
    assert!(t.is_done());
    assert_eq!(report.watched_imported, 2);
    assert_eq!(report.watched_exported, 1);
    assert_eq!(report.ratings_imported, 1);
    assert_eq!(report.ratings_exported, 1);
    // Trakt's times came with the marks.
    let marks = lib.watched.lock().unwrap().clone();
    assert!(marks.contains(&Watched {
        target: Target::Episode {
            show: Ids {
                trakt: Some(1),
                imdb: Some("tt0903747".into()),
                tmdb: Some(1396),
                tvdb: Some(81189),
            },
            season: 1,
            number: 1,
        },
        watched_at_ms: parse_ms("2014-10-11T17:00:54.000Z").unwrap(),
    }));
    let history = body(&t, 2);
    assert_eq!(history["movies"][0]["ids"]["tmdb"], 603);
    assert_eq!(
        history["movies"][0]["watched_at"],
        "2020-09-13T12:26:40.000Z"
    );
    let ratings = body(&t, 5);
    assert_eq!(ratings["movies"][0]["rating"], 8);
    assert!(
        lib.ratings
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.target.key().as_deref() == Some("m:272") && r.stars == 5)
    );

    // A second run with the same remote state changes nothing here.
    let (s, t, _) = session(
        vec![
            get("/sync/watched/movies", "watched_movies.json"),
            get("/sync/watched/shows", "watched_shows.json"),
            post("/sync/history", 201),
            get("/sync/ratings/movies", "ratings_movies.json"),
            Exchange::new(Method::Get, "/sync/ratings/shows", 200, "[]"),
            post("/sync/ratings", 201),
        ],
        signed_in(),
        None,
    );
    let again = sync::run(&s, &lib).unwrap();
    assert_eq!(again.watched_imported, 0);
    assert_eq!(again.ratings_imported, 0);
    drop(t);
}

#[test]
fn history_and_watchlist_writes_group_episodes_by_show() {
    let (s, t, _) = session(
        vec![
            post("/sync/history/remove", 200),
            post("/sync/watchlist", 201),
            post("/sync/watchlist/remove", 200),
        ],
        signed_in(),
        None,
    );
    let ep = |n| Target::Episode {
        show: Ids::tmdb(1396),
        season: 1,
        number: n,
    };
    s.remove_history(&[ep(1), ep(2), Target::Movie(Ids::tmdb(603))])
        .unwrap();
    s.set_watchlisted(&Target::Show(Ids::tmdb(1402)), true)
        .unwrap();
    s.set_watchlisted(&Target::Movie(Ids::tmdb(603)), false)
        .unwrap();
    let removed = body(&t, 0);
    assert_eq!(removed["movies"][0]["ids"]["tmdb"], 603);
    assert_eq!(removed["shows"][0]["ids"]["tmdb"], 1396);
    assert_eq!(
        removed["shows"][0]["seasons"][0]["episodes"][1]["number"],
        2
    );
    assert_eq!(body(&t, 1)["shows"][0]["ids"]["tmdb"], 1402);
    assert_eq!(body(&t, 2)["movies"][0]["ids"]["tmdb"], 603);
    let rated = Rated {
        target: Target::Movie(Ids::tmdb(1)),
        rating: 8,
        rated_at_ms: 0,
    };
    assert_eq!(rated.rating, 8);
}
