// SPDX-License-Identifier: MPL-2.0
//! Anime recognised by its frames (META-031, ADR-0105): the last fallback
//! after the name, the folder, the local anime database, TMDB and AniList.
//!
//! **Only with the user's consent** ([`FrameSearchOptions::consent`], off
//! by default; the app's «Распознавать аниме по кадрам» switch): frames of
//! the user's files go to trace.moe, an outside service. Without it
//! [`Catalog::search_frames`] returns at once and sends nothing.
//!
//! Which files: the video files (content kind `video`, LIB-025; never
//! «Мои видео», personal media or missing files) of **anime** works that
//! the review inbox holds (META-016): `auto_review`, or nothing found.
//! Works the user decided, «Это не фильм», and NFO ids are never asked
//! about. Movies and live-action shows are not sent: trace.moe knows only
//! anime, and their frames would leave the device for nothing.
//!
//! Per file:
//! 1. **Frames.** Positions spread over the middle of the file (from 20 %
//!    to 85 % of an episode, to keep clear of the opening, the ending and
//!    the preview; 10–90 % of a film), first far apart, then in between
//!    ([`positions`]). Each keyframe there is decoded at 640 px wide
//!    (`pa-probe`, the LGPL FFmpeg); black, dark (credits on black) and
//!    flat frames are skipped ([`is_usable`]).
//! 2. **Votes.** Every frame goes to `/search?anilistInfo`; its best hit
//!    votes for its AniList entry and episode if its similarity is at
//!    least [`FrameSearchOptions::similarity`] (0.9, the API's own line);
//!    a weaker hit or none is a frame that does not agree. From
//!    [`FrameSearchOptions::min_frames`] (5) frames on, one entry with at
//!    least [`FrameSearchOptions::agreement`] (90 %) of the frames decides;
//!    frames are added up to [`FrameSearchOptions::max_frames`] (10), and
//!    the search stops early once no entry can reach 90 % any more
//!    ([`Tally`]).
//! 3. **Match** (≥ 90 %): the work takes the AniList id (a work that has
//!    it already absorbs it), its automatic matches become `auto` with the
//!    agreement as confidence and the frames in `explanation_json`, the
//!    searched file gets the agreed episode (AniList order: trace.moe
//!    numbers episodes as AniList does) if 90 % of the winning frames name
//!    the same one, and the work's metadata is fetched again by its AniList
//!    id ([`Catalog::fetch_metadata_with`] then skips the TMDB title search
//!    for it).
//! 4. **Review** (anything else): the work stays in the inbox; the entries
//!    the frames named are offered there as `anilist` candidates
//!    ([`crate::review`]), with the share of frames as the score.
//!
//! **Quota** (trace.moe free tier, no key): 100 searches a day, one at a
//! time. A run first asks `/me` (no quota) and spends at most what is
//! left and [`FrameSearchOptions::max_searches`]; a file is started only
//! with enough quota for [`FrameSearchOptions::min_frames`]. HTTP 402
//! «quota used up» or a server still busy after the client's backoff ends
//! the run; the next run goes on. Every finished file is kept by its
//! content fingerprint (`frame_searches`): a file is never searched twice,
//! also after a rescan, a move or a new database.

use std::collections::BTreeMap;
use std::sync::Arc;

use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_metadata::tracemoe::{Hit, Me, TraceMoeClient, TraceMoeError};
use pa_probe::VideoFrame;
use pa_scanner::CancelToken;
use pa_vfs::Vfs;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::metadata::MODEL_VERSION;
use crate::recognize::EpisodeNo;
use crate::{Catalog, Error};

/// `explanation_json` key and `matches.model_version` suffix of frame
/// decisions.
pub const PROVIDER: &str = "trace_moe";

/// How a frame search runs.
#[derive(Debug, Clone)]
pub struct FrameSearchOptions {
    /// The user turned «Распознавать аниме по кадрам» on. `false` (the
    /// default): nothing is read and nothing is sent.
    pub consent: bool,
    /// Frames before a decision.
    pub min_frames: usize,
    /// Frames at most per file.
    pub max_frames: usize,
    /// Share of the frames one AniList entry needs.
    pub agreement: f64,
    /// Least similarity of a hit that votes.
    pub similarity: f64,
    /// Searches at most per run (also below the quota left).
    pub max_searches: u32,
    /// Files searched at most per work and run.
    pub max_files_per_work: usize,
    /// Frame width sent (trace.moe recommends 640×360).
    pub width: u32,
    /// JPEG quality of the frames sent.
    pub quality: u8,
}

impl Default for FrameSearchOptions {
    fn default() -> Self {
        Self {
            consent: false,
            min_frames: 5,
            max_frames: 10,
            agreement: 0.9,
            similarity: 0.9,
            max_searches: 30,
            max_files_per_work: 2,
            width: pa_metadata::tracemoe::RECOMMENDED_WIDTH,
            quality: 85,
        }
    }
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameSearchReport {
    /// Anime works waiting in the inbox.
    pub works: usize,
    /// Files whose frames were searched in this run.
    pub files: usize,
    /// Searches sent (quota spent).
    pub searches: u32,
    /// Works matched by their frames.
    pub matched: usize,
    /// Files whose frames did not agree (left for review).
    pub review: usize,
    /// Quota left after the run, if known.
    pub quota_left: Option<u32>,
    /// Why the run stopped early (quota, network, busy server).
    pub error: Option<String>,
}

/// Progress of [`Catalog::search_frames`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSearchProgress {
    /// Works done.
    pub done: usize,
    /// Works of the run.
    pub total: usize,
    /// Searches sent so far.
    pub searches: u32,
}

/// Opens video files for frame extraction (the live one is
/// [`FfmpegFrames`]; tests give their own frames).
pub trait FrameSource: Send + Sync {
    /// Opens `path` (relative to the source root) of a source's file
    /// system.
    ///
    /// # Errors
    ///
    /// A message when the file cannot be opened or decoded.
    fn open(&self, vfs: Arc<dyn Vfs>, path: &str) -> Result<Box<dyn FrameReader>, String>;
}

/// Frames of one open file.
pub trait FrameReader {
    /// Duration of the file, if the container knows it.
    fn duration_ms(&self) -> Option<u64>;

    /// The keyframe at or before `at_ms`, scaled to the width the source
    /// was opened with; `None` past the end.
    ///
    /// # Errors
    ///
    /// A message when reading or decoding fails.
    fn frame_at(&mut self, at_ms: u64) -> Result<Option<VideoFrame>, String>;
}

/// Frames through the LGPL FFmpeg of `pa-probe` (loaded at run time).
#[derive(Debug, Clone)]
pub struct FfmpegFrames {
    /// Frame width.
    pub width: u32,
}

impl Default for FfmpegFrames {
    fn default() -> Self {
        Self {
            width: pa_metadata::tracemoe::RECOMMENDED_WIDTH,
        }
    }
}

struct FfmpegReader {
    grabber: pa_probe::FrameGrabber<'static>,
}

impl FrameReader for FfmpegReader {
    fn duration_ms(&self) -> Option<u64> {
        self.grabber.duration_ms()
    }

    fn frame_at(&mut self, at_ms: u64) -> Result<Option<VideoFrame>, String> {
        match self.grabber.grab(at_ms, |_| false) {
            Ok(pa_probe::Grab::Frame(f)) => Ok(Some(f)),
            Ok(_) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl FrameSource for FfmpegFrames {
    fn open(&self, vfs: Arc<dyn Vfs>, path: &str) -> Result<Box<dyn FrameReader>, String> {
        let ffmpeg = pa_probe::Ffmpeg::shared().map_err(|e| e.to_string())?;
        let file = pa_vfs::VfsPath::parse(path)
            .and_then(|p| vfs.open(&p))
            .map_err(|e| e.to_string())?;
        // A handful of seeks, not a stream.
        file.hint_random_access();
        let grabber = ffmpeg
            .frame_grabber(
                Arc::from(file),
                pa_probe::GrabOptions {
                    width: self.width,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(Box::new(FfmpegReader { grabber }))
    }
}

/// The van der Corput sequence in base 2: 1/2, 1/4, 3/4, 1/8, 5/8, …
fn spread(k: u32) -> f64 {
    let mut x = 0.0;
    let mut f = 0.5;
    let mut k = k;
    while k > 0 {
        if k & 1 == 1 {
            x += f;
        }
        f /= 2.0;
        k >>= 1;
    }
    x
}

/// Where to take up to `count` frames of a file of `duration_ms`: the
/// middle of the file (an episode of up to an hour: 20–85 %, clear of the
/// opening, the ending and the next-episode preview; longer: 10–90 %),
/// first far apart, then in between.
#[must_use]
pub fn positions(duration_ms: u64, count: usize) -> Vec<u64> {
    if duration_ms == 0 {
        return Vec::new();
    }
    let (start, end) = if duration_ms <= 60 * 60 * 1000 {
        (0.20, 0.85)
    } else {
        (0.10, 0.90)
    };
    #[allow(clippy::cast_precision_loss)]
    let d = duration_ms as f64;
    (1..=u32::try_from(count).unwrap_or(u32::MAX))
        .map(|k| {
            let at = d * (start + (end - start) * spread(k));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let ms = at.round() as u64;
            ms
        })
        .collect()
}

/// Whether a frame is worth a search: not black, not mostly dark (credits
/// or a title card on black), not flat (a fade, a white flash).
#[must_use]
pub fn is_usable(frame: &VideoFrame) -> bool {
    let pixels = frame.rgb.chunks_exact(3);
    let mut n = 0u64;
    let mut sum = 0u64;
    let mut sum_sq = 0u64;
    let mut dark = 0u64;
    for (i, p) in pixels.enumerate() {
        // Every 4th pixel is plenty.
        if i % 4 != 0 {
            continue;
        }
        let y = (54 * u64::from(p[0]) + 183 * u64::from(p[1]) + 19 * u64::from(p[2])) >> 8;
        n += 1;
        sum += y;
        sum_sq += y * y;
        if y < 32 {
            dark += 1;
        }
    }
    if n == 0 {
        return false;
    }
    #[allow(clippy::cast_precision_loss)]
    let (mean, var, dark) = {
        let n = n as f64;
        let mean = sum as f64 / n;
        (mean, sum_sq as f64 / n - mean * mean, dark as f64 / n)
    };
    mean >= 20.0 && dark <= 0.85 && var.max(0.0).sqrt() >= 10.0
}

/// An AniList entry the frames named.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameCandidate {
    pub anilist: u64,
    /// English, romaji or native title.
    pub title: String,
    /// Native title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<String>,
    /// The episode most of its frames named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    /// Frames that voted for it.
    pub votes: u32,
    /// `votes` / frames searched.
    pub share: f64,
}

/// The decision of a [`Tally`].
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// More frames are needed.
    More,
    /// One entry has the agreement: its id, the agreed episode (if the
    /// winning frames agree on one) and the share of frames.
    Match {
        anilist: u64,
        episode: Option<u32>,
        agreement: f64,
    },
    /// No entry has or can reach the agreement.
    Review,
}

#[derive(Debug, Clone, Default)]
struct Entry {
    title: String,
    native: Option<String>,
    votes: u32,
    episodes: BTreeMap<u32, u32>,
}

/// Votes of the frames of one file.
#[derive(Debug, Clone)]
pub struct Tally {
    options: FrameSearchOptions,
    /// Frames searched.
    frames: u32,
    entries: BTreeMap<u64, Entry>,
}

impl Tally {
    #[must_use]
    pub fn new(options: &FrameSearchOptions) -> Self {
        Self {
            options: options.clone(),
            frames: 0,
            entries: BTreeMap::new(),
        }
    }

    /// Frames searched.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The best hit of one searched frame (`None`: no hit).
    pub fn add(&mut self, best: Option<&Hit>) {
        self.frames += 1;
        let Some(hit) = best.filter(|h| h.similarity >= self.options.similarity) else {
            return;
        };
        let e = self.entries.entry(hit.anilist.id()).or_default();
        if e.title.is_empty() {
            e.title = hit.anilist.title();
            e.native = hit.anilist.info().and_then(|i| i.title.native.clone());
        }
        e.votes += 1;
        if let Some(ep) = hit.episode_number() {
            *e.episodes.entry(ep).or_default() += 1;
        }
    }

    fn leader(&self) -> Option<(u64, &Entry)> {
        // Most votes; on a tie the lower id, so the result is stable.
        self.entries
            .iter()
            .max_by(|a, b| a.1.votes.cmp(&b.1.votes).then(b.0.cmp(a.0)))
            .map(|(id, e)| (*id, e))
    }

    /// The decision after the frames so far; `exhausted`: no more frames
    /// will come.
    #[must_use]
    pub fn verdict(&self, exhausted: bool) -> Verdict {
        let need = self.options.agreement;
        let frames = f64::from(self.frames);
        let max = u32::try_from(self.options.max_frames).unwrap_or(u32::MAX);
        let votes = self.leader().map_or(0, |(_, e)| e.votes);
        let enough = self.frames as usize >= self.options.min_frames;
        if enough && f64::from(votes) / frames >= need - 1e-9 {
            let (id, e) = self.leader().expect("votes > 0");
            let episode = e
                .episodes
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .filter(|(_, n)| f64::from(**n) / f64::from(e.votes) >= need - 1e-9)
                .map(|(ep, _)| *ep);
            return Verdict::Match {
                anilist: id,
                episode,
                agreement: f64::from(votes) / frames,
            };
        }
        if exhausted || self.frames >= max {
            return Verdict::Review;
        }
        // Even if every frame left voted for the leader, would it get
        // there?
        let left = max - self.frames;
        if f64::from(votes + left) / f64::from(self.frames + left) < need - 1e-9 {
            return Verdict::Review;
        }
        Verdict::More
    }

    /// The entries the frames named, most votes first (at most five).
    #[must_use]
    pub fn candidates(&self) -> Vec<FrameCandidate> {
        let mut out: Vec<FrameCandidate> = self
            .entries
            .iter()
            .map(|(id, e)| FrameCandidate {
                anilist: *id,
                title: e.title.clone(),
                native: e.native.clone(),
                episode: e
                    .episodes
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                    .map(|(ep, _)| *ep),
                votes: e.votes,
                share: if self.frames == 0 {
                    0.0
                } else {
                    f64::from(e.votes) / f64::from(self.frames)
                },
            })
            .collect();
        out.sort_by(|a, b| b.votes.cmp(&a.votes).then(a.anilist.cmp(&b.anilist)));
        out.truncate(crate::metadata::CANDIDATES);
        out
    }
}

/// A file of an anime work in the inbox.
#[derive(Debug, Clone)]
struct FileRow {
    id: i64,
    source_id: i64,
    path: String,
    fingerprint: String,
    duration_ms: Option<i64>,
}

/// The stored outcome of a file (`frame_searches`).
#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    /// `matched`, `review` or `no_frames`.
    kind: &'static str,
    anilist: Option<u64>,
    episode: Option<u32>,
    agreement: f64,
    frames: u32,
    candidates: Vec<FrameCandidate>,
}

/// Anime works of the inbox, oldest first.
fn eligible_works(conn: &Connection) -> Result<Vec<i64>, Error> {
    let mut stmt = conn.prepare(
        "SELECT w.id FROM works w
         WHERE w.kind = 'anime'
           AND NOT EXISTS (SELECT 1 FROM work_reviews wr WHERE wr.work_id = w.id)
           AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id
                           AND (m.state IN ('user_confirmed', 'user_set')
                                OR m.decided_by IN ('user', 'nfo')))
           AND (EXISTS (SELECT 1 FROM matches m WHERE m.work_id = w.id
                        AND m.state = 'auto_review')
                OR EXISTS (SELECT 1 FROM work_metadata wm
                           WHERE wm.work_id = w.id AND wm.outcome = 'not_found'))
         ORDER BY w.id",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Video files of `work` that may be searched: automatic matches, present,
/// content kind `video`, not personal media, with a fingerprint.
fn work_files(conn: &Connection, work: i64) -> Result<Vec<FileRow>, Error> {
    let mut stmt = conn.prepare(concat!(
        "SELECT DISTINCT f.id, f.source_id, f.path, f.fingerprint, f.duration_ms
         FROM matches m JOIN files f ON f.id = m.file_id
         WHERE m.work_id = ?1 AND m.state IN ('auto', 'auto_review')
           AND f.missing_since_ms IS NULL AND f.fingerprint IS NOT NULL
           AND ifnull((SELECT k.kind FROM file_content_kinds k WHERE k.file_id = f.id),
                      'video') = 'video'
           AND NOT ",
        pa_library::file_personal!("f"),
        " ORDER BY f.path, f.id"
    ))?;
    let rows = stmt.query_map([work], |r| {
        Ok(FileRow {
            id: r.get(0)?,
            source_id: r.get(1)?,
            path: r.get(2)?,
            fingerprint: r.get(3)?,
            duration_ms: r.get(4)?,
        })
    })?;
    let files: Vec<FileRow> = rows.collect::<Result<_, _>>()?;
    Ok(files
        .into_iter()
        .filter(|f| pa_scanner::media_kind(&f.path) == Some(pa_scanner::MediaKind::Video))
        .collect())
}

fn cached(conn: &Connection, fingerprint: &str) -> Result<Option<Outcome>, Error> {
    let row = conn
        .query_row(
            "SELECT outcome, anilist_id, episode, agreement, frames, candidates_json
             FROM frame_searches WHERE fingerprint = ?1",
            [fingerprint],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    Ok(row.map(|(kind, anilist, episode, agreement, frames, json)| Outcome {
        kind: match kind.as_str() {
            "matched" => "matched",
            "review" => "review",
            _ => "no_frames",
        },
        anilist: anilist.and_then(|a| u64::try_from(a).ok()),
        episode: episode.and_then(|e| u32::try_from(e).ok()),
        agreement,
        frames: u32::try_from(frames).unwrap_or(0),
        candidates: serde_json::from_str(&json).unwrap_or_default(),
    }))
}

fn store(conn: &Connection, fingerprint: &str, o: &Outcome, now: i64) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO frame_searches (fingerprint, outcome, anilist_id, episode, agreement,
                                     frames, candidates_json, searched_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (fingerprint) DO UPDATE SET
             outcome = excluded.outcome, anilist_id = excluded.anilist_id,
             episode = excluded.episode, agreement = excluded.agreement,
             frames = excluded.frames, candidates_json = excluded.candidates_json,
             searched_at_ms = excluded.searched_at_ms",
        params![
            fingerprint,
            o.kind,
            o.anilist.and_then(|a| i64::try_from(a).ok()),
            o.episode.map(i64::from),
            o.agreement.clamp(0.0, 1.0),
            i64::from(o.frames),
            serde_json::to_string(&o.candidates).unwrap_or_else(|_| "[]".to_owned()),
            now,
        ],
    )?;
    Ok(())
}

/// The frame candidates stored for the files of `work` that did not agree
/// (for the review inbox).
pub(crate) fn work_candidates(
    conn: &Connection,
    work: i64,
) -> Result<Vec<FrameCandidate>, Error> {
    let mut stmt = conn.prepare(
        "SELECT fs.candidates_json FROM frame_searches fs
         WHERE fs.outcome = 'review'
           AND fs.fingerprint IN (SELECT f.fingerprint FROM matches m
                                  JOIN files f ON f.id = m.file_id
                                  WHERE m.work_id = ?1 AND m.state <> 'rejected')
         ORDER BY fs.searched_at_ms, fs.fingerprint",
    )?;
    let mut out: Vec<FrameCandidate> = Vec::new();
    for json in stmt.query_map([work], |r| r.get::<_, String>(0))? {
        let list: Vec<FrameCandidate> = serde_json::from_str(&json?).unwrap_or_default();
        for c in list {
            match out.iter_mut().find(|o| o.anilist == c.anilist) {
                Some(o) if c.share > o.share => *o = c,
                Some(_) => {}
                None => out.push(c),
            }
        }
    }
    out.sort_by(|a, b| {
        b.share
            .partial_cmp(&a.share)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.anilist.cmp(&b.anilist))
    });
    Ok(out)
}

/// Whether `work`'s AniList id came from its frames (the metadata run then
/// reads it by id and does not search TMDB by the file names that failed).
pub(crate) fn decided_by_frames(conn: &Connection, work: i64) -> Result<bool, Error> {
    Ok(conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM matches m JOIN files f ON f.id = m.file_id
             JOIN frame_searches fs ON fs.fingerprint = f.fingerprint
             JOIN external_ids x ON x.work_id = m.work_id AND x.provider = 'anilist'
                                AND x.ext_kind = '' AND x.ext_id = CAST(fs.anilist_id AS TEXT)
             WHERE m.work_id = ?1 AND m.state <> 'rejected' AND fs.outcome = 'matched')",
        [work],
        |r| r.get(0),
    )?)
}

/// Why searching stopped.
enum Stop {
    /// Quota used up, server busy, network down: the run ends.
    Run(String),
}

/// What searching one file gave.
enum Searched {
    Done(Outcome),
    /// Not enough quota left to start; nothing was sent.
    NoBudget,
}

fn search_file(
    client: &TraceMoeClient,
    reader: &mut dyn FrameReader,
    duration_ms: Option<u64>,
    options: &FrameSearchOptions,
    budget: &mut u32,
    sent: &mut u32,
    cancel: &CancelToken,
) -> Result<Searched, Stop> {
    let duration = reader.duration_ms().or(duration_ms).unwrap_or(0);
    // Three tries per frame wanted: some positions are black or repeat a
    // keyframe already taken.
    let mut spots = positions(duration, options.max_frames * 3).into_iter();
    let mut tally = Tally::new(options);
    let mut seen: Vec<u64> = Vec::new();
    let mut started = false;
    loop {
        if cancel.is_cancelled() {
            return Err(Stop::Run("cancelled".to_owned()));
        }
        let frame = loop {
            let Some(at) = spots.next() else {
                break None;
            };
            match reader.frame_at(at) {
                Ok(Some(f)) if !seen.iter().any(|s| s.abs_diff(f.position_ms) < 2000) => {
                    seen.push(f.position_ms);
                    if is_usable(&f) {
                        break Some(f);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(target: "pa_catalog::frame_search", error = %e, "frame failed");
                }
            }
        };
        let Some(frame) = frame else {
            // No more usable frames.
            break;
        };
        if !started {
            if (*budget as usize) < options.min_frames {
                return Ok(Searched::NoBudget);
            }
            started = true;
        }
        if *budget == 0 {
            return Err(Stop::Run("trace.moe quota used up".to_owned()));
        }
        let jpeg = encode(&frame, options.quality)
            .map_err(|e| Stop::Run(format!("cannot encode a frame: {e}")))?;
        *budget -= 1;
        *sent += 1;
        match client.search(&jpeg) {
            Ok(r) => tally.add(r.best()),
            // A frame the server could not read does not count.
            Err(TraceMoeError::Http { status: 400, .. }) => {}
            Err(e @ (TraceMoeError::QuotaDepleted { .. } | TraceMoeError::Busy { .. })) => {
                *budget = 0;
                return Err(Stop::Run(e.to_string()));
            }
            Err(e) => return Err(Stop::Run(e.to_string())),
        }
        match tally.verdict(false) {
            Verdict::More => {}
            v => return Ok(Searched::Done(outcome(&tally, &v))),
        }
    }
    if tally.frames() == 0 {
        return Ok(Searched::Done(Outcome {
            kind: "no_frames",
            anilist: None,
            episode: None,
            agreement: 0.0,
            frames: 0,
            candidates: Vec::new(),
        }));
    }
    let v = tally.verdict(true);
    Ok(Searched::Done(outcome(&tally, &v)))
}

fn outcome(tally: &Tally, verdict: &Verdict) -> Outcome {
    match verdict {
        Verdict::Match {
            anilist,
            episode,
            agreement,
        } => Outcome {
            kind: "matched",
            anilist: Some(*anilist),
            episode: *episode,
            agreement: *agreement,
            frames: tally.frames(),
            candidates: tally.candidates(),
        },
        _ => {
            let candidates = tally.candidates();
            Outcome {
                kind: "review",
                anilist: None,
                episode: None,
                agreement: candidates.first().map_or(0.0, |c| c.share),
                frames: tally.frames(),
                candidates,
            }
        }
    }
}

fn encode(frame: &VideoFrame, quality: u8) -> Result<Vec<u8>, String> {
    let w = u16::try_from(frame.width).map_err(|_| "frame too wide".to_owned())?;
    let h = u16::try_from(frame.height).map_err(|_| "frame too tall".to_owned())?;
    let mut out = Vec::with_capacity(frame.rgb.len() / 8);
    jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100))
        .encode(&frame.rgb, w, h, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Writes a frame match: `work` takes the AniList id, the matches of its
/// files become `auto`, the searched file gets the episode, and the work
/// is fetched again. Returns the work that carries the id.
fn apply(conn: &mut Connection, work: i64, file: i64, o: &Outcome) -> Result<i64, Error> {
    let Some(anilist) = o.anilist else {
        return Ok(work);
    };
    let now = crate::now_ms();
    let tx = conn.transaction()?;
    let files: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT file_id FROM matches WHERE work_id = ?1 AND state <> 'rejected'",
        )?;
        stmt.query_map([work], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    crate::review::clear_provider_data(&tx, work, now)?;
    let mut merged = 0;
    let target = crate::metadata::claim_external(
        &tx,
        work,
        "anilist",
        "",
        &anilist.to_string(),
        &mut merged,
    )?;
    let explanation = json!({
        PROVIDER: {
            "anilist": anilist,
            "episode": o.episode,
            "agreement": (o.agreement * 1000.0).round() / 1000.0,
            "frames": o.frames,
        },
        "signals": {
            "provider": PROVIDER,
            "title": (o.agreement * 1000.0).round() / 1000.0,
            "tier": "frames",
            "file_kind": "anime",
            "kind": "tv",
            "episodes_ok": 0,
            "episodes_bad": 0,
            "tied": false,
        },
    });
    for f in &files {
        tx.execute(
            concat!(
                "UPDATE matches AS m SET state = 'auto', decided_by = 'scorer',
                     model_version = ?3, confidence = ?4, explanation_json = ?5,
                     updated_at_ms = ?6
                 WHERE m.file_id = ?1 AND m.work_id = ?2 AND ",
                crate::numbering::recomputable!()
            ),
            params![
                f,
                target,
                MODEL_VERSION,
                o.agreement.clamp(0.0, 1.0),
                explanation.to_string(),
                now
            ],
        )?;
    }
    if let Some(ep) = o.episode {
        let m: Option<i64> = tx
            .query_row(
                "SELECT id FROM matches WHERE file_id = ?1 AND work_id = ?2
                   AND state IN ('auto', 'auto_review') ORDER BY id DESC LIMIT 1",
                params![file, target],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(m) = m {
            let episode = crate::recognize::ensure_episode(
                &tx,
                target,
                EpisodeNo {
                    season: Some(1),
                    episode: Some(i64::from(ep)),
                    absolute: None,
                    kind: "regular",
                },
                None,
            )?;
            tx.execute("DELETE FROM match_episodes WHERE match_id = ?1", [m])?;
            tx.execute(
                "INSERT INTO match_episodes (match_id, position, episode_id) VALUES (?1, 0, ?2)",
                params![m, episode],
            )?;
        }
    }
    // Fetched again, by the AniList id.
    tx.execute("DELETE FROM work_metadata WHERE work_id = ?1", [target])?;
    crate::recognize::remove_orphans(&tx)?;
    tx.commit()?;
    pa_library::search::sync(conn)?;
    tracing::info!(
        target: "pa_catalog::frame_search",
        work,
        into = target,
        anilist,
        episode = ?o.episode,
        agreement = o.agreement,
        frames = o.frames,
        "frames: matched"
    );
    Ok(target)
}

impl Catalog {
    /// Recognises anime of the review inbox by their frames (see the
    /// [module docs](self)). Without [`FrameSearchOptions::consent`]
    /// nothing is read or sent. Runs are serialized with scans; the
    /// network and decoding run without holding the database.
    /// `on_progress` is called after every work.
    ///
    /// Matched works have no metadata until the next
    /// [`fetch_metadata_with`](Self::fetch_metadata_with).
    pub fn search_frames(
        &self,
        client: &TraceMoeClient,
        frames: &dyn FrameSource,
        options: &FrameSearchOptions,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(FrameSearchProgress),
    ) -> Result<FrameSearchReport, Error> {
        let mut report = FrameSearchReport::default();
        if !options.consent {
            return Ok(report);
        }
        let _scan = self.scan_lock.lock().unwrap_or_else(|e| e.into_inner());
        let works = eligible_works(&self.lock())?;
        report.works = works.len();
        if works.is_empty() {
            return Ok(report);
        }
        // Works with something left to search, and what is cached.
        let mut plan: Vec<(i64, Vec<FileRow>)> = Vec::new();
        {
            let conn = self.lock();
            for w in works {
                let files = work_files(&conn, w)?;
                if !files.is_empty() {
                    plan.push((w, files));
                }
            }
        }
        let total = plan.len();
        let mut budget: Option<u32> = None;
        for (done, (work, files)) in plan.into_iter().enumerate() {
            if cancel.is_cancelled() {
                report.error = Some("cancelled".to_owned());
                break;
            }
            let mut searched_here = 0;
            for file in files {
                let known = cached(&self.lock(), &file.fingerprint)?;
                let outcome = match known {
                    Some(o) => o,
                    None => {
                        if searched_here >= options.max_files_per_work {
                            break;
                        }
                        // The quota is asked for once, before the first
                        // search of the run.
                        let left = match budget {
                            Some(b) => b,
                            None => {
                                let me: Me = match client.me() {
                                    Ok(me) => me,
                                    Err(e) => {
                                        report.error = Some(e.to_string());
                                        return Ok(report);
                                    }
                                };
                                let b = me.left().min(options.max_searches);
                                budget = Some(b);
                                b
                            }
                        };
                        let mut left = left;
                        let vfs = match self.connect(file.source_id) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::info!(
                                    target: "pa_catalog::frame_search",
                                    file = file.id,
                                    error = %e,
                                    "frames: source unreachable"
                                );
                                continue;
                            }
                        };
                        let mut reader = match frames.open(vfs, &file.path) {
                            Ok(r) => r,
                            Err(e) => {
                                tracing::info!(
                                    target: "pa_catalog::frame_search",
                                    file = file.id,
                                    error = %e,
                                    "frames: cannot open"
                                );
                                continue;
                            }
                        };
                        let result = search_file(
                            client,
                            reader.as_mut(),
                            file.duration_ms.and_then(|d| u64::try_from(d).ok()),
                            options,
                            &mut left,
                            &mut report.searches,
                            cancel,
                        );
                        budget = Some(left);
                        report.quota_left = Some(left);
                        match result {
                            Ok(Searched::Done(o)) => {
                                searched_here += 1;
                                report.files += 1;
                                store(&self.lock(), &file.fingerprint, &o, crate::now_ms())?;
                                o
                            }
                            Ok(Searched::NoBudget) => {
                                report.error = Some("trace.moe quota used up".to_owned());
                                on_progress(FrameSearchProgress {
                                    done,
                                    total,
                                    searches: report.searches,
                                });
                                return Ok(report);
                            }
                            Err(Stop::Run(e)) => {
                                tracing::info!(
                                    target: "pa_catalog::frame_search",
                                    error = %e,
                                    "frames: run stopped"
                                );
                                report.error = Some(e);
                                return Ok(report);
                            }
                        }
                    }
                };
                match outcome.kind {
                    "matched" => {
                        apply(&mut self.lock(), work, file.id, &outcome)?;
                        report.matched += 1;
                        break;
                    }
                    "review" => report.review += 1,
                    _ => {}
                }
            }
            on_progress(FrameSearchProgress {
                done: done + 1,
                total,
                searches: report.searches,
            });
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(f: impl Fn(usize) -> [u8; 3]) -> VideoFrame {
        let (w, h) = (64usize, 36usize);
        VideoFrame {
            position_ms: 0,
            width: 64,
            height: 36,
            rgb: (0..w * h).flat_map(f).collect(),
        }
    }

    #[test]
    fn positions_avoid_opening_and_ending() {
        let d = 24 * 60 * 1000;
        let p = positions(d, 10);
        assert_eq!(p.len(), 10);
        assert!(p.iter().all(|&t| t >= d / 5 && t <= d * 85 / 100), "{p:?}");
        // The first ones are far apart.
        assert!(p[0].abs_diff(p[1]) > d / 8);
        let film = positions(2 * 3600 * 1000, 4);
        assert!(film.iter().all(|&t| t >= 720_000 && t <= 6_480_000));
        assert!(positions(0, 5).is_empty());
    }

    #[test]
    fn black_dark_and_flat_frames_are_skipped() {
        assert!(!is_usable(&frame(|_| [0, 0, 0])));
        assert!(!is_usable(&frame(|_| [128, 128, 128])));
        assert!(!is_usable(&frame(|_| [255, 255, 255])));
        // White credits on black: few bright pixels.
        assert!(!is_usable(&frame(|i| if i % 20 == 0 {
            [240, 240, 240]
        } else {
            [5, 5, 5]
        })));
        // A picture: a gradient with colour.
        assert!(is_usable(&frame(|i| {
            let x = u8::try_from(i % 64 * 4).unwrap_or(255);
            [x, 255 - x, 90]
        })));
    }

    fn hit(anilist: u64, ep: Option<u32>, similarity: f64) -> Hit {
        Hit {
            anilist: pa_metadata::tracemoe::AniListRef::Id(anilist),
            filename: String::new(),
            episode: serde_json::Value::Null,
            episode_start: ep,
            episode_end: ep,
            duration: None,
            at: None,
            from: None,
            to: None,
            similarity,
        }
    }

    #[test]
    fn five_agreeing_frames_decide() {
        let mut t = Tally::new(&FrameSearchOptions::default());
        for _ in 0..4 {
            t.add(Some(&hit(1, Some(3), 0.95)));
            assert_eq!(t.verdict(false), Verdict::More);
        }
        t.add(Some(&hit(1, Some(3), 0.97)));
        assert_eq!(
            t.verdict(false),
            Verdict::Match {
                anilist: 1,
                episode: Some(3),
                agreement: 1.0
            }
        );
    }

    #[test]
    fn one_stray_frame_needs_ten() {
        let mut t = Tally::new(&FrameSearchOptions::default());
        t.add(Some(&hit(2, Some(1), 0.93)));
        for _ in 0..8 {
            t.add(Some(&hit(1, Some(3), 0.95)));
            assert_eq!(t.verdict(false), Verdict::More);
        }
        t.add(Some(&hit(1, Some(3), 0.95)));
        assert!(matches!(
            t.verdict(false),
            Verdict::Match { anilist: 1, agreement, .. } if (agreement - 0.9).abs() < 1e-9
        ));
    }

    #[test]
    fn disagreement_stops_early() {
        let mut t = Tally::new(&FrameSearchOptions::default());
        t.add(Some(&hit(1, None, 0.95)));
        t.add(Some(&hit(2, None, 0.95)));
        // 1 of 2; with 8 more for #1 it could still be 9/10.
        assert_eq!(t.verdict(false), Verdict::More);
        t.add(None);
        // 1 of 3: 8/10 at best.
        assert_eq!(t.verdict(false), Verdict::Review);
        let c = t.candidates();
        assert_eq!(c.len(), 2);
        assert!((c[0].share - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn weak_hits_do_not_vote() {
        let mut t = Tally::new(&FrameSearchOptions::default());
        for _ in 0..5 {
            t.add(Some(&hit(1, None, 0.85)));
        }
        assert_eq!(t.verdict(false), Verdict::Review);
        assert!(t.candidates().is_empty());
    }

    #[test]
    fn title_agrees_but_episode_does_not() {
        let mut t = Tally::new(&FrameSearchOptions::default());
        for ep in [1, 1, 1, 2, 2] {
            t.add(Some(&hit(7, Some(ep), 0.95)));
        }
        assert_eq!(
            t.verdict(false),
            Verdict::Match {
                anilist: 7,
                episode: None,
                agreement: 1.0
            }
        );
    }
}
