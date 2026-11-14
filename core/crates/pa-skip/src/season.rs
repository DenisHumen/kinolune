// SPDX-License-Identifier: MPL-2.0
//! Agreement across a season: every episode is compared with its
//! neighbours, and the stretch most of them share with it wins.
//!
//! A pair of episodes may share more than the intro (a recap repeats the
//! previous episode, a song plays twice), but only the intro is shared by
//! almost every pair. Each episode collects what it shares with up to
//! [`NEIGHBOURS`] episodes on each side; candidates that start and end
//! within a few seconds of each other form a cluster, and the biggest
//! cluster gives the segment: the median start and end of its members.

use crate::fingerprint::Fingerprint;
use crate::matching::{MatchOptions, shared_segment};

/// Episodes compared with each one on each side (all of them in seasons
/// up to twice this size).
pub const NEIGHBOURS: usize = 6;
/// Candidates of one episode closer than this agree.
const START_TOLERANCE_MS: u64 = 3_000;
const END_TOLERANCE_MS: u64 = 5_000;

/// A segment the season agrees on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Agreed {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Share of the compared episodes that agree, 0–1.
    pub confidence: f64,
    /// How many episodes agree.
    pub support: usize,
}

/// For each fingerprint (one per episode, in order; `None` for an episode
/// without one), the stretch it shares with the others.
#[must_use]
pub fn agree(fps: &[Option<&Fingerprint>], opts: &MatchOptions) -> Vec<Option<Agreed>> {
    let n = fps.len();
    let mut found: Vec<Vec<(u64, u64)>> = vec![Vec::new(); n];
    let mut compared = vec![0usize; n];
    let window = if n <= 2 * NEIGHBOURS + 1 {
        n
    } else {
        NEIGHBOURS
    };
    for i in 0..n {
        let Some(a) = fps[i] else { continue };
        for j in i + 1..n.min(i + window + 1) {
            let Some(b) = fps[j] else { continue };
            compared[i] += 1;
            compared[j] += 1;
            if let Some(s) = shared_segment(a, b, opts) {
                found[i].push((s.a_start_ms, s.a_end_ms));
                found[j].push((s.b_start_ms, s.b_end_ms));
            }
        }
    }
    found
        .iter()
        .zip(&compared)
        .map(|(cands, &total)| cluster(cands, total))
        .collect()
}

/// The biggest cluster of `cands` as a segment.
fn cluster(cands: &[(u64, u64)], compared: usize) -> Option<Agreed> {
    let mut best: Option<Vec<(u64, u64)>> = None;
    for c in cands {
        let members: Vec<(u64, u64)> = cands
            .iter()
            .copied()
            .filter(|m| {
                m.0.abs_diff(c.0) <= START_TOLERANCE_MS && m.1.abs_diff(c.1) <= END_TOLERANCE_MS
            })
            .collect();
        let better = match &best {
            None => true,
            Some(b) => {
                members.len() > b.len()
                    || (members.len() == b.len() && span(&members) > span(b))
            }
        };
        if better {
            best = Some(members);
        }
    }
    let members = best?;
    let mut starts: Vec<u64> = members.iter().map(|m| m.0).collect();
    let mut ends: Vec<u64> = members.iter().map(|m| m.1).collect();
    let (start_ms, end_ms) = (median(&mut starts), median(&mut ends));
    if end_ms <= start_ms {
        return None;
    }
    Some(Agreed {
        start_ms,
        end_ms,
        confidence: (members.len() as f64 / compared.max(1) as f64).min(1.0),
        support: members.len(),
    })
}

/// Median duration of a cluster.
fn span(members: &[(u64, u64)]) -> u64 {
    let mut d: Vec<u64> = members.iter().map(|m| m.1.saturating_sub(m.0)).collect();
    median(&mut d)
}

fn median(v: &mut [u64]) -> u64 {
    v.sort_unstable();
    match v.len() {
        0 => 0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biggest_cluster_wins() {
        // Three agree on 60–90 s, one outlier (a recap match).
        let c = [
            (60_000, 90_000),
            (61_000, 90_500),
            (300_000, 340_000),
            (59_800, 89_900),
        ];
        let a = cluster(&c, 5).unwrap();
        assert_eq!((a.start_ms, a.end_ms), (60_000, 90_000));
        assert_eq!(a.support, 3);
        assert!((a.confidence - 0.6).abs() < 1e-9);
        assert_eq!(cluster(&[], 3), None);
    }

    #[test]
    fn median_of_even_and_odd() {
        assert_eq!(median(&mut [3, 1, 2]), 2);
        assert_eq!(median(&mut [4, 1, 2, 3]), 2);
        assert_eq!(median(&mut []), 0);
    }
}
