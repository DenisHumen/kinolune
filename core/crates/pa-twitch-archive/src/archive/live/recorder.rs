// SPDX-License-Identifier: MPL-2.0
//! The `twitch_live_record` job.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use pa_jobs::{JobContext, JobHandler, JobQueue, NetworkPolicy, NewJob, RunOutcome, priority};
use pa_probe::RemuxAttachment;
use pa_storage::{NewItem, StorageManager};
use pa_twitch::chapters::LiveTimeline;
use pa_twitch::fill_image_template;
use pa_twitch::playback::Variant;
use pa_twitch::playback::live::{MediaPlaylist, format_date_time, resolve_uri};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::journal::{AdEntry, CatEntry, GapEntry, Journal};
use super::source::{LiveChat, LiveClock, LiveSource, LiveStream, SystemClock};
use super::timeline::RecTimeline;
use crate::archive::job::{DURATION_TOLERANCE, Quality};
use crate::archive::meta::{date_of, escape};
use crate::archive::naming::{NameVars, Naming};
use crate::archive::mux::{Container, MuxJob, Muxer};
use crate::archive::source::ArchiveError;

/// Job type of live recordings.
pub const TWITCH_LIVE_RECORD: &str = "twitch_live_record";

/// Format of the `.info.json` of a recording.
pub const INFO_FORMAT: &str = "kinolune.twitch.live/1";

const CHECKPOINT_VERSION: u64 = 1;

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

fn yes() -> bool {
    true
}

fn default_part_minutes() -> u32 {
    60
}

/// Parameters of a recording (`jobs.params_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveRequest {
    /// Broadcaster id.
    pub user_id: String,
    /// Login, if known (else from Helix).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// The broadcast this job is for (from `stream.online`), if known. A
    /// different broadcast found later ends this recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(default)]
    pub quality: Quality,
    #[serde(default)]
    pub container: Container,
    /// Record the chat alongside (TWI-010).
    #[serde(default = "yes")]
    pub with_chat: bool,
    /// Media per part file, minutes (06 §4.2: 1–2 hours).
    #[serde(default = "default_part_minutes")]
    pub part_minutes: u32,
    /// File name templates (TWI-019); the defaults of 06 §4.3 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub naming: Option<Naming>,
}

impl LiveRequest {
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            login: None,
            stream_id: None,
            quality: Quality::Source,
            container: Container::Mkv,
            with_chat: true,
            part_minutes: default_part_minutes(),
            naming: None,
        }
    }

    /// The queue entry: priority [`priority::LIVE`] (ignores the type's
    /// concurrency limit; first on low space, CORE-012), any network, on
    /// battery too, into storage target `target`.
    pub fn to_job(&self, target: &str) -> NewJob {
        let params = serde_json::to_value(self).unwrap_or(Value::Null);
        NewJob::new(TWITCH_LIVE_RECORD, params)
            .priority(priority::LIVE)
            .network(NetworkPolicy::Any)
            .allow_on_battery(true)
            .max_attempts(50)
            .target_storage(target)
            .bytes_total(self.quality.typical_bps() / 8 * 3600)
    }
}

/// Timings of the recorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveSettings {
    /// Shortest pause between two playlist requests.
    pub poll_min: Duration,
    /// How long usher must report the channel offline before the recording
    /// ends (a stream that drops for a minute continues in the same file).
    pub offline_grace: Duration,
    /// How long network errors may last before the recording is finalised
    /// with what it has.
    pub max_outage: Duration,
    /// No new segment for this long: ask usher again (token expired,
    /// broadcast ended without `#EXT-X-ENDLIST`).
    pub stale_after: Duration,
    /// How often the remote-config flag and Helix are checked.
    pub recheck_every: Duration,
    /// How often the job checkpoint is saved.
    pub checkpoint_every: Duration,
    /// Attempts per segment before it is given up as a gap.
    pub segment_attempts: u32,
    /// `fsync` parts and the journal after every segment.
    pub fsync: bool,
}

impl Default for LiveSettings {
    fn default() -> Self {
        Self {
            poll_min: Duration::from_millis(500),
            offline_grace: Duration::from_secs(90),
            max_outage: Duration::from_secs(30 * 60),
            stale_after: Duration::from_secs(30),
            recheck_every: Duration::from_secs(60),
            checkpoint_every: Duration::from_secs(10),
            segment_attempts: 4,
            fsync: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Checkpoint {
    v: u64,
    segments: usize,
    last_seq: Option<u64>,
    bytes: u64,
    /// Set when the run stopped on request (pause): the next run notes the
    /// missed segments as `paused`, else as `restart`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stopped: Option<String>,
    /// Gaps so far (the "gap" indicator of the recordings list, TWI-019).
    #[serde(default, skip_serializing_if = "is_zero")]
    gaps: usize,
    /// Media recorded so far, ms.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    media_ms: u64,
}

/// Why recording ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// usher reports the channel offline (after the grace period).
    Offline,
    /// `#EXT-X-ENDLIST`.
    Endlist,
    /// Helix reports another broadcast of the channel.
    NewBroadcast,
    /// Network errors for longer than `max_outage`.
    Network,
    /// The remote config turned archiving off.
    Disallowed,
    /// usher refused the broadcast (`usher_403`) after recording began.
    Restricted,
}

impl End {
    fn as_str(self) -> &'static str {
        match self {
            End::Offline => "offline",
            End::Endlist => "endlist",
            End::NewBroadcast => "new_broadcast",
            End::Network => "network",
            End::Disallowed => "archive_disabled",
            End::Restricted => "usher_403",
        }
    }
}

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

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// State of one run between playlist polls.
struct Run<'a> {
    ctx: &'a JobContext,
    req: &'a LiveRequest,
    journal: Journal,
    stream: Option<LiveStream>,
    login: String,
    media_url: Option<String>,
    variant: Option<Variant>,
    chat: Option<Box<dyn LiveChat>>,
    /// Reason for the next gap (set when the run starts after a pause).
    pending_gap: Option<&'static str>,
    edge_pdt: Option<i64>,
    outage_since: Option<i64>,
    offline_since: Option<i64>,
    last_new_segment: i64,
    last_recheck: i64,
    last_checkpoint: i64,
    notes: Vec<String>,
}

/// The handler of [`TWITCH_LIVE_RECORD`]: follows the live media playlist
/// and appends every new complete segment — ad segments included, as
/// Twitch serves them — to part files, records the chat alongside, notes
/// gaps, and on the end of the broadcast remuxes everything into one MKV
/// (or MP4) with chapters and `TWITCH_*` tags.
pub struct LiveRecorder {
    source: Arc<dyn LiveSource>,
    storage: StorageManager,
    muxer: Arc<dyn Muxer>,
    clock: Arc<dyn LiveClock>,
    settings: LiveSettings,
}

impl LiveRecorder {
    pub fn new(
        source: Arc<dyn LiveSource>,
        storage: StorageManager,
        muxer: Arc<dyn Muxer>,
    ) -> Self {
        Self {
            source,
            storage,
            muxer,
            clock: Arc::new(SystemClock),
            settings: LiveSettings::default(),
        }
    }

    /// Time source (virtual time in tests).
    pub fn with_clock(mut self, clock: Arc<dyn LiveClock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_settings(mut self, settings: LiveSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Registers this handler for [`TWITCH_LIVE_RECORD`].
    pub fn register(self, queue: &JobQueue) {
        queue.register(TWITCH_LIVE_RECORD, self);
    }

    fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    fn record(&self, ctx: &JobContext) -> Result<Value, Stop> {
        let req: LiveRequest = serde_json::from_value(ctx.params().clone())
            .map_err(|e| ArchiveError::fatal("bad_params", e.to_string()))?;
        let target = ctx
            .job()
            .target_storage
            .clone()
            .ok_or_else(|| ArchiveError::fatal("no_storage_target", "the job has no target"))?;
        let tmp = self
            .storage
            .job_tmp_dir(&target, ctx.job().id)
            .map_err(storage)?;
        let journal = Journal::open(&tmp, self.settings.fsync).map_err(io)?;
        let saved: Option<Checkpoint> = ctx
            .checkpoint()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .filter(|c: &Checkpoint| c.v == CHECKPOINT_VERSION);
        let resumed = !journal.segs.is_empty();
        let pending_gap = resumed.then(|| {
            if saved.and_then(|c| c.stopped).is_some() {
                "paused"
            } else {
                "restart"
            }
        });
        let now = self.now();
        let mut run = Run {
            ctx,
            req: &req,
            login: req.login.clone().unwrap_or_default(),
            stream: journal.meta.as_ref().map(|m| m.0.clone()),
            journal,
            media_url: None,
            variant: None,
            chat: None,
            pending_gap,
            edge_pdt: None,
            outage_since: None,
            offline_since: None,
            last_new_segment: now,
            last_recheck: i64::MIN,
            last_checkpoint: now,
            notes: Vec::new(),
        };
        if run.login.is_empty()
            && let Some(s) = &run.stream
        {
            run.login = s.login.clone();
        }
        let end = match self.follow(&mut run) {
            Ok(end) => end,
            Err(Stop::Interrupted) => {
                if let Some(chat) = run.chat.as_mut() {
                    chat.close();
                }
                let _ = run.journal.close_part();
                self.checkpoint(&run, Some("interrupted"));
                return Err(Stop::Interrupted);
            }
            Err(Stop::Error(e)) => {
                if let Some(chat) = run.chat.as_mut() {
                    chat.close();
                }
                if run.journal.segs.is_empty() || e.retryable {
                    let _ = run.journal.close_part();
                    return Err(e.into());
                }
                // A permanent refusal after recording began: keep what we
                // have.
                run.notes.push(e.to_string());
                if e.code == "usher_403" {
                    End::Restricted
                } else {
                    End::Disallowed
                }
            }
        };
        if let Some(chat) = run.chat.as_mut() {
            chat.close();
        }
        // Chat that arrived until now.
        self.drain_chat(&mut run);
        run.journal.close_part().map_err(io)?;
        if run.journal.segs.is_empty() {
            let _ = fs::remove_dir_all(&tmp);
            return Err(ArchiveError::fatal(
                "offline",
                format!(
                    "the channel went offline before anything was recorded ({})",
                    end.as_str()
                ),
            )
            .into());
        }
        self.finalize(&mut run, &target, &tmp, end)
    }

    fn checkpoint(&self, run: &Run<'_>, stopped: Option<&str>) -> bool {
        let c = Checkpoint {
            v: CHECKPOINT_VERSION,
            segments: run.journal.segs.len(),
            last_seq: run.journal.last_seq(),
            bytes: run.journal.bytes,
            stopped: stopped.map(str::to_owned),
            gaps: run.journal.gaps.len(),
            media_ms: run.journal.segs.iter().map(|s| s.dur_ms).sum(),
        };
        let value = serde_json::to_value(&c).unwrap_or(Value::Null);
        run.ctx
            .save_checkpoint(&value, run.journal.bytes)
            .unwrap_or(true)
    }

    fn drain_chat(&self, run: &mut Run<'_>) {
        let Some(chat) = run.chat.as_mut() else {
            return;
        };
        for line in chat.drain() {
            if let Err(e) = run.journal.chat(line, run.edge_pdt) {
                run.notes.push(format!("chat: {e}"));
                break;
            }
        }
    }

    /// Checks the flag and Helix (metadata, categories, a new broadcast).
    fn recheck(&self, run: &mut Run<'_>) -> Result<Option<End>, Stop> {
        let now = self.now();
        if run.last_recheck != i64::MIN && now - run.last_recheck < ms(self.settings.recheck_every)
        {
            return Ok(None);
        }
        run.last_recheck = now;
        if let Err(e) = self.source.check_allowed() {
            if run.journal.segs.is_empty() {
                return Err(e.into());
            }
            run.notes.push(e.to_string());
            return Ok(Some(End::Disallowed));
        }
        match self.source.stream(&run.req.user_id) {
            Ok(Some(s)) => {
                let expected = run
                    .stream
                    .as_ref()
                    .map(|k| k.stream_id.clone())
                    .or_else(|| run.req.stream_id.clone());
                if let Some(expected) = expected
                    && expected != s.stream_id
                    && !run.journal.segs.is_empty()
                {
                    return Ok(Some(End::NewBroadcast));
                }
                if run.login.is_empty() {
                    run.login = s.login.clone();
                }
                let cat = CatEntry {
                    at: now,
                    game_id: s.game_id.clone(),
                    game_name: s.game_name.clone(),
                    title: s.title.clone(),
                };
                run.journal.category(cat).map_err(io)?;
                let variant = run.variant.as_ref().map_or("", |v| v.group_id.as_str());
                let quality = run.variant.as_ref().map_or("", |v| v.quality.as_str());
                run.journal
                    .set_meta(s.clone(), variant, quality)
                    .map_err(io)?;
                if run.chat.is_none() && run.req.with_chat {
                    run.chat = self.source.chat(&s);
                }
                run.stream = Some(s);
            }
            // Helix lags behind usher (cache): usher decides when it ends.
            Ok(None) => {}
            Err(e) if e.retryable => {}
            Err(e) => run.notes.push(format!("helix: {e}")),
        }
        Ok(None)
    }

    fn follow(&self, run: &mut Run<'_>) -> Result<End, Stop> {
        let mut backoff = Duration::from_secs(1);
        loop {
            if run.ctx.stop_requested() {
                return Err(Stop::Interrupted);
            }
            if let Some(end) = self.recheck(run)? {
                return Ok(end);
            }
            if run.login.is_empty() {
                return Err(ArchiveError::retry(
                    "no_login",
                    "the channel login is unknown (Helix unavailable)",
                )
                .into());
            }
            let now = self.now();
            if let Some(since) = run.outage_since
                && now - since >= ms(self.settings.max_outage)
            {
                run.notes.push(format!(
                    "network errors for {} s: finalised with what was recorded",
                    (now - since) / 1000
                ));
                if run.journal.segs.is_empty() {
                    return Err(ArchiveError::retry("network", "no network").into());
                }
                return Ok(End::Network);
            }
            if now - run.last_new_segment >= ms(self.settings.stale_after) {
                run.media_url = None;
                run.last_new_segment = now;
            }
            // usher: the live master playlist.
            if run.media_url.is_none() {
                match self.source.master_playlist(&run.login) {
                    Ok(Some(master)) => {
                        let variant = master
                            .select(run.req.quality.preference())
                            .cloned()
                            .ok_or_else(|| {
                                ArchiveError::fatal("no_variant", "no rendition of this quality")
                            })?;
                        run.outage_since = None;
                        run.offline_since = None;
                        run.media_url = Some(variant.url.clone());
                        run.variant = Some(variant);
                    }
                    Ok(None) => {
                        run.outage_since = None;
                        let since = *run.offline_since.get_or_insert(now);
                        if now - since >= ms(self.settings.offline_grace) {
                            return Ok(End::Offline);
                        }
                        self.wait(run, Duration::from_secs(5))?;
                        continue;
                    }
                    Err(e) if e.retryable => {
                        run.outage_since.get_or_insert(now);
                        self.wait(run, backoff)?;
                        backoff = (backoff * 2).min(Duration::from_secs(4));
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            let Some(url) = run.media_url.clone() else {
                continue;
            };
            // The media playlist.
            let playlist = match self.source.get(&url) {
                Ok(r) if r.status == 200 => {
                    MediaPlaylist::parse(&String::from_utf8_lossy(&r.body)).ok()
                }
                Ok(r) if matches!(r.status, 403 | 404 | 410) => {
                    // Token expired or the broadcast ended: ask usher.
                    run.media_url = None;
                    continue;
                }
                Ok(_) | Err(_) => None,
            };
            let Some(playlist) = playlist else {
                run.outage_since.get_or_insert(now);
                self.wait(run, backoff)?;
                backoff = (backoff * 2).min(Duration::from_secs(4));
                continue;
            };
            backoff = Duration::from_secs(1);
            run.outage_since = None;
            run.offline_since = None;
            self.take_segments(run, &playlist, &url)?;
            self.drain_chat(run);
            if playlist.ended {
                return Ok(End::Endlist);
            }
            let now = self.now();
            if now - run.last_checkpoint >= ms(self.settings.checkpoint_every) {
                run.last_checkpoint = now;
                if !self.checkpoint(run, None) {
                    return Err(Stop::Interrupted);
                }
            }
            let poll = Duration::from_secs_f64((playlist.target_duration / 2.0).clamp(0.5, 5.0))
                .max(self.settings.poll_min);
            self.wait(run, poll)?;
        }
    }

    /// Sleeps `d`, waking up for a stop request.
    fn wait(&self, run: &Run<'_>, d: Duration) -> Result<(), Stop> {
        let step = Duration::from_millis(500);
        let mut left = d;
        while !left.is_zero() {
            if run.ctx.stop_requested() {
                return Err(Stop::Interrupted);
            }
            let s = left.min(step);
            self.clock.sleep(s);
            left -= s;
        }
        Ok(())
    }

    fn take_segments(
        &self,
        run: &mut Run<'_>,
        playlist: &MediaPlaylist,
        url: &str,
    ) -> Result<(), Stop> {
        for b in playlist.ad_breaks() {
            run.journal
                .ad(AdEntry {
                    id: b.id,
                    start: b.start,
                    duration: b.duration,
                })
                .map_err(io)?;
        }
        run.edge_pdt = playlist.edge().or(run.edge_pdt);
        let part_ms = u64::from(run.req.part_minutes.max(1)) * 60_000;
        let target_ms = (playlist.target_duration * 1000.0) as u64;
        // Only complete segments: prefetch ones are still being encoded.
        let segments: Vec<_> = playlist.complete().cloned().collect();
        for s in segments {
            if run
                .journal
                .last_seq()
                .is_some_and(|last| s.sequence <= last)
            {
                continue;
            }
            if run.ctx.stop_requested() {
                return Err(Stop::Interrupted);
            }
            let now = self.now();
            // A jump in the recorded sequence is a discontinuity too.
            let mut disc = s.discontinuity
                || run
                    .journal
                    .segs
                    .last()
                    .is_some_and(|p| s.sequence != p.seq + 1);
            if let Some(last) = run.journal.last_seq()
                && s.sequence > last + 1
            {
                let from_pdt = run.journal.last_end_pdt();
                let missing_ms = match (from_pdt, s.program_date_time) {
                    (Some(a), Some(b)) if b > a => u64::try_from(b - a).unwrap_or(0),
                    _ => (s.sequence - last - 1) * target_ms,
                };
                let reason = run.pending_gap.take().unwrap_or("window");
                run.journal
                    .gap(GapEntry {
                        from_seq: last + 1,
                        to_seq: s.sequence,
                        from_pdt,
                        to_pdt: s.program_date_time,
                        missing_ms,
                        reason: reason.to_owned(),
                        at: now,
                    })
                    .map_err(io)?;
                disc = true;
            }
            run.pending_gap = None;
            let seg_url = resolve_uri(url, &s.uri);
            match self.fetch_segment(run, &seg_url)? {
                Some(body) => {
                    let dur_ms = (s.duration * 1000.0).round() as u64;
                    run.journal
                        .append(
                            s.sequence,
                            dur_ms,
                            s.program_date_time,
                            s.ad,
                            disc,
                            &body,
                            part_ms,
                            now,
                        )
                        .map_err(io)?;
                    run.last_new_segment = self.now();
                    let done = run.journal.bytes;
                    let rate = run
                        .variant
                        .as_ref()
                        .map_or(run.req.quality.typical_bps(), |v| v.bandwidth.max(1));
                    run.ctx.report_bytes(done, Some(done + rate / 8 * 3600));
                }
                None => {
                    // Given up: a one-segment gap, the edge goes on.
                    run.journal
                        .gap(GapEntry {
                            from_seq: s.sequence,
                            to_seq: s.sequence + 1,
                            from_pdt: s.program_date_time,
                            to_pdt: s.end_date_time(),
                            missing_ms: (s.duration * 1000.0).round() as u64,
                            reason: "segment_failed".to_owned(),
                            at: now,
                        })
                        .map_err(io)?;
                }
            }
        }
        Ok(())
    }

    fn fetch_segment(&self, run: &Run<'_>, url: &str) -> Result<Option<Vec<u8>>, Stop> {
        let mut delay = Duration::from_millis(250);
        for attempt in 0..self.settings.segment_attempts.max(1) {
            if attempt > 0 {
                self.wait(run, delay)?;
                delay *= 2;
            }
            match self.source.get(url) {
                Ok(r) if r.status == 200 && !r.body.is_empty() => {
                    let declared = r
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.trim().parse::<usize>().ok());
                    if declared.is_none_or(|n| n == r.body.len()) {
                        return Ok(Some(r.body));
                    }
                }
                Ok(r) if matches!(r.status, 403 | 404 | 410) => return Ok(None),
                Ok(_) | Err(_) => {}
            }
        }
        Ok(None)
    }

    fn finalize(
        &self,
        run: &mut Run<'_>,
        target: &str,
        tmp: &Path,
        end: End,
    ) -> Result<Value, Stop> {
        let ctx = run.ctx;
        let req = run.req;
        let journal = &run.journal;
        let stream = run.stream.clone().unwrap_or_else(|| LiveStream {
            stream_id: req.stream_id.clone().unwrap_or_default(),
            user_id: req.user_id.clone(),
            login: run.login.clone(),
            display_name: run.login.clone(),
            ..LiveStream::default()
        });
        let tl = RecTimeline::new(&journal.segs);
        let expected_ms = tl.total_ms();

        // Chapters: the TWI-008 timeline, else what Helix showed us.
        let timeline = (!stream.stream_id.is_empty())
            .then(|| self.source.timeline(&stream.stream_id))
            .flatten();
        let chapters = chapters(&tl, timeline.as_ref(), &journal.cats);
        let categories: Vec<String> = {
            let mut out: Vec<String> = Vec::new();
            for (_, _, name) in &chapters {
                if !name.is_empty() && !out.contains(name) {
                    out.push(name.clone());
                }
            }
            out
        };
        let ads = tl.ad_ranges(&journal.segs);
        let gaps = tl.gaps(&journal.gaps, &journal.segs);
        let missing_ms: u64 = journal.gaps.iter().map(|g| g.missing_ms).sum();

        // Names (TWI-019 templates): by default
        // `<login> [<user_id>]/<year>/<date> LIVE <categories> — <title> [s<id>]`.
        let start_pdt = tl.start_pdt().unwrap_or(stream.started_at_ms);
        let started = if stream.started_at_ms > 0 {
            stream.started_at_ms
        } else {
            start_pdt
        };
        let date_text = format_date_time(started);
        let date = date_of(&date_text).unwrap_or("0000-00-00").to_owned();
        let id = if stream.stream_id.is_empty() {
            format!(" [s{}-{}]", req.user_id, ctx.job().id)
        } else {
            format!(" [s{}]", stream.stream_id)
        };
        let title = if stream.title.trim().is_empty() {
            "Twitch live"
        } else {
            stream.title.trim()
        };
        let login = if stream.login.is_empty() {
            run.login.clone()
        } else {
            stream.login.clone()
        };
        let vars = NameVars {
            at: date_text.clone(),
            login: login.clone(),
            display_name: stream.display_name.clone(),
            user_id: req.user_id.clone(),
            title: title.to_owned(),
            categories: categories.clone(),
            part: None,
            id: stream.stream_id.clone(),
            kind: "live",
        };
        let layout = req
            .naming
            .clone()
            .unwrap_or_default()
            .layout(&vars, &id);
        let (base, folder) = (layout.base, layout.folder);

        let out_dir = tmp.join("out").join(&base);
        if out_dir.exists() {
            fs::remove_dir_all(&out_dir).map_err(io)?;
        }
        fs::create_dir_all(&out_dir).map_err(io)?;

        // Cover: the stream thumbnail, best effort.
        let mut attachments = Vec::new();
        if !stream.thumbnail_url.is_empty() {
            let url = fill_image_template(&stream.thumbnail_url, 1280, 720);
            match self.source.get(&url) {
                Ok(r) if r.status == 200 && r.body.starts_with(&[0xFF, 0xD8]) => {
                    let _ = fs::write(out_dir.join(format!("{base}-thumb.jpg")), &r.body);
                    attachments.push(RemuxAttachment {
                        file_name: "cover.jpg".to_owned(),
                        mime_type: "image/jpeg".to_owned(),
                        data: r.body,
                    });
                }
                Ok(r) => run.notes.push(format!("thumbnail: HTTP {}", r.status)),
                Err(e) => run.notes.push(format!("thumbnail: {e}")),
            }
        }

        // Chat.
        let chat = self.write_chat(run, &tl, &stream, &out_dir, &base);

        // Tags and chapters.
        let journal = &run.journal;
        let meta_path = tmp.join("meta.ffmetadata");
        let mut tags: Vec<(&str, String)> = vec![
            ("title", stream.title.clone()),
            ("artist", stream.display_name.clone()),
            ("comment", format!("https://www.twitch.tv/{login}")),
            ("date", date.clone()),
            ("TWITCH_USER_ID", req.user_id.clone()),
            ("TWITCH_LOGIN", login.clone()),
            ("TWITCH_VIDEO_TYPE", "live_recording".to_owned()),
        ];
        if !stream.stream_id.is_empty() {
            tags.push(("TWITCH_STREAM_ID", stream.stream_id.clone()));
        }
        if stream.started_at_ms > 0 {
            tags.push(("TWITCH_STARTED_AT", format_date_time(stream.started_at_ms)));
        }
        if let (Some(a), Some(b)) = (tl.start_pdt(), tl.end_pdt()) {
            tags.push(("TWITCH_RECORDED_FROM", format_date_time(a)));
            tags.push(("TWITCH_RECORDED_TO", format_date_time(b)));
        }
        if !categories.is_empty() {
            tags.push(("TWITCH_CATEGORIES", categories.join(", ")));
        }
        let ranges = |r: &[(u64, u64)]| {
            r.iter()
                .map(|(s, e)| format!("{s}-{e}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        if !ads.is_empty() {
            tags.push(("TWITCH_ADS_MS", ranges(&ads)));
        }
        if !gaps.is_empty() {
            // At offset, how much is missing there.
            let g: Vec<(u64, u64)> = gaps.clone();
            tags.push((
                "TWITCH_GAPS_MS",
                g.iter()
                    .map(|(at, len)| format!("{at}+{len}"))
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
        let mut text = String::from(";FFMETADATA1\n");
        for (k, v) in &tags {
            if !v.is_empty() {
                text.push_str(&format!("{}={}\n", escape(k), escape(v)));
            }
        }
        for (start, end, title) in &chapters {
            text.push_str(&format!(
                "\n[CHAPTER]\nTIMEBASE=1/1000\nSTART={start}\nEND={end}\ntitle={}\n",
                escape(title)
            ));
        }
        fs::write(&meta_path, text).map_err(io)?;

        // Remux: one file, timestamps folded across discontinuities and
        // gaps.
        let inputs = journal.parts();
        let output = out_dir.join(format!("{base}.{}", req.container.extension()));
        let bytes = journal.bytes;
        let progress = |ms: u64| {
            let share = bytes.saturating_mul(ms.min(expected_ms)) / expected_ms.max(1);
            ctx.report_bytes(bytes + share, Some(bytes * 2));
        };
        let cancel = || ctx.is_cancelled();
        let written_ms = self.muxer.remux(&MuxJob {
            inputs: &inputs,
            output: &output,
            container: req.container,
            metadata: &meta_path,
            attachments: &attachments,
            cut_end_ms: None,
            fold_gaps: true,
            cancel: &cancel,
            progress: &progress,
        })?;
        let probed = self.muxer.probe(&output)?;
        let got_ms = probed.duration_ms.unwrap_or(written_ms);
        if got_ms.abs_diff(expected_ms) as f64 > expected_ms as f64 * DURATION_TOLERANCE {
            // Hours of a broadcast are not thrown away for this: noted.
            run.notes.push(format!(
                "duration_mismatch: the file is {got_ms} ms, the segments {expected_ms} ms"
            ));
        }

        let journal = &run.journal;
        let info = json!({
            "format": INFO_FORMAT,
            "stream": stream,
            "end": end.as_str(),
            "variant": run.variant.as_ref().map(|v| json!({
                "group_id": v.group_id, "quality": v.quality, "is_source": v.is_source,
                "bandwidth": v.bandwidth, "resolution": v.resolution, "codecs": v.codecs,
            })),
            "chapters": chapters.iter().map(|(s, e, t)| json!({"start_ms": s, "end_ms": e, "title": t})).collect::<Vec<_>>(),
            "chapter_source": if timeline.is_some() { "timeline" } else { "helix_polling" },
            "ads": ads.iter().map(|(s, e)| json!({"start_ms": s, "end_ms": e})).collect::<Vec<_>>(),
            "ad_breaks": journal.ads,
            "gaps": journal.gaps.iter().zip(&gaps).map(|(g, (at, _))| json!({
                "at_ms": at, "missing_ms": g.missing_ms, "reason": g.reason,
                "from_seq": g.from_seq, "to_seq": g.to_seq,
                "from_pdt": g.from_pdt.map(format_date_time), "to_pdt": g.to_pdt.map(format_date_time),
            })).collect::<Vec<_>>(),
            "segments": {
                "count": journal.segs.len(),
                "ad": journal.segs.iter().filter(|s| s.ad).count(),
                "discontinuities": journal.segs.iter().filter(|s| s.disc).count(),
                "first_seq": journal.segs.first().map(|s| s.seq),
                "last_seq": journal.last_seq(),
                "bytes": journal.bytes,
                "parts": inputs.len(),
            },
            "recorded": {
                "from": tl.start_pdt().map(format_date_time),
                "to": tl.end_pdt().map(format_date_time),
                "media_ms": expected_ms,
                "missing_ms": missing_ms,
            },
            "file": {
                "name": output.file_name().and_then(|n| n.to_str()),
                "container": req.container, "duration_ms": got_ms, "expected_ms": expected_ms,
                "chapters": probed.chapters, "attachments": probed.attachments,
            },
            "chat": chat,
            "notes": run.notes,
            "recorded_at_ms": self.now(),
        });
        fs::write(
            out_dir.join(format!("{base}.info.json")),
            serde_json::to_vec_pretty(&info).map_err(io)?,
        )
        .map_err(io)?;
        if ctx.is_cancelled() {
            return Err(Stop::Interrupted);
        }

        let mut folder = folder;
        let item = loop {
            let mut item =
                NewItem::new(target, folder.clone()).group(format!("twitch:{}", req.user_id));
            item.profile_id = ctx.job().owner_profile_id;
            match self
                .storage
                .commit_file(&out_dir, &item, Some(ctx.job().id))
            {
                Err(pa_storage::Error::Exists(_)) => folder.push_str(" (2)"),
                other => break other.map_err(storage)?,
            }
        };
        let _ = fs::remove_dir_all(tmp);
        Ok(json!({
            "target": target,
            "item_id": item.id,
            "rel_path": item.rel_path,
            "file": output.file_name().and_then(|n| n.to_str()),
            "end": end.as_str(),
            "stream_id": stream.stream_id,
            "duration_ms": got_ms,
            "expected_ms": expected_ms,
            "missing_ms": missing_ms,
            "segments": run.journal.segs.len(),
            "gaps": run.journal.gaps.len(),
            "ads": ads.len(),
            "chapters": chapters.len(),
            "chat_messages": chat.get("messages").cloned().unwrap_or(Value::Null),
            "bytes": item.size_bytes,
        }))
    }

    /// `.chat.json.gz`: TwitchDownloader-style JSON, offsets on the
    /// recording's timeline (by Twitch's message time and the segments'
    /// program date-times).
    fn write_chat(
        &self,
        run: &mut Run<'_>,
        tl: &RecTimeline,
        stream: &LiveStream,
        out_dir: &Path,
        base: &str,
    ) -> Value {
        let lines = match run.journal.chat_lines() {
            Ok(l) => l,
            Err(e) => {
                run.notes.push(format!("chat: {e}"));
                return Value::Null;
            }
        };
        if lines.is_empty() {
            return Value::Null;
        }
        let mut outside = 0usize;
        let comments: Vec<Value> = lines
            .iter()
            .map(|e| {
                let placed = tl.place(e.line.sent_ms);
                outside += usize::from(placed.outside);
                let mut fragments = Vec::new();
                let chars: Vec<char> = e.line.text.chars().collect();
                let mut at = 0usize;
                for (id, start, end) in &e.line.emotes {
                    let (start, end) = ((*start).min(chars.len()), (*end).min(chars.len()));
                    if start > at {
                        fragments.push(json!({"text": chars[at..start].iter().collect::<String>(), "emoticon": null}));
                    }
                    if end > start {
                        fragments.push(json!({"text": chars[start..end].iter().collect::<String>(), "emoticon": {"emoticon_id": id}}));
                    }
                    at = end.max(at);
                }
                if at < chars.len() || fragments.is_empty() {
                    fragments.push(json!({"text": chars[at.min(chars.len())..].iter().collect::<String>(), "emoticon": null}));
                }
                json!({
                    "_id": e.line.id,
                    "created_at": format_date_time(e.line.sent_ms),
                    "content_offset_seconds": placed.offset_ms as f64 / 1000.0,
                    "outside_recording": placed.outside,
                    "commenter": {"_id": e.line.user_id, "name": e.line.login, "display_name": e.line.display_name},
                    "message": {
                        "body": e.line.text, "fragments": fragments, "user_color": e.line.color,
                        "bits_consumed": e.line.bits.unwrap_or(0), "notice": e.line.notice,
                    },
                    "received_at": format_date_time(e.line.received_ms),
                    "edge_pdt": e.edge_pdt.map(format_date_time),
                })
            })
            .collect();
        let doc = json!({
            "FileInfo": {"Version": {"Major": 1, "Minor": 0, "Patch": 0}, "CreatedAt": format_date_time(self.now())},
            "streamer": {"name": stream.display_name, "id": stream.user_id, "login": stream.login},
            "video": {
                "title": stream.title, "id": stream.stream_id, "kind": "live_recording",
                "start": 0, "end": tl.total_ms() as f64 / 1000.0,
                "recorded_from": tl.start_pdt().map(format_date_time),
            },
            "comments": comments,
        });
        let name = format!("{base}.chat.json.gz");
        let written = fs::File::create(out_dir.join(&name)).and_then(|f| {
            let mut gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            gz.write_all(&serde_json::to_vec(&doc).map_err(std::io::Error::other)?)?;
            gz.finish().map(|_| ())
        });
        match written {
            Ok(()) => json!({"file": name, "messages": lines.len(), "outside_recording": outside}),
            Err(e) => {
                run.notes.push(format!("chat: {e}"));
                Value::Null
            }
        }
    }
}

/// Chapters on the recording's timeline: (start, end, title).
fn chapters(
    tl: &RecTimeline,
    timeline: Option<&LiveTimeline>,
    cats: &[CatEntry],
) -> Vec<(u64, u64, String)> {
    let total = tl.total_ms();
    // (absolute start, name)
    let starts: Vec<(i64, String)> = match timeline {
        Some(t) if !t.segments.is_empty() => t
            .segments
            .iter()
            .map(|s| {
                let name = s
                    .game
                    .as_ref()
                    .map(|g| g.name.clone())
                    .filter(|n| !n.is_empty())
                    .or_else(|| s.title.clone())
                    .unwrap_or_default();
                (s.started_at_ms, name)
            })
            .collect(),
        _ => {
            let mut out: Vec<(i64, String)> = Vec::new();
            for c in cats {
                let name = if c.game_name.is_empty() {
                    c.title.clone()
                } else {
                    c.game_name.clone()
                };
                if out.last().is_none_or(|(_, n)| *n != name) {
                    out.push((c.at, name));
                }
            }
            out
        }
    };
    let mut out: Vec<(u64, u64, String)> = Vec::new();
    for (i, (at, name)) in starts.iter().enumerate() {
        let start = if i == 0 { 0 } else { tl.place(*at).offset_ms };
        let end = starts
            .get(i + 1)
            .map_or(total, |(next, _)| tl.place(*next).offset_ms)
            .min(total);
        if end <= start || name.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.2 == *name => last.1 = end,
            _ => out.push((start, end, name.clone())),
        }
    }
    out
}

impl JobHandler for LiveRecorder {
    fn run(&self, ctx: &JobContext) -> RunOutcome {
        match self.record(ctx) {
            Ok(result) => RunOutcome::Done(Some(result)),
            Err(Stop::Interrupted) => RunOutcome::Interrupted,
            Err(Stop::Error(e)) if e.retryable => RunOutcome::Retryable(e.to_string()),
            Err(Stop::Error(e)) => RunOutcome::Fatal(e.to_string()),
        }
    }
}
