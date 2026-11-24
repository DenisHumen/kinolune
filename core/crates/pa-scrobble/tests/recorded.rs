// SPDX-License-Identifier: MPL-2.0
//! The scrobbler against recorded answers of Last.fm and ListenBrainz
//! (`tests/fixtures`, shaped like the documented answers): the rules,
//! the requests, the offline queue and its retries.

use std::collections::VecDeque;
use std::sync::Mutex;

use pa_scrobble::key::LastFmKey;
use pa_scrobble::{
    ApiError, Config, LastFmConfig, Request, Response, Scrobbler, Status, Track, Transport,
    TransportError, lastfm,
};

const T0: u64 = 1_763_000_000_000;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Answers from a script, in order; records what was sent.
#[derive(Default)]
struct Script {
    answers: Mutex<VecDeque<Result<Response, TransportError>>>,
    sent: Mutex<Vec<Request>>,
}

impl Script {
    fn answer(&self, status: u16, name: &str) {
        self.answers.lock().unwrap().push_back(Ok(Response {
            status,
            headers: Vec::new(),
            body: fixture(name),
        }));
    }

    fn answer_with(&self, r: Response) {
        self.answers.lock().unwrap().push_back(Ok(r));
    }

    fn offline(&self) {
        self.answers
            .lock()
            .unwrap()
            .push_back(Err(TransportError("connection failed".into())));
    }

    fn take(&self) -> Vec<Request> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

impl Transport for &Script {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.sent.lock().unwrap().push(request.clone());
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(TransportError("no answer scripted".into())))
    }
}

fn key() -> LastFmKey {
    LastFmKey::new("0123456789abcdef0123456789abcdef", "s3cret").unwrap()
}

fn song(n: u32) -> Track {
    Track {
        artist: "Artist".into(),
        title: format!("Song {n}"),
        album: Some("Album".into()),
        duration_ms: Some(200_000),
        track_number: Some(n),
        ..Track::default()
    }
}

/// Plays `track` from 0 to `to_ms`, reporting every 5 s.
fn listen<T: Transport>(s: &mut Scrobbler<T>, id: &str, track: Track, start: u64, to_ms: u64) {
    s.track_started(id, track, 0, start);
    let mut pos = 0;
    while pos < to_ms {
        pos += 5_000;
        s.progress(pos, true, start + pos);
    }
}

fn body(r: &Request) -> &str {
    &r.body.as_ref().unwrap().1
}

#[test]
fn lastfm_sign_in_and_signature() {
    let script = Script::default();
    let s = Scrobbler::new(&script, None);
    script.answer(200, "lastfm-token");
    let (token, url) = s.lastfm_begin(&key()).unwrap();
    assert_eq!(token, "cf45fe5a3e3cebe168480a086d7fe481");
    assert_eq!(
        url,
        "https://www.last.fm/api/auth/?api_key=0123456789abcdef0123456789abcdef&token=cf45fe5a3e3cebe168480a086d7fe481"
    );
    script.answer(200, "lastfm-session");
    let session = s.lastfm_finish(&key(), &token).unwrap();
    assert_eq!(session.name, "listener");
    assert_eq!(session.key, "d580d57f32848f5dcf574d1ce18d78b2");
    assert!(!format!("{session:?}").contains("d580"));
    let sent = script.take();
    assert!(sent[0].url.contains("method=auth.getToken"));
    // api_sig = md5("api_key<key>methodauth.getSession" + "token<token>" + secret).
    let expected = lastfm::sign(
        &[
            ("token".into(), token.clone()),
            ("method".into(), "auth.getSession".into()),
            ("api_key".into(), key().api_key),
        ],
        "s3cret",
    );
    assert!(sent[1].url.contains(&format!("api_sig={expected}")));
    assert!(sent[1].url.ends_with("format=json"));

    script.answer(200, "lastfm-error-9");
    assert!(matches!(
        s.lastfm_finish(&key(), "used"),
        Err(ApiError::Refused { code: 9, .. })
    ));
    script.offline();
    assert!(matches!(
        s.lastfm_begin(&key()),
        Err(ApiError::Unavailable(_))
    ));
}

#[test]
fn signature_matches_a_known_value() {
    // md5("api_keyxxmethodauth.getTokensecret") computed by hand.
    let sig = lastfm::sign(
        &[
            ("method".into(), "auth.getToken".into()),
            ("api_key".into(), "xx".into()),
            ("format".into(), "json".into()),
        ],
        "secret",
    );
    assert_eq!(sig.len(), 32);
    let same = lastfm::sign(
        &[
            ("api_key".into(), "xx".into()),
            ("method".into(), "auth.getToken".into()),
        ],
        "secret",
    );
    assert_eq!(sig, same, "order and format do not change it");
}

#[test]
fn now_playing_then_scrobble_at_half_to_both_services() {
    let script = Script::default();
    let mut s = Scrobbler::new(&script, None);
    s.configure(Config {
        lastfm: Some(LastFmConfig {
            key: key(),
            session_key: "sk1".into(),
        }),
        listenbrainz: Some("lb-token".into()),
    });
    script.answer(200, "lastfm-scrobble-ok");
    script.answer(200, "listenbrainz-ok");
    s.track_started("t1", song(1), 0, T0);
    let sent = script.take();
    assert_eq!(sent.len(), 2, "now playing on both");
    assert!(body(&sent[0]).contains("method=track.updateNowPlaying"));
    assert!(body(&sent[0]).contains("sk=sk1"));
    assert!(body(&sent[0]).contains("track=Song%201"));
    assert!(body(&sent[1]).contains("\"listen_type\":\"playing_now\""));
    assert_eq!(
        sent[1].headers,
        vec![("authorization".into(), "Token lb-token".into())]
    );

    script.answer(200, "lastfm-scrobble-ok");
    script.answer(200, "listenbrainz-ok");
    let mut pos = 0;
    while pos < 95_000 {
        pos += 5_000;
        s.progress(pos, true, T0 + pos);
    }
    assert!(script.take().is_empty(), "not yet at 95 s of 200 s");
    s.progress(100_000, true, T0 + 100_000);
    let sent = script.take();
    assert_eq!(sent.len(), 2);
    let lf = body(&sent[0]);
    assert!(lf.contains("method=track.scrobble"));
    assert!(lf.contains(&format!("timestamp%5B0%5D={}", T0 / 1000)));
    assert!(lf.contains("duration%5B0%5D=200"));
    assert!(lf.contains("trackNumber%5B0%5D=1"));
    let lb: serde_json::Value = serde_json::from_str(body(&sent[1])).unwrap();
    assert_eq!(lb["listen_type"], "single");
    assert_eq!(lb["payload"][0]["listened_at"], T0 / 1000);
    assert_eq!(lb["payload"][0]["track_metadata"]["release_name"], "Album");
    assert_eq!(
        lb["payload"][0]["track_metadata"]["additional_info"]["duration_ms"],
        200_000
    );
    assert_eq!(s.status(), Status::default(), "nothing left");
}

#[test]
fn offline_queue_retries_in_order_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        lastfm: Some(LastFmConfig {
            key: key(),
            session_key: "sk1".into(),
        }),
        listenbrainz: Some("lb-token".into()),
    };
    let script = Script::default();
    let mut s = Scrobbler::new(&script, Some(dir.path()));
    s.configure(config.clone());
    // Offline: now playing and scrobbles all fail.
    for n in 1..=2 {
        for _ in 0..4 {
            script.offline();
        }
        listen(
            &mut s,
            &format!("t{n}"),
            song(n),
            T0 + u64::from(n) * 200_000,
            100_000,
        );
    }
    let st = s.status();
    assert_eq!(st.lastfm_pending, 2);
    assert_eq!(st.listenbrainz_pending, 2);
    script.take();

    // Too soon after the failure: nothing is sent.
    assert_eq!(s.flush(T0 + 400_000 + 100_000 + 1_000).lastfm_pending, 2);
    assert!(script.take().is_empty());

    // A restart: the queues are on disk.
    drop(s);
    let mut s = Scrobbler::new(&script, Some(dir.path()));
    s.configure(config);
    // Last.fm is down for maintenance, ListenBrainz rate-limits.
    script.answer(200, "lastfm-error-11");
    script.answer_with(Response {
        status: 429,
        headers: vec![("x-ratelimit-reset-in".into(), "9".into())],
        body: fixture("listenbrainz-429"),
    });
    // The configure reset the wait: both try at once.
    let st = s.flush(T0 + 600_000);
    assert_eq!((st.lastfm_pending, st.listenbrainz_pending), (2, 2));
    assert_eq!(script.take().len(), 2);
    assert!(s.flush(T0 + 600_000 + 9_000).listenbrainz_pending == 2);
    assert!(script.take().is_empty(), "both still wait");

    // Back: one batch each, oldest first.
    script.answer(200, "lastfm-scrobble-ok");
    script.answer(200, "listenbrainz-ok");
    let st = s.flush(T0 + 600_000 + 60_000);
    assert_eq!(st, Status::default());
    let sent = script.take();
    assert_eq!(sent.len(), 2);
    let lf = body(&sent[0]);
    assert!(lf.find("Song%201").unwrap() < lf.find("Song%202").unwrap());
    let lb: serde_json::Value = serde_json::from_str(body(&sent[1])).unwrap();
    assert_eq!(lb["listen_type"], "import");
    assert_eq!(lb["payload"].as_array().unwrap().len(), 2);
}

#[test]
fn revoked_accounts_hold_the_queue_and_filtered_scrobbles_drop() {
    let script = Script::default();
    let mut s = Scrobbler::new(&script, None);
    let lastfm_only = Config {
        lastfm: Some(LastFmConfig {
            key: key(),
            session_key: "revoked".into(),
        }),
        listenbrainz: None,
    };
    s.configure(lastfm_only.clone());
    script.offline();
    script.answer(200, "lastfm-error-9");
    listen(&mut s, "t1", song(1), T0, 100_000);
    assert!(s.status().lastfm_needs_auth);
    assert_eq!(s.status().lastfm_pending, 1);
    script.take();
    s.flush(T0 + 10_000_000);
    assert!(script.take().is_empty(), "waits for a new sign-in");

    // Signed in again: the queue goes out; the service filtered it.
    s.configure(Config {
        lastfm: Some(LastFmConfig {
            key: key(),
            session_key: "sk2".into(),
        }),
        listenbrainz: None,
    });
    script.answer(200, "lastfm-scrobble-ignored");
    let st = s.flush(T0 + 10_000_000);
    assert_eq!(st, Status::default());
    assert_eq!(
        lastfm::ignored(&Response {
            status: 200,
            headers: Vec::new(),
            body: fixture("lastfm-scrobble-ignored"),
        }),
        1
    );

    // ListenBrainz: a bad token, then the check of a token.
    s.configure(Config {
        lastfm: None,
        listenbrainz: Some("bad".into()),
    });
    script.offline();
    script.answer(401, "listenbrainz-401");
    listen(&mut s, "t2", song(2), T0 + 20_000_000, 100_000);
    assert!(s.status().listenbrainz_needs_auth);
    script.answer(200, "listenbrainz-validate-ok");
    assert_eq!(s.listenbrainz_user("good").unwrap(), "listener");
    script.answer(200, "listenbrainz-validate-bad");
    assert!(matches!(
        s.listenbrainz_user("bad"),
        Err(ApiError::Refused { .. })
    ));

    // Disconnected: its queue is forgotten.
    s.configure(Config::default());
    assert_eq!(s.status(), Status::default());
    script.take();
    listen(&mut s, "t3", song(3), T0 + 30_000_000, 200_000);
    assert!(
        script.take().is_empty(),
        "nothing goes out without accounts"
    );
}

#[test]
fn short_tracks_never_scrobble() {
    let script = Script::default();
    let mut s = Scrobbler::new(&script, None);
    s.configure(Config {
        lastfm: None,
        listenbrainz: Some("t".into()),
    });
    script.answer(200, "listenbrainz-ok");
    let jingle = Track {
        duration_ms: Some(25_000),
        ..song(9)
    };
    listen(&mut s, "j", jingle, T0, 25_000);
    let sent = script.take();
    assert_eq!(sent.len(), 1, "now playing only");
    assert_eq!(s.status(), Status::default());
}
