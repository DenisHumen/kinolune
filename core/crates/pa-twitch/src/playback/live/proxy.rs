// SPDX-License-Identifier: MPL-2.0
//! Loopback live proxy for the native player (TWI-006), full builds only.
//!
//! The player (libmpv through media_kit) opens
//! `http://127.0.0.1:<port>/<token>/live.m3u8`. For every request the proxy
//! fetches the Twitch media playlist, renders it with
//! [`MediaPlaylist::rewrite`] (prefetch segments become ordinary segments
//! in low-latency mode) and maps each segment to `s/<sequence>.<ext>`.
//! Segment requests are streamed from Twitch to the player chunk by chunk,
//! byte for byte: the proxy never drops, replaces, delays or reorders a
//! segment, ad segments included (06 §2.4).
//!
//! What the proxy learns on the way is the live telemetry of
//! [`LiveProxy::report`]: the program date-time of the media handed to the
//! player (hence the latency), the live edge, the ad breaks announced by
//! `#EXT-X-DATERANGE` and whether the playhead is inside one. The same
//! report is served as JSON at `/<token>/stats.json?ahead=<s>&rate=<x>`;
//! `/<token>/reset` forgets the playhead before a "go to live" reload.
//!
//! Every URL carries a random 128-bit token; requests without it get `403`.

use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use super::latency::{CatchUpPolicy, latency_seconds};
use super::playlist::{AdBreak, LiveMode, MediaPlaylist, resolve_uri, secs_to_ms};
use crate::playback::PlaybackError;
use crate::transport::BoxFuture;

/// Idle keep-alive connections are closed after this long.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Segments and deliveries remembered for telemetry.
const HISTORY: usize = 256;
/// Deliveries listed in a report.
const REPORTED_DELIVERIES: usize = 64;

/// A segment body being downloaded.
pub trait SegmentBody: Send {
    /// `Content-Length`, when the server sent one (Twitch prefetch segments
    /// are chunked).
    fn content_length(&self) -> Option<u64>;
    /// The next chunk, `None` at the end.
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, PlaybackError>>;
}

/// Where the proxy gets playlists and segments (reqwest in the app,
/// anything in tests).
pub trait LiveUpstream: Send + Sync {
    /// The media playlist text.
    fn playlist<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, PlaybackError>>;
    /// Starts downloading a segment.
    fn segment<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn SegmentBody>, PlaybackError>>;
}

/// Ad break under the playhead. The ad is played in full; this only lets
/// the UI show "Ad · 0:15", block seeking and not count it as progress.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdStatus {
    /// `DATERANGE` id, when the break was announced by one.
    pub id: Option<String>,
    /// Seconds left in the break, when known.
    pub remaining: Option<f64>,
    /// Position in the pod (0-based), when announced.
    pub pod_position: Option<u32>,
    /// Ads in the pod, when announced.
    pub pod_length: Option<u32>,
}

/// Low-latency features of the last playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LowLatencyInfo {
    /// `#EXT-X-TWITCH-PREFETCH` segments in the last playlist.
    pub prefetch: usize,
    /// Standard LL-HLS `#EXT-X-PART` tags (counted, not played: FFmpeg's
    /// HLS demuxer reads whole segments).
    pub parts: usize,
    /// Standard LL-HLS `#EXT-X-PRELOAD-HINT` present.
    pub preload_hint: bool,
    /// `#EXT-X-TARGETDURATION`.
    pub target_duration: f64,
    /// Playlists served to the player.
    pub playlists: u64,
}

/// Counters of what was handed to the player.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LiveTotals {
    /// Segments delivered completely.
    pub segments: u64,
    /// Of them, ad segments.
    pub ad_segments: u64,
    /// Of them, requested while they were prefetch segments.
    pub prefetch_segments: u64,
    /// Bytes delivered.
    pub bytes: u64,
    /// Of them, bytes of ad segments.
    pub ad_bytes: u64,
}

/// One segment handed to the player.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeliveredSegment {
    /// Media sequence number.
    pub sequence: u64,
    /// Bytes streamed.
    pub bytes: u64,
    /// The whole segment was streamed.
    pub complete: bool,
    /// An ad segment (as far as the playlists told so far).
    pub ad: bool,
    /// Requested while it was a prefetch segment.
    pub prefetch: bool,
}

/// Live telemetry for the OSD stats and the catch-up logic.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveReport {
    /// `low_latency` or `stable`.
    pub mode: &'static str,
    /// Wall clock of the report, Unix ms.
    pub now_ms: i64,
    /// `now − PDT(playhead)`, seconds.
    pub latency: Option<f64>,
    /// Latency the catch-up aims for, seconds.
    pub target: f64,
    /// Program date-time under the playhead, Unix ms.
    pub playhead_ms: Option<i64>,
    /// Program date-time of the end of the media handed to the player.
    pub delivered_end_ms: Option<i64>,
    /// Live edge: end of the newest complete segment, Unix ms.
    pub edge_ms: Option<i64>,
    /// Playback rate the player should use now (1.0 during ads).
    pub rate: f64,
    /// Far behind: jump to live instead of speeding up.
    pub should_jump: bool,
    /// The playhead is in an ad break.
    pub ad: Option<AdStatus>,
    /// Seeking (and "go to live") is allowed: not during an ad.
    pub seek_allowed: bool,
    /// Low-latency features of the stream.
    pub low_latency: LowLatencyInfo,
    /// Counters.
    pub totals: LiveTotals,
    /// The latest deliveries, oldest first.
    pub delivered: Vec<DeliveredSegment>,
}

#[derive(Debug, Clone)]
struct Entry {
    url: String,
    start: Option<i64>,
    duration: f64,
    ad: bool,
    prefetch: bool,
}

#[derive(Debug, Clone)]
struct Delivery {
    bytes: u64,
    complete: bool,
    prefetch: bool,
}

#[derive(Debug)]
struct State {
    upstream_url: String,
    mode: LiveMode,
    entries: BTreeMap<u64, Entry>,
    ad_breaks: Vec<AdBreak>,
    edge: Option<i64>,
    low_latency: LowLatencyInfo,
    deliveries: BTreeMap<u64, Delivery>,
    totals: LiveTotals,
}

impl State {
    fn new(upstream_url: String, mode: LiveMode) -> Self {
        Self {
            upstream_url,
            mode,
            entries: BTreeMap::new(),
            ad_breaks: Vec::new(),
            edge: None,
            low_latency: LowLatencyInfo::default(),
            deliveries: BTreeMap::new(),
            totals: LiveTotals::default(),
        }
    }

    fn register(&mut self, base: &str, playlist: &MediaPlaylist) {
        for s in &playlist.segments {
            let entry = Entry {
                url: resolve_uri(base, &s.uri),
                start: s.program_date_time,
                duration: s.duration,
                ad: s.ad,
                prefetch: s.prefetch,
            };
            self.entries.insert(s.sequence, entry);
        }
        for b in playlist.ad_breaks() {
            match self.ad_breaks.iter_mut().find(|known| known.id == b.id) {
                Some(known) => *known = b,
                None => self.ad_breaks.push(b),
            }
        }
        // An ad range announced after its segments were listed.
        for e in self.entries.values_mut() {
            if let Some(start) = e.start {
                let mid = start + secs_to_ms(e.duration) / 2;
                if self.ad_breaks.iter().any(|b| b.contains(mid)) {
                    e.ad = true;
                }
            }
        }
        if let Some(edge) = playlist.edge() {
            self.edge = Some(self.edge.map_or(edge, |e| e.max(edge)));
        }
        self.low_latency = LowLatencyInfo {
            prefetch: playlist.prefetch().count(),
            parts: playlist.parts,
            preload_hint: playlist.preload_hint,
            target_duration: playlist.target_duration,
            playlists: self.low_latency.playlists + 1,
        };
        trim(&mut self.entries);
        let keep = self.ad_breaks.len().saturating_sub(16);
        self.ad_breaks.drain(..keep);
    }

    /// Program date-time of the end of the media handed to the player:
    /// the end of the newest complete delivery, or into the segment being
    /// streamed after it (its bytes over the byte rate of the complete one).
    fn delivered_end(&self) -> Option<i64> {
        let (&seq, done) = self.deliveries.iter().rev().find(|(_, d)| d.complete)?;
        let entry = self.entries.get(&seq)?;
        let end = entry.start? + secs_to_ms(entry.duration);
        let partial = self
            .deliveries
            .range(seq + 1..)
            .next()
            .filter(|(_, d)| !d.complete && d.bytes > 0)
            .and_then(|(next, d)| {
                let next = self.entries.get(next)?;
                let byte_rate = done.bytes as f64 / entry.duration.max(0.1);
                (byte_rate > 0.0).then(|| (d.bytes as f64 / byte_rate).min(next.duration))
            })
            .unwrap_or(0.0);
        Some(end + secs_to_ms(partial))
    }

    fn ad_at(&self, at: i64) -> Option<AdStatus> {
        if let Some(b) = self.ad_breaks.iter().find(|b| b.contains(at)) {
            return Some(AdStatus {
                id: Some(b.id.clone()),
                remaining: b.end().map(|end| (end - at) as f64 / 1000.0),
                pod_position: b.pod_position,
                pod_length: b.pod_length,
            });
        }
        // Ad segments without a range (titled as ads): until the next
        // content segment.
        let (seq, _) = self.entries.iter().find(|(_, e)| {
            e.start
                .is_some_and(|s| at >= s && at < s + secs_to_ms(e.duration))
        })?;
        let mut end = None;
        for e in self.entries.range(seq..).map(|(_, e)| e) {
            if !e.ad {
                break;
            }
            end = e.start.map(|s| s + secs_to_ms(e.duration));
        }
        end.map(|end| AdStatus {
            id: None,
            remaining: Some((end - at) as f64 / 1000.0),
            pod_position: None,
            pod_length: None,
        })
    }

    fn report(&self, now_ms: i64, cache_ahead: f64, current_rate: f64) -> LiveReport {
        let policy = CatchUpPolicy::for_mode(self.mode);
        let delivered_end = self.delivered_end();
        let latency = latency_seconds(now_ms, delivered_end, cache_ahead);
        let playhead = delivered_end.map(|end| end - secs_to_ms(cache_ahead.max(0.0)));
        let ad = playhead.and_then(|at| self.ad_at(at));
        let delivered = self
            .deliveries
            .iter()
            .rev()
            .take(REPORTED_DELIVERIES)
            .map(|(&sequence, d)| DeliveredSegment {
                sequence,
                bytes: d.bytes,
                complete: d.complete,
                ad: self.entries.get(&sequence).is_some_and(|e| e.ad),
                prefetch: d.prefetch,
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        LiveReport {
            mode: match self.mode {
                LiveMode::LowLatency => "low_latency",
                LiveMode::Stable => "stable",
            },
            now_ms,
            latency,
            target: policy.target,
            playhead_ms: playhead,
            delivered_end_ms: delivered_end,
            edge_ms: self.edge,
            rate: policy.rate(latency, ad.is_some(), current_rate),
            should_jump: ad.is_none() && policy.should_jump(latency),
            seek_allowed: ad.is_none(),
            ad,
            low_latency: self.low_latency.clone(),
            totals: self.totals.clone(),
            delivered,
        }
    }
}

fn trim<V>(map: &mut BTreeMap<u64, V>) {
    while map.len() > HISTORY {
        map.pop_first();
    }
}

struct Ctx {
    token: String,
    upstream: Arc<dyn LiveUpstream>,
    state: Mutex<State>,
}

impl Ctx {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The running proxy. Dropping it stops the server.
pub struct LiveProxy {
    addr: SocketAddr,
    ctx: Arc<Ctx>,
    stop: Arc<Notify>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for LiveProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveProxy")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl LiveProxy {
    /// Starts the proxy on `127.0.0.1:<random port>` for the media playlist
    /// `media_playlist_url` (a [`super::super::Variant::url`]).
    pub fn start(
        upstream: Arc<dyn LiveUpstream>,
        media_playlist_url: &str,
        mode: LiveMode,
    ) -> io::Result<Self> {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let ctx = Arc::new(Ctx {
            token: random_token()?,
            upstream,
            state: Mutex::new(State::new(media_playlist_url.to_owned(), mode)),
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let stop = Arc::new(Notify::new());
        let (thread_ctx, thread_stop) = (ctx.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("pa-twitch-live".into())
            .spawn(move || {
                runtime.block_on(async move {
                    match TcpListener::from_std(listener) {
                        Ok(listener) => {
                            tokio::spawn(accept(listener, thread_ctx));
                            thread_stop.notified().await;
                        }
                        Err(e) => eprintln!("pa-twitch live proxy: {e}"),
                    }
                });
                // Dropping the runtime cancels the open connections.
            })?;
        Ok(Self {
            addr,
            ctx,
            stop,
            thread: Some(thread),
        })
    }

    /// Address the proxy listens on.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// URL for the player.
    pub fn playlist_url(&self) -> String {
        format!("http://{}/{}/live.m3u8", self.addr, self.ctx.token)
    }

    /// URL of the JSON [`LiveReport`] (`?ahead=<s>&rate=<x>`).
    pub fn stats_url(&self) -> String {
        format!("http://{}/{}/stats.json", self.addr, self.ctx.token)
    }

    /// URL that does [`Self::reset_playhead`] (for the Dart player).
    pub fn reset_url(&self) -> String {
        format!("http://{}/{}/reset", self.addr, self.ctx.token)
    }

    /// Current mode.
    pub fn mode(&self) -> LiveMode {
        self.ctx.state().mode
    }

    /// Switches between low latency and stable. The player should reload
    /// the playlist URL (its start position depends on the mode).
    pub fn set_mode(&self, mode: LiveMode) {
        let mut st = self.ctx.state();
        st.mode = mode;
        st.deliveries.clear();
    }

    /// Switches to another media playlist (quality change); the player
    /// reloads the playlist URL.
    pub fn set_media_playlist(&self, url: &str) {
        let mut st = self.ctx.state();
        let mode = st.mode;
        *st = State::new(url.to_owned(), mode);
    }

    /// Forgets what the player received ("go to live": the player reloads
    /// the playlist URL and starts at the edge again). Ad breaks and
    /// counters are kept.
    pub fn reset_playhead(&self) {
        self.ctx.state().deliveries.clear();
    }

    /// Telemetry for a player holding `cache_ahead` seconds ahead of its
    /// playhead and playing at `current_rate`.
    pub fn report(&self, cache_ahead: f64, current_rate: f64) -> LiveReport {
        self.ctx.state().report(now_ms(), cache_ahead, current_rate)
    }
}

impl Drop for LiveProxy {
    fn drop(&mut self) {
        self.stop.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

async fn accept(listener: TcpListener, ctx: Arc<Ctx>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let _ = stream.set_nodelay(true);
                tokio::spawn(serve(stream, ctx.clone()));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    close: bool,
}

async fn read_request<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad request line",
        ));
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut request = Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        close: version == "HTTP/1.0",
    };
    for _ in 0..100 {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            return Ok(Some(request));
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("connection")
        {
            let value = value.trim();
            request.close = value.eq_ignore_ascii_case("close");
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "too many headers",
    ))
}

async fn serve(stream: TcpStream, ctx: Arc<Ctx>) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    loop {
        let request = match tokio::time::timeout(IDLE_TIMEOUT, read_request(&mut reader)).await {
            Ok(Ok(Some(r))) => r,
            _ => return,
        };
        let keep = match route(&ctx, &request, &mut writer).await {
            Ok(keep) => keep && !request.close,
            Err(_) => false,
        };
        if !keep {
            let _ = writer.shutdown().await;
            return;
        }
    }
}

/// Handles one request; `Ok(false)` closes the connection.
async fn route<W: AsyncWrite + Unpin>(ctx: &Ctx, req: &Request, w: &mut W) -> io::Result<bool> {
    let Some(rest) = req
        .path
        .strip_prefix('/')
        .and_then(|p| p.strip_prefix(ctx.token.as_str()))
        .and_then(|p| p.strip_prefix('/'))
    else {
        return simple(w, 403, "text/plain", b"forbidden").await;
    };
    if req.method != "GET" {
        return simple(w, 405, "text/plain", b"method not allowed").await;
    }
    if rest == "live.m3u8" {
        return playlist(ctx, w).await;
    }
    if rest == "stats.json" {
        let param = |name: &str| {
            req.query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| *k == name)
                .and_then(|(_, v)| v.parse::<f64>().ok())
        };
        let report = ctx.state().report(
            now_ms(),
            param("ahead").unwrap_or(0.0),
            param("rate").unwrap_or(1.0),
        );
        let body = serde_json::to_vec(&report).map_err(io::Error::other)?;
        return simple(w, 200, "application/json", &body).await;
    }
    if rest == "reset" {
        // "Go to live": the player reloads and starts at the edge again.
        ctx.state().deliveries.clear();
        return simple(w, 200, "text/plain", b"ok").await;
    }
    if let Some(name) = rest.strip_prefix("s/") {
        let seq = name.split('.').next().and_then(|n| n.parse::<u64>().ok());
        if let Some(seq) = seq {
            return segment(ctx, seq, w).await;
        }
    }
    simple(w, 404, "text/plain", b"not found").await
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        502 => "Bad Gateway",
        _ => "Error",
    }
}

async fn simple<W: AsyncWrite + Unpin>(
    w: &mut W,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<bool> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
        status_text(status),
        body.len()
    );
    w.write_all(head.as_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(true)
}

fn upstream_status(e: &PlaybackError) -> u16 {
    match e {
        PlaybackError::Unavailable | PlaybackError::NotFound => 404,
        PlaybackError::Restricted { .. } => 403,
        _ => 502,
    }
}

async fn playlist<W: AsyncWrite + Unpin>(ctx: &Ctx, w: &mut W) -> io::Result<bool> {
    let url = ctx.state().upstream_url.clone();
    let text = match ctx.upstream.playlist(&url).await {
        Ok(t) => t,
        Err(e) => return simple(w, upstream_status(&e), "text/plain", e.code().as_bytes()).await,
    };
    let parsed = match MediaPlaylist::parse(&text) {
        Ok(p) => p,
        Err(e) => return simple(w, 502, "text/plain", e.code().as_bytes()).await,
    };
    let rendered = {
        let mut st = ctx.state();
        // Quality switched while fetching: let the player retry.
        (st.upstream_url == url).then(|| {
            st.register(&url, &parsed);
            parsed.rewrite(
                &text,
                st.mode,
                |s| format!("s/{}{}", s.sequence, extension(&s.uri)),
                |uri| resolve_uri(&url, uri),
            )
        })
    };
    match rendered {
        Some(r) => simple(w, 200, "application/vnd.apple.mpegurl", r.as_bytes()).await,
        None => simple(w, 404, "text/plain", b"switched").await,
    }
}

/// `.ts`, `.mp4`, `.m4s`… of a segment URI (the player probes by it).
fn extension(uri: &str) -> &str {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if name.len() - i <= 5 => &name[i..],
        _ => ".ts",
    }
}

async fn segment<W: AsyncWrite + Unpin>(ctx: &Ctx, seq: u64, w: &mut W) -> io::Result<bool> {
    let Some(entry) = ctx.state().entries.get(&seq).cloned() else {
        return simple(w, 404, "text/plain", b"unknown segment").await;
    };
    let mut body = match ctx.upstream.segment(&entry.url).await {
        Ok(b) => b,
        Err(e) => return simple(w, upstream_status(&e), "text/plain", e.code().as_bytes()).await,
    };
    {
        let mut st = ctx.state();
        st.deliveries.insert(
            seq,
            Delivery {
                bytes: 0,
                complete: false,
                prefetch: entry.prefetch,
            },
        );
        trim(&mut st.deliveries);
    }
    let length = body.content_length();
    let head = match length {
        Some(n) => format!(
            "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {n}\r\nConnection: keep-alive\r\n\r\n"
        ),
        None => "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n".to_owned(),
    };
    w.write_all(head.as_bytes()).await?;
    w.flush().await?;
    let mut sent = 0u64;
    loop {
        let chunk = match body.next_chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            // Upstream broke mid-segment: close, the player retries.
            Err(_) => return Ok(false),
        };
        if chunk.is_empty() {
            continue;
        }
        if length.is_none() {
            w.write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await?;
            w.write_all(&chunk).await?;
            w.write_all(b"\r\n").await?;
        } else {
            w.write_all(&chunk).await?;
        }
        w.flush().await?;
        sent += chunk.len() as u64;
        if let Some(d) = ctx.state().deliveries.get_mut(&seq) {
            d.bytes = sent;
        }
    }
    if length.is_none() {
        w.write_all(b"0\r\n\r\n").await?;
        w.flush().await?;
    }
    let mut st = ctx.state();
    let ad = st.entries.get(&seq).is_some_and(|e| e.ad);
    if let Some(d) = st.deliveries.get_mut(&seq) {
        d.complete = true;
        d.bytes = sent;
    }
    st.totals.segments += 1;
    st.totals.bytes += sent;
    if ad {
        st.totals.ad_segments += 1;
        st.totals.ad_bytes += sent;
    }
    if entry.prefetch {
        st.totals.prefetch_segments += 1;
    }
    Ok(length.is_none_or(|n| n == sent))
}

/// [`LiveUpstream`] over reqwest (rustls, ring), as in the rest of the crate.
#[cfg(feature = "reqwest")]
#[derive(Debug, Clone)]
pub struct ReqwestUpstream {
    client: reqwest::Client,
}

#[cfg(feature = "reqwest")]
impl ReqwestUpstream {
    /// A client without a total timeout (prefetch segments stream for
    /// seconds) but with connect and read timeouts.
    pub fn new() -> Result<Self, PlaybackError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| PlaybackError::Transport(e.to_string()))?;
        Ok(Self { client })
    }

    async fn get(&self, url: &str, accept: &str) -> Result<reqwest::Response, PlaybackError> {
        let response = self
            .client
            .get(url)
            .header("Accept", accept)
            .send()
            .await
            .map_err(|e| PlaybackError::Transport(e.without_url().to_string()))?;
        match response.status().as_u16() {
            200..=299 => Ok(response),
            403 => Err(PlaybackError::Restricted {
                code: "forbidden".into(),
            }),
            404 | 410 => Err(PlaybackError::Unavailable),
            status => Err(PlaybackError::Usher { status }),
        }
    }
}

#[cfg(feature = "reqwest")]
struct ReqwestBody(reqwest::Response);

#[cfg(feature = "reqwest")]
impl SegmentBody for ReqwestBody {
    fn content_length(&self) -> Option<u64> {
        self.0.content_length()
    }

    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, PlaybackError>> {
        Box::pin(async move {
            self.0
                .chunk()
                .await
                .map(|c| c.map(|b| b.to_vec()))
                .map_err(|e| PlaybackError::Transport(e.without_url().to_string()))
        })
    }
}

#[cfg(feature = "reqwest")]
impl LiveUpstream for ReqwestUpstream {
    fn playlist<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, PlaybackError>> {
        Box::pin(async move {
            self.get(url, "application/vnd.apple.mpegurl")
                .await?
                .text()
                .await
                .map_err(|e| PlaybackError::Transport(e.without_url().to_string()))
        })
    }

    fn segment<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn SegmentBody>, PlaybackError>> {
        Box::pin(async move {
            let response = self.get(url, "*/*").await?;
            Ok(Box::new(ReqwestBody(response)) as Box<dyn SegmentBody>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(extension("a/b/seg1.ts?x=1"), ".ts");
        assert_eq!(extension("https://h/v1/segment/abc.mp4"), ".mp4");
        assert_eq!(extension("https://h/v1/segment/CqYF7xyz"), ".ts");
    }

    fn state_with(text: &str) -> State {
        let mut st = State::new("http://o/live.m3u8".into(), LiveMode::LowLatency);
        st.register("http://o/live.m3u8", &MediaPlaylist::parse(text).unwrap());
        st
    }

    const PLAYLIST: &str = "#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:1
#EXT-X-DATERANGE:ID=\"stitched-ad-7\",CLASS=\"twitch-stitched-ad\",START-DATE=\"1970-01-01T00:00:02.000Z\",DURATION=4.0
#EXT-X-PROGRAM-DATE-TIME:1970-01-01T00:00:00.000Z
#EXTINF:2.000,live
a.ts
#EXTINF:2.000,Amazon|x
b.ts
#EXTINF:2.000,Amazon|x
c.ts
#EXTINF:2.000,live
d.ts
#EXT-X-TWITCH-PREFETCH:e.ts
";

    #[test]
    fn delivered_end_counts_the_partial_segment() {
        let mut st = state_with(PLAYLIST);
        assert_eq!(st.entries[&5].url, "http://o/e.ts");
        assert!(st.entries[&5].prefetch);
        st.deliveries.insert(
            4,
            Delivery {
                bytes: 1000,
                complete: true,
                prefetch: false,
            },
        );
        assert_eq!(st.delivered_end(), Some(8000));
        st.deliveries.insert(
            5,
            Delivery {
                bytes: 250,
                complete: false,
                prefetch: true,
            },
        );
        assert_eq!(st.delivered_end(), Some(8500));
        let r = st.report(9000, 0.5, 1.0);
        assert_eq!(r.latency, Some(1.0));
        assert_eq!(r.playhead_ms, Some(8000));
        assert!(r.ad.is_none());
        assert!(r.seek_allowed);
        assert_eq!(r.edge_ms, Some(8000));
    }

    #[test]
    fn playhead_in_an_ad_break() {
        let mut st = state_with(PLAYLIST);
        st.deliveries.insert(
            4,
            Delivery {
                bytes: 1000,
                complete: true,
                prefetch: false,
            },
        );
        // Playhead at 3 s: 5 s of cache ahead of it.
        let r = st.report(20_000, 5.0, 1.1);
        assert_eq!(r.playhead_ms, Some(3000));
        let ad = r.ad.unwrap();
        assert_eq!(ad.id.as_deref(), Some("stitched-ad-7"));
        assert_eq!(ad.remaining, Some(3.0));
        assert!(!r.seek_allowed);
        assert!(!r.should_jump);
        // Far behind, but ads play at normal speed.
        assert_eq!(r.rate, 1.0);
        assert!(r.delivered.iter().all(|d| !d.ad));
    }

    #[test]
    fn titled_ads_without_a_range() {
        let text = PLAYLIST.replace(
            "#EXT-X-DATERANGE:ID=\"stitched-ad-7\",CLASS=\"twitch-stitched-ad\",START-DATE=\"1970-01-01T00:00:02.000Z\",DURATION=4.0\n",
            "",
        );
        let st = state_with(&text);
        let ad = st.ad_at(2500).unwrap();
        assert_eq!(ad.id, None);
        assert_eq!(ad.remaining, Some(3.5));
        assert!(st.ad_at(6500).is_none());
    }
}
