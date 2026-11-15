// SPDX-License-Identifier: MPL-2.0
//! Mapping wall-clock moments (program date-times, chat timestamps,
//! category changes) onto the recording's own timeline, where gaps are
//! folded away (the remux makes the output continuous).

use super::journal::{GapEntry, SegEntry};

/// Where a moment falls in the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Offset in the recording, ms.
    pub offset_ms: u64,
    /// The moment is in a gap (or before or after the recording): the
    /// offset is the nearest recorded point.
    pub outside: bool,
}

#[derive(Debug, Clone, Copy)]
struct Span {
    /// Program date-time of the first frame, Unix ms.
    pdt: i64,
    dur_ms: u64,
    /// Offset of the segment in the recording.
    at_ms: u64,
}

/// The recording's timeline: recorded segments back to back.
#[derive(Debug, Clone, Default)]
pub struct RecTimeline {
    spans: Vec<Span>,
    total_ms: u64,
}

impl RecTimeline {
    /// From the journal. Segments without a program date-time get one from
    /// the previous segment.
    pub fn new(segs: &[SegEntry]) -> Self {
        let mut spans = Vec::with_capacity(segs.len());
        let mut at = 0u64;
        let mut prev_end: Option<i64> = None;
        for s in segs {
            let pdt = s.pdt.or(prev_end).unwrap_or(0);
            spans.push(Span {
                pdt,
                dur_ms: s.dur_ms,
                at_ms: at,
            });
            at += s.dur_ms;
            prev_end = Some(pdt + i64::try_from(s.dur_ms).unwrap_or(0));
        }
        Self {
            spans,
            total_ms: at,
        }
    }

    pub fn total_ms(&self) -> u64 {
        self.total_ms
    }

    /// Program date-time of the first recorded frame.
    pub fn start_pdt(&self) -> Option<i64> {
        self.spans.first().map(|s| s.pdt)
    }

    /// Program date-time of the end of the recording.
    pub fn end_pdt(&self) -> Option<i64> {
        self.spans
            .last()
            .map(|s| s.pdt + i64::try_from(s.dur_ms).unwrap_or(0))
    }

    /// Places the wall-clock moment `at` (Unix ms).
    pub fn place(&self, at: i64) -> Placed {
        // Last segment starting at or before `at`.
        let i = self.spans.partition_point(|s| s.pdt <= at);
        if i == 0 {
            return Placed {
                offset_ms: 0,
                outside: true,
            };
        }
        let s = self.spans[i - 1];
        let into = u64::try_from(at - s.pdt).unwrap_or(0);
        if into < s.dur_ms {
            Placed {
                offset_ms: s.at_ms + into,
                outside: false,
            }
        } else {
            Placed {
                offset_ms: s.at_ms + s.dur_ms,
                outside: true,
            }
        }
    }

    /// Offset in the recording of the start of each gap, with its length.
    pub fn gaps(&self, gaps: &[GapEntry], segs: &[SegEntry]) -> Vec<(u64, u64)> {
        gaps.iter()
            .map(|g| {
                // The gap sits before the first recorded segment after it.
                let i = segs.partition_point(|s| s.seq < g.to_seq);
                let at = self.spans.get(i).map_or(self.total_ms, |s| s.at_ms);
                (at, g.missing_ms)
            })
            .collect()
    }

    /// Runs of ad segments as (start, end) offsets.
    pub fn ad_ranges(&self, segs: &[SegEntry]) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        for (s, span) in segs.iter().zip(&self.spans) {
            if !s.ad {
                continue;
            }
            let end = span.at_ms + span.dur_ms;
            match out.last_mut() {
                Some(last) if last.1 == span.at_ms => last.1 = end,
                _ => out.push((span.at_ms, end)),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(seq: u64, pdt: i64, ad: bool) -> SegEntry {
        SegEntry {
            seq,
            dur_ms: 2000,
            pdt: Some(pdt),
            ad,
            disc: false,
            part: 1,
            off: 0,
            len: 1,
            at: 0,
        }
    }

    #[test]
    fn gaps_are_folded() {
        // 0..6 s recorded, 6..10 s missing, 10..14 s recorded.
        let segs = vec![
            seg(1, 1000, false),
            seg(2, 3000, true),
            seg(3, 5000, true),
            seg(6, 11_000, false),
            seg(7, 13_000, false),
        ];
        let t = RecTimeline::new(&segs);
        assert_eq!(t.total_ms(), 10_000);
        assert_eq!(
            t.place(500),
            Placed {
                offset_ms: 0,
                outside: true
            }
        );
        assert_eq!(t.place(1000).offset_ms, 0);
        assert_eq!(
            t.place(4500),
            Placed {
                offset_ms: 3500,
                outside: false
            }
        );
        assert_eq!(
            t.place(8000),
            Placed {
                offset_ms: 6000,
                outside: true
            }
        );
        assert_eq!(
            t.place(12_000),
            Placed {
                offset_ms: 7000,
                outside: false
            }
        );
        assert_eq!(
            t.place(99_000),
            Placed {
                offset_ms: 10_000,
                outside: true
            }
        );
        assert_eq!(t.ad_ranges(&segs), vec![(2000, 6000)]);
        let gap = GapEntry {
            from_seq: 4,
            to_seq: 6,
            from_pdt: Some(7000),
            to_pdt: Some(11_000),
            missing_ms: 4000,
            reason: "window".into(),
            at: 0,
        };
        assert_eq!(t.gaps(&[gap], &segs), vec![(6000, 4000)]);
    }
}
