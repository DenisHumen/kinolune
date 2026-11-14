// SPDX-License-Identifier: MPL-2.0
//! What one file contributes, read through FFmpeg ([`analyze_file`]), and
//! how the files of a season are combined into segments
//! ([`season_segments`]).

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pa_library::skip::{SkipKind, SkipOrigin};
use pa_probe::{AudioOptions, Ffmpeg, Grab, GrabOptions, ProbeError, ProbeOptions};
use pa_vfs::ByteSource;

use crate::chapters::{self, ChapterMark};
use crate::credits::{self, CreditsOptions, FrameStats};
use crate::fingerprint::{Builder, Fingerprint};
use crate::matching::MatchOptions;
use crate::season;

/// Where to look and how hard.
#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    /// The intro window: the first `intro_share` of the file, at most
    /// `intro_max_ms` (Jellyfin's intro-skipper uses 25 % and 10 min).
    pub intro_share: f64,
    pub intro_max_ms: u64,
    /// The outro window: the last `outro_share` of the file, at most
    /// `outro_max_ms`.
    pub outro_share: f64,
    pub outro_max_ms: u64,
    /// Step between sampled frames of the outro window.
    pub frame_step_ms: u64,
    /// Width of the sampled frames.
    pub frame_width: u32,
    /// Decode the outro window's picture for credits.
    pub video_credits: bool,
    /// Set to `true` from another thread to abort.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            intro_share: 0.25,
            intro_max_ms: 10 * 60_000,
            outro_share: 0.3,
            outro_max_ms: 6 * 60_000,
            frame_step_ms: 1_000,
            frame_width: 160,
            video_credits: true,
            cancel: None,
        }
    }
}

/// What one file contributes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileAnalysis {
    pub duration_ms: Option<u64>,
    pub chapters: Vec<ChapterMark>,
    /// Fingerprint of the intro window.
    pub intro: Option<Fingerprint>,
    /// Fingerprint of the outro window.
    pub outro: Option<Fingerprint>,
    /// Credits in the picture.
    pub video_credits: Option<(u64, u64)>,
}

/// A segment found for a file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Found {
    pub kind: SkipKind,
    pub origin: SkipOrigin,
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: f64,
}

/// Reads one file: its chapters, the audio fingerprints of the intro and
/// outro windows, and the credits in the picture of the outro window.
///
/// # Errors
///
/// The file cannot be opened or has no audio (a file without video only
/// loses the picture analysis); [`pa_probe::ProbeErrorKind::Cancelled`].
pub fn analyze_file(
    ffmpeg: &Ffmpeg,
    source: &Arc<dyn ByteSource>,
    opts: &AnalyzeOptions,
) -> Result<FileAnalysis, ProbeError> {
    let info = ffmpeg.probe(Arc::clone(source), &ProbeOptions::default())?;
    let mut out = FileAnalysis {
        duration_ms: info.duration_ms,
        chapters: info
            .chapters
            .iter()
            .map(|c| ChapterMark {
                start_ms: c.start_ms,
                end_ms: c.end_ms,
                title: c.title.clone(),
            })
            .collect(),
        ..FileAnalysis::default()
    };
    let mut reader = ffmpeg.audio_reader(
        Arc::clone(source),
        AudioOptions {
            cancel: opts.cancel.clone(),
            ..AudioOptions::default()
        },
    )?;
    let Some(duration) = out.duration_ms.or(reader.duration_ms()).filter(|d| *d > 0) else {
        return Ok(out);
    };
    out.duration_ms = Some(duration);
    let (intro_end, outro_start) = windows(duration, opts);
    out.intro = Some(fingerprint(&mut reader, 0, intro_end)?);
    out.outro = Some(fingerprint(&mut reader, outro_start, duration)?);
    drop(reader);
    if opts.video_credits {
        match video_credits(ffmpeg, source, outro_start, duration, opts) {
            Ok(found) => out.video_credits = found,
            Err(e) if e.kind() == pa_probe::ProbeErrorKind::Cancelled => return Err(e),
            // No video stream or no decoder: audio still counts.
            Err(_) => {}
        }
    }
    Ok(out)
}

/// End of the intro window and start of the outro window.
#[must_use]
pub fn windows(duration_ms: u64, opts: &AnalyzeOptions) -> (u64, u64) {
    let d = duration_ms as f64;
    let intro = ((d * opts.intro_share) as u64).min(opts.intro_max_ms);
    let outro = ((d * opts.outro_share) as u64).min(opts.outro_max_ms);
    (intro, duration_ms.saturating_sub(outro))
}

fn fingerprint(
    reader: &mut pa_probe::AudioReader<'_>,
    from_ms: u64,
    to_ms: u64,
) -> Result<Fingerprint, ProbeError> {
    let mut builder: Option<Builder> = None;
    reader.decode_mono(from_ms, to_ms, &mut |rate, at_ms, samples| {
        builder
            .get_or_insert_with(|| Builder::new(at_ms))
            .push(rate, samples);
    })?;
    Ok(builder.map_or(
        Fingerprint {
            start_ms: from_ms,
            hashes: Vec::new(),
        },
        Builder::finish,
    ))
}

/// Credits in the picture of `[from_ms, to_ms)`: keyframes sampled every
/// step, then the edges narrowed down on exact frames.
fn video_credits(
    ffmpeg: &Ffmpeg,
    source: &Arc<dyn ByteSource>,
    from_ms: u64,
    to_ms: u64,
    opts: &AnalyzeOptions,
) -> Result<Option<(u64, u64)>, ProbeError> {
    let mut grabber = ffmpeg.frame_grabber(
        Arc::clone(source),
        GrabOptions {
            width: opts.frame_width,
            cancel: opts.cancel.clone(),
            ..GrabOptions::default()
        },
    )?;
    let mut seen: HashSet<u64> = HashSet::new();
    let mut frames = Vec::new();
    let step = opts.frame_step_ms.max(100);
    let mut t = from_ms;
    while t < to_ms {
        match grabber.grab(t, |p| seen.contains(&p))? {
            Grab::Frame(f) => {
                seen.insert(f.position_ms);
                if f.position_ms + step >= from_ms {
                    frames.push(stats(&f));
                }
            }
            Grab::Same(_) => {}
            Grab::End => break,
        }
        t += step;
    }
    frames.sort_by_key(|f| f.position_ms);
    let Some(run) = credits::detect(&frames, to_ms, &CreditsOptions::default()) else {
        return Ok(None);
    };
    let mut start = run.start_ms;
    if let Some(before) = run.before_ms {
        // The first credits frame is in (before, start].
        start = narrow(&mut grabber, before, start, true)?;
    }
    let mut end = run.end_ms;
    if !run.to_end {
        // The first frame after the credits is in (last, end].
        end = narrow(&mut grabber, run.last_ms, end, false)?;
    }
    Ok(Some((start, end)))
}

fn stats(f: &pa_probe::VideoFrame) -> FrameStats {
    FrameStats::of_rgb(f.position_ms, f.width as usize, f.height as usize, &f.rgb)
}

/// Exactness of [`narrow`].
const NARROW_MS: u64 = 200;

/// The first frame in `(lo, hi]` that is credits (`credits`) or not
/// (`!credits`), by bisection on exact frames; `hi` if exact frames are
/// not available.
fn narrow(
    grabber: &mut pa_probe::FrameGrabber<'_>,
    mut lo: u64,
    mut hi: u64,
    credits: bool,
) -> Result<u64, ProbeError> {
    for _ in 0..10 {
        if hi.saturating_sub(lo) <= NARROW_MS {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        let Grab::Frame(f) = grabber.grab_exact(mid)? else {
            break;
        };
        if f.position_ms >= hi || f.position_ms < mid {
            // A frame outside the interval: nothing to learn.
            break;
        }
        if stats(&f).is_credits() == credits {
            hi = f.position_ms;
        } else {
            lo = f.position_ms;
        }
    }
    Ok(hi)
}

/// Intro matches.
fn intro_options() -> MatchOptions {
    MatchOptions::default()
}

/// Ending-theme matches: longer stretches are fine.
fn outro_options() -> MatchOptions {
    MatchOptions {
        min_ms: 15_000,
        max_ms: 10 * 60_000,
        ..MatchOptions::default()
    }
}

/// Segments of each file of a season (files in episode order): chapter
/// marks, the intro the episodes share, and the credits — the picture's
/// start when it shows them (an ending song often starts over the last
/// scene), else the ending song the episodes share.
#[must_use]
pub fn season_segments(files: &[&FileAnalysis]) -> Vec<Vec<Found>> {
    let intros: Vec<Option<&Fingerprint>> = files.iter().map(|f| f.intro.as_ref()).collect();
    let outros: Vec<Option<&Fingerprint>> = files.iter().map(|f| f.outro.as_ref()).collect();
    let intro = season::agree(&intros, &intro_options());
    let outro = season::agree(&outros, &outro_options());
    files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let mut out: Vec<Found> = chapters::segments(&f.chapters)
                .into_iter()
                .map(|(kind, start_ms, end_ms)| Found {
                    kind,
                    origin: SkipOrigin::Chapter,
                    start_ms,
                    end_ms,
                    confidence: 1.0,
                })
                .collect();
            if let Some(a) = intro[i] {
                out.push(Found {
                    kind: SkipKind::Intro,
                    origin: SkipOrigin::Audio,
                    start_ms: a.start_ms,
                    end_ms: a.end_ms,
                    confidence: a.confidence,
                });
            }
            if let Some(c) = credits_of(f, outro[i]) {
                out.push(c);
            }
            out
        })
        .collect()
}

/// Credits of one file from the picture and the shared ending song.
fn credits_of(f: &FileAnalysis, song: Option<season::Agreed>) -> Option<Found> {
    let snap_end = |end: u64| match f.duration_ms {
        Some(d) if d.saturating_sub(end) <= 5_000 => d,
        _ => end,
    };
    match (f.video_credits, song) {
        (Some((vs, ve)), Some(a)) => Some(Found {
            kind: SkipKind::Credits,
            origin: SkipOrigin::Video,
            start_ms: vs,
            end_ms: snap_end(if a.start_ms < ve { ve.max(a.end_ms) } else { ve }),
            confidence: (0.5 + a.confidence / 2.0).min(1.0),
        }),
        (Some((vs, ve)), None) => Some(Found {
            kind: SkipKind::Credits,
            origin: SkipOrigin::Video,
            start_ms: vs,
            end_ms: snap_end(ve),
            confidence: 0.5,
        }),
        (None, Some(a)) => Some(Found {
            kind: SkipKind::Credits,
            origin: SkipOrigin::Audio,
            start_ms: a.start_ms,
            end_ms: snap_end(a.end_ms),
            confidence: a.confidence,
        }),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_windows() {
        let o = AnalyzeOptions::default();
        // 24 min: intro 6 min, outro 6 min (cap).
        assert_eq!(windows(1_440_000, &o), (360_000, 1_080_000));
        // 60 min: intro 10 min (cap).
        assert_eq!(windows(3_600_000, &o).0, 600_000);
        // 10 min: 2.5 and 3 min.
        assert_eq!(windows(600_000, &o), (150_000, 420_000));
    }

    #[test]
    fn credits_combination() {
        let f = FileAnalysis {
            duration_ms: Some(1_400_000),
            video_credits: Some((1_310_000, 1_398_000)),
            ..FileAnalysis::default()
        };
        let song = season::Agreed {
            start_ms: 1_300_000,
            end_ms: 1_390_000,
            confidence: 1.0,
            support: 3,
        };
        let c = credits_of(&f, Some(song)).unwrap();
        assert_eq!((c.origin, c.start_ms, c.end_ms), (SkipOrigin::Video, 1_310_000, 1_400_000));
        let c = credits_of(&FileAnalysis::default(), Some(song)).unwrap();
        assert_eq!((c.origin, c.start_ms), (SkipOrigin::Audio, 1_300_000));
        assert_eq!(credits_of(&FileAnalysis::default(), None), None);
    }
}
