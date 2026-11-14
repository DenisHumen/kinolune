// SPDX-License-Identifier: MPL-2.0
//! The rendering session (TWI-012): batches, one download per image key,
//! moderation pass-through and the synthetic 100 msg/s chat. No network.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_imagecache::{FetchError, Fetched, ImageFetcher};
use pa_twitch::{BoxFuture, SentChatMessage};
use pa_twitch_chat::{
    ChannelAssets, ChannelRef, ChatError, ChatEvent, ChatImages, ChatMessage, ChatSession,
    ChatSource, EmoteRange, ImagePrefs, SessionConfig, SessionEvent, SourceKind,
    SyntheticChatSource, demo_assets,
};

/// Plays a fixed list of events, then stays quiet.
struct ListSource(Mutex<VecDeque<ChatEvent>>);

impl ChatSource for ListSource {
    fn kind(&self) -> SourceKind {
        SourceKind::AnonymousIrc
    }
    fn can_send(&self) -> bool {
        false
    }
    fn next_event(&self) -> BoxFuture<'_, Option<ChatEvent>> {
        // Like a live chat: after the list, wait (no end of stream).
        let e = self.0.lock().unwrap().pop_front();
        Box::pin(async move {
            match e {
                Some(e) => Some(e),
                None => std::future::pending().await,
            }
        })
    }
    fn send_message<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<SentChatMessage, ChatError>> {
        Box::pin(async { Err(ChatError::ReadOnly) })
    }
    fn close(&self) {}
}

#[derive(Default)]
struct CountingFetcher {
    calls: AtomicUsize,
}

const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;";

impl ImageFetcher for CountingFetcher {
    fn fetch(&self, _url: &str) -> Result<Fetched, FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Fetched {
            status: 200,
            body: GIF.to_vec(),
        })
    }
}

fn line(id: &str, text: &str, emotes: &[(&str, usize, usize)]) -> ChatEvent {
    ChatEvent::Message(Box::new(ChatMessage {
        id: id.into(),
        text: text.into(),
        emotes: emotes
            .iter()
            .map(|(id, start, end)| EmoteRange {
                id: (*id).into(),
                start: *start,
                end: *end,
            })
            .collect(),
        ..Default::default()
    }))
}

fn collect(
    rx: &mpsc::Receiver<Vec<SessionEvent>>,
    until: impl Fn(&[SessionEvent]) -> bool,
) -> (Vec<SessionEvent>, usize) {
    let mut all = Vec::new();
    let mut batches = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !until(&all) && Instant::now() < deadline {
        if let Ok(b) = rx.recv_timeout(Duration::from_millis(200)) {
            batches += 1;
            all.extend(b);
        }
    }
    (all, batches)
}

#[test]
fn lines_images_and_moderation_arrive_in_batches() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(CountingFetcher::default());
    let images = ChatImages::open(dir.path(), 1 << 20, fetcher.clone()).unwrap();
    let channel = ChannelRef::default();
    let events = VecDeque::from(vec![
        line("m1", "Kappa hi", &[("25", 0, 5)]),
        line("m2", "Kappa Kappa", &[("25", 0, 5), ("25", 6, 11)]),
        ChatEvent::MessageDeleted {
            channel: channel.clone(),
            message_id: "m1".into(),
            user_login: None,
        },
        ChatEvent::ChatCleared { channel },
    ]);
    let (tx, rx) = mpsc::channel();
    let _session = ChatSession::start(
        move || Ok(Box::new(ListSource(Mutex::new(events))) as Box<dyn ChatSource>),
        Arc::new(ChannelAssets::cdn_only()),
        Some(images),
        SessionConfig {
            flush_interval: Duration::from_millis(20),
            ..SessionConfig::default()
        },
        move |b| tx.send(b).is_ok(),
    )
    .unwrap();
    let (all, _) = collect(&rx, |all| {
        all.iter().any(|e| matches!(e, SessionEvent::Image(_)))
            && all
                .iter()
                .any(|e| matches!(e, SessionEvent::Event(ChatEvent::ChatCleared { .. })))
    });
    let lines: Vec<_> = all
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Line(l) => Some(l.message.id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, ["m1", "m2"]);
    assert!(all.iter().any(|e| matches!(
        e,
        SessionEvent::Event(ChatEvent::MessageDeleted { message_id, .. }) if message_id == "m1"
    )));
    let images: Vec<_> = all
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Image(i) => Some(i),
            _ => None,
        })
        .collect();
    // One emote used three times: one request, one download.
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].key, "twitch:emote:25");
    // CDN-only emote: animation unknown, sniffed from the file.
    assert!(images[0].animated);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn failing_source_reports_stopped() {
    let (tx, rx) = mpsc::channel();
    let _session = ChatSession::start(
        || Err(ChatError::Connect("no route".into())),
        Arc::new(ChannelAssets::cdn_only()),
        None,
        SessionConfig::default(),
        move |b| tx.send(b).is_ok(),
    )
    .unwrap();
    let b = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        &b[0],
        SessionEvent::Event(ChatEvent::Status(
            pa_twitch_chat::SourceStatus::Stopped { .. }
        ))
    ));
}

#[test]
fn synthetic_chat_at_100_per_second_is_batched() {
    let dir = tempfile::tempdir().unwrap();
    let assets = Arc::new(demo_assets(&dir.path().join("demo")).unwrap());
    let images = ChatImages::open(
        dir.path().join("cache"),
        1 << 20,
        Arc::new(CountingFetcher::default()),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let session = ChatSession::start(
        || Ok(Box::new(SyntheticChatSource::new(100, 42)) as Box<dyn ChatSource>),
        assets,
        Some(images),
        SessionConfig {
            prefs: ImagePrefs::default(),
            ..SessionConfig::default()
        },
        move |b| tx.send(b).is_ok(),
    )
    .unwrap();
    let start = Instant::now();
    let mut lines = 0;
    let mut batches = 0;
    let mut badges = 0;
    let mut emotes = 0;
    while start.elapsed() < Duration::from_millis(1500) {
        if let Ok(b) = rx.recv_timeout(Duration::from_millis(300)) {
            batches += 1;
            for e in b {
                match e {
                    SessionEvent::Line(l) => {
                        lines += 1;
                        badges += l.rendered.badges.len();
                        emotes += l.rendered.emotes().len();
                    }
                    SessionEvent::ImageFailed { key } => panic!("demo image {key} failed"),
                    _ => {}
                }
            }
        }
    }
    session.stop();
    // ~150 lines in 1.5 s (timers are coarse on a busy machine).
    assert!((100..=200).contains(&lines), "lines {lines}");
    // Batches every 150 ms: about ten, never one per message.
    assert!(batches <= 15, "batches {batches}");
    assert!(badges > 0 && emotes > 0);
}
