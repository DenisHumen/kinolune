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
//! Live DVR (TWI-025): the proxy keeps the segments of the live playlist
//! after they leave Twitch's short window (up to [`DVR_WINDOW_SECS`]) and
//! lists them before Twitch's playlist, so the player can go back over the
//! whole window, not only over its cache. `/<token>/seek?by=<s>` (or
//! `?at=<ms>`) moves the playhead: until the player asks for a segment
//! there, the playlist ends one (low latency) or two (stable) segments
//! after the target, so FFmpeg's `live_start_index` lands on it. Seeking is
//! refused during an ad break, and a forward jump stops at an ad break the
//! player has not played. While the player does not read the playlist
//! (paused, or playing the VOD from the start) the proxy reads it itself so
//! the window keeps growing. `?first=1&t0=<ms>` starts a VOD of a stream
//! still on air at its first segment ("Смотреть с начала эфира").
//!
//! Every URL carries a random 128-bit token; requests without it get `403`.
//!
//! VODs go through the same proxy in stable mode (TWI-014 checks): our
//! LGPL FFmpeg is built without TLS (`tools/ffmpeg`), and usher's media
//! playlists and segments are HTTPS only. The proxy fetches them with
//! rustls; a finished playlist keeps every segment reachable for seeking.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use super::latency::{CatchUpPolicy, latency_seconds};
use super::playlist::{
    AdBreak, LiveMode, MediaPlaylist, format_date_time, resolve_uri, secs_to_ms,
};
use crate::playback::PlaybackError;
use crate::transport::BoxFuture;

/// Idle keep-alive connections are closed after this long.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Deliveries remembered for telemetry.
const HISTORY: usize = 1024;
/// Live DVR window (TWI-025): how far back the player can rewind. The
/// proxy keeps the segments of the live playlist after they leave Twitch's
/// window (Twitch lists only the last ~30 s).
pub const DVR_WINDOW_SECS: f64 = 30.0 * 60.0;
/// At most this many segments in the DVR window (30 min of Twitch's 2 s
/// segments is 900).
const MAX_ENTRIES: usize = 1024;
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
    /// The DVR window the player can rewind in (TWI-025).
    pub dvr: Option<DvrReport>,
    /// Low-latency features of the stream.
    pub low_latency: LowLatencyInfo,
    /// Counters.
    pub totals: LiveTotals,
    /// The latest deliveries, oldest first.
    pub delivered: Vec<DeliveredSegment>,
}

/// The live DVR window (TWI-025, ADR-NNNN): where the player can rewind to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DvrReport {
    /// Program date-time of the oldest segment the player can rewind to,
    /// Unix ms.
    pub window_start_ms: i64,
    /// Length of the window up to the live edge, seconds.
    pub window_seconds: f64,
    /// How far the playhead is behind the live edge, seconds (the "−N мин"
    /// label); the target of a seek the player has not started yet.
    pub behind_seconds: Option<f64>,
    /// The user rewound: no catch-up, no "jump to live" hint, and forward
    /// jumps never pass an ad break the player has not played.
    pub active: bool,
    /// A seek waits for the player to reload the playlist.
    pub pending: bool,
}

/// Where a seek took the player ([`LiveProxy::seek`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DvrSeek {
    /// Back at the live edge (the seek reached it, or "go to live").
    pub live: bool,
    /// The program date-time the player starts at, Unix ms, when known.
    pub at_ms: Option<i64>,
    /// The seek stopped at an ad break the player has not played yet: a
    /// forward jump never skips an ad.
    pub stopped_at_ad: bool,
}

/// What to seek to ([`LiveProxy::seek`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeekTarget {
    /// Seconds from the playhead (negative: back).
    By(f64),
    /// A program date-time, Unix ms.
    At(i64),
    /// The first segment of the playlist: a VOD of a stream still on air
    /// played from its start ("Смотреть с начала эфира"). `t0` is the
    /// program date-time of that start, Unix ms, when the playlist has none
    /// (Twitch VODs): segment times are counted from it.
    First {
        /// Program date-time of the first segment, Unix ms.
        t0: Option<i64>,
    },
}

/// Why a seek was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekError {
    /// The playhead is in an ad break: seeking waits until it ends.
    Ad,
    /// No segments known yet.
    NoWindow,
}

impl SeekError {
    /// Stable code for the HTTP endpoint and the UI.
    pub fn code(self) -> &'static str {
        match self {
            Self::Ad => "ad",
            Self::NoWindow => "no_window",
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    url: String,
    start: Option<i64>,
    duration: f64,
    ad: bool,
    prefetch: bool,
    title: String,
    discontinuity: bool,
}

impl Entry {
    fn end(&self) -> Option<i64> {
        self.start.map(|s| s + secs_to_ms(self.duration))
    }

    fn contains(&self, at: i64) -> bool {
        self.start
            .is_some_and(|s| at >= s && at < s + secs_to_ms(self.duration))
    }
}

#[derive(Debug, Clone)]
struct Delivery {
    bytes: u64,
    complete: bool,
    prefetch: bool,
}

/// A seek the player has not started yet: the next playlists start there.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pending {
    /// Program date-time to start at; `None`: the first segment.
    at_ms: Option<i64>,
    /// The segment the last rendered playlist starts the player at.
    sequence: Option<u64>,
}

#[derive(Debug)]
struct State {
    upstream_url: String,
    mode: LiveMode,
    entries: BTreeMap<u64, Entry>,
    ad_breaks: Vec<AdBreak>,
    /// Ad `#EXT-X-DATERANGE` lines as Twitch sent them, by id: repeated for
    /// the segments kept after they left Twitch's window.
    ad_lines: BTreeMap<String, (i64, String)>,
    edge: Option<i64>,
    low_latency: LowLatencyInfo,
    deliveries: BTreeMap<u64, Delivery>,
    totals: LiveTotals,
    /// The last playlist ended (`#EXT-X-ENDLIST`) or only grows (`EVENT`).
    finite: bool,
    pending: Option<Pending>,
    /// The user rewound (see [`DvrReport::active`]).
    dvr: bool,
    /// Ad segments the player received in full.
    played_ads: BTreeSet<u64>,
    /// Seconds the player holds ahead of its playhead (last report).
    ahead: f64,
    /// Program date-time of the first segment of a playlist without one.
    synthetic_t0: Option<i64>,
    /// When the player last asked for the playlist, Unix ms.
    player_playlist_ms: i64,
}

impl State {
    fn new(upstream_url: String, mode: LiveMode) -> Self {
        Self {
            upstream_url,
            mode,
            entries: BTreeMap::new(),
            ad_breaks: Vec::new(),
            ad_lines: BTreeMap::new(),
            edge: None,
            low_latency: LowLatencyInfo::default(),
            deliveries: BTreeMap::new(),
            totals: LiveTotals::default(),
            finite: false,
            pending: None,
            dvr: false,
            played_ads: BTreeSet::new(),
            ahead: 0.0,
            synthetic_t0: None,
            player_playlist_ms: 0,
        }
    }

    fn register(&mut self, base: &str, playlist: &MediaPlaylist, text: &str) {
        // A VOD has no program date-times: count them from `t0` so the
        // playhead of "watch from the start" is comparable with the live
        // window (TWI-025).
        let mut offset = 0i64;
        let mut edge = playlist.edge();
        for s in &playlist.segments {
            let start = s
                .program_date_time
                .or_else(|| self.synthetic_t0.map(|t0| t0 + offset));
            offset += secs_to_ms(s.duration);
            if !s.prefetch && s.program_date_time.is_none() {
                edge = start.map(|st| st + secs_to_ms(s.duration)).or(edge);
            }
            let entry = Entry {
                url: resolve_uri(base, &s.uri),
                start,
                duration: s.duration,
                ad: s.ad,
                prefetch: s.prefetch,
                title: s.title.clone(),
                discontinuity: s.discontinuity,
            };
            self.entries.insert(s.sequence, entry);
        }
        for b in playlist.ad_breaks() {
            match self.ad_breaks.iter_mut().find(|known| known.id == b.id) {
                Some(known) => *known = b,
                None => self.ad_breaks.push(b),
            }
        }
        for line in text.lines().map(str::trim) {
            let Some(list) = line.strip_prefix("#EXT-X-DATERANGE:") else {
                continue;
            };
            let attrs = super::playlist::attributes(list);
            if let Some(b) = attrs
                .get("ID")
                .and_then(|id| self.ad_breaks.iter().find(|b| &b.id == id))
            {
                self.ad_lines
                    .insert(b.id.clone(), (b.start, line.to_owned()));
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
        if let Some(edge) = edge {
            self.edge = Some(self.edge.map_or(edge, |e| e.max(edge)));
        }
        self.low_latency = LowLatencyInfo {
            prefetch: playlist.prefetch().count(),
            parts: playlist.parts,
            preload_hint: playlist.preload_hint,
            target_duration: playlist.target_duration,
            playlists: self.low_latency.playlists + 1,
        };
        // A finished playlist (VOD, `#EXT-X-ENDLIST`) or one that only grows
        // (`EVENT`, the VOD of a stream on air) keeps every segment for
        // seeking. A live window slides: the proxy keeps what left it for
        // the DVR window (TWI-025).
        self.finite = playlist.ended || playlist.event;
        if !self.finite {
            self.trim_window();
        }
        let keep = self.ad_breaks.len().saturating_sub(64);
        self.ad_breaks.drain(..keep);
    }

    /// Keeps the newest [`DVR_WINDOW_SECS`] (at most [`MAX_ENTRIES`]
    /// segments) of a live stream.
    fn trim_window(&mut self) {
        if let Some(edge) = self.edge {
            let oldest = edge - secs_to_ms(DVR_WINDOW_SECS);
            while let Some((_, e)) = self.entries.first_key_value() {
                if e.end().is_some_and(|end| end <= oldest) {
                    self.entries.pop_first();
                } else {
                    break;
                }
            }
        }
        trim(&mut self.entries, MAX_ENTRIES);
        if let Some((&first, e)) = self.entries.first_key_value() {
            let start = e.start;
            self.played_ads.retain(|&s| s >= first);
            if let Some(start) = start {
                // Ad lines of breaks that ended long before the window.
                self.ad_lines
                    .retain(|_, (at, _)| *at + 60 * 60 * 1000 > start);
            }
        }
    }

    /// Drops the window up to `sequence` (Twitch no longer serves it).
    fn drop_until(&mut self, sequence: u64) {
        while let Some((&first, _)) = self.entries.first_key_value() {
            if first > sequence {
                break;
            }
            self.entries.pop_first();
        }
    }

    /// The segments the player can be sent to: the unbroken run of known
    /// segments that ends with the newest complete one. `(first sequence,
    /// program date-time of its start, live edge)`.
    fn window(&self) -> Option<(u64, i64, i64)> {
        let edge = self.edge?;
        let (&last, _) = self.entries.iter().rev().find(|(_, e)| !e.prefetch)?;
        let mut first = last;
        while first > 0 && self.entries.contains_key(&(first - 1)) {
            first -= 1;
        }
        let (first, start) = self
            .entries
            .range(first..)
            .find_map(|(&seq, e)| e.start.map(|s| (seq, s)))?;
        Some((first, start, edge))
    }

    /// The segment at program date-time `at` (or the first one after it).
    fn sequence_at(&self, at: i64) -> Option<u64> {
        let (first, _, _) = self.window()?;
        let range = || self.entries.range(first..);
        range()
            .find(|(_, e)| e.contains(at))
            .or_else(|| range().find(|(_, e)| e.start.is_some_and(|s| s >= at)))
            .map(|(&seq, _)| seq)
    }

    /// The first ad segment from `from` (the one under the playhead) up to
    /// `to` that the player has not played: a forward jump stops there.
    fn unplayed_ad(&self, from: u64, to: u64) -> Option<u64> {
        if to < from {
            return None;
        }
        self.entries
            .range(from..=to)
            .find(|(seq, e)| e.ad && !self.played_ads.contains(seq))
            .map(|(&seq, _)| seq)
    }

    /// Program date-time under the playhead.
    fn playhead(&self) -> Option<i64> {
        self.delivered_end()
            .map(|end| end - secs_to_ms(self.ahead.max(0.0)))
    }

    /// Where a seek counts from: a seek not started yet, else the playhead,
    /// else the live edge.
    fn seek_base(&self) -> Option<i64> {
        self.pending
            .and_then(|p| p.at_ms)
            .or_else(|| self.playhead())
            .or(self.edge)
    }

    /// Moves the playhead within the DVR window (TWI-025). The player
    /// reloads the playlist URL afterwards; the next playlists start it at
    /// the new position until it asks for a segment there.
    fn seek(&mut self, target: SeekTarget) -> Result<DvrSeek, SeekError> {
        if let SeekTarget::First { t0 } = target {
            if t0.is_some() {
                self.synthetic_t0 = t0;
            }
            self.pending = Some(Pending {
                at_ms: None,
                sequence: None,
            });
            self.dvr = true;
            self.deliveries.clear();
            return Ok(DvrSeek {
                live: false,
                at_ms: self.synthetic_t0,
                stopped_at_ad: false,
            });
        }
        // No seeking during an ad break (TWI-006): it plays as served.
        if self.pending.is_none()
            && let Some(at) = self.playhead()
            && self.ad_at(at).is_some()
        {
            return Err(SeekError::Ad);
        }
        let (_, window_start, edge) = self.window().ok_or(SeekError::NoWindow)?;
        let base = self.seek_base().unwrap_or(edge);
        let at = match target {
            SeekTarget::By(secs) => base + secs_to_ms(secs),
            SeekTarget::At(ms) => ms,
            SeekTarget::First { .. } => base,
        };
        let target_ms = secs_to_ms(CatchUpPolicy::for_mode(self.mode).target);
        if !self.finite && at >= edge - target_ms {
            return Ok(self.go_live());
        }
        let at = at.clamp(window_start, edge);
        let mut seq = self.sequence_at(at).ok_or(SeekError::NoWindow)?;
        let mut stopped_at_ad = false;
        if at > base
            && let Some(from) = self.sequence_at(base)
            && let Some(ad) = self.unplayed_ad(from, seq)
        {
            seq = ad;
            stopped_at_ad = true;
        }
        let at = if stopped_at_ad {
            self.entries.get(&seq).and_then(|e| e.start).unwrap_or(at)
        } else {
            at
        };
        self.pending = Some(Pending {
            at_ms: Some(at),
            sequence: None,
        });
        self.dvr = true;
        self.deliveries.clear();
        Ok(DvrSeek {
            live: false,
            at_ms: Some(at),
            stopped_at_ad,
        })
    }

    /// "К эфиру": the player reloads at the edge. After a rewind it stops
    /// at an ad break it has not played yet instead (no ad is skipped by
    /// jumping forward).
    fn go_live(&mut self) -> DvrSeek {
        if self.dvr
            && let Some(base) = self.seek_base()
            && let Some(from) = self.sequence_at(base)
            && let Some(last) = self.entries.keys().next_back().copied()
            && let Some(ad) = self.unplayed_ad(from, last)
        {
            let at = self.entries.get(&ad).and_then(|e| e.start);
            self.pending = Some(Pending {
                at_ms: at,
                sequence: None,
            });
            self.deliveries.clear();
            return DvrSeek {
                live: false,
                at_ms: at,
                stopped_at_ad: true,
            };
        }
        self.pending = None;
        self.dvr = false;
        self.deliveries.clear();
        DvrSeek {
            live: true,
            at_ms: self.edge,
            stopped_at_ad: false,
        }
    }

    /// Renders the playlist for the player: Twitch's playlist as
    /// [`MediaPlaylist::rewrite`] makes it, the segments kept for the DVR
    /// window before it, cut short while a seek waits for the player.
    fn render(&mut self, url: &str, parsed: &MediaPlaylist, text: &str) -> String {
        let mut out = parsed.rewrite(
            text,
            self.mode,
            |s| format!("s/{}{}", s.sequence, extension(&s.uri)),
            |uri| resolve_uri(url, uri),
        );
        if !self.finite
            && let Some(first_up) = parsed.segments.first().map(|s| s.sequence)
            && text.contains("#EXT-X-MEDIA-SEQUENCE:")
            && let Some((first, _, _)) = self.window()
            && first < first_up
        {
            let mut prefix = String::new();
            for (&seq, e) in self.entries.range(first..first_up) {
                if let Some((start, end)) = e.start.zip(e.end()) {
                    for (id, (at, line)) in &self.ad_lines {
                        if *at >= start && *at < end && !text.contains(&format!("\"{id}\"")) {
                            prefix.push_str(line);
                            prefix.push('\n');
                        }
                    }
                }
                if e.discontinuity {
                    prefix.push_str("#EXT-X-DISCONTINUITY\n");
                }
                if let Some(start) = e.start {
                    prefix.push_str("#EXT-X-PROGRAM-DATE-TIME:");
                    prefix.push_str(&format_date_time(start));
                    prefix.push('\n');
                }
                prefix.push_str(&format!(
                    "#EXTINF:{:.3},{}\ns/{seq}{}\n",
                    e.duration,
                    e.title,
                    extension(&e.url)
                ));
            }
            out = with_prefix(&out, first, &prefix);
        }
        if let Some(mut p) = self.pending
            && !parsed.ended
        {
            let start = match p.at_ms {
                Some(at) => self.sequence_at(at),
                None => parsed.segments.first().map(|s| s.sequence),
            };
            if let Some(start) = start {
                p.sequence = Some(start);
                self.pending = Some(p);
                // FFmpeg starts a live playlist `live_start_index` entries
                // before its end: −2 in low latency, −3 (its default) in
                // stable mode and for VODs (`liveMpvOptions`).
                let tail = match self.mode {
                    LiveMode::LowLatency => 1,
                    LiveMode::Stable => 2,
                };
                if let Some(cut) = cut_after(&out, start + tail) {
                    out = cut;
                }
            }
        }
        out
    }

    /// The player asked for segment `sequence`: a waiting seek has started.
    fn requested(&mut self, sequence: u64) {
        if let Some(p) = self.pending
            && p.sequence.is_some_and(|start| sequence >= start)
        {
            self.pending = None;
        }
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
        let (seq, _) = self.entries.iter().find(|(_, e)| e.contains(at))?;
        let mut end = None;
        for e in self.entries.range(seq..).map(|(_, e)| e) {
            if !e.ad {
                break;
            }
            end = e.end();
        }
        end.map(|end| AdStatus {
            id: None,
            remaining: Some((end - at) as f64 / 1000.0),
            pod_position: None,
            pod_length: None,
        })
    }

    fn dvr_report(&self, playhead: Option<i64>) -> Option<DvrReport> {
        let (_, start, edge) = self.window()?;
        let at = self.pending.and_then(|p| p.at_ms).or(playhead);
        Some(DvrReport {
            window_start_ms: start,
            window_seconds: (edge - start).max(0) as f64 / 1000.0,
            behind_seconds: at.map(|at| (edge - at).max(0) as f64 / 1000.0),
            active: self.dvr,
            pending: self.pending.is_some(),
        })
    }

    fn report(&mut self, now_ms: i64, cache_ahead: f64, current_rate: f64) -> LiveReport {
        self.ahead = cache_ahead;
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
        // During an ad, and after a rewind on purpose (DVR): 1×, no "jump to
        // live" hint.
        let hold = ad.is_some() || self.dvr;
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
            rate: policy.rate(latency, hold, current_rate),
            should_jump: !hold && policy.should_jump(latency),
            seek_allowed: ad.is_none(),
            ad,
            dvr: self.dvr_report(playhead),
            low_latency: self.low_latency.clone(),
            totals: self.totals.clone(),
            delivered,
        }
    }
}

/// `rendered` with `prefix` (segments kept for the DVR window) before its
/// first segment and the media sequence number of `first`.
fn with_prefix(rendered: &str, first: u64, prefix: &str) -> String {
    let mut out = String::with_capacity(rendered.len() + prefix.len());
    let mut inserted = false;
    for line in rendered.lines() {
        if line.starts_with("#EXT-X-MEDIA-SEQUENCE:") {
            out.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{first}\n"));
            continue;
        }
        if !inserted && starts_segment(line) {
            out.push_str(prefix);
            inserted = true;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !inserted {
        out.push_str(prefix);
    }
    out
}

/// A line that belongs to a segment rather than to the playlist header.
fn starts_segment(line: &str) -> bool {
    !line.starts_with('#')
        || line == "#EXT-X-DISCONTINUITY"
        || line == "#EXT-X-GAP"
        || [
            "#EXTINF:",
            "#EXT-X-PROGRAM-DATE-TIME:",
            "#EXT-X-DATERANGE:",
            "#EXT-X-BYTERANGE:",
            "#EXT-X-KEY:",
            "#EXT-X-PART:",
            "#EXT-X-TWITCH-PREFETCH:",
        ]
        .iter()
        .any(|p| line.starts_with(p))
}

/// `rendered` up to and including the segment `sequence`; `None` when it
/// is not listed (the playlist stays whole).
fn cut_after(rendered: &str, sequence: u64) -> Option<String> {
    let uri = format!("s/{sequence}.");
    let mut out = String::with_capacity(rendered.len());
    for line in rendered.lines() {
        out.push_str(line);
        out.push('\n');
        if line.starts_with(&uri) {
            return Some(out);
        }
    }
    None
}

fn trim<V>(map: &mut BTreeMap<u64, V>, max: usize) {
    while map.len() > max {
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
                            tokio::spawn(follow(thread_ctx.clone()));
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

    /// URL that does [`Self::seek`] (TWI-025): `?by=<s>`, `?at=<Unix ms>`
    /// or `?first=1[&t0=<Unix ms>]`; answers a [`DvrSeek`] as JSON, `409`
    /// with `{"error":"ad"}` during an ad break.
    pub fn seek_url(&self) -> String {
        format!("http://{}/{}/seek", self.addr, self.ctx.token)
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
        let mut next = State::new(url.to_owned(), st.mode);
        // A rewound player stays where it was (Twitch's variants share
        // sequence numbers and program date-times); the DVR window of the
        // new variant starts with its playlist.
        next.synthetic_t0 = st.synthetic_t0;
        if st.dvr {
            next.dvr = true;
            next.pending = Some(Pending {
                at_ms: st.seek_base(),
                sequence: None,
            });
        }
        *st = next;
    }

    /// "Go to live": forgets what the player received, the player reloads
    /// the playlist URL and starts at the edge again. Ad breaks and
    /// counters are kept. After a rewind it stops at an ad break the player
    /// has not played yet instead.
    pub fn reset_playhead(&self) -> DvrSeek {
        self.ctx.state().go_live()
    }

    /// Moves the playhead within the DVR window (TWI-025); the player
    /// reloads the playlist URL afterwards. Refused during an ad break.
    pub fn seek(&self, target: SeekTarget) -> Result<DvrSeek, SeekError> {
        self.ctx.state().seek(target)
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
        ctx.state().go_live();
        return simple(w, 200, "text/plain", b"ok").await;
    }
    if rest == "seek" {
        let param = |name: &str| {
            req.query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v)
        };
        let target = if param("first").is_some() {
            Some(SeekTarget::First {
                t0: param("t0").and_then(|v| v.parse().ok()),
            })
        } else if let Some(by) = param("by").and_then(|v| v.parse().ok()) {
            Some(SeekTarget::By(by))
        } else {
            param("at")
                .and_then(|v| v.parse().ok())
                .map(SeekTarget::At)
        };
        let Some(target) = target else {
            return simple(w, 400, "text/plain", b"by, at or first").await;
        };
        let result = ctx.state().seek(target);
        return match result {
            Ok(seek) => {
                let body = serde_json::to_vec(&seek).map_err(io::Error::other)?;
                simple(w, 200, "application/json", &body).await
            }
            Err(e) => {
                let body = format!("{{\"error\":\"{}\"}}", e.code());
                let status = if e == SeekError::Ad { 409 } else { 404 };
                simple(w, status, "application/json", body.as_bytes()).await
            }
        };
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
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
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
            st.register(&url, &parsed, &text);
            st.player_playlist_ms = now_ms();
            st.render(&url, &parsed, &text)
        })
    };
    match rendered {
        Some(r) => simple(w, 200, "application/vnd.apple.mpegurl", r.as_bytes()).await,
        None => simple(w, 404, "text/plain", b"switched").await,
    }
}

/// Keeps the DVR window filling while the player does not read the live
/// playlist (paused, or "watch from the start" plays the VOD): reads it
/// itself once per target duration. Nothing extra while the player reads
/// it, and never for a VOD.
async fn follow(ctx: Arc<Ctx>) {
    loop {
        let (wait, url, idle) = {
            let st = ctx.state();
            let wait = st.low_latency.target_duration.clamp(1.0, 10.0);
            let idle = !st.finite
                && !st.entries.is_empty()
                && now_ms() - st.player_playlist_ms > secs_to_ms(2.0 * wait);
            (wait, st.upstream_url.clone(), idle)
        };
        if idle
            && let Ok(text) = ctx.upstream.playlist(&url).await
            && let Ok(parsed) = MediaPlaylist::parse(&text)
        {
            let mut st = ctx.state();
            if st.upstream_url == url {
                st.register(&url, &parsed, &text);
            }
        }
        tokio::time::sleep(Duration::from_secs_f64(wait)).await;
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
    ctx.state().requested(seq);
    let mut body = match ctx.upstream.segment(&entry.url).await {
        Ok(b) => b,
        Err(e) => {
            if matches!(e, PlaybackError::Unavailable | PlaybackError::NotFound) {
                // Twitch no longer serves this part of the DVR window: it
                // starts after it from now on.
                let mut st = ctx.state();
                if !st.finite && st.window().is_some_and(|(first, _, _)| seq >= first) {
                    st.drop_until(seq);
                }
            }
            return simple(w, upstream_status(&e), "text/plain", e.code().as_bytes()).await;
        }
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
        trim(&mut st.deliveries, HISTORY);
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
        st.played_ads.insert(seq);
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
        st.register(
            "http://o/live.m3u8",
            &MediaPlaylist::parse(text).unwrap(),
            text,
        );
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

    /// A VOD playlist like a real Twitch one (TWI-014 checks: 902 segments,
    /// `-muted` names, `EVENT` + `ENDLIST`): every segment stays reachable,
    /// so the player can start at 0 and seek to the middle; so does the
    /// archive of a stream still on air (`EVENT` without `ENDLIST`,
    /// TWI-025). A live window without times keeps the newest
    /// [`MAX_ENTRIES`].
    #[test]
    fn vod_keeps_every_segment_live_window_slides() {
        let vod = |count: usize, event: bool, ended: bool| {
            let mut text = String::from(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-TWITCH-ELAPSED-SECS:0.000\n#EXT-X-TWITCH-TOTAL-SECS:9018.434\n",
            );
            if event {
                text.push_str("#EXT-X-PLAYLIST-TYPE:EVENT\n");
            }
            for k in 0..count {
                let muted = if k < 3 { "-muted" } else { "" };
                text.push_str(&format!("#EXTINF:10.000,\n{k}{muted}.ts\n"));
            }
            if ended {
                text.push_str("#EXT-X-ENDLIST\n");
            }
            text
        };
        let base = "https://vod.example.invalid/abc/360p30/index-muted-X.m3u8";
        for ended in [true, false] {
            let text = vod(902, true, ended);
            let mut st = State::new(base.into(), LiveMode::Stable);
            st.register(base, &MediaPlaylist::parse(&text).unwrap(), &text);
            assert_eq!(st.entries.len(), 902);
            assert_eq!(
                st.entries[&0].url,
                "https://vod.example.invalid/abc/360p30/0-muted.ts"
            );
            assert_eq!(
                st.entries[&451].url,
                "https://vod.example.invalid/abc/360p30/451.ts"
            );
        }
        let text = vod(1200, false, false);
        let mut live = State::new(base.into(), LiveMode::Stable);
        live.register(base, &MediaPlaylist::parse(&text).unwrap(), &text);
        assert_eq!(live.entries.len(), MAX_ENTRIES);
        assert!(!live.entries.contains_key(&0));
    }

    // ---- TWI-025: live DVR --------------------------------------------

    /// Program date-time of segment `seq` of [`live_playlist`].
    fn t(seq: u64) -> i64 {
        1_800_000_000_000 + seq as i64 * 2000
    }

    /// A Twitch-style live playlist at the moment segment `done` completes:
    /// the 6 newest complete 2 s segments, then 2 prefetch segments.
    /// Segments in `ads` are a stitched ad announced by a `DATERANGE`.
    fn live_playlist(done: u64, ads: std::ops::Range<u64>) -> String {
        let first = done.saturating_sub(6);
        let mut out = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-TWITCH-ELAPSED-SECS:{}.000\n",
            first * 2
        );
        if !ads.is_empty() && ads.start < done + 2 && ads.end > first {
            out.push_str(&format!(
                "#EXT-X-DATERANGE:ID=\"stitched-ad-{}\",CLASS=\"twitch-stitched-ad\",START-DATE=\"{}\",DURATION={}.000\n",
                ads.start,
                format_date_time(t(ads.start)),
                (ads.end - ads.start) * 2
            ));
        }
        for seq in first..done {
            let title = if ads.contains(&seq) {
                "Amazon|ad"
            } else {
                "live"
            };
            out.push_str(&format!(
                "#EXT-X-PROGRAM-DATE-TIME:{}\n#EXTINF:2.000,{title}\nhttps://edge.example.invalid/seg/{seq}.ts\n",
                format_date_time(t(seq))
            ));
        }
        for seq in done..done + 2 {
            out.push_str(&format!(
                "#EXT-X-TWITCH-PREFETCH:https://edge.example.invalid/seg/{seq}.ts\n"
            ));
        }
        out
    }

    /// The proxy after it has followed the stream from segment 0 to `done`.
    fn followed(done: u64, ads: std::ops::Range<u64>) -> (State, String, MediaPlaylist) {
        let url = "https://edge.example.invalid/live.m3u8";
        let mut st = State::new(url.into(), LiveMode::LowLatency);
        let mut last = (String::new(), MediaPlaylist::default());
        for k in 1..=done {
            let text = live_playlist(k, ads.clone());
            let parsed = MediaPlaylist::parse(&text).unwrap();
            st.register(url, &parsed, &text);
            last = (text, parsed);
        }
        (st, last.0, last.1)
    }

    fn render(st: &mut State, text: &str, parsed: &MediaPlaylist) -> String {
        st.render("https://edge.example.invalid/live.m3u8", parsed, text)
    }

    /// The player at `seq` with `ahead` seconds of cache past its playhead.
    fn delivered(st: &mut State, seq: u64) {
        st.deliveries.insert(
            seq,
            Delivery {
                bytes: 1000,
                complete: true,
                prefetch: false,
            },
        );
        st.requested(seq);
    }

    #[test]
    fn dvr_window_keeps_what_left_the_live_playlist() {
        let (mut st, text, parsed) = followed(60, 20..23);
        // Twitch lists only 54..60; the proxy kept 0..60.
        assert_eq!(parsed.segments.first().unwrap().sequence, 54);
        assert_eq!(st.window(), Some((0, t(0), t(60))));
        let out = render(&mut st, &text, &parsed);
        let again = MediaPlaylist::parse(&out).unwrap();
        assert_eq!(again.media_sequence, 0);
        let seqs: Vec<u64> = again.segments.iter().map(|s| s.sequence).collect();
        assert_eq!(seqs, (0..62).collect::<Vec<_>>(), "{out}");
        for seq in 0..62 {
            assert_eq!(
                out.matches(&format!("\ns/{seq}.ts\n")).count(),
                1,
                "segment {seq}"
            );
        }
        // Program date-times and ad markers of the kept part are there.
        assert_eq!(again.segments[10].program_date_time, Some(t(10)));
        assert_eq!(out.matches("ID=\"stitched-ad-20\"").count(), 1);
        let ads: Vec<u64> = again
            .segments
            .iter()
            .filter(|s| s.ad)
            .map(|s| s.sequence)
            .collect();
        assert_eq!(ads, [20, 21, 22]);
        // Twitch's own lines stay as sent.
        assert!(out.contains("#EXT-X-TWITCH-ELAPSED-SECS:108.000\n"));
        let report = st.report(t(61), 0.0, 1.0);
        let dvr = report.dvr.unwrap();
        assert_eq!(dvr.window_start_ms, t(0));
        assert_eq!(dvr.window_seconds, 120.0);
        assert!(!dvr.active);
    }

    #[test]
    fn dvr_window_slides_after_thirty_minutes() {
        let done = (DVR_WINDOW_SECS / 2.0) as u64 + 100;
        let (st, _, _) = followed(done, 0..0);
        let (first, start, edge) = st.window().unwrap();
        assert_eq!(first, 100);
        assert_eq!(edge - start, secs_to_ms(DVR_WINDOW_SECS));
    }

    #[test]
    fn seek_sixty_seconds_back_and_back_to_live() {
        let (mut st, text, parsed) = followed(60, 0..0);
        // At the edge: the player read 58, 1 s of it is ahead.
        delivered(&mut st, 58);
        let r = st.report(t(59) + 2500, 1.0, 1.0);
        assert_eq!(r.playhead_ms, Some(t(59) - 1000));
        assert_eq!(r.dvr.as_ref().unwrap().behind_seconds, Some(3.0));

        let seek = st.seek(SeekTarget::By(-60.0)).unwrap();
        assert!(!seek.live && !seek.stopped_at_ad);
        assert_eq!(seek.at_ms, Some(t(29) - 1000));
        // The next playlists end one segment after the target, so FFmpeg
        // (`live_start_index=-2`) starts at it, until it asks for it.
        for _ in 0..2 {
            let out = render(&mut st, &text, &parsed);
            let p = MediaPlaylist::parse(&out).unwrap();
            let n = p.segments.len();
            assert_eq!(p.segments[n - 2].sequence, 28, "{out}");
            assert_eq!(p.segments[n - 1].sequence, 29);
            assert_eq!(p.segments[0].sequence, 0);
        }
        let r = st.report(t(61), 0.0, 1.0);
        let dvr = r.dvr.unwrap();
        assert!(dvr.active && dvr.pending);
        assert_eq!(dvr.behind_seconds, Some(63.0));
        delivered(&mut st, 28);
        let full = MediaPlaylist::parse(&render(&mut st, &text, &parsed)).unwrap();
        assert_eq!(full.segments.last().unwrap().sequence, 61);
        // Rewound: no catch-up, no "jump to live" hint.
        let r = st.report(t(61), 1.0, 1.0);
        assert_eq!(r.playhead_ms, Some(t(29) - 1000));
        assert_eq!(r.rate, 1.0);
        assert!(!r.should_jump);
        assert!(r.latency.unwrap() > 60.0);
        let dvr = r.dvr.unwrap();
        assert!(dvr.active && !dvr.pending);
        assert_eq!(dvr.behind_seconds, Some(63.0));

        // Further back is clamped to the window, forward again works.
        st.seek(SeekTarget::By(-600.0)).unwrap();
        assert_eq!(st.seek_base(), Some(t(0)));
        st.seek(SeekTarget::At(t(30))).unwrap();
        assert_eq!(st.seek_base(), Some(t(30)));

        // "К эфиру": back at the edge, catch-up again.
        let live = st.go_live();
        assert!(live.live);
        assert!(st.pending.is_none() && !st.dvr);
        let out = render(&mut st, &text, &parsed);
        assert!(out.ends_with("s/61.ts\n"), "{out}");
        // A seek that reaches the edge is "go to live" too.
        delivered(&mut st, 40);
        st.report(t(41), 0.0, 1.0);
        assert!(st.seek(SeekTarget::By(600.0)).unwrap().live);
    }

    #[test]
    fn no_seeking_during_an_ad() {
        let (mut st, _, _) = followed(60, 50..55);
        delivered(&mut st, 51);
        let r = st.report(t(52) + 500, 0.5, 1.0);
        assert!(r.ad.is_some());
        assert!(!r.seek_allowed);
        assert_eq!(st.seek(SeekTarget::By(-30.0)), Err(SeekError::Ad));
        assert!(st.pending.is_none() && !st.dvr);
        // After the break: allowed.
        delivered(&mut st, 56);
        let r = st.report(t(57) + 500, 0.5, 1.0);
        assert!(r.seek_allowed);
        assert!(st.seek(SeekTarget::By(-30.0)).is_ok());
    }

    #[test]
    fn forward_jumps_never_skip_an_ad_not_played() {
        let (mut st, _, _) = followed(60, 40..43);
        // Rewound to 30 before the break was played.
        st.seek(SeekTarget::At(t(30))).unwrap();
        delivered(&mut st, 30);
        st.report(t(61), 0.0, 1.0);
        let jump = st.seek(SeekTarget::By(20.0)).unwrap();
        assert!(jump.stopped_at_ad);
        assert_eq!(jump.at_ms, Some(t(40)));
        // "К эфиру" stops there as well…
        let live = st.go_live();
        assert!(!live.live && live.stopped_at_ad);
        assert_eq!(live.at_ms, Some(t(40)));
        // …until the break has been played.
        for seq in 40..43 {
            st.played_ads.insert(seq);
        }
        delivered(&mut st, 43);
        st.report(t(61), 0.0, 1.0);
        assert!(st.go_live().live);
        // Backward jumps over a played break are free.
        let (mut st, _, _) = followed(60, 40..43);
        for seq in 40..43 {
            st.played_ads.insert(seq);
        }
        delivered(&mut st, 58);
        st.report(t(59), 0.0, 1.0);
        let back = st.seek(SeekTarget::At(t(30))).unwrap();
        assert!(!back.stopped_at_ad);
        let fwd = st.seek(SeekTarget::At(t(50))).unwrap();
        assert!(!fwd.stopped_at_ad);
    }

    #[test]
    fn segments_twitch_no_longer_serves_shrink_the_window() {
        let (mut st, _, _) = followed(60, 0..0);
        st.drop_until(9);
        assert_eq!(st.window().map(|w| w.0), Some(10));
    }

    /// "Смотреть с начала эфира": the archive of the stream still on air
    /// (`EVENT`, no program date-times) from its first segment, with times
    /// counted from the stream start.
    #[test]
    fn vod_of_a_stream_on_air_from_the_start() {
        let mut text = String::from(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-MEDIA-SEQUENCE:0\n",
        );
        for k in 0..30 {
            text.push_str(&format!("#EXTINF:10.000,\n{k}.ts\n"));
        }
        let url = "https://vod.example.invalid/x/chunked/index-dvr.m3u8";
        let mut st = State::new(url.into(), LiveMode::Stable);
        let t0 = 1_800_000_000_000;
        st.seek(SeekTarget::First { t0: Some(t0) }).unwrap();
        let parsed = MediaPlaylist::parse(&text).unwrap();
        st.register(url, &parsed, &text);
        let out = st.render(url, &parsed, &text);
        // FFmpeg's default `live_start_index=-3` lands on segment 0.
        let p = MediaPlaylist::parse(&out).unwrap();
        assert_eq!(p.segments.len(), 3, "{out}");
        assert!(!p.ended);
        assert_eq!(st.entries[&12].start, Some(t0 + 120_000));
        delivered(&mut st, 0);
        assert_eq!(
            MediaPlaylist::parse(&st.render(url, &parsed, &text))
                .unwrap()
                .segments
                .len(),
            30
        );
        let r = st.report(t0 + 600_000, 4.0, 1.0);
        assert_eq!(r.playhead_ms, Some(t0 + 6000));
        let dvr = r.dvr.unwrap();
        assert_eq!(dvr.window_start_ms, t0);
        assert_eq!(dvr.window_seconds, 300.0);
        // Seeking in it: by program date-time, never "go to live".
        let s = st.seek(SeekTarget::By(290.0)).unwrap();
        assert!(!s.live);
        assert_eq!(s.at_ms, Some(t0 + 296_000));
        // A finished VOD is never cut short.
        let ended = format!("{text}#EXT-X-ENDLIST\n");
        let parsed = MediaPlaylist::parse(&ended).unwrap();
        st.seek(SeekTarget::First { t0: None }).unwrap();
        let out = st.render(url, &parsed, &ended);
        assert!(out.ends_with("#EXT-X-ENDLIST\n"));
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
