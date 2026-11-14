// SPDX-License-Identifier: MPL-2.0
//! The monitor against the made-up Twitch (`demo::FakeTwitch`: Helix and an
//! EventSub WebSocket server) on virtual time: minutes of polling run in
//! milliseconds. Done-when of TWI-009: a notification arrives at most two
//! minutes after the broadcast starts — within seconds over EventSub, within
//! a poll interval plus Twitch's listing lag over polling.

use std::sync::Arc;
use std::time::Duration;

use pa_library::twitch::{self as lt, SegmentOwner};
use pa_twitch::Clock;
use pa_twitch::chapters::{ChapterSource, TimelineChange};
use pa_twitch_live::demo::FakeTwitch;
use pa_twitch_live::{
    EventSubSettings, EventSubState, LibraryTimelineStore, LiveAlert, LiveMonitor, MonitorConfig,
    MonitorDeps, MonitorEvent, MonitorStatus, NoStore, PollPolicy, PowerState, TimelineStore,
    TokioClock, WatchedChannel,
};
use pa_twitch_chat::Backoff;

const TWO_MINUTES: Duration = Duration::from_secs(120);

fn config() -> MonitorConfig {
    MonitorConfig {
        eventsub: EventSubSettings {
            url: pa_twitch_live::demo::FAKE_EVENTSUB_URL.to_owned(),
            backoff: Backoff {
                initial: Duration::from_secs(2),
                max: Duration::from_secs(60),
                jitter: false,
            },
            ..EventSubSettings::default()
        },
        policy: PollPolicy {
            jitter: false,
            ..PollPolicy::default()
        },
        ..MonitorConfig::default()
    }
}

struct Harness {
    fake: FakeTwitch,
    clock: Arc<dyn Clock>,
    monitor: LiveMonitor,
}

fn channels(n: usize) -> Vec<WatchedChannel> {
    (0..n)
        .map(|i| {
            WatchedChannel::new(
                &format!("{}", 81_000_000 + i),
                &format!("made_up_{i}"),
                &format!("Made Up {i}"),
            )
        })
        .collect()
}

fn fake_with(clock: &Arc<dyn Clock>, list: &[WatchedChannel]) -> FakeTwitch {
    let fake = FakeTwitch::new(clock.clone());
    for c in list {
        fake.add_channel(&c.user_id, &c.login, &c.display_name);
        fake.set_category(&c.user_id, Some(("71000001", "Invented Game")), "Warming up");
    }
    fake
}

fn start(
    list: Vec<WatchedChannel>,
    power: PowerState,
    eventsub: bool,
    store: Box<dyn TimelineStore>,
    setup: impl FnOnce(&FakeTwitch),
) -> Harness {
    let clock: Arc<dyn Clock> = Arc::new(TokioClock::at_unix(1_800_000_000));
    let fake = fake_with(&clock, &list);
    setup(&fake);
    let monitor = LiveMonitor::spawn(
        config(),
        list,
        power,
        MonitorDeps {
            api: Arc::new(fake.clone()),
            connector: eventsub.then(|| Arc::new(fake.clone()) as _),
            clock: clock.clone(),
            store,
        },
    );
    Harness {
        fake,
        clock,
        monitor,
    }
}

impl Harness {
    /// Events until `pred` matches, within `limit` of virtual time.
    async fn until<T>(
        &self,
        limit: Duration,
        mut pred: impl FnMut(&MonitorEvent) -> Option<T>,
    ) -> Option<T> {
        tokio::time::timeout(limit, async {
            loop {
                let event = self.monitor.next_event().await?;
                if let Some(t) = pred(&event) {
                    return Some(t);
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    async fn alert(&self, limit: Duration) -> Option<LiveAlert> {
        self.until(limit, |e| match e {
            MonitorEvent::Alert(a) => Some(a.clone()),
            _ => None,
        })
        .await
    }

    async fn connected(&self) -> MonitorStatus {
        self.until(Duration::from_secs(120), |e| match e {
            MonitorEvent::Status(s) if matches!(s.eventsub, EventSubState::Connected(_)) => {
                Some(s.clone())
            }
            _ => None,
        })
        .await
        .expect("EventSub connected")
    }

    /// Lets virtual time pass while draining events; returns the alerts.
    async fn idle(&self, d: Duration) -> Vec<LiveAlert> {
        let mut alerts = Vec::new();
        let _ = tokio::time::timeout(d, async {
            while let Some(e) = self.monitor.next_event().await {
                if let MonitorEvent::Alert(a) = e {
                    alerts.push(a);
                }
            }
        })
        .await;
        alerts
    }
}

#[tokio::test(start_paused = true)]
async fn eventsub_announces_within_seconds() {
    let h = start(channels(4), PowerState::Battery, true, Box::new(NoStore), |_| {});
    let status = h.connected().await;
    assert_eq!(status.eventsub_channels, 4);
    assert_eq!(status.polled_only, 0);
    // 4 channels: online + update for all, offline for two = budget of 10.
    assert_eq!(h.fake.total_cost(), 10);
    assert!(h.idle(Duration::from_secs(7 * 60)).await.is_empty());

    let id = channels(4)[2].user_id.clone();
    let stream = h.fake.go_live(&id, "Invented speedrun");
    let alert = h.alert(TWO_MINUTES).await.expect("alert");
    assert_eq!(alert.stream_id, stream);
    assert_eq!(alert.display_name, "Made Up 2");
    assert_eq!(alert.source, ChapterSource::EventSub);
    assert!(alert.delay() < Duration::from_secs(2), "{:?}", alert.delay());
    // Polls after that must not announce it again.
    assert!(h.idle(Duration::from_secs(10 * 60)).await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn channels_over_budget_are_polled_within_two_minutes() {
    for power in [PowerState::Plugged, PowerState::Battery] {
        let list = channels(15);
        let h = start(list.clone(), power, true, Box::new(NoStore), |f| {
            // Twitch lists a new stream in GET /streams with a delay.
            f.set_streams_lag(Duration::from_secs(25));
        });
        let status = h.connected().await;
        assert_eq!(status.eventsub_channels, 10, "{power:?}");
        assert_eq!(status.polled_only, 5);
        assert_eq!(h.fake.total_cost(), 10);
        // Start broadcasts at different points of the poll cycle.
        for (i, offset) in [0u64, 17, 44, 59, 73, 89].iter().enumerate() {
            h.idle(Duration::from_secs(*offset)).await;
            let id = &list[10 + i % 5].user_id;
            h.fake.go_offline(id);
            let stream = h.fake.go_live(id, "Polled broadcast");
            let alert = h.alert(Duration::from_secs(10 * 60)).await.expect("alert");
            assert_eq!(alert.stream_id, stream);
            assert_eq!(alert.source, ChapterSource::Polling);
            assert!(
                alert.delay() <= TWO_MINUTES,
                "{power:?} offset {offset}: {:?}",
                alert.delay()
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn budget_spent_elsewhere_falls_back_to_polling() {
    let list = channels(6);
    let h = start(list.clone(), PowerState::Plugged, true, Box::new(NoStore), |f| {
        // Another session of the same user (chat, a second device) holds 7.
        f.set_used_elsewhere(7);
    });
    let status = h.connected().await;
    let EventSubState::Connected(cov) = &status.eventsub else {
        unreachable!()
    };
    assert!(cov.budget_hit);
    assert_eq!(cov.online.len(), 3);
    assert_eq!(status.polled_only, 3);
    let stream = h.fake.go_live(&list[5].user_id, "");
    let alert = h.alert(TWO_MINUTES).await.expect("alert");
    assert_eq!(alert.stream_id, stream);
    assert_eq!(alert.source, ChapterSource::Polling);
}

#[tokio::test(start_paused = true)]
async fn polling_only_without_eventsub() {
    let list = channels(3);
    let h = start(list.clone(), PowerState::Plugged, false, Box::new(NoStore), |_| {});
    h.idle(Duration::from_secs(150)).await;
    assert_eq!(h.fake.connects(), 0);
    let stream = h.fake.go_live(&list[0].user_id, "No websocket today");
    let alert = h.alert(TWO_MINUTES).await.expect("alert");
    assert_eq!(alert.stream_id, stream);
    assert!(alert.delay() <= Duration::from_secs(60));
}

#[tokio::test(start_paused = true)]
async fn lost_socket_is_covered_by_polling_and_reconnects() {
    let list = channels(2);
    let h = start(list.clone(), PowerState::Plugged, true, Box::new(NoStore), |_| {});
    h.connected().await;
    assert_eq!(h.fake.total_cost(), 6);
    h.fake.fail_connects(3);
    h.fake.drop_connections();
    let stream = h.fake.go_live(&list[1].user_id, "During the outage");
    let alert = h.alert(TWO_MINUTES).await.expect("alert");
    assert_eq!(alert.stream_id, stream);
    assert_eq!(alert.source, ChapterSource::Polling);
    // Backoff 2+4+8+16 s, then a fresh session with fresh subscriptions.
    h.connected().await;
    assert_eq!(h.fake.total_cost(), 6);
    assert_eq!(h.fake.connects(), 5);
}

#[tokio::test(start_paused = true)]
async fn duplicates_and_session_reconnect_announce_once() {
    let list = channels(1);
    let h = start(list.clone(), PowerState::Plugged, true, Box::new(NoStore), |f| {
        f.duplicate_notifications(true);
    });
    h.connected().await;
    let id = &list[0].user_id;
    h.fake.go_live(id, "Twice?");
    assert!(h.alert(Duration::from_secs(5)).await.is_some());
    assert!(h.idle(Duration::from_secs(5 * 60)).await.is_empty());

    let connects = h.fake.connects();
    h.fake.send_reconnect();
    h.idle(Duration::from_secs(5)).await;
    assert_eq!(h.fake.connects(), connects + 1);
    // Subscriptions moved with the session, nothing was re-created.
    assert_eq!(h.fake.total_cost(), 3);
    h.fake.go_offline(id);
    h.idle(Duration::from_secs(60)).await;
    let stream = h.fake.go_live(id, "Second broadcast");
    let alert = h.alert(Duration::from_secs(5)).await.expect("alert");
    assert_eq!(alert.stream_id, stream);
    assert_eq!(alert.source, ChapterSource::EventSub);
}

#[tokio::test(start_paused = true)]
async fn toggle_off_tracks_without_notifying() {
    let mut list = channels(2);
    list[0].notify = false;
    let h = start(list.clone(), PowerState::Plugged, true, Box::new(NoStore), |_| {});
    h.connected().await;
    let quiet = h.fake.go_live(&list[0].user_id, "Quiet");
    let started = h
        .until(Duration::from_secs(5), |e| match e {
            MonitorEvent::Alert(a) => panic!("unexpected alert {a:?}"),
            MonitorEvent::Timeline(TimelineChange::Started(t)) => Some(t.stream_id.clone()),
            _ => None,
        })
        .await;
    assert_eq!(started.as_deref(), Some(quiet.as_str()));
    assert!(h.idle(Duration::from_secs(3 * 60)).await.is_empty());

    // Turning the toggle on does not announce the running broadcast; the
    // next one is announced.
    list[0].notify = true;
    h.monitor.set_channels(list.clone());
    assert!(h.idle(Duration::from_secs(3 * 60)).await.is_empty());
    h.fake.go_offline(&list[0].user_id);
    h.idle(Duration::from_secs(30)).await;
    let next = h.fake.go_live(&list[0].user_id, "Loud");
    assert_eq!(h.alert(TWO_MINUTES).await.expect("alert").stream_id, next);
}

#[tokio::test(start_paused = true)]
async fn broadcast_running_before_start_is_not_news() {
    let list = channels(1);
    let clock: Arc<dyn Clock> = Arc::new(TokioClock::at_unix(1_800_000_000));
    let fake = fake_with(&clock, &list);
    fake.go_live(&list[0].user_id, "Since an hour");
    tokio::time::sleep(Duration::from_secs(3600)).await;
    let monitor = LiveMonitor::spawn(
        config(),
        list,
        PowerState::Plugged,
        MonitorDeps {
            api: Arc::new(fake.clone()),
            connector: Some(Arc::new(fake.clone())),
            clock: clock.clone(),
            store: Box::new(NoStore),
        },
    );
    let h = Harness {
        fake,
        clock,
        monitor,
    };
    let mut started = 0;
    let _ = tokio::time::timeout(Duration::from_secs(5 * 60), async {
        while let Some(e) = h.monitor.next_event().await {
            match e {
                MonitorEvent::Alert(a) => panic!("unexpected alert {a:?}"),
                MonitorEvent::Timeline(TimelineChange::Started(_)) => started += 1,
                _ => {}
            }
        }
    })
    .await;
    assert_eq!(started, 1, "the timeline is still tracked for chapters");
    let _ = &h.clock;
}

#[tokio::test(start_paused = true)]
async fn poll_errors_back_off_and_recover() {
    let list = channels(1);
    let h = start(list.clone(), PowerState::Plugged, false, Box::new(NoStore), |f| {
        f.fail_polls(3);
    });
    let mut delays = Vec::new();
    let _ = h
        .until(Duration::from_secs(30 * 60), |e| match e {
            MonitorEvent::Status(s) if s.poll_errors > 0 && delays.last() != Some(&s.poll_errors) => {
                delays.push(s.poll_errors);
                assert_eq!(
                    s.next_poll_in,
                    Duration::from_secs(60 << s.poll_errors),
                    "{s:?}"
                );
                None
            }
            MonitorEvent::Status(s) if s.poll_errors == 0 && s.last_poll_ms.is_some() => Some(()),
            _ => None,
        })
        .await
        .expect("recovered");
    assert_eq!(delays, vec![1, 2, 3]);
    assert_eq!(h.fake.polls(), 4);
}

#[tokio::test(start_paused = true)]
async fn power_state_changes_the_interval() {
    let list = channels(1);
    let h = start(list, PowerState::Plugged, false, Box::new(NoStore), |_| {});
    h.idle(Duration::from_secs(1)).await;
    let before = h.fake.polls();
    h.idle(Duration::from_secs(600)).await;
    assert_eq!(h.fake.polls() - before, 10, "60 s on mains");
    h.monitor.set_power(PowerState::Saver);
    h.idle(Duration::from_secs(61)).await;
    let before = h.fake.polls();
    h.idle(Duration::from_secs(1200)).await;
    assert_eq!(h.fake.polls() - before, 4, "300 s in battery saver");
}

#[tokio::test(start_paused = true)]
async fn chapters_reach_the_library_and_the_vod() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("library.db");
    let db = db.to_str().expect("utf-8 path");
    let store = LibraryTimelineStore::open(db).expect("library");
    let list = channels(1);
    let id = list[0].user_id.clone();
    let h = start(list, PowerState::Plugged, true, Box::new(store), |_| {});
    h.connected().await;
    let stream = h.fake.go_live(&id, "Chaptered");
    h.alert(Duration::from_secs(5)).await.expect("alert");
    h.idle(Duration::from_secs(20 * 60)).await;
    h.fake
        .set_category(&id, Some(("71000002", "Second Invented Game")), "Part two");
    h.idle(Duration::from_secs(10 * 60)).await;
    h.fake.go_offline(&id);
    let ended = h
        .until(Duration::from_secs(10), |e| match e {
            MonitorEvent::Timeline(TimelineChange::Ended(t)) => Some(t.clone()),
            _ => None,
        })
        .await
        .expect("ended");
    assert_eq!(ended.stream_id, stream);

    let conn = pa_library::open(db).expect("library");
    let live = lt::segments(&conn, &SegmentOwner::Stream(stream.clone())).expect("segments");
    assert_eq!(live.len(), 2);
    let session = lt::live_session(&conn, &stream)
        .expect("session")
        .expect("stored");
    let vod = session.video_id.expect("archive linked");
    let chapters = lt::segments(&conn, &SegmentOwner::Video(vod)).expect("segments");
    assert_eq!(chapters.len(), 2, "{chapters:?}");
    assert_eq!(chapters[0].game_id, "71000001");
    assert_eq!(chapters[0].start_ms, 0);
    assert_eq!(chapters[1].game_id, "71000002");
    assert_eq!(chapters[1].start_ms, 20 * 60 * 1000);
    assert!(chapters.iter().all(|c| c.source == "eventsub"));
}

#[tokio::test(start_paused = true)]
async fn restart_resumes_without_a_second_alert() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("library.db");
    let db = db.to_str().expect("utf-8 path").to_owned();
    let list = channels(1);
    let id = list[0].user_id.clone();
    let h = start(
        list.clone(),
        PowerState::Plugged,
        true,
        Box::new(LibraryTimelineStore::open(&db).expect("library")),
        |_| {},
    );
    h.connected().await;
    h.fake.go_live(&id, "Across restarts");
    h.alert(Duration::from_secs(5)).await.expect("alert");
    h.idle(Duration::from_secs(60)).await;
    let fake = h.fake.clone();
    let clock = h.clock.clone();
    drop(h);

    // The app restarts five minutes later, the broadcast is still on.
    tokio::time::sleep(Duration::from_secs(5 * 60)).await;
    let monitor = LiveMonitor::spawn(
        config(),
        list,
        PowerState::Plugged,
        MonitorDeps {
            api: Arc::new(fake.clone()),
            connector: Some(Arc::new(fake.clone())),
            clock,
            store: Box::new(LibraryTimelineStore::open(&db).expect("library")),
        },
    );
    let _ = tokio::time::timeout(Duration::from_secs(5 * 60), async {
        while let Some(e) = monitor.next_event().await {
            match e {
                MonitorEvent::Alert(a) => panic!("announced twice: {a:?}"),
                MonitorEvent::Timeline(TimelineChange::Started(t)) => {
                    panic!("restarted the timeline: {t:?}")
                }
                _ => {}
            }
        }
    })
    .await;
}
