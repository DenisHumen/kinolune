// SPDX-License-Identifier: MPL-2.0
//! The `twitch_vod_download` job.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pa_jobs::{JobContext, JobHandler, JobQueue, NetworkPolicy, NewJob, RunOutcome, priority};
use pa_probe::RemuxAttachment;
use pa_storage::{NewItem, StorageManager};
use pa_twitch::Video;
use pa_twitch::chapters::vod_duration_ms;
use pa_twitch::playback::{QualityPreference, Variant};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::bitmap::SegmentBitmap;
use super::fetch::{self, Outcome, RetryPolicy, Task};
use super::meta::{self, Layout};
use super::mux::{Container, MuxJob, Muxer};
use super::playlist::{MediaPlaylist, Selection, TimeRange};
use super::source::{ArchiveError, ArchiveSource};

/// Job types of this crate.
pub mod kinds {
    /// Download one VOD (or a range of it).
    pub const TWITCH_VOD_DOWNLOAD: &str = "twitch_vod_download";
    /// Record a live broadcast (TWI-018, `live::LiveRecorder`).
    pub const TWITCH_LIVE_RECORD: &str = crate::archive::live::TWITCH_LIVE_RECORD;
}

/// Allowed difference between the file's duration and Helix's (06 §4.1:
/// 1–2 %).
pub const DURATION_TOLERANCE: f64 = 0.02;

/// Default parallel segment requests.
pub const DEFAULT_WORKERS: u8 = 6;

/// Checkpoint format version.
const CHECKPOINT_VERSION: u64 = 1;

/// Which rendition to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// The original (`chunked`).
    #[default]
    Source,
    /// The best video variant not taller than this.
    MaxHeight(u32),
    /// Audio only.
    AudioOnly,
}

impl Quality {
    pub(crate) fn preference(self) -> QualityPreference {
        match self {
            Quality::Source => QualityPreference::Source,
            Quality::MaxHeight(h) => QualityPreference::AtMost(h),
            Quality::AudioOnly => QualityPreference::AudioOnly,
        }
    }

    /// Typical bits per second of a Twitch rendition of this quality, for
    /// the space estimate before the playlist is known.
    pub(crate) fn typical_bps(self) -> u64 {
        match self {
            Quality::Source => 8_000_000,
            Quality::MaxHeight(h) if h >= 1080 => 6_000_000,
            Quality::MaxHeight(h) if h >= 720 => 3_500_000,
            Quality::MaxHeight(h) if h >= 480 => 1_500_000,
            Quality::MaxHeight(_) => 700_000,
            Quality::AudioOnly => 160_000,
        }
    }
}

fn default_workers() -> u8 {
    DEFAULT_WORKERS
}

/// Parameters of a download job (`jobs.params_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub video_id: String,
    #[serde(default)]
    pub quality: Quality,
    #[serde(default)]
    pub container: Container,
    /// Only this part of the VOD (a chapter), ms from its start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<TimeRange>,
    /// Name of the part (the chapter's category), for the file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_label: Option<String>,
    /// Parallel segment requests, clamped to 4–8.
    #[serde(default = "default_workers")]
    pub workers: u8,
}

impl DownloadRequest {
    /// The whole VOD in source quality as MKV.
    pub fn vod(video_id: impl Into<String>) -> Self {
        Self {
            video_id: video_id.into().trim_start_matches('v').to_owned(),
            quality: Quality::Source,
            container: Container::Mkv,
            range: None,
            part_label: None,
            workers: DEFAULT_WORKERS,
        }
    }

    /// Bytes the job will write for `duration_ms` of media: the segments
    /// plus the remuxed file, which exist side by side until the commit.
    pub fn estimate_bytes(&self, duration_ms: u64) -> u64 {
        let ms = self.range.map_or(duration_ms, |r| {
            r.end_ms.min(duration_ms).saturating_sub(r.start_ms)
        });
        self.quality.typical_bps() / 8 * ms / 1000 * 2
    }

    /// The queue entry: a job the user waits for, any network, into
    /// storage target `target`. `duration_ms` (from Helix) sizes the space
    /// estimate the storage gate admits by.
    pub fn to_job(&self, target: &str, duration_ms: Option<u64>) -> NewJob {
        let params = serde_json::to_value(self).unwrap_or(Value::Null);
        let mut job = NewJob::new(kinds::TWITCH_VOD_DOWNLOAD, params)
            .priority(priority::USER_WAITING)
            .network(NetworkPolicy::Any)
            .max_attempts(8)
            .target_storage(target);
        if let Some(ms) = duration_ms.filter(|ms| *ms > 0) {
            job = job.bytes_total(self.estimate_bytes(ms));
        }
        job
    }
}

/// What identifies the segment list a bitmap belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PlanKey {
    video_id: String,
    variant: String,
    first: usize,
    count: usize,
    duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checkpoint {
    v: u64,
    plan: PlanKey,
    bitmap: String,
    segment_bytes: u64,
}

/// The segment part of one run.
#[derive(Clone, Copy)]
struct Segments<'a> {
    playlist: &'a MediaPlaylist,
    sel: &'a Selection,
    plan: &'a PlanKey,
    bandwidth: u64,
    dir: &'a Path,
    workers: usize,
}

/// Why a run stopped early.
enum Stop {
    Interrupted,
    Error(ArchiveError),
}

impl From<ArchiveError> for Stop {
    fn from(e: ArchiveError) -> Self {
        Stop::Error(e)
    }
}

fn io(e: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::retry("io", e.to_string())
}

fn storage(e: pa_storage::Error) -> ArchiveError {
    match e {
        pa_storage::Error::NoTarget(t) => ArchiveError::fatal(
            "no_storage_target",
            format!("storage target {t:?} not found"),
        ),
        other => ArchiveError::retry("storage", other.to_string()),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn image_mime(data: &[u8]) -> Option<(&'static str, &'static str)> {
    if data.starts_with(&[0xFF, 0xD8]) {
        Some(("image/jpeg", "jpg"))
    } else if data.starts_with(b"\x89PNG") {
        Some(("image/png", "png"))
    } else {
        None
    }
}

/// The handler of [`kinds::TWITCH_VOD_DOWNLOAD`] (see the crate docs).
pub struct VodDownloader {
    source: Arc<dyn ArchiveSource>,
    storage: StorageManager,
    muxer: Arc<dyn Muxer>,
    retry: RetryPolicy,
    checkpoint_every: Duration,
}

impl VodDownloader {
    pub fn new(
        source: Arc<dyn ArchiveSource>,
        storage: StorageManager,
        muxer: Arc<dyn Muxer>,
    ) -> Self {
        Self {
            source,
            storage,
            muxer,
            retry: RetryPolicy::default(),
            checkpoint_every: Duration::from_secs(2),
        }
    }

    /// Per-segment retries.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// How often the bitmap is saved while segments arrive (also saved on
    /// every stop).
    pub fn with_checkpoint_interval(mut self, every: Duration) -> Self {
        self.checkpoint_every = every;
        self
    }

    /// Registers this handler for [`kinds::TWITCH_VOD_DOWNLOAD`].
    pub fn register(self, queue: &JobQueue) {
        queue.register(kinds::TWITCH_VOD_DOWNLOAD, self);
    }

    fn download(&self, ctx: &JobContext) -> Result<Value, Stop> {
        let req: DownloadRequest = serde_json::from_value(ctx.params().clone())
            .map_err(|e| ArchiveError::fatal("bad_params", e.to_string()))?;
        let target = ctx
            .job()
            .target_storage
            .clone()
            .ok_or_else(|| ArchiveError::fatal("no_storage_target", "the job has no target"))?;
        self.source.check_allowed()?;

        // What to download.
        let video = self.source.video(&req.video_id)?;
        let helix_ms = vod_duration_ms(&video);
        let master = self.source.master_playlist(&video.id)?;
        let variant = master
            .select(req.quality.preference())
            .cloned()
            .ok_or_else(|| ArchiveError::fatal("no_variant", "no rendition of this quality"))?;
        let playlist = self.media_playlist(&variant)?;
        if !playlist.complete && req.range.is_none_or(|r| r.end_ms > playlist.duration_ms()) {
            return Err(ArchiveError::fatal(
                "vod_live",
                "the broadcast is still live: the VOD is not complete yet",
            )
            .into());
        }
        let sel = playlist
            .select(req.range)
            .map_err(|e| ArchiveError::fatal("bad_range", e.to_string()))?;

        // Segments.
        let tmp = self
            .storage
            .job_tmp_dir(&target, ctx.job().id)
            .map_err(storage)?;
        let seg_dir = tmp.join("seg");
        let plan = PlanKey {
            video_id: video.id.clone(),
            variant: variant.group_id.clone(),
            first: sel.first,
            count: sel.count,
            duration_ms: sel.duration_ms(),
        };
        let (present, segment_bytes) = self.fetch_segments(
            ctx,
            &Segments {
                playlist: &playlist,
                sel: &sel,
                plan: &plan,
                bandwidth: variant.bandwidth,
                dir: &seg_dir,
                workers: usize::from(req.workers),
            },
        )?;

        // Assets, metadata, remux, check, commit.
        let layout = Layout::new(&video, req.part_label.as_deref());
        let out_dir = tmp.join("out").join(&layout.base);
        if out_dir.exists() {
            fs::remove_dir_all(&out_dir).map_err(io)?;
        }
        fs::create_dir_all(&out_dir).map_err(io)?;
        let mut notes: Vec<String> = Vec::new();
        let attachments = self.assets(&video, &out_dir, &layout.base, &mut notes);
        let chat = self.chat(&video, &out_dir, &layout.base, &mut notes);

        let chapters = self.source.chapters(&video);
        let out_chapters = meta::output_chapters(&chapters, &sel);
        let categories = meta::categories(&chapters);
        let muted = merge(playlist.muted_ranges(&sel), helix_muted(&video, &sel));
        let meta_path = tmp.join("meta.ffmetadata");
        fs::write(
            &meta_path,
            meta::ffmetadata(
                &video,
                &out_chapters,
                &categories,
                &muted,
                req.part_label.as_deref(),
            ),
        )
        .map_err(io)?;

        let mut inputs = Vec::with_capacity(sel.count + 1);
        if playlist.init_url.is_some() {
            inputs.push(seg_dir.join("init.mp4"));
        }
        inputs.extend((0..sel.count).map(|i| seg_path(&seg_dir, sel.first + i)));
        let output = out_dir.join(format!("{}.{}", layout.base, req.container.extension()));
        let expected_ms = if req.range.is_some() || helix_ms == 0 {
            sel.duration_ms()
        } else {
            helix_ms
        };
        let total = segment_bytes.saturating_mul(2);
        let progress = |ms: u64| {
            let share = segment_bytes.saturating_mul(ms.min(expected_ms)) / expected_ms.max(1);
            ctx.report_bytes(segment_bytes + share, Some(total));
        };
        let stop = || ctx.stop_requested();
        let written = self.muxer.remux(&MuxJob {
            inputs: &inputs,
            output: &output,
            container: req.container,
            metadata: &meta_path,
            attachments: &attachments,
            cut_end_ms: sel.cut_end_ms,
            fold_gaps: false,
            cancel: &stop,
            progress: &progress,
        });
        let written_ms = match written {
            Err(_) if ctx.stop_requested() => return Err(Stop::Interrupted),
            other => other?,
        };
        let probed = self.muxer.probe(&output)?;
        let got_ms = probed.duration_ms.unwrap_or(written_ms);
        let diff = got_ms.abs_diff(expected_ms) as f64;
        if diff > expected_ms as f64 * DURATION_TOLERANCE {
            return Err(ArchiveError::fatal(
                "duration_mismatch",
                format!(
                    "the file is {got_ms} ms long, Twitch says {expected_ms} ms (more than {} % apart)",
                    DURATION_TOLERANCE * 100.0
                ),
            )
            .into());
        }

        let info = json!({
            "format": "kinolune.twitch.vod/1",
            "video": video,
            "chapters": out_chapters.iter().map(|(s, e, t)| json!({"start_ms": s, "end_ms": e, "title": t})).collect::<Vec<_>>(),
            "chapter_sources": chapters.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(),
            "muted": muted,
            "range": req.range.map(|_| json!({"start_ms": sel.start_ms, "end_ms": sel.end_ms, "requested": req.range, "label": req.part_label})),
            "variant": {
                "group_id": variant.group_id, "quality": variant.quality, "is_source": variant.is_source,
                "bandwidth": variant.bandwidth, "resolution": variant.resolution, "codecs": variant.codecs,
            },
            "segments": {
                "count": sel.count, "muted": (sel.first..sel.first + sel.count).filter(|&i| playlist.segments[i].muted).count(),
                "discontinuities": playlist.discontinuities, "bytes": segment_bytes,
            },
            "file": {
                "name": output.file_name().and_then(|n| n.to_str()),
                "container": req.container, "duration_ms": got_ms, "expected_ms": expected_ms,
                "helix_duration_ms": helix_ms, "chapters": probed.chapters, "attachments": probed.attachments,
            },
            "chat": chat,
            "notes": notes,
            "downloaded_at_ms": now_ms(),
        });
        fs::write(
            out_dir.join(format!("{}.info.json", layout.base)),
            serde_json::to_vec_pretty(&info).map_err(io)?,
        )
        .map_err(io)?;
        if ctx.is_cancelled() {
            return Err(Stop::Interrupted);
        }

        let item = self.commit(&out_dir, &layout, &target, &video, ctx)?;
        let _ = fs::remove_dir_all(&tmp);
        Ok(json!({
            "target": target,
            "item_id": item.id,
            "rel_path": item.rel_path,
            "file": output.file_name().and_then(|n| n.to_str()),
            "duration_ms": got_ms,
            "expected_ms": expected_ms,
            "helix_duration_ms": helix_ms,
            "segments": sel.count,
            "segments_present_at_start": present,
            "muted": muted,
            "chapters": out_chapters.len(),
            "bytes": item.size_bytes,
        }))
    }

    fn media_playlist(&self, variant: &Variant) -> Result<MediaPlaylist, ArchiveError> {
        let response = self.source.get(&variant.url)?;
        match response.status {
            200 => {}
            s if s >= 500 || s == 429 => {
                return Err(ArchiveError::retry(
                    "playlist_unavailable",
                    format!("media playlist answered {s}"),
                ));
            }
            s => {
                return Err(ArchiveError::fatal(
                    "playlist_refused",
                    format!("media playlist answered {s}"),
                ));
            }
        }
        let text = String::from_utf8_lossy(&response.body);
        MediaPlaylist::parse(&text, &variant.url)
            .map_err(|e| ArchiveError::fatal("bad_playlist", e.to_string()))
    }

    /// Downloads the missing segments. Returns how many were already
    /// present when the run started and the bytes of all segments.
    fn fetch_segments(&self, ctx: &JobContext, job: &Segments<'_>) -> Result<(usize, u64), Stop> {
        let Segments {
            playlist,
            sel,
            plan,
            bandwidth,
            dir: seg_dir,
            workers,
        } = *job;
        let saved: Option<Checkpoint> = ctx
            .checkpoint()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .filter(|c: &Checkpoint| c.v == CHECKPOINT_VERSION);
        let mut bitmap = match &saved {
            Some(c) if c.plan == *plan => SegmentBitmap::from_hex(&c.bitmap, sel.count)
                .unwrap_or_else(|| SegmentBitmap::new(sel.count)),
            Some(_) => {
                // Another rendition or range: the old segments are useless.
                let _ = fs::remove_dir_all(seg_dir);
                SegmentBitmap::new(sel.count)
            }
            None => SegmentBitmap::new(sel.count),
        };
        fs::create_dir_all(seg_dir).map_err(io)?;
        // Files renamed into place after the last checkpoint are complete.
        let mut bytes = 0u64;
        for i in 0..sel.count {
            match fs::metadata(seg_path(seg_dir, sel.first + i)) {
                Ok(m) if m.len() > 0 => {
                    bitmap.set(i);
                    bytes += m.len();
                }
                // In the bitmap but gone (tmp cleaned by hand): fetch again.
                _ => bitmap.clear(i),
            }
        }
        let present = bitmap.count();
        if let Some(init) = &playlist.init_url {
            let dest = seg_dir.join("init.mp4");
            if !dest.exists() {
                let r = self.source.get(init)?;
                if r.status != 200 || r.body.is_empty() {
                    return Err(ArchiveError::retry(
                        "segment_failed",
                        format!("init segment answered {}", r.status),
                    )
                    .into());
                }
                fetch::write_atomic(&dest, &r.body).map_err(io)?;
            }
        }

        let estimate = |done: usize, bytes: u64| -> u64 {
            let by_rate = bandwidth / 8 * sel.duration_ms() / 1000;
            let by_avg = if done > 0 {
                bytes / done as u64 * sel.count as u64
            } else {
                by_rate
            };
            by_avg.max(bytes) * 2
        };
        let save = |bitmap: &SegmentBitmap, bytes: u64| -> Result<bool, ArchiveError> {
            let c = Checkpoint {
                v: CHECKPOINT_VERSION,
                plan: plan.clone(),
                bitmap: bitmap.to_hex(),
                segment_bytes: bytes,
            };
            ctx.save_checkpoint(&serde_json::to_value(&c).map_err(io)?, bytes)
                .map_err(|e| ArchiveError::retry("jobs", e.to_string()))
        };
        if !save(&bitmap, bytes)? {
            return Err(Stop::Interrupted);
        }
        ctx.report_bytes(bytes, Some(estimate(bitmap.count(), bytes)));

        let tasks: Vec<Task> = bitmap
            .missing()
            .into_iter()
            .map(|i| Task {
                index: i,
                url: playlist.segments[sel.first + i].url.clone(),
                dest: seg_path(seg_dir, sel.first + i),
            })
            .collect();
        let mut last_save = Instant::now();
        let mut save_error: Option<ArchiveError> = None;
        let outcome = fetch::fetch_all(
            &*self.source,
            tasks,
            workers,
            self.retry,
            &|| ctx.stop_requested(),
            &mut |i, b| {
                bitmap.set(i);
                bytes += b;
                ctx.report_bytes(bytes, Some(estimate(bitmap.count(), bytes)));
                if last_save.elapsed() >= self.checkpoint_every {
                    last_save = Instant::now();
                    match save(&bitmap, bytes) {
                        Ok(running) => return running,
                        Err(e) => {
                            save_error = Some(e);
                            return false;
                        }
                    }
                }
                true
            },
        );
        if ctx.is_cancelled() {
            return Err(Stop::Interrupted);
        }
        // Every way out saves the bitmap.
        let running = save(&bitmap, bytes)?;
        if let Some(e) = save_error {
            return Err(e.into());
        }
        match outcome {
            Outcome::Complete if bitmap.is_complete() => Ok((present, bytes)),
            Outcome::Failed(e) => Err(e.into()),
            _ if !running || ctx.stop_requested() => Err(Stop::Interrupted),
            _ => {
                Err(ArchiveError::retry("segment_failed", "segments missing after the pass").into())
            }
        }
    }

    /// Thumbnail and avatar: `-thumb.jpg` and attachments. Best effort.
    fn assets(
        &self,
        video: &Video,
        out_dir: &Path,
        base: &str,
        notes: &mut Vec<String>,
    ) -> Vec<RemuxAttachment> {
        let mut out = Vec::new();
        let mut image = |url: &str, what: &str| -> Option<Vec<u8>> {
            match self.source.get(url) {
                Ok(r) if r.status == 200 && image_mime(&r.body).is_some() => Some(r.body),
                Ok(r) => {
                    notes.push(format!("{what}: HTTP {} or not an image", r.status));
                    None
                }
                Err(e) => {
                    notes.push(format!("{what}: {e}"));
                    None
                }
            }
        };
        if let Some(thumb) = video
            .thumbnail(1280, 720)
            .and_then(|u| image(&u, "thumbnail"))
        {
            let (mime, ext) = image_mime(&thumb).unwrap_or(("image/jpeg", "jpg"));
            if ext == "jpg" {
                let _ = fs::write(out_dir.join(format!("{base}-thumb.jpg")), &thumb);
            }
            out.push(RemuxAttachment {
                file_name: format!("cover.{ext}"),
                mime_type: mime.to_owned(),
                data: thumb,
            });
        }
        let avatar = match self.source.avatar_url(&video.user_id) {
            Ok(Some(url)) => image(&url, "avatar"),
            Ok(None) => None,
            Err(e) => {
                notes.push(format!("avatar: {e}"));
                None
            }
        };
        if let Some(avatar) = avatar {
            let (mime, ext) = image_mime(&avatar).unwrap_or(("image/jpeg", "jpg"));
            out.push(RemuxAttachment {
                file_name: format!("avatar.{ext}"),
                mime_type: mime.to_owned(),
                data: avatar,
            });
        }
        out
    }

    /// `.chat.json.gz` from the chat replay hook (TWI-016), if any.
    fn chat(&self, video: &Video, out_dir: &Path, base: &str, notes: &mut Vec<String>) -> Value {
        match self.source.chat_replay(video) {
            Ok(Some(json)) => {
                let path = out_dir.join(format!("{base}.chat.json.gz"));
                let written = fs::File::create(&path).and_then(|f| {
                    let mut gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
                    gz.write_all(&json)?;
                    gz.finish().map(|_| ())
                });
                match written {
                    Ok(()) => json!({"file": format!("{base}.chat.json.gz"), "bytes": json.len()}),
                    Err(e) => {
                        notes.push(format!("chat: {e}"));
                        Value::Null
                    }
                }
            }
            Ok(None) => Value::Null,
            Err(e) => {
                notes.push(format!("chat: {e}"));
                Value::Null
            }
        }
    }

    fn commit(
        &self,
        out_dir: &Path,
        layout: &Layout,
        target: &str,
        video: &Video,
        ctx: &JobContext,
    ) -> Result<pa_storage::Item, ArchiveError> {
        let mut layout = layout.clone();
        for n in 2..100u32 {
            let mut item = NewItem::new(target, layout.folder.clone())
                .group(format!("twitch:{}", video.user_id));
            item.profile_id = ctx.job().owner_profile_id;
            match self.storage.commit_file(out_dir, &item, Some(ctx.job().id)) {
                Err(pa_storage::Error::Exists(_)) => layout = layout.numbered(n),
                other => return other.map_err(storage),
            }
        }
        Err(ArchiveError::fatal(
            "exists",
            "this VOD was downloaded too many times",
        ))
    }
}

impl JobHandler for VodDownloader {
    fn run(&self, ctx: &JobContext) -> RunOutcome {
        match self.download(ctx) {
            Ok(result) => RunOutcome::Done(Some(result)),
            Err(Stop::Interrupted) => RunOutcome::Interrupted,
            Err(Stop::Error(e)) if e.retryable => RunOutcome::Retryable(e.to_string()),
            Err(Stop::Error(e)) => RunOutcome::Fatal(e.to_string()),
        }
    }
}

fn seg_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("{index:06}.seg"))
}

/// Helix's muted ranges (seconds of the VOD) mapped onto the output.
fn helix_muted(video: &Video, sel: &Selection) -> Vec<TimeRange> {
    video
        .muted_segments
        .iter()
        .filter_map(|m| {
            let start = (m.offset * 1000).max(sel.start_ms);
            let end = ((m.offset + m.duration) * 1000).min(sel.end_ms);
            (end > start).then(|| TimeRange {
                start_ms: start - sel.start_ms,
                end_ms: end - sel.start_ms,
            })
        })
        .collect()
}

/// Union of two sets of ranges, sorted and merged.
fn merge(mut a: Vec<TimeRange>, b: Vec<TimeRange>) -> Vec<TimeRange> {
    a.extend(b);
    a.sort_by_key(|r| r.start_ms);
    let mut out: Vec<TimeRange> = Vec::new();
    for r in a {
        match out.last_mut() {
            Some(p) if p.end_ms >= r.start_ms => p.end_ms = p.end_ms.max(r.end_ms),
            _ => out.push(r),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_json_and_estimate() {
        let mut r = DownloadRequest::vod("v123");
        assert_eq!(r.video_id, "123");
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["quality"], "source");
        assert_eq!(v["container"], "mkv");
        let back: DownloadRequest = serde_json::from_value(json!({"video_id": "123"})).unwrap();
        assert_eq!(back, r);
        // 1 hour of source: 3.6 GB of segments, twice that with the output.
        assert_eq!(r.estimate_bytes(3_600_000), 7_200_000_000);
        r.quality = Quality::MaxHeight(720);
        r.range = Some(TimeRange {
            start_ms: 0,
            end_ms: 1_800_000,
        });
        assert_eq!(
            serde_json::to_value(&r).unwrap()["quality"],
            json!({"max_height": 720})
        );
        assert_eq!(r.estimate_bytes(3_600_000), 1_575_000_000);
        let job = r.to_job("downloads", Some(3_600_000));
        assert_eq!(job.kind, kinds::TWITCH_VOD_DOWNLOAD);
        assert_eq!(job.target_storage.as_deref(), Some("downloads"));
        assert_eq!(job.bytes_total, Some(1_575_000_000));
    }

    #[test]
    fn ranges_merge() {
        let r = |s, e| TimeRange {
            start_ms: s,
            end_ms: e,
        };
        assert_eq!(
            merge(vec![r(0, 10), r(30, 40)], vec![r(5, 20), r(50, 60)]),
            vec![r(0, 20), r(30, 40), r(50, 60)]
        );
    }
}
