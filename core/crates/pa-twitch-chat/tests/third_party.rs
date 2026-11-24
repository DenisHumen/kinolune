// SPDX-License-Identifier: MPL-2.0
//! Third-party emotes (TWI-023) on recorded provider answers: parsing,
//! precedence, rendering by word with zero-width overlays, the disk cache,
//! and the watcher refreshing a channel when 7TV says its set changed.
//! No live network: answers come from `tests/fixtures/third_party`
//! (recorded from the public APIs, trimmed) through a fake transport.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use pa_twitch::testing::ManualClock;
use pa_twitch::{BoxFuture, EmoteList, HttpRequest, HttpResponse, Transport, TransportError};
use pa_twitch_chat::testing::{Script, ScriptedConnector};
use pa_twitch_chat::third_party::{self, parse};
use pa_twitch_chat::{
    Animation, ChannelAssets, ChatMessage, EmoteAsset, EmoteProvider, EmoteRange, EmoteSet,
    EmoteSetsClient, Fragment, ImagePrefs, ImageRequest, Scale, SetScope, ThirdPartyConfig,
    ThirdPartyEmotes, ThirdPartyWatcher, image_requests,
};

const CHANNEL: &str = "71092938";

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/third_party/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Answers by URL; anything else is a 500. Records requests.
#[derive(Default)]
struct Routes {
    bodies: Mutex<HashMap<String, (u16, Vec<u8>)>>,
    seen: Mutex<Vec<String>>,
}

impl Routes {
    fn all_fixtures() -> Arc<Self> {
        let r = Arc::new(Self::default());
        for (p, scope, file) in [
            (EmoteProvider::SevenTv, SetScope::Global, "7tv_global.json"),
            (EmoteProvider::SevenTv, SetScope::Channel, "7tv_user.json"),
            (EmoteProvider::Bttv, SetScope::Global, "bttv_global.json"),
            (EmoteProvider::Bttv, SetScope::Channel, "bttv_user.json"),
            (EmoteProvider::Ffz, SetScope::Global, "ffz_global.json"),
            (EmoteProvider::Ffz, SetScope::Channel, "ffz_room.json"),
        ] {
            r.set(p, scope, 200, fixture(file));
        }
        r
    }

    fn set(&self, p: EmoteProvider, scope: SetScope, status: u16, body: Vec<u8>) {
        let url = EmoteSetsClient::url(p, scope, CHANNEL).unwrap();
        self.bodies.lock().unwrap().insert(url, (status, body));
    }

    fn count(&self, needle: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.contains(needle))
            .count()
    }
}

impl Transport for Routes {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        assert!(
            request.header("authorization").is_none(),
            "no token goes to a third party"
        );
        assert!(request.header("client-id").is_none());
        self.seen.lock().unwrap().push(request.url.clone());
        let found = self.bodies.lock().unwrap().get(&request.url).cloned();
        Box::pin(async move {
            let (status, body) = found.unwrap_or((500, Vec::new()));
            Ok(HttpResponse {
                status,
                headers: Vec::new(),
                body,
            })
        })
    }
}

fn block<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

fn by_code<'a>(set: &'a EmoteSet, code: &str) -> &'a EmoteAsset {
    set.emotes
        .iter()
        .find(|e| e.code == code)
        .unwrap_or_else(|| panic!("{code} not in set"))
}

fn prefs(animate: bool) -> ImagePrefs {
    ImagePrefs {
        scale: Scale::X2,
        animate,
        ..ImagePrefs::default()
    }
}

#[test]
fn seventv_global_set_with_zero_width_and_animation() {
    let set = parse::seventv_set(&fixture("7tv_global.json"), SetScope::Global).unwrap();
    assert_eq!(set.provider, EmoteProvider::SevenTv);
    assert_eq!(set.emotes.len(), 5);
    let rain = by_code(&set, "RainTime");
    assert!(rain.zero_width);
    assert_eq!(rain.animation, Animation::Animated);
    assert_eq!(rain.key(), "7tv:emote:01FCY771D800007PQ2DF3GDTN6");
    assert_eq!(
        rain.url(&prefs(true)),
        "https://cdn.7tv.app/emote/01FCY771D800007PQ2DF3GDTN6/2x.webp"
    );
    // Motion off: 7TV's still variant.
    assert_eq!(
        rain.url(&prefs(false)),
        "https://cdn.7tv.app/emote/01FCY771D800007PQ2DF3GDTN6/2x_static.webp"
    );
    let still = by_code(&set, "ppL");
    assert!(!still.zero_width);
    assert_eq!(still.animation, Animation::Static);
    assert!(still.url(&prefs(true)).ends_with("/2x.webp"));
    let x4 = ImagePrefs {
        scale: Scale::X4,
        ..prefs(true)
    };
    assert!(still.url(&x4).ends_with("/4x.webp"));
}

#[test]
fn seventv_channel_uses_the_alias_and_remembers_the_set() {
    let set = parse::seventv_user(&fixture("7tv_user.json")).unwrap();
    assert_eq!(set.scope, SetScope::Channel);
    assert_eq!(set.set_id.as_deref(), Some("01FE9DRF000009TR6M9N941CYW"));
    assert!(set.owner_id.is_some());
    assert_eq!(set.emotes.len(), 6);
    // The channel renamed this emote: the alias is what chat types.
    assert!(set.emotes.iter().any(|e| e.code == "PutAFingerDown"));
    assert!(!set.emotes.iter().any(|e| e.code == "Me:"));
    assert_eq!(by_code(&set, "GAMBA").animation, Animation::Animated);
}

#[test]
fn seventv_user_without_a_set_is_empty() {
    let set = parse::seventv_user(br#"{"id":"1","emote_set":null,"user":{"id":"u"}}"#).unwrap();
    assert!(set.emotes.is_empty());
    assert_eq!(set.owner_id.as_deref(), Some("u"));
    assert!(parse::seventv_user(b"{}").unwrap().emotes.is_empty());
}

#[test]
fn bttv_global_skips_text_modifiers_and_marks_overlays() {
    let set = parse::bttv_global(&fixture("bttv_global.json")).unwrap();
    assert!(set.emotes.iter().all(|e| e.provider == EmoteProvider::Bttv));
    // `h!`, `w!`… transform the next emote: not supported, stay text.
    assert!(!set.emotes.iter().any(|e| e.code == "h!"));
    assert!(by_code(&set, "cvMask").zero_width);
    assert!(!by_code(&set, "FeelsGoodMan").zero_width);
    let sour = by_code(&set, "SourPls");
    assert_eq!(sour.animation, Animation::Animated);
    assert_eq!(
        sour.url(&prefs(true)),
        "https://cdn.betterttv.net/emote/566ca38765dbbdab32ec0560/2x"
    );
    assert_eq!(
        sour.url(&prefs(false)),
        "https://cdn.betterttv.net/emote/566ca38765dbbdab32ec0560/static/2x.webp"
    );
    let x4 = ImagePrefs {
        scale: Scale::X4,
        ..prefs(true)
    };
    assert!(by_code(&set, "monkaS").url(&x4).ends_with("/3x"));
}

#[test]
fn bttv_channel_has_shared_and_own_emotes() {
    let set = parse::bttv_user(&fixture("bttv_user.json")).unwrap();
    assert_eq!(set.emotes.len(), 8);
    assert!(set.emotes.iter().any(|e| e.code == "forsenBlunder"));
    assert!(set.emotes.iter().any(|e| e.code == "RebeccaBlack"));
    assert_eq!(by_code(&set, "RareParrot").animation, Animation::Animated);
}

#[test]
fn ffz_global_uses_default_sets_and_skips_hidden_and_modifiers() {
    let set = parse::ffz_global(&fixture("ffz_global.json")).unwrap();
    assert!(!set.emotes.is_empty());
    // Hidden in the global set.
    assert!(!set.emotes.iter().any(|e| e.code == "ZrehplaR"));
    // Effect modifiers.
    assert!(!set.emotes.iter().any(|e| e.code == "ffzSpin"));
    let yoohoo = by_code(&set, "YooHoo");
    assert_eq!(yoohoo.key(), "ffz:emote:6");
    assert_eq!(
        yoohoo.url(&prefs(true)),
        "https://cdn.frankerfacez.com/emote/6/2"
    );
}

#[test]
fn ffz_room_with_an_animated_emote() {
    let set = parse::ffz_room(&fixture("ffz_room.json")).unwrap();
    assert_eq!(set.scope, SetScope::Channel);
    assert!(set.emotes.iter().any(|e| e.code == "WideHard"));
    let cat = by_code(&set, "catJAM");
    assert_eq!(cat.animation, Animation::Animated);
    assert_eq!(
        cat.url(&prefs(true)),
        "https://cdn.frankerfacez.com/emote/720564/animated/2"
    );
    assert_eq!(
        cat.url(&prefs(false)),
        "https://cdn.frankerfacez.com/emote/720564/2"
    );
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    for p in third_party::THIRD_PARTY_PROVIDERS {
        for scope in [SetScope::Global, SetScope::Channel] {
            assert!(parse::set(p, scope, b"<html>").is_err());
        }
    }
}

fn emote(p: EmoteProvider, id: &str, code: &str, zero_width: bool) -> Arc<EmoteAsset> {
    let url = format!("https://example.invalid/{id}");
    Arc::new(EmoteAsset::third_party(
        p,
        id,
        code,
        zero_width,
        [url.clone(), url.clone(), url],
        Default::default(),
    ))
}

fn set(p: EmoteProvider, scope: SetScope, emotes: Vec<Arc<EmoteAsset>>) -> EmoteSet {
    EmoteSet {
        emotes,
        ..EmoteSet::empty(p, scope)
    }
}

#[test]
fn precedence_channel_over_global_then_7tv_bttv_ffz() {
    use EmoteProvider::*;
    use SetScope::*;
    let merged = ThirdPartyEmotes::merge(&[
        set(SevenTv, Global, vec![emote(SevenTv, "g7", "EZ", false)]),
        set(Ffz, Channel, vec![emote(Ffz, "c3", "EZ", false)]),
        set(Bttv, Channel, vec![emote(Bttv, "c2", "EZ", false)]),
        set(Bttv, Global, vec![emote(Bttv, "g2", "OMEGALUL", false)]),
        set(Ffz, Global, vec![emote(Ffz, "g3", "OMEGALUL", false)]),
        set(Ffz, Global, vec![emote(Ffz, "g4", "LULW", false)]),
    ]);
    assert_eq!(merged.get("EZ").unwrap().id, "c2");
    assert_eq!(merged.get("OMEGALUL").unwrap().id, "g2");
    assert_eq!(merged.get("LULW").unwrap().id, "g4");
    assert!(merged.is_channel(merged.get("EZ").unwrap()));
    assert!(!merged.is_channel(merged.get("LULW").unwrap()));
    assert!(merged.get("ez").is_none(), "codes are case-sensitive");
}

fn assets_with(tp: ThirdPartyEmotes) -> ChannelAssets {
    let empty = EmoteList {
        emotes: Vec::new(),
        template: String::new(),
    };
    ChannelAssets::new("1", &empty, &empty, &[], &[]).with_third_party(Some(Arc::new(tp)))
}

fn message(text: &str, emotes: Vec<EmoteRange>) -> ChatMessage {
    ChatMessage {
        id: "m".into(),
        text: text.into(),
        emotes,
        ..Default::default()
    }
}

fn show(fragments: &[Fragment]) -> Vec<String> {
    fragments
        .iter()
        .map(|f| match f {
            Fragment::Text(t) => format!("T({t})"),
            Fragment::Emote {
                asset,
                code,
                overlay,
            } => format!(
                "{}({}:{code})",
                if *overlay { "O" } else { "E" },
                asset.provider.as_str()
            ),
        })
        .collect()
}

#[test]
fn words_become_emotes_and_zero_width_overlays_the_previous_emote() {
    use EmoteProvider::*;
    let assets = assets_with(ThirdPartyEmotes::merge(&[
        set(
            SevenTv,
            SetScope::Channel,
            vec![
                emote(SevenTv, "a", "catJAM", false),
                emote(SevenTv, "zw", "RainTime", true),
            ],
        ),
        set(
            Bttv,
            SetScope::Global,
            vec![emote(Bttv, "b", "cvMask", true)],
        ),
    ]));
    // "Kappa" is a Twitch range; the rest is matched by word.
    let m = message(
        "hi Kappa RainTime and catJAM  cvMask RainTime ok RainTime catJAM!",
        vec![EmoteRange {
            id: "25".into(),
            start: 3,
            end: 8,
        }],
    );
    let r = assets.render(&m);
    assert_eq!(
        show(&r.fragments),
        [
            "T(hi )",
            "E(twitch:Kappa)",
            "O(7tv:RainTime)",
            "T( and )",
            "E(7tv:catJAM)",
            "O(bttv:cvMask)",
            "O(7tv:RainTime)",
            "T( ok )",
            // Not after an emote: an ordinary emote.
            "E(7tv:RainTime)",
            // Punctuation glued to a code is not the code.
            "T( catJAM!)",
        ]
    );
    // Each image once.
    let reqs = image_requests([&r], &prefs(true));
    assert_eq!(reqs.len(), 4);
    assert!(reqs.iter().any(|q| q.key == "7tv:emote:zw"));
}

#[test]
fn without_third_party_the_text_is_untouched() {
    let empty = EmoteList {
        emotes: Vec::new(),
        template: String::new(),
    };
    let tp = ThirdPartyEmotes::merge(&[set(
        EmoteProvider::SevenTv,
        SetScope::Global,
        vec![emote(EmoteProvider::SevenTv, "a", "catJAM", false)],
    )]);
    let on = ChannelAssets::new("1", &empty, &empty, &[], &[]).with_third_party(Some(Arc::new(tp)));
    let off = on.with_third_party(None);
    let m = message("catJAM catJAM", Vec::new());
    assert_eq!(show(&on.render(&m).fragments).len(), 3);
    assert_eq!(show(&off.render(&m).fragments), ["T(catJAM catJAM)"]);
    assert!(off.third_party().is_none());
}

#[test]
fn autocomplete_lists_third_party_and_knows_channel_ones() {
    let assets = assets_with(ThirdPartyEmotes::merge(&[
        parse::seventv_user(&fixture("7tv_user.json")).unwrap(),
        parse::bttv_global(&fixture("bttv_global.json")).unwrap(),
    ]));
    let all = assets.emotes();
    let gamba = all.iter().find(|e| e.code == "GAMBA").unwrap();
    assert!(assets.is_channel_emote(gamba));
    let sour = all.iter().find(|e| e.code == "SourPls").unwrap();
    assert!(!assets.is_channel_emote(sour));
}

#[test]
fn an_animated_emote_with_motion_off_asks_for_the_still_key() {
    let set = parse::seventv_set(&fixture("7tv_global.json"), SetScope::Global).unwrap();
    let rain = by_code(&set, "RainTime");
    let on = ImageRequest::emote(rain, &prefs(true));
    let off = ImageRequest::emote(rain, &prefs(false));
    assert_eq!(on.animated, Some(true));
    assert_eq!(off.animated, Some(false));
    assert_ne!(on.key, off.key);
}

#[test]
fn client_fetches_caches_and_treats_404_as_no_emotes() {
    let dir = tempfile::tempdir().unwrap();
    let routes = Routes::all_fixtures();
    routes.set(
        EmoteProvider::Bttv,
        SetScope::Channel,
        404,
        b"{\"message\":\"user not found\"}".to_vec(),
    );
    let clock = Arc::new(ManualClock::new(SystemTime::now()));
    let client = EmoteSetsClient::with_clock(
        routes.clone(),
        Some(dir.path().to_path_buf()),
        "Kinolune-test",
        clock.clone(),
    );
    block(async {
        let s = client
            .fetch(EmoteProvider::SevenTv, SetScope::Channel, CHANNEL)
            .await
            .unwrap();
        assert_eq!(s.emotes.len(), 6);
        let b = client
            .fetch(EmoteProvider::Bttv, SetScope::Channel, CHANNEL)
            .await
            .unwrap();
        assert!(b.emotes.is_empty());
        assert!(
            client
                .fetch(EmoteProvider::Ffz, SetScope::Channel, "../etc")
                .await
                .is_err()
        );
    });
    // From disk, no network.
    let before = routes.seen.lock().unwrap().len();
    let cached = client
        .cached(EmoteProvider::SevenTv, SetScope::Channel, CHANNEL)
        .unwrap();
    assert_eq!(cached.emotes.len(), 6);
    assert!(
        client
            .cached(EmoteProvider::Bttv, SetScope::Channel, CHANNEL)
            .unwrap()
            .emotes
            .is_empty()
    );
    assert_eq!(routes.seen.lock().unwrap().len(), before);
    // Older than a day: gone.
    clock.advance(Duration::from_secs(25 * 60 * 60));
    assert!(
        client
            .cached(EmoteProvider::SevenTv, SetScope::Channel, CHANNEL)
            .is_none()
    );
}

#[test]
fn server_errors_keep_the_previous_set() {
    let routes = Arc::new(Routes::default());
    let client = EmoteSetsClient::new(routes, None, "Kinolune-test");
    let e = block(client.fetch(EmoteProvider::SevenTv, SetScope::Global, ""));
    assert!(matches!(
        e,
        Err(third_party::ProviderError::Status("7TV", 500))
    ));
}

fn recv_until(
    rx: &mpsc::Receiver<Arc<ThirdPartyEmotes>>,
    what: impl Fn(&ThirdPartyEmotes) -> bool,
) -> Arc<ThirdPartyEmotes> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let got = rx.recv_timeout(left).expect("watcher update");
        if what(&got) {
            return got;
        }
    }
}

#[test]
fn watcher_loads_all_sets_then_follows_a_7tv_change() {
    let routes = Routes::all_fixtures();
    let client = Arc::new(EmoteSetsClient::new(routes.clone(), None, "Kinolune-test"));
    let set_id = "01FE9DRF000009TR6M9N941CYW";
    let hello = r#"{"op":1,"d":{"heartbeat_interval":25000,"session_id":"s"}}"#;
    let dispatch =
        format!(r#"{{"op":0,"d":{{"type":"emote_set.update","body":{{"id":"{set_id}"}}}}}}"#);
    let connector = Arc::new(ScriptedConnector::new());
    connector.push(Script::hanging([hello.to_owned(), dispatch]));
    let (tx, rx) = mpsc::channel();
    let config = ThirdPartyConfig {
        event_debounce: Duration::from_millis(200),
        ..ThirdPartyConfig::default()
    };
    // The channel's 7TV set changes before the event is handled.
    let changed = {
        let mut v: serde_json::Value = serde_json::from_slice(&fixture("7tv_user.json")).unwrap();
        v["emote_set"]["emotes"].as_array_mut().unwrap().truncate(1);
        serde_json::to_vec(&v).unwrap()
    };
    let routes2 = routes.clone();
    let mut first = true;
    let _watcher = ThirdPartyWatcher::start(
        client,
        config,
        Some(CHANNEL.into()),
        Some(connector.clone()),
        move |emotes| {
            if first && emotes.get("GAMBA").is_some() {
                first = false;
                routes2.set(
                    EmoteProvider::SevenTv,
                    SetScope::Channel,
                    200,
                    changed.clone(),
                );
            }
            let _ = tx.send(emotes);
        },
    )
    .unwrap();
    let full = recv_until(&rx, |e| e.get("GAMBA").is_some());
    assert!(full.get("SourPls").is_some(), "BTTV global");
    assert!(full.get("forsenBlunder").is_some(), "BTTV channel");
    assert!(full.get("catJAM").is_some(), "FFZ channel");
    assert!(full.get("RainTime").is_some(), "7TV global");
    let after = recv_until(&rx, |e| e.get("GAMBA").is_none());
    assert!(after.get("Hmm").is_some());
    assert!(after.get("SourPls").is_some());
    assert_eq!(routes.count("7tv.io/v3/users/twitch"), 2);
    // Subscribed to the set and to the user.
    let sent = connector.sent();
    assert!(
        sent.iter()
            .any(|(_, s)| s.contains("emote_set.update") && s.contains(set_id))
    );
    assert!(sent.iter().any(|(_, s)| s.contains("user.update")));
}

#[test]
fn anonymous_watcher_learns_the_channel_later() {
    let routes = Routes::all_fixtures();
    let client = Arc::new(EmoteSetsClient::new(routes.clone(), None, "Kinolune-test"));
    let (tx, rx) = mpsc::channel();
    let watcher = ThirdPartyWatcher::start(
        client,
        ThirdPartyConfig {
            seventv_events: false,
            ..ThirdPartyConfig::default()
        },
        None,
        None,
        move |e| {
            let _ = tx.send(e);
        },
    )
    .unwrap();
    let global = recv_until(&rx, |e| e.get("SourPls").is_some());
    assert!(global.get("GAMBA").is_none());
    assert_eq!(routes.count("/users/twitch/"), 0);
    watcher.set_channel(CHANNEL);
    let channel = recv_until(&rx, |e| e.get("GAMBA").is_some());
    assert!(channel.get("SourPls").is_some());
}

#[test]
fn event_frames_are_classified() {
    use third_party::{SevenTvEvent, seventv_event, seventv_subscriptions};
    assert_eq!(
        seventv_event(r#"{"op":1,"d":{"heartbeat_interval":30000}}"#),
        SevenTvEvent::Hello(Duration::from_secs(30))
    );
    assert_eq!(
        seventv_event(r#"{"op":0,"d":{"type":"user.update","body":{}}}"#),
        SevenTvEvent::Changed
    );
    assert_eq!(
        seventv_event(r#"{"op":0,"d":{"type":"cosmetic.create"}}"#),
        SevenTvEvent::Other
    );
    assert_eq!(seventv_event(r#"{"op":4,"d":{}}"#), SevenTvEvent::Reconnect);
    assert_eq!(seventv_event("not json"), SevenTvEvent::Other);
    let subs = seventv_subscriptions(Some("set"), Some("user"));
    assert_eq!(subs.len(), 2);
    let v: serde_json::Value = serde_json::from_str(&subs[0]).unwrap();
    assert_eq!(v["op"], 35);
    assert_eq!(v["d"]["condition"]["object_id"], "set");
}

/// CPU cost of rendering with a big third-party set (TWI-012 budget):
/// `cargo test --release -p pa-twitch-chat --test third_party -- --ignored
/// --nocapture render_cost`. A minute of a 100 messages/s chat is 6000
/// messages.
#[test]
#[ignore = "measurement, run by hand"]
fn render_cost() {
    use std::time::Instant;
    let mut sets = vec![
        parse::seventv_set(&fixture("7tv_global.json"), SetScope::Global).unwrap(),
        parse::seventv_user(&fixture("7tv_user.json")).unwrap(),
        parse::bttv_global(&fixture("bttv_global.json")).unwrap(),
        parse::bttv_user(&fixture("bttv_user.json")).unwrap(),
        parse::ffz_global(&fixture("ffz_global.json")).unwrap(),
        parse::ffz_room(&fixture("ffz_room.json")).unwrap(),
    ];
    // A big channel: 1000 more 7TV emotes, some zero-width.
    let big: Vec<_> = (0..1000)
        .map(|i| {
            emote(
                EmoteProvider::SevenTv,
                &format!("big{i}"),
                &format!("emote{i}W"),
                i % 50 == 0,
            )
        })
        .collect();
    sets.push(set(EmoteProvider::SevenTv, SetScope::Channel, big));
    let tp = Arc::new(ThirdPartyEmotes::merge(&sets));
    let empty = EmoteList {
        emotes: Vec::new(),
        template: String::new(),
    };
    let base = ChannelAssets::new("1", &empty, &empty, &[], &[]);
    let with = base.with_third_party(Some(tp));
    let words = [
        "hello",
        "chat",
        "that",
        "was",
        "great",
        "play",
        "again",
        "wow",
        "nice",
        "LUL",
        "привет",
        "emote12W",
        "catJAM",
        "RainTime",
        "SourPls",
        "emote950W",
        "monkaS",
        "OMEGALUL",
        "gg",
    ];
    let mut rng = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let messages: Vec<ChatMessage> = (0..6000)
        .map(|_| {
            let n = 2 + (next() % 12) as usize;
            let text: Vec<&str> = (0..n)
                .map(|_| words[(next() % words.len() as u64) as usize])
                .collect();
            message(&text.join(" "), Vec::new())
        })
        .collect();
    for (name, assets) in [("twitch only", &base), ("with 7TV/BTTV/FFZ", &with)] {
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            let mut emotes = 0usize;
            for m in &messages {
                emotes += assets.render(m).emotes().len();
            }
            std::hint::black_box(emotes);
            best = best.min(t.elapsed());
        }
        println!(
            "{name}: 6000 messages (one minute at 100/s) in {:.2} ms = {:.4} % of one core",
            best.as_secs_f64() * 1e3,
            best.as_secs_f64() / 60.0 * 100.0
        );
    }
}
