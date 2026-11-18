// SPDX-License-Identifier: MPL-2.0
//! NET-013 against the Plex Media Server stand-in (`tests/support/mock.rs`,
//! synthetic answers in the documented shape): the timeline, scrobble,
//! unscrobble and Continue Watching calls go out with the parameters the
//! Plex API documents and the server token in a header; the user's state
//! the server keeps after them (`viewOffset`, `viewCount`) reads back
//! through `/library/metadata/{id}` and the library listing; the
//! transcode URL names the item, its version and file, the quality and
//! the session.

use std::sync::Arc;

use pa_plex::{
    ClientInfo, PlayState, PlexFs, Server, Timeline, Token, TranscodeRequest, new_session_id,
};
use pa_vfs::VfsPath;

mod support;
use support::mock::{UserState, start};

fn server(base: &str) -> Arc<Server> {
    Arc::new(
        Server::new(
            &[base.to_owned()],
            ClientInfo::kinolune("Test Device", "test-client"),
            Token::new("server-token"),
        )
        .unwrap(),
    )
}

fn query(target: &str) -> Vec<(String, String)> {
    target
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default()
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn has(q: &[(String, String)], k: &str, v: &str) -> bool {
    q.iter().any(|(a, b)| a == k && b == v)
}

#[test]
fn timeline_and_marks_change_the_server_state() {
    let (mock, base) = start();
    let pms = server(&base);

    // Playing, then paused at 4:10 — what Plex Web shows under Continue
    // Watching.
    for (state, time) in [(PlayState::Playing, 240_000), (PlayState::Paused, 250_000)] {
        pms.timeline(&Timeline {
            rating_key: "101".into(),
            state,
            position_ms: time,
            duration_ms: Some(596_000),
            session: None,
        })
        .unwrap();
    }
    let seen = mock.seen.lock().unwrap().clone();
    let timeline = seen
        .iter()
        .rfind(|t| t.starts_with("/:/timeline?"))
        .expect("a timeline report");
    let q = query(timeline);
    for (k, v) in [
        ("ratingKey", "101"),
        ("key", "%2Flibrary%2Fmetadata%2F101"),
        ("identifier", "com.plexapp.plugins.library"),
        ("state", "paused"),
        ("time", "250000"),
        ("duration", "596000"),
    ] {
        assert!(has(&q, k, v), "{k}={v} in {timeline}");
    }
    // The token travels in the header, never in the URL.
    assert!(!seen.iter().any(|t| t.contains("server-token")));
    let item = pms.metadata("101").unwrap().unwrap();
    assert_eq!(item.view_offset, Some(250_000));

    // The listing of the library carries it too.
    let fs = PlexFs::new(Arc::clone(&pms));
    let movie = fs
        .media_at(
            &VfsPath::parse("/Movies/Big Buck Bunny (2008)/Big Buck Bunny (2008).mkv").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(movie.view_offset_ms, Some(250_000));
    assert!(movie.last_viewed_ms.is_some());
    assert_eq!((movie.media_index, movie.part_index), (0, 0));
    assert_eq!(movie.bitrate_kbps, Some(8000));

    // Stopped past 90 %: the server marks it watched by itself.
    pms.timeline(&Timeline {
        rating_key: "101".into(),
        state: PlayState::Stopped,
        position_ms: 580_000,
        duration_ms: Some(596_000),
        session: None,
    })
    .unwrap();
    let after = mock.user_state("101");
    assert_eq!((after.view_count, after.view_offset), (1, None));

    // Unwatched, watched, back in Continue Watching, then taken out.
    pms.unscrobble("101").unwrap();
    assert_eq!(mock.user_state("101").view_count, 0);
    pms.scrobble("101").unwrap();
    assert_eq!(mock.user_state("101").view_count, 1);
    pms.set_progress("101", 60_000).unwrap();
    assert_eq!(mock.user_state("101").view_offset, Some(60_000));
    pms.remove_from_continue_watching("101").unwrap();
    assert_eq!(mock.user_state("101").view_offset, None);
    let seen = mock.seen.lock().unwrap().clone();
    for call in [
        "/:/scrobble?key=101&identifier=com.plexapp.plugins.library",
        "/:/unscrobble?key=101&identifier=com.plexapp.plugins.library",
        "/actions/removeFromContinueWatching?ratingKey=101",
    ] {
        assert!(seen.iter().any(|t| t == call), "{call} in {seen:?}");
    }

    // Plex Web sets a position: the item reads it back.
    mock.set_user_state(
        "202",
        UserState {
            view_offset: Some(42_000),
            view_count: 0,
            last_viewed_at: Some(1_900_000_000),
        },
    );
    fs.refresh();
    let episode = fs
        .media_files()
        .unwrap()
        .into_iter()
        .find(|(_, m)| m.rating_key == "202")
        .unwrap()
        .1;
    assert_eq!(episode.view_offset_ms, Some(42_000));
    assert!(!episode.watched);
    assert_eq!(episode.last_viewed_ms, Some(1_900_000_000_000));
    assert_eq!(pms.metadata("999").unwrap(), None);
    assert!(pms.scrobble(" ").is_err());
}

#[test]
fn transcode_url_names_the_file_quality_and_session() {
    let (mock, base) = start();
    let pms = server(&base);
    let session = new_session_id();
    let url = pms.transcode_url(&TranscodeRequest {
        rating_key: "101".into(),
        media_index: 1,
        part_index: 0,
        max_bitrate_kbps: Some(4000),
        resolution: Some((1280, 720)),
        direct_stream: false,
        session: session.clone(),
    });
    assert!(url.starts_with(&format!("{base}/video/:/transcode/universal/start.m3u8?")));
    let q = query(&url);
    for (k, v) in [
        ("path", "%2Flibrary%2Fmetadata%2F101"),
        ("mediaIndex", "1"),
        ("partIndex", "0"),
        ("protocol", "hls"),
        ("directPlay", "0"),
        ("directStream", "0"),
        ("maxVideoBitrate", "4000"),
        ("videoResolution", "1280x720"),
        ("location", "lan"),
        ("session", session.as_str()),
        ("X-Plex-Client-Identifier", "test-client"),
        ("X-Plex-Product", "Kinolune"),
        // libmpv opens it itself: the token has to be in the URL.
        ("X-Plex-Token", "server-token"),
    ] {
        assert!(has(&q, k, v), "{k}={v}");
    }
    // The timeline of a transcoded play names the session; the stop ends
    // it.
    pms.timeline(&Timeline {
        rating_key: "101".into(),
        state: PlayState::Playing,
        position_ms: 1000,
        duration_ms: None,
        session: Some(session.clone()),
    })
    .unwrap();
    pms.stop_transcode(&session).unwrap();
    let seen = mock.seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|t| t.starts_with("/:/timeline?")
            && has(&query(t), "X-Plex-Session-Identifier", &session))
    );
    assert!(
        seen.iter()
            .any(|t| *t == format!("/video/:/transcode/universal/stop?session={session}"))
    );
}
