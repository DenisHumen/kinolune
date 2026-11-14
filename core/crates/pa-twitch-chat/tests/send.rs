// SPDX-License-Identifier: MPL-2.0
//! The signed-in session (TWI-024): filters applied before rendering,
//! room modes passed through, and sending through the EventSub source to
//! a fake Helix (recorded responses; no network, no real client id).

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use pa_twitch::testing::{FixtureTransport, ManualClock};
use pa_twitch::{AccessToken, ClientId, HelixClient, HelixConfig};
use pa_twitch_chat::testing::{Script, ScriptedConnector};
use pa_twitch_chat::{
    ChannelAssets, ChatError, ChatEvent, ChatFilters, ChatSession, ChatSource, EventSubChatSource,
    EventSubConfig, FilterSettings, SessionConfig, SessionEvent,
};

const ES_URL: &str = "wss://eventsub.example.invalid/ws";
const BROADCASTER: &str = "100000001";
const ME: &str = "300000001";

macro_rules! es {
    ($name:literal) => {
        include_str!(concat!("fixtures/eventsub/", $name, ".json"))
    };
}

macro_rules! helix {
    ($name:literal) => {
        include_str!(concat!("fixtures/helix/", $name, ".json"))
    };
}

struct Harness {
    transport: Arc<FixtureTransport>,
    session: ChatSession,
    rx: mpsc::Receiver<Vec<SessionEvent>>,
}

/// A signed-in session on a scripted socket: welcome, one chat message
/// (`quiet_heron` writes "@Pixel_Otter héllo Kappa 🎉 Keepo"), room
/// modes, then silence.
fn start(filters: FilterSettings) -> Harness {
    let transport = Arc::new(FixtureTransport::new());
    for _ in 0..6 {
        transport.push_fixture(helix!("subscription_created"));
    }
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let mut config = HelixConfig::new(ClientId::new("testclientid0000000000000000000").unwrap());
    config.base_url = "https://helix.test/helix".to_owned();
    let helix = HelixClient::with_clock(config, transport.clone(), clock.clone());
    helix.set_access_token(AccessToken::new("test-user-token-not-real"));
    let helix = Arc::new(helix);
    let connector = ScriptedConnector::new();
    connector.push(Script::hanging([
        es!("welcome"),
        es!("chat_message"),
        es!("chat_settings_update"),
    ]));
    let (compiled, errors) = ChatFilters::new(&filters);
    assert!(errors.is_empty());
    let (tx, rx) = mpsc::channel();
    let session = ChatSession::start_filtered(
        move || {
            let mut es = EventSubConfig::new(ME, [BROADCASTER]);
            es.url = ES_URL.to_owned();
            es.max_attempts = Some(0);
            let source =
                EventSubChatSource::spawn(es, helix, Arc::new(connector), clock)?;
            Ok(Box::new(source) as Box<dyn ChatSource>)
        },
        Arc::new(ChannelAssets::cdn_only()),
        None,
        SessionConfig {
            flush_interval: Duration::from_millis(20),
            ..SessionConfig::default()
        },
        Arc::new(compiled),
        move |batch| tx.send(batch).is_ok(),
    )
    .unwrap();
    Harness {
        transport,
        session,
        rx,
    }
}

fn collect_until(
    rx: &mpsc::Receiver<Vec<SessionEvent>>,
    done: impl Fn(&[SessionEvent]) -> bool,
) -> Vec<SessionEvent> {
    let mut all = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(&all) && Instant::now() < deadline {
        if let Ok(b) = rx.recv_timeout(Duration::from_millis(100)) {
            all.extend(b);
        }
    }
    all
}

fn has_room_modes(events: &[SessionEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, SessionEvent::Event(ChatEvent::RoomModes { .. })))
}

#[test]
fn mention_is_flagged_and_message_is_sent_through_helix() {
    let h = start(FilterSettings {
        own_login: Some("pixel_otter".into()),
        ..FilterSettings::default()
    });
    let events = collect_until(&h.rx, has_room_modes);
    let line = events
        .iter()
        .find_map(|e| match e {
            SessionEvent::Line(l) => Some(l),
            _ => None,
        })
        .expect("the chat line");
    assert_eq!(line.message.user.login, "quiet_heron");
    assert!(line.mention, "@Pixel_Otter names the signed-in user");
    let modes = events
        .iter()
        .find_map(|e| match e {
            SessionEvent::Event(ChatEvent::RoomModes { modes, .. }) => Some(*modes),
            _ => None,
        })
        .unwrap();
    assert_eq!(modes.emote_only, Some(true));
    assert_eq!(modes.slow, Some(None));

    h.transport.push_fixture(helix!("message_sent"));
    h.transport.push_fixture(helix!("message_dropped"));
    let sent = h
        .session
        .send(BROADCASTER, "  hi @quiet_heron  ", Some("parent-0001"))
        .unwrap();
    assert!(sent.is_sent);
    assert_eq!(sent.message_id, "e5f6a7b8-0100-4a5b-9c1d-000000000100");
    let dropped = h.session.send(BROADCASTER, "hi again", None).unwrap();
    assert!(!dropped.is_sent);
    assert_eq!(dropped.drop_reason.unwrap().code, "msg_duplicate");

    let requests = h.transport.requests();
    let sends: Vec<_> = requests
        .iter()
        .filter(|r| r.url == "https://helix.test/helix/chat/messages")
        .collect();
    assert_eq!(sends.len(), 2);
    let body: serde_json::Value = serde_json::from_slice(sends[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "broadcaster_id": BROADCASTER,
            "sender_id": ME,
            "message": "hi @quiet_heron",
            "reply_parent_message_id": "parent-0001",
        })
    );

    // Too long: refused before Helix.
    assert!(matches!(
        h.session.send(BROADCASTER, &"x".repeat(501), None),
        Err(ChatError::InvalidArgument(_))
    ));
    assert_eq!(
        h.transport
            .requests()
            .iter()
            .filter(|r| r.url.ends_with("/chat/messages"))
            .count(),
        2
    );
}

#[test]
fn blocked_author_never_reaches_the_consumer() {
    let h = start(FilterSettings {
        blocked: vec!["Quiet_Heron".into()],
        ..FilterSettings::default()
    });
    let events = collect_until(&h.rx, has_room_modes);
    assert!(has_room_modes(&events));
    assert!(
        !events.iter().any(|e| matches!(e, SessionEvent::Line(_))),
        "the blocked user's line was dropped"
    );
}

#[test]
fn filters_change_while_running_and_send_after_stop_fails() {
    let h = start(FilterSettings::default());
    let _ = collect_until(&h.rx, has_room_modes);
    let (f, _) = ChatFilters::new(&FilterSettings {
        hide_bots: true,
        ..FilterSettings::default()
    });
    h.session.set_filters(Arc::new(f));
    h.session.stop();
    // The session thread ends; sending reports a closed chat.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match h.session.send(BROADCASTER, "late", None) {
            Err(ChatError::Closed) => break,
            other => {
                assert!(Instant::now() < deadline, "still sending: {other:?}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
