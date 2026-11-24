// SPDX-License-Identifier: MPL-2.0
//! TWI-016: VOD chat replay. GQL answers in the shape Twitch sends for
//! `VideoCommentsByOffsetOrCursor` (made-up channel, users and ids) →
//! TwitchDownloader comments; the page cache around the playhead; the
//! binary search that keeps the chat at the video position after a seek;
//! TwitchDownloader files. No network.
#![cfg(pa_twitch_native)]

use std::io::Write;
use std::sync::Arc;

use pa_remote_config::{ClientContext, KillSwitch, RemoteConfig, flags};
use pa_twitch::chat_replay::{
    COMMENTS_OPERATION, ChatReplayError, ChatReplayFile, ChatReplayGql, ChatReplayOff,
    OnlineReplay, ReplayTimeline,
};
use pa_twitch::playback::Flavor;
use pa_twitch::testing::FixtureTransport;

const VOD: &str = "1000000001";
const HASH: &str = "00000000000000000000000000000000000000000000000000000000000c0de";

macro_rules! fx {
    ($name:literal) => {
        include_str!(concat!("fixtures/chat_replay/", $name, ".json"))
    };
}

fn ctx() -> ClientContext {
    ClientContext {
        app_version: "0.2.0".into(),
        platform: "linux".into(),
    }
}

fn config(chat_replay: bool, hash: Option<&str>, kill: bool) -> RemoteConfig {
    let mut queries = serde_json::Map::new();
    if let Some(h) = hash {
        queries.insert(COMMENTS_OPERATION.into(), h.into());
    }
    let mut c = RemoteConfig::parse(
        serde_json::json!({
            "schema": 1,
            "serial": 1,
            "flags": {
                "twitch.gql_playback": true,
                "twitch.chat_replay": chat_replay,
            },
            "kill_switches": [],
            "twitch": {
                "gql_client_id": "test-client-id-not-real",
                "persisted_queries": queries,
                "player": "native",
            },
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    if kill {
        c.kill_switches.push(KillSwitch {
            feature: flags::TWITCH_CHAT_REPLAY.into(),
            min_app_version: None,
            max_app_version: Some("0.2.0".into()),
            platforms: vec![],
            reason: Some("Twitch changed the comments API".into()),
        });
    }
    c
}

fn gql_on() -> ChatReplayGql {
    ChatReplayGql::resolve_for(&config(true, Some(HASH), false), &ctx(), Flavor::Full)
}

fn replay(transport: &Arc<FixtureTransport>) -> OnlineReplay {
    OnlineReplay::new(&gql_on(), VOD, transport.clone())
        .unwrap()
        .with_gql_endpoint("https://gql.test/gql")
}

fn variables(t: &FixtureTransport) -> Vec<serde_json::Value> {
    t.requests()
        .iter()
        .map(|r| {
            let body: serde_json::Value = serde_json::from_slice(r.body.as_deref().unwrap()).unwrap();
            assert_eq!(body["operationName"], COMMENTS_OPERATION);
            assert_eq!(body["extensions"]["persistedQuery"]["sha256Hash"], HASH);
            assert_eq!(r.url, "https://gql.test/gql");
            assert!(
                r.headers
                    .contains(&("Client-ID".into(), "test-client-id-not-real".into()))
            );
            body["variables"].clone()
        })
        .collect()
}

#[test]
fn flag_hash_kill_switch_and_flavor_decide() {
    assert!(gql_on().is_on());
    let off = |c: &RemoteConfig, f| ChatReplayGql::resolve_for(c, &ctx(), f);
    assert_eq!(
        off(&config(true, Some(HASH), false), Flavor::Store),
        ChatReplayGql::Off(ChatReplayOff::StoreBuild)
    );
    assert_eq!(
        off(&config(false, Some(HASH), false), Flavor::Full),
        ChatReplayGql::Off(ChatReplayOff::FlagOff)
    );
    assert_eq!(
        off(&config(true, None, false), Flavor::Full),
        ChatReplayGql::Off(ChatReplayOff::MissingQueryHash)
    );
    assert_eq!(
        off(&config(true, Some(HASH), true), Flavor::Full),
        ChatReplayGql::Off(ChatReplayOff::FlagOff),
        "the kill-switch turns the replay off for this version"
    );
    let t = Arc::new(FixtureTransport::new());
    let err = OnlineReplay::new(
        &off(&config(false, Some(HASH), false), Flavor::Full),
        VOD,
        t.clone(),
    )
    .unwrap_err();
    assert_eq!(err.code(), "flag_off");
    assert!(OnlineReplay::new(&gql_on(), "../1", t.clone()).is_err());
    assert!(t.requests().is_empty(), "no request when off");
}

#[tokio::test]
async fn gql_pages_become_twitchdownloader_comments() {
    let t = Arc::new(FixtureTransport::new());
    t.push_fixture(fx!("gql_page_0"))
        .push_fixture(fx!("gql_page_1"))
        .push_fixture(fx!("gql_page_last"));
    let r = replay(&t);

    // Playback starts: one page around 0 s.
    let c = r.ensure(0).await.unwrap();
    assert_eq!((c.from_ms, c.to_ms), (0, Some(3_000)), "the last second may go on");
    assert_eq!(variables(&t)[0]["contentOffsetSeconds"], 0);
    assert_eq!(variables(&t)[0]["videoID"], VOD);
    // Cached: no second request.
    r.ensure(2_500).await.unwrap();
    assert_eq!(t.requests().len(), 1);

    // Prefetch ahead goes on from the end of the cache.
    let c = r.ensure(5_000).await.unwrap();
    assert_eq!((c.from_ms, c.to_ms), (0, Some(6_000)));
    assert_eq!(variables(&t)[1]["contentOffsetSeconds"], 3);
    let c = r.ensure(9_000).await.unwrap();
    assert_eq!((c.from_ms, c.to_ms), (0, None), "last page: up to the end");
    assert_eq!(variables(&t)[2]["contentOffsetSeconds"], 6);
    assert_eq!(t.remaining(), 0);
    assert!(r.ensure(3_600_000).await.is_ok(), "the end is cached");
    assert_eq!(t.requests().len(), 3);

    let stats = r.stats();
    assert_eq!((stats.pages, stats.comments, stats.deleted), (3, 10, 1));
    assert_eq!(r.channel_id().as_deref(), Some("7001"));
    let ids: Vec<String> = r.with_timeline(|tl| {
        tl.items()
            .iter()
            .map(|x| x.comment.id.clone())
            .collect()
    });
    assert_eq!(
        ids,
        [
            "c-0001", "c-0002", "c-0003", "c-0004", "c-0005", "c-0006", "c-0008", "c-0009",
            "c-0010", "c-0011"
        ],
        "sorted, duplicates once, the comment removed by a moderator never"
    );

    // The TwitchDownloader shape of one comment.
    let file = r.to_file("fake_streamer_one", Some("Made-up"));
    let json: serde_json::Value = serde_json::from_slice(&file.to_json()).unwrap();
    let kappa = &json["comments"][1];
    assert_eq!(kappa["_id"], "c-0002");
    assert_eq!(kappa["content_offset_seconds"], 0.0);
    assert_eq!(kappa["content_id"], VOD);
    assert_eq!(kappa["channel_id"], "7001");
    assert_eq!(kappa["commenter"]["name"], "fake_viewer_two");
    assert_eq!(kappa["commenter"]["display_name"], "Fake_Viewer_Two");
    assert_eq!(kappa["message"]["body"], "hello Kappa");
    assert_eq!(kappa["message"]["fragments"][1]["text"], "Kappa");
    assert_eq!(
        kappa["message"]["fragments"][1]["emoticon"]["emoticon_id"],
        "25"
    );
    assert!(kappa["message"]["fragments"][0]["emoticon"].is_null());
    let first = &json["comments"][0];
    assert_eq!(first["message"]["user_badges"][0]["_id"], "subscriber");
    assert_eq!(first["message"]["user_badges"][0]["version"], "12");
    assert_eq!(first["message"]["user_color"], "#1E90FF");
    assert_eq!(json["streamer"]["id"], "7001");
    assert_eq!(json["video"]["id"], VOD);
    assert_eq!(json["video"]["end"], 10.0);
    // A deleted account: the comment stays, without a name.
    assert_eq!(json["comments"][9]["commenter"]["name"], "");
    // What we write reads back the same.
    let back = ChatReplayFile::parse(&file.to_json()).unwrap();
    assert_eq!(back, file);
}

/// TWI-016 "done when": after a seek the chat shows exactly the comments
/// up to the new video position.
#[tokio::test]
async fn after_a_seek_the_chat_matches_the_video_position() {
    let t = Arc::new(FixtureTransport::new());
    t.push_fixture(fx!("gql_page_0"))
        .push_fixture(fx!("gql_page_1"))
        .push_fixture(fx!("gql_page_last"));
    let r = replay(&t);
    r.ensure(0).await.unwrap();
    r.ensure(5_000).await.unwrap();
    r.ensure(9_000).await.unwrap();

    r.with_timeline(|tl| {
        let at: Vec<u64> = tl.items().iter().map(|x| x.at_ms).collect();
        // Two comments in second 0 are spread over it.
        assert_eq!(&at[..4], &[0, 500, 1_000, 2_000]);
        for position in [0u64, 499, 500, 2_400, 3_500, 6_000, 7_999, 8_000, 9_000, 60_000] {
            let window = tl.window(position, 3);
            let shown = &tl.items()[window.clone()];
            assert!(
                shown.iter().all(|x| x.at_ms <= position),
                "nothing from the future at {position}"
            );
            if let Some(next) = tl.items().get(window.end) {
                assert!(next.at_ms > position, "nothing missing at {position}");
            }
            assert!(shown.len() <= 3);
        }
        // Forward play adds exactly the comments between two ticks.
        let added = tl.between(Some(2_400), 5_600);
        let ids: Vec<&str> = tl.items()[added]
            .iter()
            .map(|x| x.comment.id.as_str())
            .collect();
        assert_eq!(ids, ["c-0005", "c-0006", "c-0008"]);
        // A seek back to 1.2 s: the window ends with the comment of 1 s.
        let back = tl.window(1_200, 50);
        assert_eq!(tl.items()[back.end - 1].comment.id, "c-0003");
        assert_eq!(back, 0..3);
    });
}

#[tokio::test]
async fn a_seek_far_ahead_starts_just_before_the_new_position() {
    let t = Arc::new(FixtureTransport::new());
    t.push_fixture(fx!("gql_page_last"));
    let r = replay(&t);
    r.ensure(3_600_000).await.unwrap();
    assert_eq!(
        variables(&t)[0]["contentOffsetSeconds"],
        3_580,
        "20 s of look-back"
    );
}

#[tokio::test]
async fn a_page_inside_one_second_goes_on_by_cursor() {
    let t = Arc::new(FixtureTransport::new());
    let errors: serde_json::Value = serde_json::from_str(fx!("gql_errors")).unwrap();
    t.push_fixture(fx!("gql_one_second"))
        .push_fixture(&errors["integrity"].to_string())
        .push_fixture(fx!("gql_after_one_second"))
        .push_fixture(fx!("gql_page_last"));
    let r = replay(&t);
    let c = r.ensure(70_000).await.unwrap();
    let v = variables(&t);
    assert_eq!(v[0]["contentOffsetSeconds"], 50);
    assert_eq!(v[1]["cursor"], "b25lLTM=", "after the last comment of the page");
    assert!(v[1].get("contentOffsetSeconds").is_none());
    // The cursor is refused: the rest of second 50 is skipped.
    assert_eq!(v[2]["contentOffsetSeconds"], 51);
    // About a second per page and 17 s still to go: start again just
    // before the playhead instead of paging through the gap.
    assert_eq!(v[3]["contentOffsetSeconds"], 69);
    assert_eq!((c.from_ms, c.to_ms), (69_000, None));
    let before = r.covered(51_500).unwrap();
    assert_eq!((before.from_ms, before.to_ms), (50_000, Some(53_000)));
    assert!(r.covered(60_000).is_none(), "the gap is fetched when needed");
    assert_eq!(r.stats().comments, 8);
}

#[tokio::test]
async fn gql_failures_have_codes() {
    let errors: serde_json::Value = serde_json::from_str(fx!("gql_errors")).unwrap();
    for (name, code) in [
        ("hash_invalid", "gql_hash_invalid"),
        ("integrity", "integrity_required"),
        ("no_video", "not_found"),
    ] {
        let t = Arc::new(FixtureTransport::new());
        t.push_fixture(&errors[name].to_string());
        let err = replay(&t).ensure(0).await.unwrap_err();
        assert_eq!(err.code(), code, "{name}");
    }
    let t = Arc::new(FixtureTransport::new());
    t.push_error("connection reset");
    let err = replay(&t).ensure(0).await.unwrap_err();
    assert!(matches!(err, ChatReplayError::Transport(_)));
}

#[test]
fn twitchdownloader_file_and_its_gzip() {
    let raw = fx!("twitchdownloader_chat").as_bytes();
    let file = ChatReplayFile::parse(raw).unwrap();
    assert_eq!(file.channel_id().as_deref(), Some("7001"));
    assert_eq!(file.channel_name(), "Fake_Streamer_One");
    assert_eq!(file.comments.len(), 3);
    let tl = ReplayTimeline::from_comments(file.comments.clone());
    let ids: Vec<&str> = tl.items().iter().map(|x| x.comment.id.as_str()).collect();
    assert_eq!(ids, ["t-0001", "t-0003"], "the deleted comment is left out");
    assert_eq!(tl.items()[1].at_ms, 7_250, "fractions are kept");
    assert_eq!(tl.count_until(7_249), 1);
    let first = &tl.items()[0].comment;
    assert_eq!(first.text(), "hello Kappa");
    assert_eq!(first.message.bits_spent, Some(0));
    // Unknown fields survive a round trip (TwitchDownloader reads them).
    let out: serde_json::Value = serde_json::from_slice(&file.to_json()).unwrap();
    assert_eq!(out["video"]["viewCount"], 42);
    assert_eq!(out["comments"][0]["commenter"]["logo"], "https://example.invalid/logo.png");
    assert_eq!(out["comments"][0]["message"]["emoticons"][0]["begin"], 6);
    assert_eq!(out["FileInfo"]["Version"]["Minor"], 4);

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(raw).unwrap();
    let packed = gz.finish().unwrap();
    assert_eq!(ChatReplayFile::parse(&packed).unwrap(), file);
}
