// SPDX-License-Identifier: MPL-2.0
//! Finding saved Twitch files on disk (TWI-019, research 06 §4.3): our own
//! downloads and recordings, and archives made by other tools.
//!
//! [`scan`] walks a folder and pairs every video file with what describes
//! it, best first:
//!
//! 1. our `.info.json` (`kinolune.twitch.vod/1`, `kinolune.twitch.live/1`);
//! 2. a yt-dlp `.info.json` (`extractor` `twitch:*`, `webpage_url`);
//! 3. a streamlink `--json` dump (`plugin: twitch`, `metadata.id`);
//! 4. a TwitchDownloader chat JSON (`.json`, `.json.gz`: `streamer`,
//!    `video.id`, `video.chapters` with category ids, `comments`);
//! 5. the id in the file name: `[v123]`, `v123`, `[s123]`, `[c-Slug]`.
//!
//! A sidecar belongs to a video when their names match without the
//! sidecar suffixes (`.info.json`, `.chat.json.gz`, `_chat.json`…), when
//! the sidecar's id appears in the video's name, or when the folder holds
//! only that one video. Chat JSON of any kind becomes the item's chat log.
//!
//! Nothing here touches the database: [`super::library`] matches the
//! findings into the TWI-007 model.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Video containers we import.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "ts", "flv", "webm", "mov", "m4v", "m4a", "aac", "mp3",
];

/// Largest sidecar we read, bytes (chat logs of long broadcasts are tens
/// of MB).
const MAX_SIDECAR: u64 = 512 * 1024 * 1024;

/// What a saved file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundKind {
    Vod,
    Recording,
    Clip,
}

/// Where the description of a file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// Our own `.info.json`.
    Kinolune,
    YtDlp,
    Streamlink,
    TwitchDownloader,
    /// Only the id in the file name.
    FileName,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Kinolune => "kinolune",
            Origin::YtDlp => "yt_dlp",
            Origin::Streamlink => "streamlink",
            Origin::TwitchDownloader => "twitch_downloader",
            Origin::FileName => "file_name",
        }
    }
}

/// A chapter of an imported file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundChapter {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    /// Twitch category id, when the tool saved it (TwitchDownloader).
    pub game_id: Option<String>,
    /// Category name (or chapter title).
    pub name: String,
}

/// A chat log next to a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundChat {
    pub path: PathBuf,
    pub message_count: u64,
    pub first_offset_ms: Option<u64>,
    pub last_offset_ms: Option<u64>,
    /// Emotes were embedded (TwitchDownloader `--embed-images`).
    pub emote_snapshot: bool,
}

/// One saved file and what we know about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The video file.
    pub video: PathBuf,
    pub kind: FoundKind,
    pub video_id: Option<String>,
    pub stream_id: Option<String>,
    pub clip_slug: Option<String>,
    pub user_id: Option<String>,
    pub login: Option<String>,
    pub display_name: Option<String>,
    pub title: String,
    /// When it was broadcast or created, Unix ms.
    pub at_ms: Option<i64>,
    pub duration_ms: Option<u64>,
    pub chapters: Vec<FoundChapter>,
    pub chat: Option<FoundChat>,
    /// The sidecar that described it.
    pub info: Option<PathBuf>,
    pub origin: Origin,
    pub quality: Option<String>,
    /// Ad breaks are marked (our recordings).
    pub has_ads_marked: bool,
    /// Part label of a partial download (our `.info.json` `range.label`).
    pub part: Option<String>,
}

impl Found {
    /// The id this file is deduplicated by (06 §4.3): `video_id`,
    /// `stream_id` or `clip_slug`.
    pub fn key(&self) -> Option<&str> {
        match self.kind {
            FoundKind::Vod => self.video_id.as_deref(),
            FoundKind::Recording => self.stream_id.as_deref(),
            FoundKind::Clip => self.clip_slug.as_deref(),
        }
    }
}

/// A video file we could not identify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    /// `no_id`.
    pub reason: &'static str,
}

/// What a sidecar says.
#[derive(Debug, Clone, Default)]
struct Meta {
    path: PathBuf,
    /// Name without the sidecar suffix, lower-case.
    stem: String,
    origin: Option<Origin>,
    kind: Option<FoundKind>,
    video_id: Option<String>,
    stream_id: Option<String>,
    clip_slug: Option<String>,
    user_id: Option<String>,
    login: Option<String>,
    display_name: Option<String>,
    title: Option<String>,
    at_ms: Option<i64>,
    duration_ms: Option<u64>,
    chapters: Vec<FoundChapter>,
    chat: Option<FoundChat>,
    /// File name of the video, if the sidecar names it.
    file: Option<String>,
    quality: Option<String>,
    has_ads_marked: bool,
    part: Option<String>,
}

impl Meta {
    fn ids(&self) -> Vec<&str> {
        [&self.video_id, &self.stream_id, &self.clip_slug]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn is_json(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".json") || n.ends_with(".json.gz")
}

/// Name without sidecar suffixes: `X.info.json`, `X.chat.json.gz`,
/// `X_chat.json`, `X - chat.json` → `x`.
fn sidecar_stem(name: &str) -> String {
    let mut n = name.to_ascii_lowercase();
    for suffix in [".gz", ".json"] {
        if let Some(s) = n.strip_suffix(suffix) {
            n = s.to_owned();
        }
    }
    for suffix in [
        ".info",
        ".chat",
        "_chat",
        "-chat",
        " chat",
        " - chat",
        ".live_chat",
    ] {
        if let Some(s) = n.strip_suffix(suffix) {
            n = s.trim_end().to_owned();
            break;
        }
    }
    n
}

fn video_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn read_json(path: &Path) -> Option<Value> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > MAX_SIDECAR {
        return None;
    }
    let raw = fs::read(path).ok()?;
    let bytes = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&raw[..])
            .take(MAX_SIDECAR)
            .read_to_end(&mut out)
            .ok()?;
        out
    } else {
        raw
    };
    serde_json::from_slice(&bytes).ok()
}

fn digits(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches('v');
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0'))
        .then(|| s.to_owned())
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn secs_ms(v: &Value) -> Option<u64> {
    v.as_f64()
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(|s| (s * 1000.0).round() as u64)
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Ids in a Twitch URL: `/videos/123`, `clips.twitch.tv/Slug`, `/clip/Slug`.
fn url_ids(url: &str) -> (Option<String>, Option<String>) {
    let u = url.split(['?', '#']).next().unwrap_or("");
    let segs: Vec<&str> = u.split('/').filter(|s| !s.is_empty()).collect();
    for (i, s) in segs.iter().enumerate() {
        if (*s == "videos" || *s == "video") && i + 1 < segs.len() {
            return (digits(segs[i + 1]), None);
        }
        if *s == "clip" && i + 1 < segs.len() && is_slug(segs[i + 1]) {
            return (None, Some(segs[i + 1].to_owned()));
        }
    }
    if u.contains("clips.twitch.tv")
        && let Some(last) = segs.last().filter(|l| is_slug(l) && !l.contains('.'))
    {
        return (None, Some((*last).to_owned()));
    }
    (None, None)
}

/// Statistics of a TwitchDownloader-style `comments` array.
fn chat_stats(path: &Path, v: &Value) -> Option<FoundChat> {
    let comments = v.get("comments")?.as_array()?;
    let offsets: Vec<u64> = comments
        .iter()
        .filter_map(|c| c.get("content_offset_seconds").and_then(secs_ms))
        .collect();
    Some(FoundChat {
        path: path.to_owned(),
        message_count: comments.len() as u64,
        first_offset_ms: offsets.iter().min().copied(),
        last_offset_ms: offsets.iter().max().copied(),
        emote_snapshot: v.get("embeddedData").is_some_and(|e| !e.is_null())
            || v.get("emotes").is_some_and(|e| !e.is_null()),
    })
}

fn our_chapters(v: &Value) -> Vec<FoundChapter> {
    v.get("chapters")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    Some(FoundChapter {
                        start_ms: c.get("start_ms")?.as_u64()?,
                        end_ms: c.get("end_ms").and_then(Value::as_u64),
                        game_id: c.get("game_id").and_then(text),
                        name: c.get("title").and_then(text).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parses one sidecar; `None` when it is not about Twitch.
fn parse_sidecar(path: &Path) -> Option<Meta> {
    let name = path.file_name()?.to_str()?;
    let v = read_json(path)?;
    let mut m = Meta {
        path: path.to_owned(),
        stem: sidecar_stem(name),
        ..Meta::default()
    };
    let format = v.get("format").and_then(Value::as_str).unwrap_or("");
    if format == "kinolune.twitch.vod/1" {
        let video = v.get("video")?;
        m.origin = Some(Origin::Kinolune);
        m.kind = Some(FoundKind::Vod);
        m.video_id = video.get("id").and_then(text).and_then(|s| digits(&s));
        m.stream_id = video
            .get("stream_id")
            .and_then(text)
            .and_then(|s| digits(&s));
        m.user_id = video.get("user_id").and_then(text);
        m.login = video.get("user_login").and_then(text);
        m.display_name = video.get("user_name").and_then(text);
        m.title = video.get("title").and_then(text);
        m.at_ms = video
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(pa_twitch::parse_rfc3339_ms);
        m.chapters = our_chapters(&v);
        m.part = v.pointer("/range/label").and_then(text);
        m.quality = v.pointer("/variant/quality").and_then(text);
        m.duration_ms = v.pointer("/file/duration_ms").and_then(Value::as_u64);
        m.file = v.pointer("/file/name").and_then(text);
        return Some(m);
    }
    if format == "kinolune.twitch.live/1" {
        let stream = v.get("stream")?;
        m.origin = Some(Origin::Kinolune);
        m.kind = Some(FoundKind::Recording);
        m.stream_id = stream
            .get("stream_id")
            .and_then(text)
            .and_then(|s| digits(&s));
        m.user_id = stream.get("user_id").and_then(text);
        m.login = stream.get("login").and_then(text);
        m.display_name = stream.get("display_name").and_then(text);
        m.title = stream.get("title").and_then(text);
        m.at_ms = stream
            .get("started_at_ms")
            .and_then(Value::as_i64)
            .filter(|t| *t > 0);
        m.chapters = our_chapters(&v);
        m.quality = v.pointer("/variant/quality").and_then(text);
        m.duration_ms = v.pointer("/file/duration_ms").and_then(Value::as_u64);
        m.file = v.pointer("/file/name").and_then(text);
        m.has_ads_marked = v
            .get("ads")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        return Some(m);
    }
    // yt-dlp.
    let extractor = v
        .get("extractor")
        .or_else(|| v.get("extractor_key"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if extractor.contains("twitch") {
        m.origin = Some(Origin::YtDlp);
        let url = v
            .get("webpage_url")
            .or_else(|| v.get("original_url"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let (url_video, url_clip) = url_ids(url);
        let id = v.get("id").and_then(text).unwrap_or_default();
        if extractor.contains("clip") || url_clip.is_some() {
            m.kind = Some(FoundKind::Clip);
            m.clip_slug = url_clip.or_else(|| {
                v.get("display_id")
                    .and_then(text)
                    .filter(|s| is_slug(s) && digits(s).is_none())
            });
        } else if extractor.contains("stream") || extractor.contains("live") {
            m.kind = Some(FoundKind::Recording);
            m.stream_id = digits(&id);
        } else {
            m.kind = Some(FoundKind::Vod);
            m.video_id = url_video.or_else(|| digits(&id));
        }
        m.login = v
            .get("uploader_id")
            .or_else(|| v.get("channel_id"))
            .and_then(text)
            .filter(|s| digits(s).is_none())
            .map(|s| s.to_ascii_lowercase());
        m.display_name = v
            .get("uploader")
            .or_else(|| v.get("channel"))
            .and_then(text);
        m.title = v.get("title").or_else(|| v.get("fulltitle")).and_then(text);
        m.at_ms = v
            .get("timestamp")
            .and_then(Value::as_f64)
            .map(|t| (t * 1000.0) as i64)
            .or_else(|| {
                let d = v.get("upload_date")?.as_str()?;
                (d.len() == 8).then(|| {
                    pa_twitch::parse_rfc3339_ms(&format!(
                        "{}-{}-{}T00:00:00Z",
                        &d[..4],
                        &d[4..6],
                        &d[6..]
                    ))
                })?
            });
        m.duration_ms = v.get("duration").and_then(secs_ms);
        m.chapters = v
            .get("chapters")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        Some(FoundChapter {
                            start_ms: c.get("start_time").and_then(secs_ms)?,
                            end_ms: c.get("end_time").and_then(secs_ms),
                            game_id: None,
                            name: c.get("title").and_then(text).unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        m.quality = v.get("format_id").and_then(text);
        m.file = v
            .get("_filename")
            .or_else(|| v.get("filename"))
            .and_then(Value::as_str)
            .and_then(|f| Path::new(f).file_name()?.to_str().map(str::to_owned));
        return Some(m);
    }
    // streamlink --json.
    if v.get("plugin").and_then(Value::as_str) == Some("twitch") {
        let md = v.get("metadata").cloned().unwrap_or(Value::Null);
        m.origin = Some(Origin::Streamlink);
        let url = v
            .get("url")
            .or_else(|| v.pointer("/args/url"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let (url_video, url_clip) = url_ids(url);
        let id = md.get("id").and_then(text).unwrap_or_default();
        if let Some(slug) = url_clip {
            m.kind = Some(FoundKind::Clip);
            m.clip_slug = Some(slug);
        } else if url_video.is_some() {
            m.kind = Some(FoundKind::Vod);
            m.video_id = url_video;
        } else {
            m.kind = Some(FoundKind::Recording);
            m.stream_id = digits(&id);
        }
        let author = md.get("author").and_then(text);
        m.login = author.as_ref().map(|a| a.to_ascii_lowercase());
        m.display_name = author;
        m.title = md.get("title").and_then(text);
        if let Some(cat) = md.get("category").and_then(text) {
            m.chapters.push(FoundChapter {
                start_ms: 0,
                end_ms: None,
                game_id: None,
                name: cat,
            });
        }
        return Some(m);
    }
    // TwitchDownloader chat JSON (also our `.chat.json.gz`).
    if v.get("comments").is_some_and(Value::is_array) {
        m.chat = chat_stats(path, &v);
        let video = v.get("video").cloned().unwrap_or(Value::Null);
        let streamer = v.get("streamer").cloned().unwrap_or(Value::Null);
        let id = video.get("id").and_then(text);
        if id.is_some() || streamer.is_object() {
            m.origin = Some(Origin::TwitchDownloader);
        }
        match id {
            Some(id) if digits(&id).is_some() => {
                m.kind = Some(FoundKind::Vod);
                m.video_id = digits(&id);
            }
            Some(id) if is_slug(&id) => {
                m.kind = Some(FoundKind::Clip);
                m.clip_slug = Some(id);
            }
            _ => {}
        }
        m.user_id = video
            .get("user_id")
            .and_then(text)
            .or_else(|| streamer.get("id").and_then(text))
            .and_then(|s| digits(&s));
        m.login = streamer
            .get("login")
            .and_then(text)
            .map(|s| s.to_ascii_lowercase());
        m.display_name = streamer
            .get("name")
            .and_then(text)
            .or_else(|| video.get("user_name").and_then(text));
        if m.login.is_none() {
            m.login = m.display_name.as_ref().map(|s| s.to_ascii_lowercase());
        }
        m.title = video.get("title").and_then(text);
        m.at_ms = video
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(pa_twitch::parse_rfc3339_ms);
        m.duration_ms = video
            .get("length")
            .and_then(secs_ms)
            .or_else(|| {
                let s = video.get("start").and_then(Value::as_f64)?;
                let e = video.get("end").and_then(Value::as_f64)?;
                (e > s).then_some(((e - s) * 1000.0) as u64)
            })
            .filter(|d| *d > 0);
        m.chapters = video
            .get("chapters")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        let start = c.get("startMilliseconds")?.as_u64()?;
                        let len = c.get("lengthMilliseconds").and_then(Value::as_u64);
                        Some(FoundChapter {
                            start_ms: start,
                            end_ms: len.filter(|l| *l > 0).map(|l| start + l),
                            game_id: c.get("gameId").and_then(text).and_then(|g| digits(&g)),
                            name: c
                                .get("gameDisplayName")
                                .or_else(|| c.get("description"))
                                .and_then(text)
                                .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Some(m);
    }
    None
}

/// Ids in a file name: `[v123]` or `v123` (VOD), `[s123]` (recording),
/// `[c-Slug]` (clip).
pub fn ids_in_name(name: &str) -> Option<(FoundKind, String)> {
    let bracketed = |open: &str| -> Option<String> {
        let at = name.rfind(open)?;
        let rest = &name[at + open.len()..];
        let end = rest.find(']')?;
        Some(rest[..end].to_owned())
    };
    if let Some(id) = bracketed("[v").and_then(|s| digits(&s)) {
        return Some((FoundKind::Vod, id));
    }
    if let Some(id) = bracketed("[s").and_then(|s| digits(&s)) {
        return Some((FoundKind::Recording, id));
    }
    if let Some(slug) = bracketed("[c-").filter(|s| is_slug(s)) {
        return Some((FoundKind::Clip, slug));
    }
    // yt-dlp without brackets, or `v123.mp4`: `v` + 6 or more digits as a
    // token.
    let b = name.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'v' && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let n = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            let after = b.get(i + 1 + n);
            if n >= 6
                && after.is_none_or(|c| !c.is_ascii_alphanumeric())
                && let Some(id) = digits(&name[i + 1..i + 1 + n])
            {
                return Some((FoundKind::Vod, id));
            }
        }
    }
    None
}

/// Whether `id` appears in `name` as a whole token.
fn has_token(name: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(at) = name[from..].find(id) {
        let start = from + at;
        let end = start + id.len();
        let before = name[..start].chars().next_back();
        let after = name[end..].chars().next();
        let sep = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric() || c == 'v');
        if sep(before) && after.is_none_or(|c| !c.is_ascii_alphanumeric()) {
            return true;
        }
        from = end;
    }
    false
}

fn merge(found: &mut Found, m: &Meta) {
    let better = m.origin.is_some_and(|o| o < found.origin);
    macro_rules! fill {
        ($f:ident) => {
            if found.$f.is_none() || (better && m.$f.is_some()) {
                if m.$f.is_some() {
                    found.$f = m.$f.clone();
                }
            }
        };
    }
    if better {
        fill!(video_id);
        fill!(stream_id);
        fill!(clip_slug);
    } else {
        // A weaker sidecar (a chat log next to our `.info.json`) only
        // fills the item's own id: a recording's chat must not make it a
        // VOD.
        match found.kind {
            FoundKind::Vod => fill!(video_id),
            FoundKind::Recording => fill!(stream_id),
            FoundKind::Clip => fill!(clip_slug),
        }
    }
    fill!(user_id);
    fill!(login);
    fill!(display_name);
    fill!(at_ms);
    fill!(duration_ms);
    fill!(quality);
    fill!(part);
    if let Some(t) = &m.title
        && (found.title.is_empty() || better)
    {
        found.title = t.clone();
    }
    // Chapters with category ids win over names only.
    let ids = |c: &[FoundChapter]| c.iter().filter(|c| c.game_id.is_some()).count();
    if !m.chapters.is_empty()
        && (found.chapters.is_empty() || ids(&m.chapters) > ids(&found.chapters))
    {
        found.chapters = m.chapters.clone();
    }
    if found.chat.is_none() {
        found.chat = m.chat.clone();
    }
    found.has_ads_marked |= m.has_ads_marked;
    if let Some(origin) = m.origin
        && origin < found.origin
    {
        found.origin = origin;
        found.info = Some(m.path.clone());
        if let Some(kind) = m.kind {
            found.kind = kind;
        }
    }
}

fn scan_dir(dir: &Path, out: &mut Vec<Found>, skipped: &mut Vec<Skipped>, depth: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut videos: Vec<PathBuf> = Vec::new();
    let mut sidecars: Vec<PathBuf> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        // `.tmp`, `.trash` of storage targets, `.part` downloads.
        if name.starts_with('.') || name.ends_with(".part") {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            dirs.push(path);
        } else if ft.is_file() {
            if is_video(&path) {
                videos.push(path);
            } else if is_json(&name) {
                sidecars.push(path);
            }
        }
    }
    videos.sort();
    sidecars.sort();
    dirs.sort();
    let metas: Vec<Meta> = sidecars.iter().filter_map(|p| parse_sidecar(p)).collect();
    let mut used: HashSet<usize> = HashSet::new();
    for video in &videos {
        let file_name = video
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        let stem = video_stem(video);
        let mut matched: Vec<usize> = (0..metas.len())
            .filter(|&i| {
                let m = &metas[i];
                m.stem == stem
                    || m.file.as_deref() == Some(file_name.as_str())
                    || m.ids().iter().any(|id| has_token(&file_name, id))
            })
            .collect();
        if matched.is_empty() && videos.len() == 1 {
            matched = (0..metas.len()).filter(|i| !used.contains(i)).collect();
        }
        let from_name = ids_in_name(&file_name);
        let mut found = Found {
            video: video.clone(),
            kind: from_name.as_ref().map_or(FoundKind::Vod, |(k, _)| *k),
            video_id: None,
            stream_id: None,
            clip_slug: None,
            user_id: None,
            login: None,
            display_name: None,
            title: String::new(),
            at_ms: None,
            duration_ms: None,
            chapters: Vec::new(),
            chat: None,
            info: None,
            origin: Origin::FileName,
            quality: None,
            has_ads_marked: false,
            part: None,
        };
        match &from_name {
            Some((FoundKind::Vod, id)) => found.video_id = Some(id.clone()),
            Some((FoundKind::Recording, id)) => found.stream_id = Some(id.clone()),
            Some((FoundKind::Clip, id)) => found.clip_slug = Some(id.clone()),
            None => {}
        }
        // Best description first.
        matched.sort_by_key(|&i| metas[i].origin.unwrap_or(Origin::FileName));
        for &i in &matched {
            // A sidecar about another item (its id differs) is not ours.
            let m = &metas[i];
            let conflicts = found
                .key()
                .is_some_and(|k| !m.ids().is_empty() && !m.ids().contains(&k))
                && found.origin != Origin::FileName;
            if conflicts {
                continue;
            }
            used.insert(i);
            merge(&mut found, m);
        }
        if found.key().is_none() {
            skipped.push(Skipped {
                path: video.clone(),
                reason: "no_id",
            });
            continue;
        }
        if found.title.is_empty() {
            found.title = video_stem_title(video);
        }
        out.push(found);
    }
    if depth < 12 {
        for d in dirs {
            scan_dir(&d, out, skipped, depth + 1);
        }
    }
}

/// The file name without extension and id suffix, as a title of last
/// resort.
fn video_stem_title(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Twitch")
        .to_owned();
    match stem.rfind(" [") {
        Some(at) if stem.ends_with(']') && at > 0 => stem[..at].to_owned(),
        _ => stem,
    }
}

/// Finds saved Twitch files under `root` (recursively, hidden folders
/// such as `.tmp` and `.trash` skipped). Returns the files and the video
/// files that could not be identified.
pub fn scan(root: &Path) -> (Vec<Found>, Vec<Skipped>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    scan_dir(root, &mut out, &mut skipped, 0);
    (out, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_from_names() {
        assert_eq!(
            ids_in_name("2026-11-12 Title [v2290012345].mkv"),
            Some((FoundKind::Vod, "2290012345".to_owned()))
        );
        assert_eq!(
            ids_in_name("Title v2290012345.mp4"),
            Some((FoundKind::Vod, "2290012345".to_owned()))
        );
        assert_eq!(
            ids_in_name("2026-11-13 LIVE Chat [s51234567890].mkv"),
            Some((FoundKind::Recording, "51234567890".to_owned()))
        );
        assert_eq!(
            ids_in_name("Clip [c-FunnySlug_1-x].mp4"),
            Some((FoundKind::Clip, "FunnySlug_1-x".to_owned()))
        );
        assert_eq!(ids_in_name("review v12.mp4"), None);
        assert_eq!(ids_in_name("movie.mkv"), None);
    }

    #[test]
    fn stems_and_tokens() {
        assert_eq!(sidecar_stem("A b.info.json"), "a b");
        assert_eq!(sidecar_stem("A b.chat.json.gz"), "a b");
        assert_eq!(sidecar_stem("A b_chat.json"), "a b");
        assert!(has_token("[2026-11-12] x - 123456789.mp4", "123456789"));
        assert!(has_token("x v123456789.mp4", "123456789"));
        assert!(!has_token("x 1234567890.mp4", "123456789"));
        assert_eq!(
            url_ids("https://www.twitch.tv/videos/2290012345?t=1"),
            (Some("2290012345".to_owned()), None)
        );
        assert_eq!(
            url_ids("https://clips.twitch.tv/FunnySlug"),
            (None, Some("FunnySlug".to_owned()))
        );
        assert_eq!(
            url_ids("https://www.twitch.tv/someone/clip/Funny-Slug"),
            (None, Some("Funny-Slug".to_owned()))
        );
    }
}
