// SPDX-License-Identifier: MPL-2.0
//! A made-up Twitch in process: Helix (`GET /streams`,
//! `POST /eventsub/subscriptions` with the cost budget, archive VODs) and
//! an EventSub WebSocket server (welcome, keepalives, notifications,
//! `session_reconnect`). It serves the tests and the app's demo mode; it
//! never touches the network, and every channel in it is invented.
//!
//! Behaviour that matters for the monitor mirrors Twitch: a subscription
//! needs a connected session, costs 1, and is refused with `429` past
//! `max_total_cost`; subscriptions die with their socket and move on
//! `session_reconnect`; a new stream shows up in `GET /streams` only after
//! a configurable lag.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use pa_twitch::{
    BoxFuture, Clock, EventSubSubscription, EventSubSubscriptionRequest, Stream, Video,
};
use pa_twitch_chat::{ChatError, WsConnection, WsConnector};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::api::{LiveApi, LiveError};
use crate::plan::{DEFAULT_MAX_TOTAL_COST, WatchedChannel};

/// URL the fake EventSub server answers on.
pub const FAKE_EVENTSUB_URL: &str = "wss://eventsub.fake.invalid/ws";

/// Made-up channels of the demo: (id, login, display name, category id,
/// category name).
pub const DEMO_CHANNELS: [(&str, &str, &str, &str, &str); 4] = [
    ("91000001", "moonlit_pancake", "Moonlit Pancake", "92000001", "Cozy Farming Sim"),
    ("91000002", "quiet_harbor_fm", "Quiet Harbor FM", "92000002", "Music Making"),
    ("91000003", "tiny_robot_league", "Tiny Robot League", "92000003", "Arena Tactics"),
    ("91000004", "paper_lantern_lab", "Paper Lantern Lab", "92000004", "Science & Craft"),
];

#[derive(Debug, Clone)]
struct Live {
    stream_id: String,
    started_at: SystemTime,
}

#[derive(Debug, Clone)]
struct Channel {
    login: String,
    name: String,
    game: Option<(String, String)>,
    title: String,
    live: Option<Live>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Sub {
    id: String,
    kind: String,
    user_id: String,
    session_id: String,
}

#[derive(Debug)]
struct Session {
    id: String,
    tx: mpsc::UnboundedSender<String>,
}

#[derive(Debug)]
struct State {
    channels: BTreeMap<String, Channel>,
    sessions: Vec<Session>,
    subs: Vec<Sub>,
    max_total_cost: u32,
    used_elsewhere: u32,
    streams_lag: Duration,
    poll_failures: u32,
    connect_failures: u32,
    duplicate_notifications: bool,
    polls: u32,
    connects: u32,
    next_id: u64,
    keepalive: Duration,
    archives: Vec<Video>,
}

/// The made-up Twitch. Cheap to clone; clones share the state.
#[derive(Clone)]
pub struct FakeTwitch {
    state: Arc<Mutex<State>>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for FakeTwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeTwitch").finish_non_exhaustive()
    }
}

/// RFC 3339 UTC with milliseconds.
pub fn rfc3339(t: SystemTime) -> String {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = i64::try_from(d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        d.subsec_millis()
    )
}

impl FakeTwitch {
    /// Empty Twitch.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                channels: BTreeMap::new(),
                sessions: Vec::new(),
                subs: Vec::new(),
                max_total_cost: DEFAULT_MAX_TOTAL_COST,
                used_elsewhere: 0,
                streams_lag: Duration::ZERO,
                poll_failures: 0,
                connect_failures: 0,
                duplicate_notifications: false,
                polls: 0,
                connects: 0,
                next_id: 1,
                keepalive: Duration::from_secs(10),
                archives: Vec::new(),
            })),
            clock,
        }
    }

    /// Twitch with the made-up [`DEMO_CHANNELS`], all offline.
    pub fn demo(clock: Arc<dyn Clock>) -> Self {
        let fake = Self::new(clock);
        for (id, login, name, game_id, game) in DEMO_CHANNELS {
            fake.add_channel(id, login, name);
            fake.set_category(id, Some((game_id, game)), "Just chatting before the show");
        }
        fake
    }

    /// The demo channels as watched channels (notifications on).
    pub fn demo_channels() -> Vec<WatchedChannel> {
        DEMO_CHANNELS
            .iter()
            .map(|(id, login, name, _, _)| WatchedChannel::new(id, login, name))
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn id(state: &mut State) -> String {
        state.next_id += 1;
        format!("{}", 93_000_000 + state.next_id)
    }

    /// Adds an offline channel.
    pub fn add_channel(&self, user_id: &str, login: &str, name: &str) {
        self.lock().channels.insert(
            user_id.to_owned(),
            Channel {
                login: login.to_owned(),
                name: name.to_owned(),
                game: None,
                title: String::new(),
                live: None,
            },
        );
    }

    /// Delay before a new stream is listed by `GET /streams`.
    pub fn set_streams_lag(&self, lag: Duration) {
        self.lock().streams_lag = lag;
    }

    /// Cost spent by other sessions of the same user and app.
    pub fn set_used_elsewhere(&self, cost: u32) {
        self.lock().used_elsewhere = cost;
    }

    /// The next `n` polls fail with a network error.
    pub fn fail_polls(&self, n: u32) {
        self.lock().poll_failures = n;
    }

    /// The next `n` WebSocket connections fail.
    pub fn fail_connects(&self, n: u32) {
        self.lock().connect_failures = n;
    }

    /// Deliver every notification twice (Twitch may).
    pub fn duplicate_notifications(&self, on: bool) {
        self.lock().duplicate_notifications = on;
    }

    /// Keepalive interval of new sessions.
    pub fn set_keepalive(&self, keepalive: Duration) {
        self.lock().keepalive = keepalive;
    }

    /// Polls served.
    pub fn polls(&self) -> u32 {
        self.lock().polls
    }

    /// WebSocket connections opened.
    pub fn connects(&self) -> u32 {
        self.lock().connects
    }

    /// Active subscriptions as (type, broadcaster id).
    pub fn subscriptions(&self) -> Vec<(String, String)> {
        self.lock()
            .subs
            .iter()
            .map(|s| (s.kind.clone(), s.user_id.clone()))
            .collect()
    }

    /// Cost of the active subscriptions (without other sessions).
    pub fn total_cost(&self) -> u32 {
        u32::try_from(self.lock().subs.len()).unwrap_or(u32::MAX)
    }

    fn frame(&self, kind: &str, payload: Value, subscription_type: Option<&str>) -> String {
        let mut state = self.lock();
        let message_id = format!("msg-{}", Self::id(&mut state));
        drop(state);
        let mut metadata = json!({
            "message_id": message_id,
            "message_type": kind,
            "message_timestamp": rfc3339(self.clock.now()),
        });
        if let Some(t) = subscription_type {
            metadata["subscription_type"] = json!(t);
            metadata["subscription_version"] = json!(if t == "channel.update" { "2" } else { "1" });
        }
        json!({ "metadata": metadata, "payload": payload }).to_string()
    }

    fn notify(&self, kind: &str, user_id: &str, event: Value) {
        let subs: Vec<Sub> = self
            .lock()
            .subs
            .iter()
            .filter(|s| s.kind == kind && s.user_id == user_id)
            .cloned()
            .collect();
        for sub in subs {
            let payload = json!({
                "subscription": {
                    "id": sub.id,
                    "status": "enabled",
                    "type": kind,
                    "version": if kind == "channel.update" { "2" } else { "1" },
                    "cost": 1,
                    "condition": { "broadcaster_user_id": user_id },
                    "transport": { "method": "websocket", "session_id": sub.session_id },
                    "created_at": rfc3339(self.clock.now()),
                },
                "event": event.clone(),
            });
            let text = self.frame("notification", payload, Some(kind));
            let state = self.lock();
            if let Some(session) = state.sessions.iter().find(|s| s.id == sub.session_id) {
                let _ = session.tx.send(text.clone());
                if state.duplicate_notifications {
                    let _ = session.tx.send(text);
                }
            }
        }
    }

    fn who(&self, user_id: &str) -> Option<(String, String)> {
        self.lock()
            .channels
            .get(user_id)
            .map(|c| (c.login.clone(), c.name.clone()))
    }

    /// The channel goes live now; returns the stream id.
    pub fn go_live(&self, user_id: &str, title: &str) -> String {
        let now = self.clock.now();
        let stream_id = {
            let mut state = self.lock();
            let id = Self::id(&mut state);
            let Some(ch) = state.channels.get_mut(user_id) else {
                return String::new();
            };
            if !title.is_empty() {
                title.clone_into(&mut ch.title);
            }
            ch.live = Some(Live {
                stream_id: id.clone(),
                started_at: now,
            });
            id
        };
        let (login, name) = self.who(user_id).unwrap_or_default();
        self.notify(
            "stream.online",
            user_id,
            json!({
                "id": stream_id,
                "broadcaster_user_id": user_id,
                "broadcaster_user_login": login,
                "broadcaster_user_name": name,
                "type": "live",
                "started_at": rfc3339(now),
            }),
        );
        stream_id
    }

    /// Category and title change (also while offline).
    pub fn set_category(&self, user_id: &str, game: Option<(&str, &str)>, title: &str) {
        {
            let mut state = self.lock();
            let Some(ch) = state.channels.get_mut(user_id) else {
                return;
            };
            ch.game = game.map(|(i, n)| (i.to_owned(), n.to_owned()));
            title.clone_into(&mut ch.title);
        }
        let (login, name) = self.who(user_id).unwrap_or_default();
        let (gid, gname) = game.unwrap_or(("", ""));
        self.notify(
            "channel.update",
            user_id,
            json!({
                "broadcaster_user_id": user_id,
                "broadcaster_user_login": login,
                "broadcaster_user_name": name,
                "title": title,
                "language": "en",
                "category_id": gid,
                "category_name": gname,
                "content_classification_labels": [],
            }),
        );
    }

    /// The broadcast ends; its archive VOD appears.
    pub fn go_offline(&self, user_id: &str) {
        let now = self.clock.now();
        {
            let mut state = self.lock();
            state.next_id += 1;
            let video_id = format!("{}", 94_000_000 + state.next_id);
            let Some(ch) = state.channels.get_mut(user_id) else {
                return;
            };
            let Some(live) = ch.live.take() else {
                return;
            };
            let secs = now
                .duration_since(live.started_at)
                .unwrap_or_default()
                .as_secs();
            let video = json!({
                "id": video_id,
                "stream_id": live.stream_id,
                "user_id": user_id,
                "user_login": ch.login,
                "user_name": ch.name,
                "title": ch.title,
                "created_at": rfc3339(live.started_at),
                "published_at": rfc3339(live.started_at),
                "type": "archive",
                "duration": format!("{}h{}m{}s", secs / 3600, secs % 3600 / 60, secs % 60),
            });
            if let Ok(v) = serde_json::from_value::<Video>(video) {
                state.archives.push(v);
            }
        }
        let (login, name) = self.who(user_id).unwrap_or_default();
        self.notify(
            "stream.offline",
            user_id,
            json!({
                "broadcaster_user_id": user_id,
                "broadcaster_user_login": login,
                "broadcaster_user_name": name,
            }),
        );
    }

    /// Closes every socket; their subscriptions are gone (as on Twitch).
    pub fn drop_connections(&self) {
        let mut state = self.lock();
        state.sessions.clear();
        state.subs.clear();
    }

    /// Asks every session to move (`session_reconnect`).
    pub fn send_reconnect(&self) {
        let ids: Vec<String> = self.lock().sessions.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            let url = format!("{FAKE_EVENTSUB_URL}?reconnect={id}");
            let text = self.frame(
                "session_reconnect",
                json!({ "session": {
                    "id": id,
                    "status": "reconnecting",
                    "keepalive_timeout_seconds": null,
                    "reconnect_url": url,
                    "connected_at": rfc3339(self.clock.now()),
                }}),
                None,
            );
            if let Some(s) = self.lock().sessions.iter().find(|s| s.id == id) {
                let _ = s.tx.send(text);
            }
        }
    }

    fn stream_of(&self, user_id: &str, ch: &Channel, live: &Live) -> Option<Stream> {
        let (gid, gname) = ch.game.clone().unwrap_or_default();
        serde_json::from_value(json!({
            "id": live.stream_id,
            "user_id": user_id,
            "user_login": ch.login,
            "user_name": ch.name,
            "game_id": gid,
            "game_name": gname,
            "type": "live",
            "title": ch.title,
            "viewer_count": 42,
            "started_at": rfc3339(live.started_at),
            "language": "en",
            "thumbnail_url": "",
            "tags": [],
            "is_mature": false,
        }))
        .ok()
    }
}

impl LiveApi for FakeTwitch {
    fn subscribe(
        &self,
        request: EventSubSubscriptionRequest,
    ) -> BoxFuture<'_, Result<EventSubSubscription, LiveError>> {
        let result = (|| {
            let mut state = self.lock();
            if !state.sessions.iter().any(|s| s.id == request.session_id) {
                return Err(LiveError::Other("400: session does not exist".to_owned()));
            }
            let user_id = request
                .condition
                .get("broadcaster_user_id")
                .cloned()
                .unwrap_or_default();
            if !state.channels.contains_key(&user_id) {
                return Err(LiveError::Other("400: unknown broadcaster".to_owned()));
            }
            if state
                .subs
                .iter()
                .any(|s| s.kind == request.kind && s.user_id == user_id)
            {
                return Err(LiveError::Conflict);
            }
            let used = u32::try_from(state.subs.len()).unwrap_or(u32::MAX) + state.used_elsewhere;
            if used + 1 > state.max_total_cost {
                return Err(LiveError::BudgetExceeded);
            }
            let id = format!("sub-{}", Self::id(&mut state));
            state.subs.push(Sub {
                id: id.clone(),
                kind: request.kind.clone(),
                user_id,
                session_id: request.session_id.clone(),
            });
            serde_json::from_value(json!({
                "id": id,
                "status": "enabled",
                "type": request.kind,
                "version": request.version,
                "cost": 1,
            }))
            .map_err(|e| LiveError::Other(e.to_string()))
        })();
        Box::pin(async move { result })
    }

    fn live_streams(&self, user_ids: Vec<String>) -> BoxFuture<'_, Result<Vec<Stream>, LiveError>> {
        let now = self.clock.now();
        let result = {
            let mut state = self.lock();
            state.polls += 1;
            if state.poll_failures > 0 {
                state.poll_failures -= 1;
                Err(LiveError::Network("made-up network failure".to_owned()))
            } else {
                let lag = state.streams_lag;
                Ok(user_ids
                    .iter()
                    .filter_map(|id| {
                        let ch = state.channels.get(id)?;
                        let live = ch.live.as_ref()?;
                        (live.started_at + lag <= now).then(|| self.stream_of(id, ch, live))?
                    })
                    .collect())
            }
        };
        Box::pin(async move { result })
    }

    fn archive_for_stream(
        &self,
        user_id: String,
        stream_id: String,
    ) -> BoxFuture<'_, Result<Option<Video>, LiveError>> {
        let found = self
            .lock()
            .archives
            .iter()
            .find(|v| v.user_id == user_id && v.stream_id.as_deref() == Some(stream_id.as_str()))
            .cloned();
        Box::pin(async move { Ok(found) })
    }
}

impl WsConnector for FakeTwitch {
    fn connect<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn WsConnection>, ChatError>> {
        let result = (|| {
            let mut state = self.lock();
            state.connects += 1;
            if state.connect_failures > 0 {
                state.connect_failures -= 1;
                return Err(ChatError::Connect("made-up connection failure".to_owned()));
            }
            if !url.starts_with(FAKE_EVENTSUB_URL) {
                return Err(ChatError::Connect(format!("unknown host: {url}")));
            }
            let session_id = format!("session-{}", Self::id(&mut state));
            // A reconnect URL carries the subscriptions of the old session.
            if let Some(old) = url.split("?reconnect=").nth(1) {
                for sub in state.subs.iter_mut().filter(|s| s.session_id == old) {
                    sub.session_id.clone_from(&session_id);
                }
            }
            let (tx, rx) = mpsc::unbounded_channel();
            state.sessions.push(Session {
                id: session_id.clone(),
                tx: tx.clone(),
            });
            let keepalive = state.keepalive;
            Ok((session_id, tx, rx, keepalive))
        })();
        Box::pin(async move {
            let (session_id, tx, rx, keepalive) = result?;
            let welcome = self.frame(
                "session_welcome",
                json!({ "session": {
                    "id": session_id,
                    "status": "connected",
                    "keepalive_timeout_seconds": keepalive.as_secs(),
                    "reconnect_url": null,
                    "connected_at": rfc3339(self.clock.now()),
                }}),
                None,
            );
            let _ = tx.send(welcome);
            Ok(Box::new(FakeConn {
                fake: self.clone(),
                session_id,
                rx,
                keepalive,
            }) as Box<dyn WsConnection>)
        })
    }
}

struct FakeConn {
    fake: FakeTwitch,
    session_id: String,
    rx: mpsc::UnboundedReceiver<String>,
    keepalive: Duration,
}

impl WsConnection for FakeConn {
    fn send_text(&mut self, _text: String) -> BoxFuture<'_, Result<(), ChatError>> {
        // EventSub clients never send.
        Box::pin(async { Ok(()) })
    }

    fn recv_text(&mut self) -> BoxFuture<'_, Option<Result<String, ChatError>>> {
        Box::pin(async move {
            // Keepalives a little before the timeout, as Twitch does.
            let idle = self.keepalive.mul_f32(0.8);
            tokio::select! {
                biased;
                text = self.rx.recv() => text.map(Ok),
                () = self.fake.clock.sleep(idle) => {
                    Some(Ok(self.fake.frame("session_keepalive", json!({}), None)))
                }
            }
        })
    }

    fn close(&mut self) -> BoxFuture<'_, ()> {
        self.end();
        Box::pin(async {})
    }
}

impl FakeConn {
    /// The socket is gone: so are its session and subscriptions.
    fn end(&self) {
        let mut state = self.fake.lock();
        let id = &self.session_id;
        state.sessions.retain(|s| &s.id != id);
        state.subs.retain(|s| &s.session_id != id);
    }
}

impl Drop for FakeConn {
    fn drop(&mut self) {
        self.end();
    }
}

/// The demo's story: after `first_after`, Moonlit Pancake goes live; a
/// minute later it switches category; Quiet Harbor FM follows two minutes
/// in. Runs until the returned task is aborted.
pub fn spawn_demo_script(fake: FakeTwitch, first_after: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let clock = fake.clock.clone();
        clock.sleep(first_after).await;
        let (a, _, _, _, _) = DEMO_CHANNELS[0];
        let (b, _, _, _, _) = DEMO_CHANNELS[1];
        fake.go_live(a, "Planting moon carrots until sunrise");
        clock.sleep(Duration::from_secs(60)).await;
        fake.set_category(a, Some(("92000005", "Puzzle Detective")), "Solving the carrot heist");
        clock.sleep(Duration::from_secs(60)).await;
        fake.go_live(b, "Late-night synth jam");
        std::future::pending::<()>().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trips() {
        for secs in [0u64, 951_782_400, 1_763_100_000, 4_102_444_799] {
            let t = SystemTime::UNIX_EPOCH + Duration::from_millis(secs * 1000 + 123);
            let s = rfc3339(t);
            assert_eq!(
                pa_twitch::parse_rfc3339_ms(&s),
                Some(i64::try_from(secs).unwrap_or(0) * 1000 + 123),
                "{s}"
            );
        }
        assert_eq!(
            rfc3339(SystemTime::UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00.000Z"
        );
    }
}
