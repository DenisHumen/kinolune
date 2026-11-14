// SPDX-License-Identifier: MPL-2.0
//! TWI-008: VOD chapters and categories. Hand-written fixtures with a
//! made-up channel (`fake_cozy_caster`, id 7001) and made-up categories; no
//! network: the fixture transport fails on any unplanned request.
//!
//! The channel streamed "Pixel Dungeon" (1001), then "Cozy Chat Corner"
//! (3003); now it is in "Moonlit Farm" (2002). The filter "VODs where Pixel
//! Dungeon was played" must find the VOD by its chapters, not by the
//! channel's current category.

use std::sync::Arc;

use pa_library::open_in_memory;
use pa_library::twitch::{self as lib, SegmentOwner, VideoQuery};
use pa_remote_config::{ClientContext, KillSwitch, RemoteConfig, flags};
use pa_twitch::chapters::{
    self, CHAPTERS_OPERATION, ChapterResolver, ChapterSource, ChaptersGql, ChaptersGqlOff,
    LiveTimeline, TimelineChange, TimelineEvent, TimelineTracker, VodChaptersRequest, store,
};
use pa_twitch::playback::Flavor;
use pa_twitch::testing::{FixtureTransport, ManualClock};
use pa_twitch::{
    AccessToken, ClientId, HelixClient, HelixConfig, StreamsQuery, Video, parse_rfc3339_ms,
};

macro_rules! fx {
    ($name:literal) => {
        include_str!(concat!("fixtures/chapters/", $name, ".json"))
    };
}

const USER: &str = "7001";
const STREAM: &str = "50000000001";
const VOD: &str = "2300000002";
const OLD_VOD: &str = "2300000001";
const PIXEL_DUNGEON: &str = "1001";
const MOONLIT_FARM: &str = "2002";
const CHAT_CORNER: &str = "3003";
const HOUR_MS: u64 = 3_600_000;

fn ms(rfc3339: &str) -> i64 {
    parse_rfc3339_ms(rfc3339).unwrap()
}

fn now() -> i64 {
    ms("2026-11-12T21:30:00Z")
}

struct Harness {
    helix: HelixClient,
    transport: Arc<FixtureTransport>,
}

fn harness() -> Harness {
    let transport = Arc::new(FixtureTransport::new());
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let mut config = HelixConfig::new(ClientId::new("testclientid0000000000000000000").unwrap());
    config.base_url = "https://helix.test/helix".to_owned();
    let helix = HelixClient::with_clock(config, transport.clone(), clock);
    helix.set_access_token(AccessToken::new("test-user-token-not-real"));
    Harness { helix, transport }
}

impl Harness {
    fn urls(&self) -> Vec<String> {
        self.transport
            .requests()
            .into_iter()
            .map(|r| r.url.replace("https://helix.test/helix/", ""))
            .collect()
    }
}

fn ctx() -> ClientContext {
    ClientContext {
        app_version: "0.1.0".into(),
        platform: "linux".into(),
    }
}

fn chapters_config() -> RemoteConfig {
    RemoteConfig::parse(fx!("remote-config-chapters").as_bytes()).unwrap()
}

fn videos() -> Vec<Video> {
    let body: serde_json::Value = serde_json::from_str(fx!("videos_archive")).unwrap();
    serde_json::from_value(body["body"]["data"].clone()).unwrap()
}

fn video(id: &str) -> Video {
    videos().into_iter().find(|v| v.id == id).unwrap()
}

fn off() -> ChaptersGql {
    ChaptersGql::Off(ChaptersGqlOff::FlagOff)
}

fn resolver(gql: ChaptersGql, h: &Harness) -> ChapterResolver {
    let r = ChapterResolver::new(gql, h.transport.clone());
    #[cfg(pa_twitch_native)]
    let r = r.with_gql_endpoint("https://gql.test/gql");
    r
}

/// Feeds the recorded EventSub messages to a tracker.
fn eventsub_timeline() -> (TimelineTracker, Vec<TimelineChange>) {
    let mut tracker = TimelineTracker::new();
    let mut changes = Vec::new();
    for message in [
        fx!("eventsub_update_before_live"),
        fx!("eventsub_keepalive"),
        fx!("eventsub_online"),
        fx!("eventsub_update_switch"),
        fx!("eventsub_offline"),
    ] {
        if let Some(event) = chapters::parse_eventsub_message(message.as_bytes()).unwrap() {
            changes.extend(tracker.apply(event, ChapterSource::EventSub));
        }
    }
    (tracker, changes)
}

#[tokio::test]
async fn vod_filter_finds_the_game_by_chapters_not_by_current_category() {
    let h = harness();
    let mut conn = open_in_memory().unwrap();

    // 1. Own timeline from EventSub, stored as it goes.
    let (tracker, changes) = eventsub_timeline();
    assert!(matches!(changes[0], TimelineChange::Started(_)));
    assert!(matches!(changes[1], TimelineChange::Updated(_)));
    let TimelineChange::Ended(timeline) = changes.last().unwrap().clone() else {
        panic!("broadcast must end: {changes:?}");
    };
    assert!(tracker.live(USER).is_none());
    for change in &changes {
        store::store_live_timeline(&mut conn, change.timeline(), now()).unwrap();
    }
    assert_eq!(timeline.started_at_ms, ms("2026-11-12T18:00:00Z"));
    assert_eq!(timeline.ended_at_ms, Some(ms("2026-11-12T21:00:00Z")));
    let live = lib::segments(&conn, &SegmentOwner::Stream(STREAM.into())).unwrap();
    assert_eq!(live.len(), 2);
    assert_eq!(live[0].source, "eventsub");

    // 2. After the broadcast: its archive VOD by stream id (Helix only).
    h.transport.push_fixture(fx!("videos_archive"));
    let vod = h
        .helix
        .archive_for_stream(USER, STREAM)
        .await
        .unwrap()
        .expect("archive of the stream");
    assert_eq!(vod.id, VOD);

    // 3. Chapters from the timeline: no GQL, no fallback request.
    let resolved = resolver(off(), &h)
        .resolve(
            &h.helix,
            VodChaptersRequest {
                video: &vod,
                timeline: Some(&timeline),
                stored: None,
                now_ms: now(),
            },
        )
        .await
        .unwrap();
    assert_eq!(resolved.gql_failure, None);
    let chapters = resolved.chapters.clone().unwrap();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].game.id, PIXEL_DUNGEON);
    assert_eq!(chapters[0].game.name, "Pixel Dungeon");
    assert_eq!(
        (chapters[0].start_ms, chapters[0].end_ms),
        (0, Some(90 * 60_000))
    );
    assert_eq!(chapters[1].game.id, CHAT_CORNER);
    assert_eq!(
        (chapters[1].start_ms, chapters[1].end_ms),
        (90 * 60_000, Some(3 * HOUR_MS))
    );
    assert!(chapters.iter().all(|c| !c.is_approximate()));
    assert!(store::store_vod_chapters(&mut conn, &vod, &resolved, now()).unwrap());
    assert_eq!(h.urls(), ["videos?user_id=7001&type=archive&first=20"]);

    // The stored VOD is linked to its live session, categories are named.
    let session = lib::live_session(&conn, STREAM).unwrap().unwrap();
    assert_eq!(session.video_id.as_deref(), Some(VOD));
    let category = lib::category(&conn, PIXEL_DUNGEON).unwrap().unwrap();
    assert_eq!(category.name, "Pixel Dungeon");
    assert_eq!(
        store::stored_source(&conn, VOD).unwrap(),
        Some(ChapterSource::EventSub)
    );

    // 4. The channel is in Moonlit Farm now. The filter by Pixel Dungeon
    //    finds the VOD across channels and on the channel page; the
    //    current category does not.
    let found = lib::category_videos(&conn, 1, PIXEL_DUNGEON, &VideoQuery::default()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].video.video_id, VOD);
    let by_channel = lib::channel_videos(
        &conn,
        1,
        USER,
        &VideoQuery {
            game_id: Some(PIXEL_DUNGEON.into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(by_channel.len(), 1);
    assert!(
        lib::category_videos(&conn, 1, MOONLIT_FARM, &VideoQuery::default())
            .unwrap()
            .is_empty()
    );
    let chat = lib::category_videos(&conn, 1, CHAT_CORNER, &VideoQuery::default()).unwrap();
    assert_eq!(chat.len(), 1);
}

#[tokio::test]
async fn polling_builds_the_same_timeline_with_poll_precision() {
    let h = harness();
    let mut tracker = TimelineTracker::new();
    let polls = [
        (fx!("streams_poll_1"), "2026-11-12T18:01:30Z"),
        (fx!("streams_poll_1"), "2026-11-12T19:00:00Z"),
        (fx!("streams_poll_2"), "2026-11-12T19:31:00Z"),
        (fx!("streams_empty"), "2026-11-12T21:01:00Z"),
    ];
    let mut changes = Vec::new();
    for (fixture, at) in polls {
        h.transport.push_fixture(fixture);
        let page = h
            .helix
            .get_streams(&StreamsQuery {
                user_ids: vec![USER.into()],
                ..Default::default()
            })
            .await
            .unwrap();
        changes.push(tracker.observe_poll(&[USER], &page.items, ms(at)));
    }
    assert!(matches!(changes[0][..], [TimelineChange::Started(_)]));
    assert!(changes[1].is_empty(), "same category: nothing to store");
    assert!(matches!(changes[2][..], [TimelineChange::Updated(_)]));
    let [TimelineChange::Ended(timeline)] = &changes[3][..] else {
        panic!("{:?}", changes[3]);
    };
    let ch = timeline.chapters();
    assert_eq!(ch.len(), 2);
    assert!(ch.iter().all(|c| c.source == ChapterSource::Polling));
    // The first category covers the stream from its start; the switch and
    // the end are seen at the poll.
    assert_eq!((ch[0].start_ms, ch[0].end_ms), (0, Some(91 * 60_000)));
    assert_eq!(ch[1].game.id, CHAT_CORNER);
    assert_eq!(ch[1].end_ms, Some(3 * HOUR_MS + 60_000));
    // Mapped onto the 3-hour VOD the last chapter is clamped.
    let vod = timeline.vod_chapters(3 * HOUR_MS);
    assert_eq!(vod[1].end_ms, Some(3 * HOUR_MS));

    // Stored and resumed after a restart.
    let mut conn = open_in_memory().unwrap();
    let TimelineChange::Updated(live) = &changes[2][0] else {
        unreachable!()
    };
    store::store_live_timeline(&mut conn, live, now()).unwrap();
    let loaded = store::load_live_timeline(&conn, STREAM).unwrap().unwrap();
    assert_eq!(loaded.ended_at_ms, None);
    assert_eq!(loaded.chapters(), live.chapters());
    let mut resumed = TimelineTracker::new();
    resumed.resume(loaded);
    let end = resumed.apply(
        TimelineEvent::Offline {
            user_id: USER.into(),
            at_ms: ms("2026-11-12T21:00:00Z"),
        },
        ChapterSource::EventSub,
    );
    let [TimelineChange::Ended(done)] = &end[..] else {
        panic!("{end:?}");
    };
    assert_eq!(done.vod_chapters(3 * HOUR_MS).len(), 2);
}

#[test]
fn eventsub_messages_and_subscriptions() {
    assert_eq!(
        chapters::parse_eventsub_message(fx!("eventsub_keepalive").as_bytes()).unwrap(),
        None
    );
    // A rerun has no archive of its own: not a broadcast.
    assert_eq!(
        chapters::parse_eventsub_message(fx!("eventsub_online_rerun").as_bytes()).unwrap(),
        None
    );
    assert!(chapters::parse_eventsub_message(b"{not json").is_err());
    let update = chapters::parse_eventsub_message(fx!("eventsub_update_switch").as_bytes())
        .unwrap()
        .unwrap();
    let TimelineEvent::Update {
        user_id,
        game,
        title,
        at_ms,
    } = update
    else {
        panic!("{update:?}");
    };
    assert_eq!(user_id, USER);
    assert_eq!(game.unwrap().name, "Cozy Chat Corner");
    assert_eq!(title.as_deref(), Some("Chatting with chat"));
    assert_eq!(at_ms, ms("2026-11-12T19:30:00Z"));

    let subs = chapters::timeline_subscriptions(USER, "test-session");
    let kinds: Vec<_> = subs
        .iter()
        .map(|s| (s.kind.as_str(), s.version.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("stream.online", "1"),
            ("channel.update", "2"),
            ("stream.offline", "1")
        ]
    );
    assert!(
        subs.iter()
            .all(|s| s.condition["broadcaster_user_id"] == USER && s.session_id == "test-session")
    );
}

#[test]
fn online_without_a_known_category_is_seeded_from_helix_channels() {
    let mut tracker = TimelineTracker::new();
    let event = chapters::parse_eventsub_message(fx!("eventsub_online").as_bytes())
        .unwrap()
        .unwrap();
    let started = tracker.apply(event, ChapterSource::EventSub);
    assert!(started[0].timeline().segments.is_empty());
    let body: serde_json::Value = serde_json::from_str(fx!("channels_now")).unwrap();
    let channel: pa_twitch::ChannelInfo =
        serde_json::from_value(body["body"]["data"][0].clone()).unwrap();
    let seeded = tracker.apply(
        TimelineEvent::from_channel(&channel, ms("2026-11-12T18:05:00Z")),
        ChapterSource::EventSub,
    );
    let ch = seeded[0].timeline().chapters();
    assert_eq!(ch[0].game.id, MOONLIT_FARM);
    assert_eq!(ch[0].start_ms, 0);
}

#[tokio::test]
async fn fresh_vod_without_timeline_gets_an_approximate_chapter_once() {
    let h = harness();
    let mut conn = open_in_memory().unwrap();
    let vod = video(VOD);
    h.transport.push_fixture(fx!("channels_now"));
    let r = resolver(off(), &h);
    let resolved = r
        .resolve(
            &h.helix,
            VodChaptersRequest {
                video: &vod,
                timeline: None,
                stored: None,
                now_ms: now(),
            },
        )
        .await
        .unwrap();
    let ch = resolved.chapters.clone().unwrap();
    assert_eq!(ch.len(), 1);
    assert!(ch[0].is_approximate(), "fallback is marked ≈");
    assert_eq!(ch[0].game.id, MOONLIT_FARM);
    assert_eq!((ch[0].start_ms, ch[0].end_ms), (0, Some(3 * HOUR_MS)));
    store::store_vod_chapters(&mut conn, &vod, &resolved, now()).unwrap();
    assert_eq!(h.urls(), ["channels?broadcaster_id=7001"]);
    let stored = store::stored_source(&conn, VOD).unwrap();
    assert_eq!(stored, Some(ChapterSource::Fallback));
    assert_eq!(
        lib::segments(&conn, &SegmentOwner::Video(VOD.into())).unwrap()[0].source,
        "fallback"
    );

    // Next time: kept, no request.
    let again = r
        .resolve(
            &h.helix,
            VodChaptersRequest {
                video: &vod,
                timeline: None,
                stored,
                now_ms: now(),
            },
        )
        .await
        .unwrap();
    assert_eq!(again.chapters, None);
    assert_eq!(h.transport.requests().len(), 1);

    // A later own timeline replaces the fallback.
    let (_, changes) = eventsub_timeline();
    let timeline = changes.last().unwrap().timeline().clone();
    let better = r
        .resolve(
            &h.helix,
            VodChaptersRequest {
                video: &vod,
                timeline: Some(&timeline),
                stored,
                now_ms: now(),
            },
        )
        .await
        .unwrap();
    assert_eq!(better.chapters.unwrap()[0].source, ChapterSource::EventSub);
}

#[tokio::test]
async fn old_vod_without_sources_gets_no_guess() {
    let h = harness();
    let vod = video(OLD_VOD);
    // A timeline of another broadcast does not apply either.
    let other = LiveTimeline::new(STREAM, USER, ms("2026-11-12T18:00:00Z"));
    let resolved = resolver(off(), &h)
        .resolve(
            &h.helix,
            VodChaptersRequest {
                video: &vod,
                timeline: Some(&other),
                stored: None,
                now_ms: now(),
            },
        )
        .await
        .unwrap();
    assert_eq!(resolved.chapters, None);
    assert!(h.transport.requests().is_empty());
}

#[test]
fn gql_chapters_need_flag_full_build_and_a_hash_from_config() {
    let config = chapters_config();
    let ChaptersGql::On(settings) = ChaptersGql::resolve_for(&config, &ctx(), Flavor::Full) else {
        panic!("flag on, hash present");
    };
    assert_eq!(settings.client_id, "test-client-id-not-real");
    assert_eq!(settings.moments_hash, "0".repeat(63) + "2");
    assert_eq!(
        ChaptersGql::resolve_for(&config, &ctx(), Flavor::Store),
        ChaptersGql::Off(ChaptersGqlOff::StoreBuild)
    );

    let mut no_flag = config.clone();
    no_flag.flags.insert(flags::TWITCH_CHAPTERS_GQL.into(), false);
    assert_eq!(
        ChaptersGql::resolve_for(&no_flag, &ctx(), Flavor::Full),
        ChaptersGql::Off(ChaptersGqlOff::FlagOff)
    );

    let mut killed = config.clone();
    killed.kill_switches.push(KillSwitch {
        feature: flags::TWITCH_CHAPTERS_GQL.into(),
        min_app_version: None,
        max_app_version: Some("0.1.0".into()),
        platforms: vec![],
        reason: Some("moments query changed".into()),
    });
    assert_eq!(
        ChaptersGql::resolve_for(&killed, &ctx(), Flavor::Full),
        ChaptersGql::Off(ChaptersGqlOff::FlagOff)
    );

    let mut no_hash = config.clone();
    no_hash.twitch.persisted_queries.remove(CHAPTERS_OPERATION);
    assert_eq!(
        ChaptersGql::resolve_for(&no_hash, &ctx(), Flavor::Full),
        ChaptersGql::Off(ChaptersGqlOff::MissingQueryHash)
    );

    // The built-in default config keeps GQL off.
    assert!(!ChaptersGql::resolve(&pa_remote_config::builtin(), &ctx()).is_on());
    assert!(
        !ChaptersGql::resolve_for(&pa_remote_config::builtin(), &ctx(), Flavor::Full).is_on()
    );
}

#[cfg(not(pa_twitch_native))]
#[tokio::test]
async fn store_build_never_sends_gql_even_if_the_config_allows_it() {
    let h = harness();
    let gql = ChaptersGql::resolve_for(&chapters_config(), &ctx(), Flavor::Full);
    assert!(gql.is_on());
    let r = resolver(gql, &h);
    assert!(!r.uses_gql());
    h.transport.push_fixture(fx!("channels_now"));
    let resolved = r
        .resolve(
            &h.helix,
            VodChaptersRequest {
                video: &video(VOD),
                timeline: None,
                stored: None,
                now_ms: now(),
            },
        )
        .await
        .unwrap();
    assert_eq!(resolved.chapters.unwrap()[0].source, ChapterSource::Fallback);
    assert_eq!(h.urls(), ["channels?broadcaster_id=7001"]);
}

#[cfg(pa_twitch_native)]
mod gql {
    use super::*;

    fn gql_on() -> ChaptersGql {
        ChaptersGql::resolve_for(&chapters_config(), &ctx(), Flavor::Full)
    }

    fn request(video: &Video, stored: Option<ChapterSource>) -> VodChaptersRequest<'_> {
        VodChaptersRequest {
            video,
            timeline: None,
            stored,
            now_ms: now(),
        }
    }

    #[tokio::test]
    async fn moments_become_chapters_and_are_never_fetched_again() {
        let h = harness();
        let mut conn = open_in_memory().unwrap();
        let vod = video(OLD_VOD);
        let r = resolver(gql_on(), &h);
        assert!(r.uses_gql());
        h.transport.push_fixture(fx!("gql_moments"));
        let resolved = r.resolve(&h.helix, request(&vod, None)).await.unwrap();
        let ch = resolved.chapters.clone().unwrap();
        assert_eq!(ch.len(), 2, "non-GAME_CHANGE moments are ignored");
        assert_eq!(ch[0].game.id, MOONLIT_FARM);
        assert_eq!((ch[0].start_ms, ch[0].end_ms), (0, Some(2_700_000)));
        assert_eq!(ch[1].game.id, PIXEL_DUNGEON);
        assert_eq!(
            (ch[1].start_ms, ch[1].end_ms),
            (2_700_000, Some(2 * HOUR_MS))
        );
        assert!(ch.iter().all(|c| c.source == ChapterSource::GqlMoments));
        assert_eq!(
            ch[1].game.box_art_url.as_deref(),
            Some("https://example.invalid/boxart/1001-40x53.jpg")
        );

        // The request carries the client-id and the hash from the config.
        let sent = h.transport.requests();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url, "https://gql.test/gql");
        assert!(
            sent[0]
                .headers
                .contains(&("Client-ID".into(), "test-client-id-not-real".into()))
        );
        let body: serde_json::Value =
            serde_json::from_slice(sent[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["operationName"], CHAPTERS_OPERATION);
        assert_eq!(body["variables"]["videoID"], OLD_VOD);
        assert_eq!(
            body["extensions"]["persistedQuery"]["sha256Hash"],
            "0".repeat(63) + "2"
        );

        store::store_vod_chapters(&mut conn, &vod, &resolved, now()).unwrap();
        let stored = store::stored_source(&conn, OLD_VOD).unwrap();
        assert_eq!(stored, Some(ChapterSource::GqlMoments));
        let found = lib::category_videos(&conn, 1, PIXEL_DUNGEON, &VideoQuery::default()).unwrap();
        assert_eq!(found[0].video.video_id, OLD_VOD);
        assert_eq!(
            lib::category(&conn, PIXEL_DUNGEON)
                .unwrap()
                .unwrap()
                .box_art_url
                .as_deref(),
            Some("https://example.invalid/boxart/1001-40x53.jpg")
        );

        // Cached: no second request.
        let again = r.resolve(&h.helix, request(&vod, stored)).await.unwrap();
        assert_eq!(again.chapters, None);
        assert_eq!(h.transport.requests().len(), 1);
    }

    #[tokio::test]
    async fn vod_without_moments_gets_its_category_as_one_chapter() {
        let h = harness();
        h.transport.push_fixture(fx!("gql_moments_none_with_game"));
        let resolved = resolver(gql_on(), &h)
            .resolve(&h.helix, request(&video(OLD_VOD), None))
            .await
            .unwrap();
        let ch = resolved.chapters.unwrap();
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].game.id, MOONLIT_FARM);
        assert_eq!(ch[0].game.box_art_url, None);
        assert_eq!(ch[0].end_ms, Some(2 * HOUR_MS));
        assert!(!ch[0].is_approximate());
    }

    #[tokio::test]
    async fn stale_hash_falls_through_to_the_fallback() {
        let h = harness();
        h.transport
            .push_fixture(fx!("gql_persisted_query_not_found"))
            .push_fixture(fx!("channels_now"));
        let resolved = resolver(gql_on(), &h)
            .resolve(&h.helix, request(&video(VOD), None))
            .await
            .unwrap();
        assert_eq!(resolved.gql_failure, Some("gql_hash_invalid"));
        assert!(resolved.chapters.unwrap()[0].is_approximate());

        // An old VOD: the failure is reported, nothing is guessed.
        let h = harness();
        h.transport.push_error("offline");
        let resolved = resolver(gql_on(), &h)
            .resolve(&h.helix, request(&video(OLD_VOD), None))
            .await
            .unwrap();
        assert_eq!(resolved.gql_failure, Some("network"));
        assert_eq!(resolved.chapters, None);
        assert_eq!(h.transport.requests().len(), 1);
    }

    #[tokio::test]
    async fn flag_off_sends_no_gql() {
        let h = harness();
        let resolved = resolver(off(), &h)
            .resolve(&h.helix, request(&video(OLD_VOD), None))
            .await
            .unwrap();
        assert_eq!(resolved.chapters, None);
        assert!(h.transport.requests().is_empty());
    }
}
