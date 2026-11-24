// SPDX-License-Identifier: MPL-2.0
//! Emotes and badges (TWI-011) on hand-written fixtures with made-up names.
//! No test touches the network: Helix goes through the fixture transport,
//! images through a counting in-memory fetcher.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_imagecache::{FetchError, Fetched, ImageFetcher};
use pa_twitch::testing::{FixtureTransport, ManualClock};
use pa_twitch::{AccessToken, ClientId, HelixClient, HelixConfig};
use pa_twitch_chat::{
    Animation, AssetStore, Badge, ChannelAssets, ChatError, ChatImages, ChatMessage, ChatUser,
    EMOTE_MAP_TTL, EmoteRange, Fragment, ImagePrefs, ImageRequest, RenderedMessage, Scale, Theme,
    image_requests,
};

const START: u64 = 1_800_000_000;
const CHANNEL: &str = "424242";

macro_rules! fx {
    ($name:literal) => {
        include_str!(concat!("fixtures/", $name, ".json"))
    };
}

struct Harness {
    store: AssetStore,
    transport: Arc<FixtureTransport>,
    clock: Arc<ManualClock>,
}

fn harness() -> Harness {
    let transport = Arc::new(FixtureTransport::new());
    let clock = Arc::new(ManualClock::at_unix(START));
    let mut config = HelixConfig::new(ClientId::new("testclientid0000000000000000000").unwrap());
    config.base_url = "https://helix.test/helix".to_owned();
    let client = HelixClient::with_clock(config, transport.clone(), clock.clone());
    client.set_access_token(AccessToken::new("test-user-token-not-real"));
    Harness {
        store: AssetStore::with_clock(Arc::new(client), clock.clone()),
        transport,
        clock,
    }
}

impl Harness {
    fn push_channel(&self, with_global: bool) {
        if with_global {
            self.transport
                .push_fixture(fx!("assets/global_emotes"))
                .push_fixture(fx!("assets/global_badges"));
        }
        self.transport
            .push_fixture(fx!("assets/channel_emotes"))
            .push_fixture(fx!("assets/channel_badges"));
    }

    fn urls(&self) -> Vec<String> {
        self.transport
            .requests()
            .into_iter()
            .map(|r| r.url.replace("https://helix.test/helix/", ""))
            .collect()
    }
}

async fn assets() -> Arc<ChannelAssets> {
    let h = harness();
    h.push_channel(true);
    h.store.channel(CHANNEL).await.unwrap()
}

/// `text` with emote ranges given as (id, code): the ranges are computed
/// from the first occurrence of each code after the previous one.
fn message(text: &str, emotes: &[(&str, &str)], badges: &[(&str, &str)]) -> ChatMessage {
    let mut ranges = Vec::new();
    let mut from_byte = 0;
    for (id, code) in emotes {
        let at = from_byte + text[from_byte..].find(code).expect("code in text");
        let start = text[..at].chars().count();
        let end = start + code.chars().count();
        ranges.push(EmoteRange {
            id: (*id).to_owned(),
            start,
            end,
        });
        from_byte = at + code.len();
    }
    ChatMessage {
        id: "msg-1".to_owned(),
        text: text.to_owned(),
        emotes: ranges,
        user: ChatUser {
            id: "5550001".to_owned(),
            login: "quillfeather".to_owned(),
            display_name: "QuillFeather".to_owned(),
            color: None,
            badges: badges
                .iter()
                .map(|(set, ver)| Badge {
                    set_id: (*set).to_owned(),
                    version: (*ver).to_owned(),
                    info: (*set == "subscriber").then(|| "14".to_owned()),
                })
                .collect(),
        },
        ..ChatMessage::default()
    }
}

fn shape(r: &RenderedMessage) -> Vec<String> {
    r.fragments
        .iter()
        .map(|f| match f {
            Fragment::Text(t) => format!("T({t})"),
            Fragment::Emote { asset, code, .. } => format!("E({}:{code})", asset.id),
        })
        .collect()
}

// ----- Helix catalogs and the 1-hour emote map --------------------------

#[tokio::test]
async fn store_fetches_global_and_channel_catalogs_and_keeps_them_an_hour() {
    let h = harness();
    h.push_channel(true);
    let a = h.store.channel(CHANNEL).await.unwrap();
    assert_eq!(
        h.urls(),
        vec![
            "chat/emotes/global".to_owned(),
            "chat/badges/global".to_owned(),
            format!("chat/emotes?broadcaster_id={CHANNEL}"),
            format!("chat/badges?broadcaster_id={CHANNEL}"),
        ]
    );
    assert_eq!(a.len(), (4, 3));
    assert_eq!(
        a.emotes()
            .iter()
            .map(|e| e.code.as_str())
            .collect::<Vec<_>>(),
        ["<3", "GlimSmile", "zorpNod", "zorpWave"]
    );

    // Within the hour: same snapshot, no request.
    h.clock.advance(EMOTE_MAP_TTL - Duration::from_secs(1));
    let again = h.store.channel(CHANNEL).await.unwrap();
    assert!(Arc::ptr_eq(&a, &again));
    assert_eq!(h.transport.requests().len(), 4);

    // After the hour: everything again.
    h.clock.advance(Duration::from_secs(2));
    h.push_channel(true);
    let fresh = h.store.channel(CHANNEL).await.unwrap();
    assert!(!Arc::ptr_eq(&a, &fresh));
    assert_eq!(h.transport.requests().len(), 8);
    assert_eq!(h.transport.remaining(), 0);
}

#[tokio::test]
async fn global_catalog_is_shared_between_channels() {
    let h = harness();
    h.push_channel(true);
    h.store.channel(CHANNEL).await.unwrap();
    h.push_channel(false);
    let other = h.store.channel("515151").await.unwrap();
    let urls = h.urls();
    assert_eq!(urls.len(), 6);
    assert_eq!(urls[4], "chat/emotes?broadcaster_id=515151");
    assert_eq!(other.channel_id, "515151");
    // Global emotes are there for the second channel too.
    assert_eq!(other.emote("9001").code, "GlimSmile");
}

#[tokio::test]
async fn invalidate_drops_the_cache() {
    let h = harness();
    h.push_channel(true);
    h.store.channel(CHANNEL).await.unwrap();
    h.store.invalidate();
    h.push_channel(true);
    h.store.channel(CHANNEL).await.unwrap();
    assert_eq!(h.transport.requests().len(), 8);
}

#[tokio::test]
async fn helix_errors_surface_as_chat_errors() {
    let h = harness();
    h.transport.push_fixture(fx!("helix/unauthorized"));
    let err = h.store.channel(CHANNEL).await.unwrap_err();
    assert_eq!(err, ChatError::Unauthorized);
    assert_eq!(
        h.store.channel("").await.unwrap_err(),
        ChatError::InvalidArgument("broadcaster id is empty")
    );
}

// ----- Rendering ---------------------------------------------------------

#[tokio::test]
async fn emote_ranges_become_fragments() {
    let a = assets().await;
    let m = message(
        "hi zorpWave GlimSmile and zorpNod",
        &[
            ("emotesv2_zorpwave0", "zorpWave"),
            ("9001", "GlimSmile"),
            ("777123", "zorpNod"),
        ],
        &[],
    );
    let r = a.render(&m);
    assert_eq!(
        shape(&r),
        [
            "T(hi )",
            "E(emotesv2_zorpwave0:zorpWave)",
            "T( )",
            "E(9001:GlimSmile)",
            "T( and )",
            "E(777123:zorpNod)",
        ]
    );
}

#[tokio::test]
async fn offsets_are_characters_not_bytes() {
    let a = assets().await;
    // Cyrillic, an emoji (4 bytes) and adjacent emotes without spaces.
    let m = message(
        "привет 🎉<3<3 ёж GlimSmile",
        &[("9002", "<3"), ("9002", "<3"), ("9001", "GlimSmile")],
        &[],
    );
    let r = a.render(&m);
    assert_eq!(
        shape(&r),
        [
            "T(привет 🎉)",
            "E(9002:<3)",
            "E(9002:<3)",
            "T( ёж )",
            "E(9001:GlimSmile)",
        ]
    );
}

#[tokio::test]
async fn malformed_ranges_keep_the_text() {
    let a = assets().await;
    let mut m = message("GlimSmile zorpNod", &[], &[]);
    m.emotes = vec![
        // Out of order on purpose: sorted before use.
        EmoteRange {
            id: "777123".to_owned(),
            start: 10,
            end: 17,
        },
        EmoteRange {
            id: "9001".to_owned(),
            start: 0,
            end: 9,
        },
        // Overlaps the first one.
        EmoteRange {
            id: "9002".to_owned(),
            start: 5,
            end: 11,
        },
        // Empty and out of bounds.
        EmoteRange {
            id: "9002".to_owned(),
            start: 3,
            end: 3,
        },
        EmoteRange {
            id: "9002".to_owned(),
            start: 16,
            end: 99,
        },
    ];
    let r = a.render(&m);
    assert_eq!(
        shape(&r),
        ["E(9001:GlimSmile)", "T( )", "E(777123:zorpNod)"]
    );
    let joined: String = r
        .fragments
        .iter()
        .map(|f| match f {
            Fragment::Text(t) => t.clone(),
            Fragment::Emote { code, .. } => code.clone(),
        })
        .collect();
    assert_eq!(joined, m.text);

    // No ranges, empty text.
    assert_eq!(
        shape(&a.render(&message("just words", &[], &[]))),
        ["T(just words)"]
    );
    assert!(a.render(&message("", &[], &[])).fragments.is_empty());
}

#[tokio::test]
async fn unknown_emotes_render_from_the_cdn_by_id() {
    let a = assets().await;
    let m = message(
        "look borrowedHi",
        &[("emotesv2_ffff0000", "borrowedHi")],
        &[],
    );
    let r = a.render(&m);
    let Fragment::Emote { asset, code, .. } = &r.fragments[1] else {
        panic!("emote expected");
    };
    assert_eq!(code, "borrowedHi");
    assert_eq!(asset.animation, Animation::Unknown);
    assert_eq!(
        asset.url(&ImagePrefs::default()),
        "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_ffff0000/default/dark/2.0"
    );
    // Same id → same shared asset.
    assert!(Arc::ptr_eq(asset, &a.emote("emotesv2_ffff0000")));

    // Anonymous chat without catalogs renders emotes the same way.
    let anon = ChannelAssets::cdn_only();
    let r = anon.render(&message(
        "x GlimSmile",
        &[("9001", "GlimSmile")],
        &[("moderator", "1")],
    ));
    assert!(r.badges.is_empty());
    assert_eq!(shape(&r), ["T(x )", "E(9001:GlimSmile)"]);
}

#[tokio::test]
async fn badges_resolve_with_channel_overrides() {
    let a = assets().await;
    let m = message(
        "hello",
        &[],
        &[
            ("moderator", "1"),
            ("subscriber", "12"),
            ("subscriber", "0"),
            ("glitchcon2020", "1"),
        ],
    );
    let r = a.render(&m);
    let keys: Vec<String> = r.badges.iter().map(|b| b.asset.key()).collect();
    assert_eq!(
        keys,
        [
            "twitch:badge:global:moderator/1",
            "twitch:badge:424242:subscriber/12",
            // The channel's own `subscriber/0` replaces the global one.
            "twitch:badge:424242:subscriber/0",
        ]
    );
    assert_eq!(r.badges[1].info.as_deref(), Some("14"));
    assert_eq!(r.badges[1].asset.title, "1-Year Subscriber");
    assert!(r.badges[1].asset.url(Scale::X4).ends_with("b012/3"));

    // A global badge without a 4× image falls back to 2×.
    let global_sub = ChannelAssets::new(
        "",
        &empty_emotes(),
        &empty_emotes(),
        &serde_json::from_value::<Vec<pa_twitch::BadgeSet>>(
            serde_json::from_str::<serde_json::Value>(fx!("assets/global_badges")).unwrap()["body"]
                ["data"]
                .clone(),
        )
        .unwrap(),
        &[],
    );
    let b = global_sub.badge("subscriber", "0").unwrap();
    assert!(b.url(Scale::X4).ends_with("a002/2"));
    assert!(b.url(Scale::X1).ends_with("a002/1"));
}

fn empty_emotes() -> pa_twitch::EmoteList {
    pa_twitch::EmoteList {
        emotes: Vec::new(),
        template: String::new(),
    }
}

// ----- Animation and image URLs -------------------------------------------

#[tokio::test]
async fn animated_emotes_keep_format_info_for_the_renderer() {
    let a = assets().await;
    let wave = a.emote("emotesv2_zorpwave0");
    assert_eq!(wave.animation, Animation::Animated);
    assert!(!wave.zero_width);
    let animated = ImagePrefs {
        scale: Scale::X4,
        theme: Theme::Light,
        animate: true,
    };
    assert_eq!(
        wave.url(&animated),
        "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_zorpwave0/animated/light/3.0"
    );
    let req = ImageRequest::emote(&wave, &animated);
    assert_eq!(req.key, "twitch:emote:emotesv2_zorpwave0");
    assert_eq!(req.animated, Some(true));

    // Motion off (reduced motion, thinned TV chat): static variant, own key.
    let still = ImagePrefs {
        animate: false,
        ..animated
    };
    let req = ImageRequest::emote(&wave, &still);
    assert!(req.url.contains("/static/light/3.0"));
    assert_eq!(req.key, "twitch:emote:emotesv2_zorpwave0#static");
    assert_eq!(req.animated, Some(false));

    // A still emote is still in both modes, with one key.
    let nod = a.emote("777123");
    assert_eq!(nod.animation, Animation::Static);
    assert_eq!(
        ImageRequest::emote(&nod, &animated).key,
        ImageRequest::emote(&nod, &still).key
    );
    assert_eq!(Scale::for_ratio(1.0), Scale::X1);
    assert_eq!(Scale::for_ratio(1.75), Scale::X2);
    assert_eq!(Scale::for_ratio(2.75), Scale::X4);
}

// ----- Image cache ---------------------------------------------------------

/// Serves a tiny GIF or PNG by URL and counts downloads.
#[derive(Default)]
struct CountingFetcher {
    calls: AtomicUsize,
    urls: Mutex<Vec<String>>,
}

impl ImageFetcher for CountingFetcher {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.urls.lock().unwrap().push(url.to_owned());
        let body = if url.contains("/animated/") || url.contains("/default/") {
            b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec()
        } else {
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec()
        };
        Ok(Fetched { status: 200, body })
    }
}

/// One minute of a busy chat: 100 messages per second, every message with
/// badges and several emotes from a small set.
fn busy_minute() -> Vec<ChatMessage> {
    let pool: [(&str, &str); 5] = [
        ("emotesv2_zorpwave0", "zorpWave"),
        ("9001", "GlimSmile"),
        ("777123", "zorpNod"),
        ("9002", "<3"),
        ("emotesv2_ffff0000", "borrowedHi"),
    ];
    (0..6000)
        .map(|i| {
            let picks = [pool[i % 5], pool[(i + 2) % 5], pool[(i + 3) % 5]];
            let text = format!(
                "сообщение {i} {} ок {} и {} конец",
                picks[0].1, picks[1].1, picks[2].1
            );
            message(&text, &picks, &[("moderator", "1"), ("subscriber", "12")])
        })
        .collect()
}

#[tokio::test]
async fn each_image_is_downloaded_once_for_a_busy_chat() {
    let a = assets().await;
    let rendered: Vec<RenderedMessage> = busy_minute().iter().map(|m| a.render(m)).collect();

    // Every use of an emote shares one asset: one decode per emote.
    let waves: Vec<_> = rendered
        .iter()
        .flat_map(|r| r.fragments.iter())
        .filter_map(|f| match f {
            Fragment::Emote { asset, .. } if asset.id == "emotesv2_zorpwave0" => Some(asset),
            _ => None,
        })
        .collect();
    assert!(waves.len() > 1000);
    assert!(waves.iter().all(|w| Arc::ptr_eq(w, waves[0])));

    let prefs = ImagePrefs::default();
    let requests = image_requests(&rendered, &prefs);
    // 5 emotes + 2 badges, whatever the number of messages.
    assert_eq!(requests.len(), 7);
    let animated: Vec<_> = requests
        .iter()
        .filter(|r| r.animated == Some(true))
        .collect();
    assert_eq!(animated.len(), 1);
    assert!(requests.iter().any(|r| r.animated.is_none()));

    let dir = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(CountingFetcher::default());
    let images = ChatImages::open(dir.path(), 1024 * 1024, fetcher.clone()).unwrap();
    assert!(images.cached(&requests[0]).is_none());
    for _ in 0..3 {
        for r in &requests {
            let path = images.get(r).unwrap();
            let ext = path.extension().unwrap().to_str().unwrap();
            let gif = r.url.contains("/animated/") || r.url.contains("/default/");
            assert_eq!(ext, if gif { "gif" } else { "png" }, "{}", r.url);
        }
    }
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 7);
    assert!(images.cached(&requests[0]).is_some());
    assert_eq!(images.cache().usage().files, 7);
    assert!(images.dir().starts_with(dir.path()));
}

/// Done-when of TWI-011 for the core part: resolving a minute of chat at
/// 100 messages per second must take a small share of that minute. The
/// bound here (10 % of one core, in a debug build) is far above the
/// measured cost so that the test never flakes; the measured numbers are
/// printed (`--nocapture`) and quoted in the PR.
#[tokio::test]
async fn a_minute_of_busy_chat_renders_in_a_fraction_of_a_minute() {
    let a = assets().await;
    let minute = busy_minute();
    let prefs = ImagePrefs::default();
    let started = Instant::now();
    let mut fragments = 0usize;
    for chunk in minute.chunks(20) {
        // The renderer batches every 100-200 ms (TWI-012): 20 messages.
        let rendered: Vec<_> = chunk.iter().map(|m| a.render(m)).collect();
        fragments += rendered.iter().map(|r| r.fragments.len()).sum::<usize>();
        assert!(!image_requests(&rendered, &prefs).is_empty());
    }
    let spent = started.elapsed();
    let share = spent.as_secs_f64() / 60.0;
    println!(
        "rendered 6000 messages ({fragments} fragments) in {spent:?}: {:.4} % of one core at 100 msg/s",
        share * 100.0
    );
    assert!(share < 0.10, "{spent:?} for one minute of chat");
}
