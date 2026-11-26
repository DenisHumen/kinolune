// SPDX-License-Identifier: MPL-2.0
//! A made-up live Twitch on a loopback port, driven by a virtual clock
//! (`testing` feature), so a 6-hour broadcast is recorded in seconds.
//! Nothing here talks to Twitch; names, ids and tokens are made up, and
//! there is no Twitch client id.
//!
//! [`FakeLiveTwitch`] answers what a live recording asks for:
//!
//! - `POST /gql`: `PlaybackAccessToken` for the live channel;
//! - `GET /api/channel/hls/<login>.m3u8`: usher's live master playlist
//!   (`chunked` and `audio_only`), `404` while offline;
//! - `GET /live/<login>/<group>.m3u8`: the sliding media playlist at the
//!   clock's time: complete segments with `#EXT-X-PROGRAM-DATE-TIME`, then
//!   `#EXT-X-TWITCH-PREFETCH` ones; ad runs titled `Amazon|…`, announced
//!   with an `#EXT-X-DATERANGE` of class `twitch-stitched-ad` and framed by
//!   `#EXT-X-DISCONTINUITY`; `404` once the broadcast is over;
//! - `GET /live/<login>/seg/<sequence>.ts`: the segment;
//! - `GET /helix/streams?user_id=`: the broadcast while it is live;
//! - `GET /helix/users?id=`: the channel, with its avatar;
//! - during a network blip every request is dropped without an answer.
//!
//! [`FakeLiveChat`] is a chat on the same clock: one message every few
//! seconds, delivered with a small delay, lost during blips.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::source::{ChatRecord, LiveChat, LiveClock};
use crate::archive::testing::{FakeSegment, tiny_jpeg};
use pa_twitch::playback::live::format_date_time;

/// Virtual time: `sleep` advances it at once.
#[derive(Debug)]
pub struct VirtualClock {
    now: Mutex<i64>,
}

impl VirtualClock {
    pub fn new(start_ms: i64) -> Self {
        Self {
            now: Mutex::new(start_ms),
        }
    }

    pub fn advance(&self, ms: i64) {
        *self.now.lock().unwrap_or_else(|e| e.into_inner()) += ms;
    }
}

impl LiveClock for VirtualClock {
    fn now_ms(&self) -> i64 {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sleep(&self, d: Duration) {
        self.advance(i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    }
}

/// Shape of the fake broadcast.
#[derive(Debug, Clone)]
pub struct FakeLiveStream {
    pub login: String,
    pub user_id: String,
    pub display_name: String,
    pub stream_id: String,
    pub title: String,
    /// Program date-time of the first segment = `started_at`, Unix ms.
    pub start_ms: i64,
    pub segment_ms: u64,
    pub segments: usize,
    pub first_sequence: u64,
    /// Complete segments in the playlist.
    pub window: usize,
    pub prefetch: usize,
    /// Ad runs: (first segment index, count).
    pub ads: Vec<(usize, usize)>,
    /// Network blips: [from, to) Unix ms on the clock.
    pub blips: Vec<(i64, i64)>,
    /// Categories from a segment index on: (index, game id, game name).
    pub categories: Vec<(usize, String, String)>,
    /// Real segments to serve in a loop (an FFmpeg-made rendition); empty:
    /// made-up `FAKESEG` segments.
    pub media: Vec<FakeSegment>,
}

impl FakeLiveStream {
    /// `hours` of 2-second segments starting at `start_ms`.
    pub fn new(start_ms: i64, segments: usize) -> Self {
        Self {
            login: "madeupstreamer".to_owned(),
            user_id: "4242".to_owned(),
            display_name: "MadeUpStreamer".to_owned(),
            stream_id: "90001".to_owned(),
            title: "Speedrun practice: all bosses".to_owned(),
            start_ms,
            segment_ms: 2000,
            segments,
            first_sequence: 5000,
            window: 15,
            prefetch: 2,
            ads: Vec::new(),
            blips: Vec::new(),
            categories: vec![(0, "1001".to_owned(), "Elden Ring".to_owned())],
            media: Vec::new(),
        }
    }

    /// Program date-time where segment `i` starts.
    pub fn pdt(&self, i: usize) -> i64 {
        self.start_ms + i as i64 * self.segment_ms as i64
    }

    /// When the broadcast ends (the last segment is complete).
    pub fn end_ms(&self) -> i64 {
        self.pdt(self.segments)
    }

    fn is_ad(&self, i: usize) -> bool {
        self.ads.iter().any(|(a, n)| i >= *a && i < a + n)
    }

    pub fn in_blip(&self, at: i64) -> bool {
        self.blips.iter().any(|(a, b)| at >= *a && at < *b)
    }

    fn category(&self, i: usize) -> Option<&(usize, String, String)> {
        self.categories.iter().rev().find(|(from, _, _)| *from <= i)
    }

    fn segment_bytes(&self, i: usize) -> std::io::Result<Vec<u8>> {
        if self.media.is_empty() {
            let mut b = format!("FAKESEG {i} {}\n", self.segment_ms).into_bytes();
            b.resize(188, b'.');
            return Ok(b);
        }
        self.media[i % self.media.len()].bytes()
    }
}

struct State {
    stream: FakeLiveStream,
    clock: Arc<VirtualClock>,
    base: String,
    /// Segment requests answered, by index.
    served: HashMap<usize, usize>,
    dropped: usize,
    requests: usize,
}

/// The fake origin; stops when dropped.
pub struct FakeLiveTwitch {
    base: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeLiveTwitch {
    pub fn start(stream: FakeLiveStream, clock: Arc<VirtualClock>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(Mutex::new(State {
            stream,
            clock,
            base: base.clone(),
            served: HashMap::new(),
            dropped: 0,
            requests: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stop) = (Arc::clone(&state), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((conn, _)) => {
                            let state = Arc::clone(&state);
                            std::thread::spawn(move || {
                                let _ = serve(conn, &state);
                            });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(1)),
                    }
                }
            })
        };
        Ok(Self {
            base,
            state,
            stop,
            thread: Some(thread),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn gql_url(&self) -> String {
        format!("{}/gql", self.base)
    }

    pub fn helix_url(&self) -> String {
        format!("{}/helix", self.base)
    }

    /// Distinct segments served.
    pub fn served_segments(&self) -> usize {
        self.lock().served.len()
    }

    /// Requests dropped by blips.
    pub fn dropped(&self) -> usize {
        self.lock().dropped
    }

    pub fn requests(&self) -> usize {
        self.lock().requests
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for FakeLiveTwitch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn respond(conn: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    // One write: small writes and keep-alive meet Nagle and delayed ACKs
    // (40 ms per answer).
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    conn.write_all(&out)?;
    conn.flush()
}

fn query(path: &str, key: &str) -> Option<String> {
    path.split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(str::to_owned))
}

/// Serves requests on one keep-alive connection.
fn serve(conn: TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    conn.set_nonblocking(false)?;
    conn.set_nodelay(true)?;
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut conn = conn;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let (method, target) = (
            parts.next().unwrap_or("").to_owned(),
            parts.next().unwrap_or("").to_owned(),
        );
        let mut length = 0usize;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':')
                && k.trim().eq_ignore_ascii_case("content-length")
            {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        let answer = {
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.requests += 1;
            if s.stream.in_blip(s.clock.now_ms()) {
                s.dropped += 1;
                None
            } else {
                Some(answer(&mut s, &method, &target, &body))
            }
        };
        let Some((status, kind, bytes)) = answer else {
            // The network is gone: no answer at all.
            let _ = conn.shutdown(Shutdown::Both);
            return Ok(());
        };
        respond(&mut conn, status, kind, &bytes)?;
    }
}

fn answer(s: &mut State, method: &str, target: &str, body: &[u8]) -> (u16, &'static str, Vec<u8>) {
    let path = target.split('?').next().unwrap_or("").to_owned();
    let now = s.clock.now_ms();
    let st = s.stream.clone();
    let live = now >= st.start_ms && now < st.end_ms() + 1000;
    let json_ok = |v: Value| (200, "application/json", v.to_string().into_bytes());
    if method == "POST" && path == "/gql" {
        let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let login = req["variables"]["login"].as_str().unwrap_or("");
        let token = (login == st.login).then(|| {
            json!({"value": json!({"channel": login, "made_up": true}).to_string(),
                   "signature": "made-up-signature"})
        });
        return json_ok(json!({"data": {"streamPlaybackAccessToken": token}}));
    }
    if let Some(rest) = path.strip_prefix("/api/channel/hls/") {
        if rest.trim_end_matches(".m3u8") != st.login || !live {
            return (404, "application/json", b"[]".to_vec());
        }
        let base = &s.base;
        let login = &st.login;
        let master = format!(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"160p10 (source)\",AUTOSELECT=YES,DEFAULT=YES\n\
             #EXT-X-STREAM-INF:BANDWIDTH=120000,RESOLUTION=160x90,CODECS=\"avc1.4D400C,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=10.000\n\
             {base}/live/{login}/chunked.m3u8\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"audio_only\",NAME=\"audio_only\",AUTOSELECT=NO,DEFAULT=NO\n\
             #EXT-X-STREAM-INF:BANDWIDTH=40000,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\n\
             {base}/live/{login}/audio_only.m3u8\n"
        );
        return (200, "application/vnd.apple.mpegurl", master.into_bytes());
    }
    if let Some(rest) = path.strip_prefix(&format!("/live/{}/", st.login)) {
        if let Some(seq) = rest
            .strip_prefix("seg/")
            .and_then(|f| f.strip_suffix(".ts"))
            .and_then(|n| n.parse::<u64>().ok())
        {
            let Some(i) = seq
                .checked_sub(st.first_sequence)
                .map(|i| i as usize)
                .filter(|i| *i < st.segments && st.pdt(*i) < now)
            else {
                return (404, "text/plain", Vec::new());
            };
            *s.served.entry(i).or_default() += 1;
            return match st.segment_bytes(i) {
                Ok(b) => (200, "video/mp2t", b),
                Err(_) => (500, "text/plain", Vec::new()),
            };
        }
        if !live {
            return (404, "text/plain", Vec::new());
        }
        return (
            200,
            "application/vnd.apple.mpegurl",
            playlist(&st, now).into_bytes(),
        );
    }
    if path == "/helix/streams" {
        let user = query(target, "user_id").unwrap_or_default();
        let current = ((now - st.start_ms).max(0) as u64 / st.segment_ms) as usize;
        let data: Vec<Value> = (user == st.user_id && live)
            .then(|| {
                let (_, game_id, game_name) = st
                    .category(current.min(st.segments.saturating_sub(1)))
                    .cloned()
                    .unwrap_or_default();
                json!({
                    "id": st.stream_id, "user_id": st.user_id, "user_login": st.login,
                    "user_name": st.display_name, "game_id": game_id, "game_name": game_name,
                    "type": "live", "title": st.title, "viewer_count": 12,
                    "started_at": format_date_time(st.start_ms), "language": "en",
                    "thumbnail_url": format!("{}/img/live-{{width}}x{{height}}.jpg", s.base),
                    "tags": [], "is_mature": false,
                })
            })
            .into_iter()
            .collect();
        return json_ok(json!({"data": data, "pagination": {}}));
    }
    if path == "/helix/users" {
        let user = query(target, "id").unwrap_or_default();
        let data: Vec<Value> = (user == st.user_id)
            .then(|| {
                json!({
                    "id": st.user_id, "login": st.login, "display_name": st.display_name,
                    "type": "", "broadcaster_type": "", "description": "",
                    "profile_image_url": format!("{}/img/avatar.jpg", s.base),
                    "offline_image_url": "", "created_at": "2020-01-01T00:00:00Z",
                })
            })
            .into_iter()
            .collect();
        return json_ok(json!({"data": data}));
    }
    if path.starts_with("/img/") {
        return (200, "image/jpeg", tiny_jpeg());
    }
    (404, "text/plain", Vec::new())
}

/// The media playlist at `now`.
fn playlist(st: &FakeLiveStream, now: i64) -> String {
    // Segment i is complete once its end is past.
    let complete = (0..st.segments)
        .take_while(|&i| st.pdt(i + 1) <= now)
        .count();
    let first = complete.saturating_sub(st.window);
    let secs = st.segment_ms as f64 / 1000.0;
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{}\n\
         #EXT-X-TWITCH-ELAPSED-SECS:{:.3}\n",
        secs.ceil() as u64,
        st.first_sequence + first as u64,
        (now - st.start_ms) as f64 / 1000.0
    );
    for (a, n) in &st.ads {
        if a + n > first && *a < complete + st.prefetch {
            out.push_str(&format!(
                "#EXT-X-DATERANGE:ID=\"stitched-ad-{a}\",CLASS=\"twitch-stitched-ad\",\
                 START-DATE=\"{}\",DURATION={:.3},X-TV-TWITCH-AD-POD-LENGTH=\"1\",\
                 X-TV-TWITCH-AD-POD-POSITION=\"0\"\n",
                format_date_time(st.pdt(*a)),
                *n as f64 * secs
            ));
        }
    }
    for i in first..complete {
        let ad = st.is_ad(i);
        if i > first && ad != st.is_ad(i - 1) {
            out.push_str("#EXT-X-DISCONTINUITY\n");
        }
        out.push_str(&format!(
            "#EXT-X-PROGRAM-DATE-TIME:{}\n#EXTINF:{secs:.3},{}\nseg/{}.ts\n",
            format_date_time(st.pdt(i)),
            if ad { "Amazon|madeup-ad" } else { "live" },
            st.first_sequence + i as u64
        ));
    }
    for i in complete..(complete + st.prefetch).min(st.segments) {
        out.push_str(&format!(
            "#EXT-X-TWITCH-PREFETCH:seg/{}.ts\n",
            st.first_sequence + i as u64
        ));
    }
    out
}

/// A chat on the virtual clock: message `n` is sent at
/// `start + n × every_ms`, arrives `delay_ms` later, and is lost if it
/// arrives during a blip.
pub struct FakeLiveChat {
    clock: Arc<VirtualClock>,
    stream: FakeLiveStream,
    every_ms: i64,
    delay_ms: i64,
    next: i64,
}

impl FakeLiveChat {
    pub fn new(clock: Arc<VirtualClock>, stream: FakeLiveStream, every_ms: i64) -> Self {
        Self {
            clock,
            stream,
            every_ms,
            delay_ms: 300,
            next: 0,
        }
    }

    /// When message `n` was sent.
    pub fn sent_at(start_ms: i64, every_ms: i64, n: i64) -> i64 {
        start_ms + n * every_ms + every_ms / 3
    }
}

impl LiveChat for FakeLiveChat {
    fn drain(&mut self) -> Vec<ChatRecord> {
        let now = self.clock.now_ms();
        let mut out = Vec::new();
        loop {
            let sent = Self::sent_at(self.stream.start_ms, self.every_ms, self.next);
            let arrives = sent + self.delay_ms;
            if arrives > now || sent >= self.stream.end_ms() {
                break;
            }
            let n = self.next;
            self.next += 1;
            if self.stream.in_blip(arrives) {
                continue;
            }
            out.push(ChatRecord {
                id: format!("msg-{n}"),
                sent_ms: sent,
                received_ms: arrives,
                user_id: format!("{}", 7000 + n % 50),
                login: format!("viewer{}", n % 50),
                display_name: format!("Viewer{}", n % 50),
                color: Some("#8A2BE2".to_owned()),
                text: format!("message {n} Kappa"),
                emotes: vec![(
                    "25".to_owned(),
                    format!("message {n} ").chars().count(),
                    format!("message {n} Kappa").chars().count(),
                )],
                bits: None,
                notice: None,
            });
        }
        out
    }
}
