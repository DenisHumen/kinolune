// SPDX-License-Identifier: MPL-2.0
//! A synthetic chat for the demo mode and load measurements (TWI-012).
//!
//! [`SyntheticChatSource`] produces a busy made-up chat at a given rate:
//! invented users with badges and name colours (some too dark for a dark
//! background, to exercise contrast correction), messages with static and
//! animated emotes, replies, actions, system notices, and now and then a
//! moderator delete or timeout. [`demo_assets`] writes the matching emote
//! and badge images (generated GIFs) into a directory and returns the
//! [`ChannelAssets`] that point at them, so nothing touches the network.
//!
//! The generator is deterministic for a seed.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pa_twitch::{BadgeSet, BoxFuture, EmoteList, SentChatMessage};
use tokio::sync::Mutex;

use crate::assets::ChannelAssets;
use crate::model::{
    Badge, ChannelRef, ChatEvent, ChatMessage, ChatUser, ClearAction, EmoteRange, NoticeKind,
    ReplyInfo, SourceStatus, SystemNotice,
};
use crate::source::{ChatError, ChatSource, SourceKind};

/// Channel of the synthetic chat.
pub const DEMO_CHANNEL: &str = "kinolune_demo";

/// Broadcaster id of the synthetic chat.
const DEMO_CHANNEL_ID: &str = "100000001";

/// Batches are produced every 50 ms (20 per second).
const TICK: Duration = Duration::from_millis(50);

/// `(id, code, animated)` of the demo emotes.
const EMOTES: &[(&str, &str, bool)] = &[
    ("demo_1", "lunaWave", false),
    ("demo_2", "lunaHype", true),
    ("demo_3", "owlLol", false),
    ("demo_4", "owlSpin", true),
    ("demo_5", "teaCup", false),
    ("demo_6", "cometZoom", true),
    ("demo_7", "mossGG", false),
    ("demo_8", "foxClap", true),
];

/// `(set, version, title, rgb)` of the demo badges.
const BADGES: &[(&str, &str, &str, [u8; 3])] = &[
    ("broadcaster", "1", "Broadcaster", [0xE9, 0x19, 0x16]),
    ("moderator", "1", "Moderator", [0x00, 0xAD, 0x03]),
    ("vip", "1", "VIP", [0xE0, 0x05, 0xB9]),
    ("subscriber", "0", "Subscriber", [0x8A, 0x5C, 0xF5]),
    ("subscriber", "12", "1-Year Subscriber", [0x5C, 0x9C, 0xF5]),
];

/// Made-up users: `(login, display name, colour)`.
const USERS: &[(&str, &str, Option<&str>)] = &[
    ("lunafox", "LunaFox", Some("#FF7F50")),
    ("pixel_owl", "pixel_owl", Some("#1E1E7A")),
    ("teakettle42", "TeaKettle42", Some("#2E8B57")),
    ("northwind", "Northwind", None),
    ("mossy_rock", "mossy_rock", Some("#000000")),
    ("quietcomet", "QuietComet", Some("#8A2BE2")),
    ("bramble", "Bramble", Some("#DAA520")),
    ("zz_orbit", "zz_orbit", Some("#0000FF")),
    ("kettu", "Kettu", Some("#FF69B4")),
    ("veterok", "Ветерок", Some("#00FF7F")),
    ("paper_crane", "paper_crane", None),
    ("driftwood", "Driftwood", Some("#B22222")),
];

const WORDS: &[&str] = &[
    "hello",
    "chat",
    "that",
    "was",
    "great",
    "play",
    "again",
    "wow",
    "nice",
    "run",
    "boss",
    "first",
    "try",
    "no",
    "way",
    "clip",
    "it",
    "lol",
    "gg",
    "what",
    "a",
    "jump",
    "привет",
    "чат",
    "круто",
    "ещё",
    "раз",
    "is",
    "this",
    "the",
    "final",
    "level",
    "?",
    "!",
    "yes",
];

/// A deterministic made-up chat at `rate` messages per second.
#[derive(Debug)]
pub struct SyntheticChatSource {
    state: Mutex<Generator>,
    closed: AtomicBool,
}

impl SyntheticChatSource {
    /// `rate` messages per second (0 means 1), generator seed `seed`.
    pub fn new(rate: u32, seed: u64) -> Self {
        Self {
            state: Mutex::new(Generator::new(rate.max(1), seed)),
            closed: AtomicBool::new(false),
        }
    }
}

impl ChatSource for SyntheticChatSource {
    fn kind(&self) -> SourceKind {
        SourceKind::AnonymousIrc
    }

    fn can_send(&self) -> bool {
        false
    }

    fn next_event(&self) -> BoxFuture<'_, Option<ChatEvent>> {
        Box::pin(async move {
            let mut g = self.state.lock().await;
            loop {
                if self.closed.load(Ordering::Relaxed) {
                    return None;
                }
                if let Some(e) = g.pending.pop_front() {
                    return Some(e);
                }
                let wait = g.interval.get_or_insert_with(|| {
                    let mut i = tokio::time::interval(TICK);
                    i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    i
                });
                wait.tick().await;
                g.fill_tick();
            }
        })
    }

    fn send_message<'a>(
        &'a self,
        _broadcaster_id: &'a str,
        _text: &'a str,
        _reply_to: Option<&'a str>,
    ) -> BoxFuture<'a, Result<SentChatMessage, ChatError>> {
        Box::pin(async { Err(ChatError::ReadOnly) })
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct Generator {
    rng: u64,
    rate: u32,
    /// Fractional messages carried to the next tick (thousandths).
    carry_milli: u32,
    seq: u64,
    pending: VecDeque<ChatEvent>,
    recent: VecDeque<(String, usize)>,
    interval: Option<tokio::time::Interval>,
}

impl Generator {
    fn new(rate: u32, seed: u64) -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(ChatEvent::Status(SourceStatus::Connected));
        pending.push_back(ChatEvent::Status(SourceStatus::Joined(channel())));
        Self {
            rng: seed | 1,
            rate,
            carry_milli: 0,
            seq: 0,
            pending,
            recent: VecDeque::new(),
            interval: None,
        }
    }

    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, per_mille: u64) -> bool {
        self.next() % 1000 < per_mille
    }

    /// Events of one tick: `rate · TICK` messages plus moderation.
    fn fill_tick(&mut self) {
        let milli = self.rate * u32::try_from(TICK.as_millis()).unwrap_or(50) + self.carry_milli;
        let count = milli / 1000;
        self.carry_milli = milli % 1000;
        for _ in 0..count {
            let event = self.event();
            self.pending.push_back(event);
        }
    }

    /// One more event (public for tests through [`SyntheticChatSource`]).
    fn event(&mut self) -> ChatEvent {
        if self.recent.len() > 20 && self.chance(8) {
            let i = self.pick(self.recent.len());
            let (id, user) = self.recent.remove(i).unwrap_or_default();
            return ChatEvent::MessageDeleted {
                channel: channel(),
                message_id: id,
                user_login: Some(USERS[user].0.to_owned()),
            };
        }
        if self.recent.len() > 20 && self.chance(3) {
            let user = self.pick(USERS.len());
            return ChatEvent::UserCleared {
                channel: channel(),
                user_id: user_id(user),
                user_login: USERS[user].0.to_owned(),
                action: ClearAction::Timeout(Duration::from_secs(60)),
            };
        }
        ChatEvent::Message(Box::new(self.message()))
    }

    fn message(&mut self) -> ChatMessage {
        self.seq += 1;
        let user = self.pick(USERS.len());
        let (login, display, color) = USERS[user];
        let mut badges = Vec::new();
        match user {
            0 => badges.push(badge(0)),
            1 | 2 => badges.push(badge(1)),
            3 => badges.push(badge(2)),
            _ => {}
        }
        if user.is_multiple_of(3) {
            badges.push(badge(if user.is_multiple_of(2) { 3 } else { 4 }));
        }

        let mut text = String::new();
        let mut chars = 0usize;
        let mut emotes = Vec::new();
        let words = 2 + self.pick(10);
        for w in 0..words {
            if w > 0 {
                text.push(' ');
                chars += 1;
            }
            if self.chance(300) {
                let (id, code, _) = EMOTES[self.pick(EMOTES.len())];
                emotes.push(EmoteRange {
                    id: id.to_owned(),
                    start: chars,
                    end: chars + code.chars().count(),
                });
                text.push_str(code);
                chars += code.chars().count();
            } else {
                let word = WORDS[self.pick(WORDS.len())];
                text.push_str(word);
                chars += word.chars().count();
            }
        }

        let id = format!("synthetic-{:08}", self.seq);
        let reply = (self.recent.len() > 5 && self.chance(40)).then(|| {
            let i = self.pick(self.recent.len());
            let (parent, parent_user) = self.recent[i].clone();
            ReplyInfo {
                parent_message_id: parent,
                parent_user_id: user_id(parent_user),
                parent_user_login: USERS[parent_user].0.to_owned(),
                parent_display_name: USERS[parent_user].1.to_owned(),
                parent_body: String::new(),
                thread_message_id: None,
                thread_user_login: None,
            }
        });
        let notice = self.seq.is_multiple_of(250).then(|| SystemNotice {
            kind: NoticeKind::Resub,
            system_text: format!("{display} subscribed for 12 months!"),
            params: Default::default(),
        });
        self.recent.push_back((id.clone(), user));
        if self.recent.len() > 64 {
            self.recent.pop_front();
        }
        ChatMessage {
            id,
            ts_ms: now_ms(),
            channel: channel(),
            user: ChatUser {
                id: user_id(user),
                login: login.to_owned(),
                display_name: display.to_owned(),
                color: color.map(str::to_owned),
                badges,
            },
            text,
            emotes,
            is_action: self.chance(20),
            bits: None,
            reply,
            notice,
            is_first_message: self.chance(10),
            is_highlighted: self.chance(5),
            reward_id: None,
            source_channel_id: None,
        }
    }
}

fn channel() -> ChannelRef {
    ChannelRef {
        id: DEMO_CHANNEL_ID.to_owned(),
        login: DEMO_CHANNEL.to_owned(),
    }
}

fn user_id(index: usize) -> String {
    format!("2000000{index:02}")
}

fn badge(i: usize) -> Badge {
    let (set, version, _, _) = BADGES[i];
    Badge {
        set_id: set.to_owned(),
        version: version.to_owned(),
        info: None,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Writes the demo emote and badge images into `dir` (created if needed)
/// and returns the channel assets that reference them by local path.
pub fn demo_assets(dir: &Path) -> std::io::Result<ChannelAssets> {
    std::fs::create_dir_all(dir)?;
    let mut emotes = Vec::new();
    for (i, (id, code, animated)) in EMOTES.iter().enumerate() {
        let hue = [0xFF, 0x60 + (i as u8) * 20, 0x40];
        std::fs::write(dir.join(format!("{id}_static.gif")), emote_gif(hue, 1))?;
        if *animated {
            std::fs::write(dir.join(format!("{id}_animated.gif")), emote_gif(hue, 4))?;
        }
        emotes.push(serde_json::json!({
            "id": id,
            "name": code,
            "images": { "url_1x": "", "url_2x": "", "url_4x": "" },
            "format": if *animated { vec!["static", "animated"] } else { vec!["static"] },
            "scale": ["1.0", "2.0", "3.0"],
            "theme_mode": ["dark", "light"],
        }));
    }
    let template = format!("{}/{{{{id}}}}_{{{{format}}}}.gif", dir.display());
    let emotes: EmoteList = serde_json::from_value(serde_json::json!({
        "emotes": emotes,
        "template": template,
    }))
    .map_err(std::io::Error::other)?;

    let mut sets: Vec<serde_json::Value> = Vec::new();
    for (set, version, title, rgb) in BADGES {
        let file = dir.join(format!("badge_{set}_{version}.gif"));
        std::fs::write(&file, badge_gif(*rgb))?;
        let url = file.display().to_string();
        let v = serde_json::json!({
            "id": version, "title": title,
            "image_url_1x": url, "image_url_2x": url, "image_url_4x": url,
        });
        match sets.iter_mut().find(|s| s["set_id"] == *set) {
            Some(s) => s["versions"]
                .as_array_mut()
                .into_iter()
                .for_each(|a| a.push(v.clone())),
            None => sets.push(serde_json::json!({ "set_id": set, "versions": [v] })),
        }
    }
    let badges: Vec<BadgeSet> =
        serde_json::from_value(serde_json::Value::Array(sets)).map_err(std::io::Error::other)?;
    let none = EmoteList {
        emotes: Vec::new(),
        template: String::new(),
    };
    Ok(ChannelAssets::new(
        DEMO_CHANNEL_ID,
        &none,
        &emotes,
        &badges,
        &[],
    ))
}

/// Shared handle for the demo source (convenience for callers that keep an
/// `Arc<dyn ChatSource>`).
pub fn synthetic_source(rate: u32, seed: u64) -> Arc<dyn ChatSource> {
    Arc::new(SyntheticChatSource::new(rate, seed))
}

// --- Tiny GIF writer -------------------------------------------------------

const EMOTE_PX: u16 = 56;
const BADGE_PX: u16 = 36;

/// A round face of `rgb`; with `frames > 1` it bounces (animated).
fn emote_gif(rgb: [u8; 3], frames: usize) -> Vec<u8> {
    let palette = [[0, 0, 0], rgb, [0x10, 0x10, 0x10], [0xFF, 0xFF, 0xFF]];
    let n = usize::from(EMOTE_PX);
    let mut out = Vec::new();
    for f in 0..frames {
        let lift = [0.0, 3.0, 6.0, 3.0][f % 4];
        let mut px = vec![0u8; n * n];
        let c = n as f32 / 2.0;
        let r = c - 4.0;
        for y in 0..n {
            for x in 0..n {
                let dx = x as f32 + 0.5 - c;
                let dy = y as f32 + 0.5 - c + lift;
                if dx * dx + dy * dy <= r * r {
                    px[y * n + x] = 1;
                    let eye = |ex: f32| {
                        let (ddx, ddy) = (dx - ex, dy + r * 0.25);
                        ddx * ddx + ddy * ddy <= (r * 0.12).powi(2)
                    };
                    if eye(-r * 0.35) || eye(r * 0.35) {
                        px[y * n + x] = 2;
                    }
                    let smile = (dx * dx + (dy - r * 0.05).powi(2)).sqrt();
                    if dy > r * 0.15 && (smile - r * 0.5).abs() < 1.6 {
                        px[y * n + x] = 2;
                    }
                }
            }
        }
        out.push(px);
    }
    gif(EMOTE_PX, &palette, &out, 12)
}

/// A rounded square of `rgb` with a white mark.
fn badge_gif(rgb: [u8; 3]) -> Vec<u8> {
    let palette = [[0, 0, 0], rgb, [0xFF, 0xFF, 0xFF]];
    let n = usize::from(BADGE_PX);
    let mut px = vec![0u8; n * n];
    for y in 2..n - 2 {
        for x in 2..n - 2 {
            let corner = |a: usize| a < 6 || a >= n - 6;
            if corner(x) && corner(y) {
                let cx = if x < n / 2 { 6.0 } else { (n - 6) as f32 };
                let cy = if y < n / 2 { 6.0 } else { (n - 6) as f32 };
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy > 16.0 {
                    continue;
                }
            }
            let mark = (n / 3..2 * n / 3).contains(&x) && (n / 3..2 * n / 3).contains(&y);
            px[y * n + x] = if mark { 2 } else { 1 };
        }
    }
    gif(BADGE_PX, &palette, &[px], 0)
}

/// GIF89a, square `size`, palette index 0 transparent. LZW codes are kept
/// at 9 bits by a clear code every 250 pixels: valid and simple, not small.
fn gif(size: u16, palette: &[[u8; 3]], frames: &[Vec<u8>], delay_cs: u16) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend(size.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend([0xF7, 0, 0]); // global table, 256 colours
    for i in 0..256 {
        out.extend(palette.get(i).copied().unwrap_or([0, 0, 0]));
    }
    if frames.len() > 1 {
        out.extend([0x21, 0xFF, 0x0B]);
        out.extend(b"NETSCAPE2.0");
        out.extend([3, 1, 0, 0, 0]);
    }
    for f in frames {
        out.extend([0x21, 0xF9, 4, 0x09]); // restore to background, transparent
        out.extend(delay_cs.to_le_bytes());
        out.extend([0, 0]);
        out.push(0x2C);
        out.extend([0, 0, 0, 0]);
        out.extend(size.to_le_bytes());
        out.extend(size.to_le_bytes());
        out.push(0);
        out.push(8);
        for chunk in lzw(f).chunks(255) {
            out.push(chunk.len() as u8);
            out.extend(chunk);
        }
        out.push(0);
    }
    out.push(0x3B);
    out
}

fn lzw(pixels: &[u8]) -> Vec<u8> {
    const CLEAR: u32 = 256;
    const END: u32 = 257;
    let mut out = Vec::with_capacity(pixels.len() * 9 / 8 + 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut put = |code: u32, out: &mut Vec<u8>| {
        acc |= code << bits;
        bits += 9;
        while bits >= 8 {
            out.push((acc & 0xFF) as u8);
            acc >>= 8;
            bits -= 8;
        }
    };
    put(CLEAR, &mut out);
    for (i, p) in pixels.iter().enumerate() {
        if i > 0 && i.is_multiple_of(250) {
            put(CLEAR, &mut out);
        }
        put(u32::from(*p), &mut out);
    }
    put(END, &mut out);
    if bits > 0 {
        out.push((acc & 0xFF) as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_is_deterministic_and_has_the_rate() {
        let mut a = Generator::new(100, 7);
        let mut b = Generator::new(100, 7);
        a.pending.clear();
        b.pending.clear();
        for _ in 0..20 {
            a.fill_tick();
            b.fill_tick();
        }
        let shape = |g: &Generator| -> Vec<String> {
            g.pending
                .iter()
                .map(|e| match e {
                    ChatEvent::Message(m) => format!("{} {} {}", m.id, m.user.login, m.text),
                    other => format!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(shape(&a), shape(&b));
        // 20 ticks of 50 ms = 1 s at 100/s.
        assert_eq!(a.pending.len(), 100);
    }

    #[test]
    fn emote_ranges_point_at_codes() {
        let mut g = Generator::new(10, 3);
        for _ in 0..500 {
            let m = g.message();
            for r in &m.emotes {
                let code: String = m.text.chars().skip(r.start).take(r.end - r.start).collect();
                assert!(EMOTES.iter().any(|(id, c, _)| *id == r.id && *c == code));
            }
        }
    }

    #[test]
    fn gif_has_header_frames_and_trailer() {
        let g = emote_gif([1, 2, 3], 4);
        assert!(g.starts_with(b"GIF89a"));
        assert_eq!(g.last(), Some(&0x3B));
        assert_eq!(pa_imagecache::sniff_image(&g), Some("gif"));
        assert_eq!(g.windows(3).filter(|w| w == &[0x21, 0xF9, 4]).count(), 4);
    }

    #[test]
    fn demo_assets_resolve_to_local_files() {
        let dir = tempfile::tempdir().unwrap();
        let assets = demo_assets(dir.path()).unwrap();
        let prefs = crate::ImagePrefs::default();
        let e = assets.emote("demo_2");
        assert_eq!(e.code, "lunaHype");
        assert!(Path::new(&e.url(&prefs)).exists());
        let b = assets.badge("moderator", "1").unwrap();
        assert!(Path::new(b.url(prefs.scale)).exists());
        let still = crate::ImagePrefs {
            animate: false,
            ..prefs
        };
        assert!(e.url(&still).ends_with("demo_2_static.gif"));
    }
}
