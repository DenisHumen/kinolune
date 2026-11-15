// SPDX-License-Identifier: MPL-2.0
//! A made-up Twitch on a loopback port and a muxer double (`testing`
//! feature). Nothing here talks to Twitch; ids, names and tokens are made
//! up, and there is no Twitch client id anywhere.
//!
//! [`FakeTwitch`] answers what a VOD download asks for:
//!
//! - `POST /gql`: `PlaybackAccessToken` → a made-up token;
//! - `GET /vod/<id>.m3u8`: usher's master playlist (`chunked` source and
//!   `audio_only`), or `403 vod_manifest_restricted` for a restricted VOD;
//! - `GET /media/<id>/<group>/index-dvr.m3u8` and its segments;
//! - `GET /helix/videos?id=` and `/helix/users?id=`;
//! - `GET /img/…`: a small JPEG for the thumbnail and the avatar.
//!
//! [`ConcatMuxer`] stands in for FFmpeg where it is not installed (CI): it
//! concatenates the inputs, sums the durations written into the segments
//! by [`FakeSegment::synthetic`] and records what it was asked to embed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::mux::{MuxJob, Muxer, Probed};
use super::playlist::MediaPlaylist;
use super::source::ArchiveError;

/// Where a segment's bytes come from.
#[derive(Debug, Clone)]
pub enum SegmentData {
    Bytes(Arc<Vec<u8>>),
    File(PathBuf),
}

/// One segment of a fake VOD.
#[derive(Debug, Clone)]
pub struct FakeSegment {
    pub data: SegmentData,
    pub duration_ms: u64,
    pub muted: bool,
}

impl FakeSegment {
    /// A made-up segment: a header line `FAKESEG <index> <duration_ms>`
    /// padded to `size` bytes, readable by [`ConcatMuxer`].
    pub fn synthetic(index: usize, duration_ms: u64, size: usize) -> Self {
        let mut bytes = format!("FAKESEG {index} {duration_ms}\n").into_bytes();
        bytes.resize(size.max(bytes.len()), b'.');
        Self {
            data: SegmentData::Bytes(Arc::new(bytes)),
            duration_ms,
            muted: false,
        }
    }

    pub(crate) fn bytes(&self) -> std::io::Result<Vec<u8>> {
        match &self.data {
            SegmentData::Bytes(b) => Ok(b.as_ref().clone()),
            SegmentData::File(p) => std::fs::read(p),
        }
    }
}

/// Segments of a real HLS rendition on disk (`index.m3u8` + its files),
/// e.g. made with FFmpeg's `hls` muxer.
pub fn segments_from_dir(dir: &Path) -> std::io::Result<Vec<FakeSegment>> {
    let text = std::fs::read_to_string(dir.join("index.m3u8"))?;
    let playlist = MediaPlaylist::parse(&text, "file:///media/index.m3u8")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    Ok(playlist
        .segments
        .iter()
        .map(|s| FakeSegment {
            data: SegmentData::File(dir.join(s.url.rsplit('/').next().unwrap_or(""))),
            duration_ms: s.duration_ms,
            muted: false,
        })
        .collect())
}

/// A VOD of the fake Twitch.
#[derive(Debug, Clone)]
pub struct FakeVod {
    pub id: String,
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub title: String,
    pub created_at: String,
    /// Helix `duration`; by default the sum of the segments.
    pub helix_duration: Option<String>,
    /// Helix `muted_segments` (seconds).
    pub helix_muted: Vec<(u64, u64)>,
    pub segments: Vec<FakeSegment>,
    /// usher refuses it with this `error_code` (sub-only and the like).
    pub restricted: Option<String>,
    /// `#EXT-X-ENDLIST` is missing (still live).
    pub live: bool,
}

impl FakeVod {
    pub fn new(id: &str, segments: Vec<FakeSegment>) -> Self {
        Self {
            id: id.to_owned(),
            user_id: "4040".to_owned(),
            login: "made_up_streamer".to_owned(),
            display_name: "Made_Up_Streamer".to_owned(),
            title: "Practice run: all bosses".to_owned(),
            created_at: "2026-11-12T18:00:00Z".to_owned(),
            helix_duration: None,
            helix_muted: Vec::new(),
            segments,
            restricted: None,
            live: false,
        }
    }

    fn duration_ms(&self) -> u64 {
        self.segments.iter().map(|s| s.duration_ms).sum()
    }
}

/// `1h2m3s` for Helix.
pub fn helix_duration(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}h{}m{}s", s / 3600, s / 60 % 60, s % 60)
}

/// A small valid JPEG (16×16 grey), for thumbnails and avatars.
pub fn tiny_jpeg() -> Vec<u8> {
    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, 80);
    encoder
        .encode(&[128u8; 16 * 16 * 3], 16, 16, jpeg_encoder::ColorType::Rgb)
        .expect("encode jpeg");
    out
}

#[derive(Default)]
struct State {
    vods: HashMap<String, FakeVod>,
    requests: Vec<String>,
    /// Segment paths that fail once with 503.
    flaky: HashSet<String>,
    delay: Duration,
    jpeg: Vec<u8>,
}

/// The fake Twitch server; stops when dropped.
pub struct FakeTwitch {
    base: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeTwitch {
    /// Starts on `127.0.0.1` with these VODs.
    pub fn start(vods: Vec<FakeVod>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(Mutex::new(State {
            vods: vods.into_iter().map(|v| (v.id.clone(), v)).collect(),
            jpeg: tiny_jpeg(),
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stop, base) = (Arc::clone(&state), Arc::clone(&stop), base.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (state, base) = (Arc::clone(&state), base.clone());
                            std::thread::spawn(move || {
                                let _ = serve(stream, &state, &base);
                            });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
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

    /// `http://127.0.0.1:<port>`.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// GQL endpoint for `TwitchSource::with_endpoints`.
    pub fn gql_url(&self) -> String {
        format!("{}/gql", self.base)
    }

    /// Helix base URL for `HelixConfig::base_url`.
    pub fn helix_url(&self) -> String {
        format!("{}/helix", self.base)
    }

    /// Delay before every segment answer (makes a kill window).
    pub fn set_segment_delay(&self, delay: Duration) {
        self.lock().delay = delay;
    }

    /// Segment `index` of `vod` answers 503 once.
    pub fn fail_once(&self, vod: &str, index: usize) {
        self.lock().flaky.insert(format!("{vod}/{index}"));
    }

    /// Paths requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.lock().requests.clone()
    }

    /// Segment requests so far.
    pub fn segment_requests(&self) -> usize {
        self.lock()
            .requests
            .iter()
            .filter(|p| p.starts_with("/media/") && !p.ends_with(".m3u8"))
            .count()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for FakeTwitch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn respond(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn query(path: &str, key: &str) -> Option<String> {
    path.split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(str::to_owned))
}

fn serve(mut stream: TcpStream, state: &Mutex<State>, base: &str) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
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
    let path = target.split('?').next().unwrap_or("").to_owned();
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    lock().requests.push(path.clone());

    if method == "POST" && path == "/gql" {
        let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let id = req["variables"]["vodID"].as_str().unwrap_or("").to_owned();
        let known = lock().vods.contains_key(&id);
        let token = known.then(|| {
            json!({"value": json!({"vod_id": id, "made_up": true}).to_string(), "signature": "made-up-signature"})
        });
        let answer = json!({"data": {"videoPlaybackAccessToken": token}});
        return respond(
            &mut stream,
            200,
            "application/json",
            answer.to_string().as_bytes(),
        );
    }
    if let Some(rest) = path.strip_prefix("/vod/") {
        let id = rest.trim_end_matches(".m3u8");
        let vod = lock().vods.get(id).cloned();
        let Some(vod) = vod else {
            return respond(&mut stream, 404, "application/json", b"[]");
        };
        if let Some(code) = &vod.restricted {
            let body = json!([{"error": "Forbidden", "error_code": code}]).to_string();
            return respond(&mut stream, 403, "application/json", body.as_bytes());
        }
        let master = format!(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"160p10 (source)\",AUTOSELECT=YES,DEFAULT=YES\n\
             #EXT-X-STREAM-INF:BANDWIDTH=120000,RESOLUTION=160x90,CODECS=\"avc1.4D400C,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=10.000\n\
             {base}/media/{id}/chunked/index-dvr.m3u8\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"audio_only\",NAME=\"audio_only\",AUTOSELECT=NO,DEFAULT=NO\n\
             #EXT-X-STREAM-INF:BANDWIDTH=40000,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\n\
             {base}/media/{id}/audio_only/index-dvr.m3u8\n"
        );
        return respond(
            &mut stream,
            200,
            "application/vnd.apple.mpegurl",
            master.as_bytes(),
        );
    }
    if let Some(rest) = path.strip_prefix("/media/") {
        let mut it = rest.splitn(3, '/');
        let (id, _group, file) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        let vod = lock().vods.get(id).cloned();
        let Some(vod) = vod else {
            return respond(&mut stream, 404, "text/plain", b"");
        };
        if file == "index-dvr.m3u8" {
            let mut text = format!(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:EVENT\n\
                 #EXT-X-TWITCH-TOTAL-SECS:{:.3}\n",
                vod.duration_ms() as f64 / 1000.0
            );
            for (i, s) in vod.segments.iter().enumerate() {
                let name = if s.muted {
                    format!("{i}-muted.ts")
                } else {
                    format!("{i}.ts")
                };
                text.push_str(&format!(
                    "#EXTINF:{:.3},\n{name}\n",
                    s.duration_ms as f64 / 1000.0
                ));
            }
            if !vod.live {
                text.push_str("#EXT-X-ENDLIST\n");
            }
            return respond(
                &mut stream,
                200,
                "application/vnd.apple.mpegurl",
                text.as_bytes(),
            );
        }
        let index: Option<usize> = file
            .trim_end_matches(".ts")
            .trim_end_matches("-muted")
            .parse()
            .ok();
        let Some(seg) = index.and_then(|i| vod.segments.get(i)) else {
            return respond(&mut stream, 404, "text/plain", b"");
        };
        let (delay, flaky) = {
            let mut s = lock();
            let key = format!("{id}/{}", index.unwrap_or(0));
            (s.delay, s.flaky.remove(&key))
        };
        std::thread::sleep(delay);
        if flaky {
            return respond(&mut stream, 503, "text/plain", b"busy");
        }
        let bytes = seg.bytes()?;
        return respond(&mut stream, 200, "video/mp2t", &bytes);
    }
    if path == "/helix/videos" {
        let id = query(&target, "id").unwrap_or_default();
        let vod = lock().vods.get(&id).cloned();
        let data: Vec<Value> = vod
            .map(|v| {
                let muted = if v.helix_muted.is_empty() {
                    Value::Null
                } else {
                    v.helix_muted
                        .iter()
                        .map(|(o, d)| json!({"offset": o, "duration": d}))
                        .collect()
                };
                let duration = v
                    .helix_duration
                    .clone()
                    .unwrap_or_else(|| helix_duration(v.duration_ms()));
                json!({
                    "id": v.id, "stream_id": format!("9{}", v.id), "user_id": v.user_id,
                    "user_login": v.login, "user_name": v.display_name, "title": v.title,
                    "description": "", "created_at": v.created_at, "published_at": v.created_at,
                    "url": format!("https://www.twitch.tv/videos/{}", v.id),
                    "thumbnail_url": format!("{base}/img/thumb-%{{width}}x%{{height}}.jpg"),
                    "viewable": "public", "view_count": 1, "language": "en", "type": "archive",
                    "duration": duration, "muted_segments": muted,
                })
            })
            .into_iter()
            .collect();
        let body = json!({"data": data, "pagination": {}}).to_string();
        return respond(&mut stream, 200, "application/json", body.as_bytes());
    }
    if path == "/helix/users" {
        let id = query(&target, "id").unwrap_or_default();
        let user = lock().vods.values().find(|v| v.user_id == id).cloned();
        let data: Vec<Value> = user
            .map(|v| {
                json!({"id": v.user_id, "login": v.login, "display_name": v.display_name,
                       "broadcaster_type": "", "description": "", "created_at": v.created_at,
                       "profile_image_url": format!("{base}/img/avatar.jpg"), "offline_image_url": ""})
            })
            .into_iter()
            .collect();
        let body = json!({"data": data}).to_string();
        return respond(&mut stream, 200, "application/json", body.as_bytes());
    }
    if path.starts_with("/img/") {
        let jpeg = lock().jpeg.clone();
        return respond(&mut stream, 200, "image/jpeg", &jpeg);
    }
    respond(&mut stream, 404, "text/plain", b"")
}

/// [`Muxer`] double: concatenates the inputs and reads the durations that
/// [`FakeSegment::synthetic`] wrote into them. Writes `<output>.mux.json`
/// with the ffmetadata text and the attachment names.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConcatMuxer;

impl Muxer for ConcatMuxer {
    fn remux(&self, job: &MuxJob<'_>) -> Result<u64, ArchiveError> {
        let io = |e: std::io::Error| ArchiveError::retry("io", e.to_string());
        let mut out = std::fs::File::create(job.output).map_err(io)?;
        let mut total = 0u64;
        for (n, input) in job.inputs.iter().enumerate() {
            if (job.cancel)() {
                return Err(ArchiveError::retry("cancelled", "cancelled"));
            }
            let bytes = std::fs::read(input).map_err(io)?;
            // One header per segment; a live part file holds many.
            let ms = fake_durations(&bytes)
                .ok_or_else(|| ArchiveError::fatal("remux_failed", "not a fake segment"))?;
            out.write_all(&bytes).map_err(io)?;
            total += ms;
            if n % 50 == 0 {
                (job.progress)(total);
            }
        }
        if let Some(cut) = job.cut_end_ms {
            total = total.min(cut);
        }
        let meta = std::fs::read_to_string(job.metadata).map_err(io)?;
        let record = json!({
            "duration_ms": total,
            "ffmetadata": meta,
            "attachments": job.attachments.iter().map(|a| json!({"name": a.file_name, "mime": a.mime_type, "bytes": a.data.len()})).collect::<Vec<_>>(),
            "container": job.container,
        });
        std::fs::write(sidecar(job.output), record.to_string()).map_err(io)?;
        Ok(total)
    }

    fn probe(&self, path: &Path) -> Result<Probed, ArchiveError> {
        let text = std::fs::read_to_string(sidecar(path))
            .map_err(|e| ArchiveError::fatal("remux_failed", e.to_string()))?;
        let record: Value = serde_json::from_str(&text)
            .map_err(|e| ArchiveError::fatal("remux_failed", e.to_string()))?;
        let meta = record["ffmetadata"].as_str().unwrap_or("");
        let mut tags = BTreeMap::new();
        for line in meta.lines().skip(1) {
            if line.starts_with('[') {
                break;
            }
            if let Some((k, v)) = line.split_once('=') {
                tags.insert(k.to_ascii_lowercase(), v.to_owned());
            }
        }
        Ok(Probed {
            duration_ms: record["duration_ms"].as_u64(),
            chapters: meta.matches("[CHAPTER]").count(),
            attachments: record["attachments"].as_array().map_or(0, Vec::len),
            tags,
        })
    }
}

/// Sum of the durations in the `FAKESEG` headers of `bytes`; `None` if it
/// does not start with one.
fn fake_durations(bytes: &[u8]) -> Option<u64> {
    const TAG: &[u8] = b"FAKESEG ";
    if !bytes.starts_with(TAG) {
        return None;
    }
    let mut total = 0u64;
    let mut at = 0usize;
    while let Some(pos) = bytes[at..].windows(TAG.len()).position(|w| w == TAG) {
        let start = at + pos + TAG.len();
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |e| start + e);
        let line = String::from_utf8_lossy(&bytes[start..end]);
        total += line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        at = end;
    }
    Some(total)
}

fn sidecar(output: &Path) -> PathBuf {
    let mut s = output.as_os_str().to_owned();
    s.push(".mux.json");
    PathBuf::from(s)
}
