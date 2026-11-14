// SPDX-License-Identifier: MPL-2.0
//! Background checks (`check_once`, the Android WorkManager path) and the
//! Helix implementation of `LiveApi` on recorded fixtures.

use std::sync::Arc;
use std::time::Duration;

use pa_library::twitch::{self as lt, SegmentOwner};
use pa_twitch::testing::{FixtureTransport, ManualClock};
use pa_twitch::{AccessToken, ClientId, Clock, HelixClient, HelixConfig, Method};
use pa_twitch_live::demo::FakeTwitch;
use pa_twitch_live::{
    LibraryTimelineStore, LiveApi, LiveError, SubscriptionPlan, WatchedChannel, check_once,
};

fn unix_ms(clock: &dyn Clock) -> i64 {
    i64::try_from(
        clock
            .now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(0)
}

#[tokio::test]
async fn background_runs_announce_each_broadcast_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("library.db");
    let db = db.to_str().expect("utf-8 path");
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let fake = FakeTwitch::demo(clock.clone());
    let channels = FakeTwitch::demo_channels();
    let (a, b) = (channels[0].user_id.clone(), channels[1].user_id.clone());
    // WorkManager: every 15 minutes at best; Doze stretches it.
    let max_age = Duration::from_secs(45 * 60);

    // Run 1: nothing live.
    let mut store = LibraryTimelineStore::open(db).expect("library");
    let r = check_once(&fake, &channels, &mut store, max_age, unix_ms(&*clock))
        .await
        .expect("check");
    assert!(r.alerts.is_empty() && r.live.is_empty());

    // A goes live; the next run (a new process) 15 minutes later announces.
    clock.advance(Duration::from_secs(5 * 60));
    let stream_a = fake.go_live(&a, "Made-up morning show");
    clock.advance(Duration::from_secs(10 * 60));
    let mut store = LibraryTimelineStore::open(db).expect("library");
    let r = check_once(&fake, &channels, &mut store, max_age, unix_ms(&*clock))
        .await
        .expect("check");
    assert_eq!(r.alerts.len(), 1);
    assert_eq!(r.alerts[0].stream_id, stream_a);
    assert_eq!(r.alerts[0].display_name, "Moonlit Pancake");
    assert_eq!(r.alerts[0].game_name.as_deref(), Some("Cozy Farming Sim"));
    assert_eq!(r.alerts[0].delay(), Duration::from_secs(10 * 60));
    drop(store);

    // Run 3: still live, B silenced by its toggle goes live too.
    let mut channels = channels;
    channels[1].notify = false;
    clock.advance(Duration::from_secs(15 * 60));
    fake.go_live(&b, "Quiet one");
    let mut store = LibraryTimelineStore::open(db).expect("library");
    let r = check_once(&fake, &channels, &mut store, max_age, unix_ms(&*clock))
        .await
        .expect("check");
    assert!(r.alerts.is_empty(), "{:?}", r.alerts);
    assert_eq!(r.live.len(), 2);
    drop(store);

    // Run 4: A ended; its VOD gets the chapters of our timeline.
    clock.advance(Duration::from_secs(5 * 60));
    fake.go_offline(&a);
    clock.advance(Duration::from_secs(10 * 60));
    let mut store = LibraryTimelineStore::open(db).expect("library");
    let r = check_once(&fake, &channels, &mut store, max_age, unix_ms(&*clock))
        .await
        .expect("check");
    assert!(r.alerts.is_empty());
    let conn = store.connection();
    let session = lt::live_session(conn, &stream_a)
        .expect("db")
        .expect("session");
    assert!(session.ended_at_ms.is_some());
    let vod = session.video_id.expect("archive linked");
    let chapters = lt::segments(conn, &SegmentOwner::Video(vod)).expect("db");
    assert_eq!(chapters.len(), 1);
    assert_eq!(chapters[0].game_id, "92000001");
    assert_eq!(chapters[0].source, "polling");
}

#[tokio::test]
async fn too_old_for_a_background_run_is_not_news() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("library.db");
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let fake = FakeTwitch::demo(clock.clone());
    let channels = FakeTwitch::demo_channels();
    fake.go_live(&channels[2].user_id, "Since the morning");
    clock.advance(Duration::from_secs(3 * 3600));
    let mut store = LibraryTimelineStore::open(db.to_str().expect("path")).expect("library");
    let r = check_once(
        &fake,
        &channels,
        &mut store,
        Duration::from_secs(45 * 60),
        unix_ms(&*clock),
    )
    .await
    .expect("check");
    assert!(r.alerts.is_empty());
    assert_eq!(r.changes.len(), 1, "tracked for chapters");
}

fn helix(transport: Arc<FixtureTransport>) -> HelixClient {
    let mut config = HelixConfig::new(ClientId::new("made-up-client-id").expect("id"));
    config.max_retries = 0;
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let client = HelixClient::with_clock(config, transport, clock);
    client.set_access_token(AccessToken::new("made-up-token"));
    client
}

#[tokio::test]
async fn helix_streams_in_batches_of_100() {
    let transport = Arc::new(FixtureTransport::new());
    transport.push_fixture(
        r#"{"status":200,"body":{"data":[{"id":"61000001","user_id":"81000001",
        "user_login":"made_up_one","user_name":"Made Up One","game_id":"71000001",
        "game_name":"Invented Game","type":"live","title":"Hello","viewer_count":5,
        "started_at":"2027-01-15T08:00:00Z","language":"en","thumbnail_url":"",
        "tags":[],"is_mature":false}],"pagination":{}}}"#,
    );
    transport.push_fixture(r#"{"status":200,"body":{"data":[],"pagination":{}}}"#);
    let api = helix(transport.clone());
    let ids: Vec<String> = (0..150).map(|i| format!("{}", 81_000_001 + i)).collect();
    let streams = api.live_streams(ids).await.expect("streams");
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].user_name, "Made Up One");
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.method == Method::Get));
    assert_eq!(requests[0].url.matches("user_id=").count(), 100);
    assert_eq!(requests[1].url.matches("user_id=").count(), 50);
}

#[tokio::test]
async fn helix_subscription_answers_map_to_budget_conflict_and_auth() {
    let transport = Arc::new(FixtureTransport::new());
    transport.push_fixture(
        r#"{"status":202,"body":{"data":[{"id":"made-up-sub","status":
        "websocket_connected","type":"stream.online","version":"1","cost":1,
        "condition":{"broadcaster_user_id":"81000001"},"transport":{"method":
        "websocket","session_id":"made-up-session"},"created_at":
        "2027-01-15T08:00:00Z"}],"total":1,"total_cost":1,"max_total_cost":10}}"#,
    );
    transport.push_fixture(
        r#"{"status":409,"body":{"error":"Conflict","status":409,"message":
        "subscription already exists"}}"#,
    );
    transport.push_fixture(
        r#"{"status":429,"body":{"error":"Too Many Requests","status":429,"message":
        "total cost exceeded"}}"#,
    );
    transport.push_fixture(
        r#"{"status":401,"body":{"error":"Unauthorized","status":401,"message":
        "Invalid OAuth token"}}"#,
    );
    let api = helix(transport.clone());
    let plan = SubscriptionPlan::new(&[WatchedChannel::new("81000001", "made_up", "Made Up")], 1);
    let request = plan.subs[0].request("made-up-session");
    let sub = api.subscribe(request.clone()).await.expect("created");
    assert_eq!(sub.cost, 1);
    assert_eq!(api.subscribe(request.clone()).await, Err(LiveError::Conflict));
    assert!(matches!(
        api.subscribe(request.clone()).await,
        Err(LiveError::BudgetExceeded | LiveError::RateLimited(_))
    ));
    assert!(matches!(
        api.subscribe(request).await,
        Err(LiveError::Unauthorized(_))
    ));
    let body = transport.requests()[0].body.clone().expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["type"], "stream.online");
    assert_eq!(body["condition"]["broadcaster_user_id"], "81000001");
    assert_eq!(body["transport"]["method"], "websocket");
}
