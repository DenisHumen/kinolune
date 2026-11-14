// SPDX-License-Identifier: MPL-2.0
//! Finding the audio two fingerprints share (the intro of two episodes).
//!
//! 1. **Offset votes.** Every non-silent sub-fingerprint of `b` is indexed
//!    by its low and its high 16 bits; each sub-fingerprint of `a` votes
//!    for the offsets of the `b` frames with an equal half. Shared audio
//!    piles votes on one offset; a 16-bit half survives re-encoding far
//!    more often than all 32 bits.
//! 2. **Runs.** For the strongest offsets, frames whose sub-fingerprints
//!    differ in at most [`MatchOptions::max_bit_errors`] bits are similar;
//!    a window density smooths them into runs (random frames are similar
//!    with probability < 0.4 %, so noise does not stretch a run), short
//!    gaps (a pause in the music) are bridged, and the run is trimmed to
//!    its first and last similar frame.
//!
//! The longest run within the length limits wins.

use crate::fingerprint::{FRAME_MS, Fingerprint, HOP_MS, SILENT};

/// Limits of [`shared_segment`].
#[derive(Debug, Clone)]
pub struct MatchOptions {
    /// Bits two similar sub-fingerprints may differ in (of 32).
    pub max_bit_errors: u32,
    /// Shortest shared stretch.
    pub min_ms: u64,
    /// Longest shared stretch; longer ones are not an intro (the same
    /// episode twice) and are ignored.
    pub max_ms: u64,
    /// Longest gap bridged inside a run.
    pub max_gap_ms: u64,
    /// Offsets tried, strongest first.
    pub candidates: usize,
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            max_bit_errors: 8,
            min_ms: 15_000,
            max_ms: 150_000,
            max_gap_ms: 1_500,
            candidates: 8,
        }
    }
}

/// A stretch of audio found in both fingerprints, in file times.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shared {
    pub a_start_ms: u64,
    pub a_end_ms: u64,
    pub b_start_ms: u64,
    pub b_end_ms: u64,
    /// Share of similar frames in the run, 0–1.
    pub density: f64,
}

impl Shared {
    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        self.a_end_ms - self.a_start_ms
    }
}

/// Half-window of the density smoothing, in frames.
const DENSITY_HALF: usize = 8;
/// Density from which a frame is inside a run.
const DENSITY_MIN: f64 = 0.35;
/// Buckets of the index with more frames are not informative (a
/// sustained tone) and are skipped.
const MAX_BUCKET: usize = 64;
/// Shift of a frame's boundaries towards its centre: a frame straddling
/// the start of the shared audio is often still similar.
const EDGE_MS: u64 = 64;

/// The longest stretch of audio `a` and `b` share, if any within the
/// limits of `opts`.
#[must_use]
pub fn shared_segment(a: &Fingerprint, b: &Fingerprint, opts: &MatchOptions) -> Option<Shared> {
    let (ha, hb) = (&a.hashes, &b.hashes);
    if ha.is_empty() || hb.is_empty() {
        return None;
    }
    let votes = offset_votes(ha, hb);
    let mut best: Option<Shared> = None;
    for delta in strongest_offsets(&votes, hb.len(), opts.candidates) {
        for d in [delta - 1, delta, delta + 1] {
            for (first, last, density) in runs(ha, hb, d, opts) {
                let shared = Shared {
                    a_start_ms: a.time_ms(first) + EDGE_MS,
                    a_end_ms: a.time_ms(last) + FRAME_MS - EDGE_MS,
                    b_start_ms: b.time_ms(offset(first, d)) + EDGE_MS,
                    b_end_ms: b.time_ms(offset(last, d)) + FRAME_MS - EDGE_MS,
                    density,
                };
                let len = shared.duration_ms();
                if len < opts.min_ms || len > opts.max_ms {
                    continue;
                }
                if best.is_none_or(|b| {
                    len > b.duration_ms() || (len == b.duration_ms() && density > b.density)
                }) {
                    best = Some(shared);
                }
            }
        }
    }
    best
}

/// Index of `b` in `a`'s frame `i` at offset `d` (`i - j`).
fn offset(i: usize, d: isize) -> usize {
    (i as isize - d) as usize
}

/// Votes per offset `i - j + len(b)`.
fn offset_votes(a: &[u32], b: &[u32]) -> Vec<u32> {
    let key = |half: u32, h: u32| {
        if half == 0 {
            h & 0xffff
        } else {
            (1 << 16) | (h >> 16)
        }
    };
    let mut index: Vec<(u32, u32)> = Vec::with_capacity(b.len() * 2);
    for (j, &h) in b.iter().enumerate() {
        if h != SILENT {
            for half in 0..2 {
                index.push((key(half, h), j as u32));
            }
        }
    }
    index.sort_unstable();
    let mut votes = vec![0u32; a.len() + b.len()];
    for (i, &h) in a.iter().enumerate() {
        if h == SILENT {
            continue;
        }
        for half in 0..2 {
            let k = key(half, h);
            let lo = index.partition_point(|e| e.0 < k);
            let hi = index.partition_point(|e| e.0 <= k);
            if hi - lo > MAX_BUCKET {
                continue;
            }
            for e in &index[lo..hi] {
                votes[i + b.len() - e.1 as usize] += 1;
            }
        }
    }
    votes
}

/// The `n` strongest offsets (`i - j`), smoothed over ±1 and at least
/// 16 frames apart.
fn strongest_offsets(votes: &[u32], b_len: usize, n: usize) -> Vec<isize> {
    let smooth: Vec<u32> = (0..votes.len())
        .map(|i| {
            votes[i.saturating_sub(1)..(i + 2).min(votes.len())]
                .iter()
                .sum()
        })
        .collect();
    let mut order: Vec<usize> = (0..smooth.len()).filter(|&i| smooth[i] >= 6).collect();
    order.sort_unstable_by(|x, y| smooth[*y].cmp(&smooth[*x]).then(x.cmp(y)));
    let mut picked: Vec<usize> = Vec::new();
    for i in order {
        if picked.len() >= n {
            break;
        }
        if picked.iter().all(|p| p.abs_diff(i) > 16) {
            // The best raw offset around the smoothed peak.
            let lo = i.saturating_sub(1);
            let hi = (i + 2).min(votes.len());
            let best = (lo..hi).max_by_key(|k| votes[*k]).unwrap_or(i);
            picked.push(best);
        }
    }
    picked
        .into_iter()
        .map(|i| i as isize - b_len as isize)
        .collect()
}

/// Runs of similar frames at offset `d`: `(first, last, density)` in `a`
/// frames.
fn runs(a: &[u32], b: &[u32], d: isize, opts: &MatchOptions) -> Vec<(usize, usize, f64)> {
    let start = d.max(0) as usize;
    let end = (b.len() as isize + d).min(a.len() as isize);
    if end <= start as isize {
        return Vec::new();
    }
    let end = end as usize;
    let sim: Vec<bool> = (start..end)
        .map(|i| {
            let (x, y) = (a[i], b[offset(i, d)]);
            x != SILENT && y != SILENT && (x ^ y).count_ones() <= opts.max_bit_errors
        })
        .collect();
    let n = sim.len();
    // Prefix sums for the window density.
    let mut prefix = vec![0u32; n + 1];
    for (i, s) in sim.iter().enumerate() {
        prefix[i + 1] = prefix[i] + u32::from(*s);
    }
    let inside = |i: usize| {
        let lo = i.saturating_sub(DENSITY_HALF);
        let hi = (i + DENSITY_HALF).min(n);
        f64::from(prefix[hi] - prefix[lo]) / (hi - lo) as f64 >= DENSITY_MIN
    };
    let max_gap = (opts.max_gap_ms / HOP_MS) as usize;
    let mut out = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    let close = |run: (usize, usize), out: &mut Vec<(usize, usize, f64)>| {
        // Trim to the first and last similar frame.
        let (lo, hi) = run;
        let Some(first) = (lo..=hi).find(|&i| sim[i]) else {
            return;
        };
        let last = (lo..=hi).rev().find(|&i| sim[i]).unwrap_or(first);
        let count = prefix[last + 1] - prefix[first];
        let density = f64::from(count) / (last - first + 1) as f64;
        out.push((first + start, last + start, density));
    };
    for i in 0..n {
        if !inside(i) {
            continue;
        }
        run = match run {
            Some((lo, hi)) if i - hi <= max_gap + 1 => Some((lo, i)),
            Some(r) => {
                close(r, &mut out);
                Some((i, i))
            }
            None => Some((i, i)),
        };
    }
    if let Some(r) = run {
        close(r, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic pseudo-random sequence.
    fn lcg(seed: u64, n: usize) -> Vec<u32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((x >> 32) as u32).max(1)
            })
            .collect()
    }

    /// `intro` placed at frame `at` of random content, with bit noise.
    fn episode(seed: u64, len: usize, intro: &[u32], at: usize, noise_bits: u32) -> Fingerprint {
        let mut h = lcg(seed, len);
        let noise = lcg(seed ^ 0xabc, intro.len());
        for (k, v) in intro.iter().enumerate() {
            // Flip `noise_bits` bits picked by the noise word.
            let mut x = *v;
            for b in 0..noise_bits {
                x ^= 1 << ((noise[k] >> (b * 5)) & 31);
            }
            h[at + k] = x.max(1);
        }
        Fingerprint {
            start_ms: 0,
            hashes: h,
        }
    }

    #[test]
    fn finds_a_shared_intro_at_different_offsets() {
        let intro = lcg(7, 1000); // 32 s
        let a = episode(1, 9000, &intro, 1500, 3);
        let b = episode(2, 9000, &intro, 4200, 3);
        let s = shared_segment(&a, &b, &MatchOptions::default()).unwrap();
        let ms = |f: usize| f as u64 * HOP_MS;
        assert!(s.a_start_ms.abs_diff(ms(1500)) <= 200, "{s:?}");
        assert!(s.a_end_ms.abs_diff(ms(2500) + FRAME_MS - HOP_MS) <= 300, "{s:?}");
        assert!(s.b_start_ms.abs_diff(ms(4200)) <= 200, "{s:?}");
        assert!(s.density > 0.9);
    }

    #[test]
    fn unrelated_audio_and_silence_do_not_match() {
        let a = Fingerprint {
            start_ms: 0,
            hashes: lcg(1, 9000),
        };
        let b = Fingerprint {
            start_ms: 0,
            hashes: lcg(2, 9000),
        };
        assert_eq!(shared_segment(&a, &b, &MatchOptions::default()), None);
        let quiet = Fingerprint {
            start_ms: 0,
            hashes: vec![SILENT; 9000],
        };
        assert_eq!(shared_segment(&quiet, &quiet, &MatchOptions::default()), None);
    }

    #[test]
    fn short_and_whole_file_matches_are_rejected() {
        let short = lcg(7, 200); // 6.4 s
        let a = episode(1, 5000, &short, 100, 2);
        let b = episode(2, 5000, &short, 900, 2);
        assert_eq!(shared_segment(&a, &b, &MatchOptions::default()), None);
        // The same file twice: one run longer than any intro.
        let same = Fingerprint {
            start_ms: 0,
            hashes: lcg(3, 9000),
        };
        assert_eq!(shared_segment(&same, &same, &MatchOptions::default()), None);
    }

    #[test]
    fn a_pause_inside_the_intro_is_bridged() {
        let mut intro = lcg(7, 1000);
        for h in &mut intro[400..420] {
            *h = SILENT; // 0.64 s of silence
        }
        let mut a = episode(1, 6000, &intro, 1000, 2);
        let mut b = episode(2, 6000, &intro, 2000, 2);
        for h in &mut a.hashes[1400..1420] {
            *h = SILENT;
        }
        for h in &mut b.hashes[2400..2420] {
            *h = SILENT;
        }
        let s = shared_segment(&a, &b, &MatchOptions::default()).unwrap();
        assert!(s.duration_ms() > 30_000, "{s:?}");
    }
}
