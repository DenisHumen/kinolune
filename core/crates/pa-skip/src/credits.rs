// SPDX-License-Identifier: MPL-2.0
//! End credits in the picture: black frames and light text on a dark
//! background, up to the end of the file.
//!
//! Each sampled frame (a keyframe thumbnail of the outro window) gets two
//! numbers: the share of dark pixels and the text density — the share of
//! bright pixels on a sharp edge, which rolling or carded credits have
//! and a dark scene has not. The credits are the last long run of black
//! or text-on-black frames; short interruptions (a logo, a still) are
//! bridged.

/// What one sampled frame looks like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    /// Time of the frame in the file.
    pub position_ms: u64,
    /// Share of pixels darker than [`DARK_LUMA`], 0–1.
    pub dark: f32,
    /// Share of bright pixels on a sharp edge, 0–1.
    pub text: f32,
}

/// Luma (0–255) below which a pixel is dark.
pub const DARK_LUMA: u8 = 40;
const BRIGHT_LUMA: u8 = 150;
const EDGE: u8 = 70;

/// Limits of [`detect`].
#[derive(Debug, Clone)]
pub struct CreditsOptions {
    /// Shortest credits.
    pub min_ms: u64,
    /// Longest stretch of other frames bridged inside the credits.
    pub max_gap_ms: u64,
    /// Share of frames of the run that must show text (the rest black).
    pub min_text_share: f32,
}

impl Default for CreditsOptions {
    fn default() -> Self {
        Self {
            min_ms: 15_000,
            max_gap_ms: 4_000,
            min_text_share: 0.3,
        }
    }
}

impl FrameStats {
    /// Statistics of a packed RGB frame (`width * height * 3` bytes).
    #[must_use]
    pub fn of_rgb(position_ms: u64, width: usize, height: usize, rgb: &[u8]) -> Self {
        let n = width * height;
        if n == 0 || rgb.len() < n * 3 {
            return Self {
                position_ms,
                dark: 0.0,
                text: 0.0,
            };
        }
        let luma: Vec<u8> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .take(n)
            .map(|p| {
                ((u32::from(p[0]) * 77 + u32::from(p[1]) * 150 + u32::from(p[2]) * 29) >> 8) as u8
            })
            .collect();
        let mut dark = 0usize;
        let mut text = 0usize;
        for y in 0..height {
            for x in 0..width {
                let v = luma[y * width + x];
                if v < DARK_LUMA {
                    dark += 1;
                } else if v >= BRIGHT_LUMA {
                    let right = if x + 1 < width {
                        luma[y * width + x + 1]
                    } else {
                        v
                    };
                    let below = if y + 1 < height {
                        luma[(y + 1) * width + x]
                    } else {
                        v
                    };
                    let left = if x > 0 { luma[y * width + x - 1] } else { v };
                    if v.abs_diff(right) > EDGE
                        || v.abs_diff(below) > EDGE
                        || v.abs_diff(left) > EDGE
                    {
                        text += 1;
                    }
                }
            }
        }
        Self {
            position_ms,
            dark: dark as f32 / n as f32,
            text: text as f32 / n as f32,
        }
    }

    /// A black frame.
    #[must_use]
    pub fn is_black(&self) -> bool {
        self.dark >= 0.97 && self.text < 0.002
    }

    /// Light text on a dark background.
    #[must_use]
    pub fn is_text_on_dark(&self) -> bool {
        self.dark >= 0.65 && (0.002..=0.25).contains(&self.text)
    }

    /// Black, or text on a dark background.
    #[must_use]
    pub fn is_credits(&self) -> bool {
        self.is_black() || self.is_text_on_dark()
    }
}

/// Credits found among the samples, with the samples around their
/// edges: the true start lies in `(before_ms, start_ms]`, the true end in
/// `(last_ms, end_ms]` (exact frames can narrow both down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditsRun {
    /// First credits sample.
    pub start_ms: u64,
    /// The first other sample after the run, or the end of the file.
    pub end_ms: u64,
    /// The last other sample before the run.
    pub before_ms: Option<u64>,
    /// The last credits sample.
    pub last_ms: u64,
    /// Whether the run reaches the end of the file.
    pub to_end: bool,
}

/// The credits in `frames` (in time order, the outro window of a file of
/// `duration_ms`): the last run long enough with enough text. A run that
/// ends within 5 s of the end of the file ends with it.
#[must_use]
pub fn detect(
    frames: &[FrameStats],
    duration_ms: u64,
    opts: &CreditsOptions,
) -> Option<CreditsRun> {
    // Runs as (first, last) indices of credit frames.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    // Time of the first other frame since the last credit frame.
    let mut other_since: Option<u64> = None;
    for (i, f) in frames.iter().enumerate() {
        if f.is_credits() {
            run = match (run, other_since) {
                (Some((lo, _)), Some(t)) if f.position_ms.saturating_sub(t) <= opts.max_gap_ms => {
                    Some((lo, i))
                }
                (Some((lo, _)), None) => Some((lo, i)),
                (Some(r), Some(_)) => {
                    runs.push(r);
                    Some((i, i))
                }
                (None, _) => Some((i, i)),
            };
            other_since = None;
        } else if run.is_some() && other_since.is_none() {
            other_since = Some(f.position_ms);
        }
    }
    if let Some(r) = run {
        runs.push(r);
    }
    // The last run that qualifies.
    runs.into_iter().rev().find_map(|(lo, hi)| {
        let start = frames[lo].position_ms;
        let last = frames[hi].position_ms;
        // The run lasts until the next frame after it, or the file's end.
        let next = frames.get(hi + 1).map(|f| f.position_ms);
        let mut end = next.unwrap_or(duration_ms).max(last);
        let to_end = next.is_none() || duration_ms.saturating_sub(end) <= 5_000;
        if to_end {
            end = duration_ms.max(end);
        }
        let members = &frames[lo..=hi];
        let credits = members.iter().filter(|f| f.is_credits()).count();
        let text = members.iter().filter(|f| f.is_text_on_dark()).count();
        (end.saturating_sub(start) >= opts.min_ms
            && credits > 0
            && text as f32 / credits as f32 >= opts.min_text_share)
            .then_some(CreditsRun {
                start_ms: start,
                end_ms: end,
                before_ms: lo.checked_sub(1).map(|i| frames[i].position_ms),
                last_ms: last,
                to_end,
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(position_ms: u64, dark: f32, text: f32) -> FrameStats {
        FrameStats {
            position_ms,
            dark,
            text,
        }
    }

    #[test]
    fn stats_of_text_on_black() {
        let (w, h) = (40, 20);
        let mut rgb = vec![0u8; w * h * 3];
        // A white "letter": a 4x6 block.
        for y in 8..14 {
            for x in 10..14 {
                let i = (y * w + x) * 3;
                rgb[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let s = FrameStats::of_rgb(0, w, h, &rgb);
        assert!(s.dark > 0.9, "{s:?}");
        assert!(s.text > 0.01 && s.text < 0.05, "{s:?}");
        assert!(s.is_text_on_dark());
        let black = FrameStats::of_rgb(0, w, h, &vec![0u8; w * h * 3]);
        assert!(black.is_black() && !black.is_text_on_dark());
        let grey = FrameStats::of_rgb(0, w, h, &vec![128u8; w * h * 3]);
        assert!(!grey.is_credits());
    }

    #[test]
    fn last_long_run_with_text() {
        let mut frames = Vec::new();
        // Scenes until 1200 s, a dark scene 600–620 s without text, credits
        // 1200–1290 s with a 2-second logo at 1250 s, then 10 s of scene
        // (a stinger).
        for t in (500..1300).step_by(1) {
            let ms = t * 1000;
            frames.push(match t {
                600..620 => frame(ms, 0.9, 0.0),
                1200..1250 | 1252..1290 => frame(ms, 0.9, 0.02),
                _ => frame(ms, 0.2, 0.01),
            });
        }
        let run = detect(&frames, 1_300_000, &CreditsOptions::default()).unwrap();
        assert_eq!((run.start_ms, run.end_ms), (1_200_000, 1_290_000));
        assert_eq!(
            (run.before_ms, run.last_ms, run.to_end),
            (Some(1_199_000), 1_289_000, false)
        );
        // Reaching the end: the end of the file.
        let frames: Vec<_> = (0..100)
            .map(|t| frame(t * 1000, 0.95, if t >= 40 { 0.01 } else { 0.3 }))
            .collect();
        let run = detect(&frames, 103_000, &CreditsOptions::default()).unwrap();
        assert_eq!(
            (run.start_ms, run.end_ms, run.to_end),
            (40_000, 103_000, true)
        );
        // Black alone is not credits.
        let frames: Vec<_> = (0..60).map(|t| frame(t * 1000, 1.0, 0.0)).collect();
        assert_eq!(detect(&frames, 60_000, &CreditsOptions::default()), None);
    }
}
