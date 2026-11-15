// SPDX-License-Identifier: MPL-2.0
//! VOD media playlists: segments, muted marks and the segments of a range.

use serde::{Deserialize, Serialize};

/// One media segment of a VOD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Absolute URL.
    pub url: String,
    /// Start in the VOD, ms (sum of the previous `#EXTINF`).
    pub start_ms: u64,
    pub duration_ms: u64,
    /// Twitch muted the audio of this segment (`*-muted.ts`); it is
    /// downloaded as served and marked.
    pub muted: bool,
}

impl Segment {
    pub fn end_ms(&self) -> u64 {
        self.start_ms + self.duration_ms
    }
}

/// A parsed VOD media playlist.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MediaPlaylist {
    /// `#EXT-X-MAP` init segment of fMP4 renditions, absolute.
    pub init_url: Option<String>,
    pub segments: Vec<Segment>,
    /// `#EXT-X-ENDLIST` seen: the VOD is complete (not still growing).
    pub complete: bool,
    /// `#EXT-X-DISCONTINUITY` tags seen (stream copy keeps them as they
    /// are; reported in `info.json`).
    pub discontinuities: u32,
}

/// Why a playlist was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlaylistError {
    #[error("not an HLS playlist")]
    NotHls,
    #[error("the playlist has no segments")]
    Empty,
    #[error("the range {0}..{1} ms is outside the VOD")]
    Range(u64, u64),
}

/// Resolves `uri` against the playlist URL `base`.
pub fn resolve_url(base: &str, uri: &str) -> String {
    if uri.contains("://") {
        return uri.to_owned();
    }
    let base = base.split(['?', '#']).next().unwrap_or(base);
    if let Some(rest) = uri.strip_prefix('/') {
        // Origin-relative.
        let scheme_end = base.find("://").map_or(0, |i| i + 3);
        let host_end = base[scheme_end..]
            .find('/')
            .map_or(base.len(), |i| scheme_end + i);
        return format!("{}/{rest}", &base[..host_end]);
    }
    match base.rfind('/') {
        Some(i) => format!("{}/{uri}", &base[..i]),
        None => uri.to_owned(),
    }
}

impl MediaPlaylist {
    /// Parses a VOD media playlist fetched from `url`. `*-unmuted.ts`
    /// segments are not public; like other downloaders the muted copy
    /// (`*-muted.ts`) is taken instead.
    pub fn parse(text: &str, url: &str) -> Result<Self, PlaylistError> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some("#EXTM3U") {
            return Err(PlaylistError::NotHls);
        }
        let mut out = MediaPlaylist::default();
        let mut duration: Option<f64> = None;
        let mut at_ms = 0u64;
        for line in lines {
            if let Some(v) = line.strip_prefix("#EXTINF:") {
                let secs = v.split(',').next().unwrap_or("").trim();
                duration = secs
                    .parse::<f64>()
                    .ok()
                    .filter(|d| d.is_finite() && *d >= 0.0);
            } else if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:") {
                if let Some(uri) = attribute(attrs, "URI") {
                    out.init_url = Some(resolve_url(url, &uri));
                }
            } else if line == "#EXT-X-ENDLIST" {
                out.complete = true;
            } else if line == "#EXT-X-DISCONTINUITY" {
                out.discontinuities += 1;
            } else if line.starts_with('#') {
                continue;
            } else {
                let uri = line.replace("-unmuted.", "-muted.");
                let muted = uri.contains("-muted.");
                // Rounded to ms: the sum of rounded durations drifts by at
                // most 0.5 ms per segment.
                let duration_ms = (duration.take().unwrap_or(0.0) * 1000.0).round() as u64;
                out.segments.push(Segment {
                    url: resolve_url(url, &uri),
                    start_ms: at_ms,
                    duration_ms,
                    muted,
                });
                at_ms += duration_ms;
            }
        }
        if out.segments.is_empty() {
            return Err(PlaylistError::Empty);
        }
        Ok(out)
    }

    /// Total duration, ms.
    pub fn duration_ms(&self) -> u64 {
        self.segments.last().map_or(0, Segment::end_ms)
    }

    /// The segments to download for `range` (ms from the VOD start), or
    /// all of them. A partial download starts at the segment containing the
    /// range start (segments begin on a keyframe, so a stream copy can only
    /// start there) and ends exactly at the range end (the remux drops later
    /// packets).
    pub fn select(&self, range: Option<TimeRange>) -> Result<Selection, PlaylistError> {
        let total = self.duration_ms();
        let Some(range) = range else {
            return Ok(Selection {
                first: 0,
                count: self.segments.len(),
                start_ms: 0,
                end_ms: total,
                cut_end_ms: None,
            });
        };
        let end = range.end_ms.min(total);
        if range.start_ms >= end {
            return Err(PlaylistError::Range(range.start_ms, range.end_ms));
        }
        let first = self
            .segments
            .iter()
            .position(|s| s.end_ms() > range.start_ms)
            .ok_or(PlaylistError::Range(range.start_ms, range.end_ms))?;
        let last = self
            .segments
            .iter()
            .rposition(|s| s.start_ms < end)
            .unwrap_or(first)
            .max(first);
        let start_ms = self.segments[first].start_ms;
        let seg_end = self.segments[last].end_ms();
        Ok(Selection {
            first,
            count: last - first + 1,
            start_ms,
            end_ms: end,
            cut_end_ms: (end < seg_end).then_some(end - start_ms),
        })
    }

    /// Muted intervals of the selection, ms from its start, merged.
    pub fn muted_ranges(&self, sel: &Selection) -> Vec<TimeRange> {
        let mut out: Vec<TimeRange> = Vec::new();
        for s in &self.segments[sel.first..sel.first + sel.count] {
            if !s.muted {
                continue;
            }
            let start = s.start_ms.saturating_sub(sel.start_ms);
            let end = (s.end_ms().min(sel.end_ms)).saturating_sub(sel.start_ms);
            match out.last_mut() {
                Some(prev) if prev.end_ms >= start => prev.end_ms = prev.end_ms.max(end),
                _ => out.push(TimeRange {
                    start_ms: start,
                    end_ms: end,
                }),
            }
        }
        out
    }
}

/// A time range, ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// The segments of one download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Index of the first segment in the playlist.
    pub first: usize,
    pub count: usize,
    /// VOD time where the output starts (start of the first segment).
    pub start_ms: u64,
    /// VOD time where the output ends.
    pub end_ms: u64,
    /// Output time to cut at (packets at or after it are dropped), when the
    /// range ends inside a segment.
    pub cut_end_ms: Option<u64>,
}

impl Selection {
    /// Expected output duration, ms.
    pub fn duration_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

fn attribute(attrs: &str, name: &str) -> Option<String> {
    let start = attrs.find(&format!("{name}="))? + name.len() + 1;
    let rest = &attrs[start..];
    if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next().map(str::to_owned)
    } else {
        rest.split(',').next().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://cdn.example/abc/chunked/index-dvr.m3u8?x=1";

    fn playlist() -> MediaPlaylist {
        let text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
                    #EXT-X-TWITCH-TOTAL-SECS:35.5\n\
                    #EXTINF:10.000,\n0.ts\n#EXTINF:10.000,\n1-muted.ts\n\
                    #EXTINF:10.000,\n2-unmuted.ts\n#EXTINF:5.500,\n3.ts\n#EXT-X-ENDLIST\n";
        MediaPlaylist::parse(text, URL).unwrap()
    }

    #[test]
    fn parses_segments_and_muted_marks() {
        let p = playlist();
        assert!(p.complete);
        assert_eq!(p.segments.len(), 4);
        assert_eq!(p.duration_ms(), 35_500);
        assert_eq!(p.segments[0].url, "https://cdn.example/abc/chunked/0.ts");
        assert!(!p.segments[0].muted);
        assert!(p.segments[1].muted);
        assert_eq!(
            p.segments[2].url,
            "https://cdn.example/abc/chunked/2-muted.ts"
        );
        assert!(p.segments[2].muted);
        assert_eq!(p.segments[3].start_ms, 30_000);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(
            MediaPlaylist::parse("hello", URL),
            Err(PlaylistError::NotHls)
        );
        assert_eq!(
            MediaPlaylist::parse("#EXTM3U\n#EXT-X-ENDLIST\n", URL),
            Err(PlaylistError::Empty)
        );
    }

    #[test]
    fn urls() {
        assert_eq!(resolve_url(URL, "/x/1.ts"), "https://cdn.example/x/1.ts");
        assert_eq!(resolve_url(URL, "https://o/1.ts"), "https://o/1.ts");
        let p = MediaPlaylist::parse(
            "#EXTM3U\n#EXT-X-MAP:URI=\"init-0.mp4\"\n#EXTINF:2,\n0.mp4\n",
            URL,
        )
        .unwrap();
        assert_eq!(
            p.init_url.as_deref(),
            Some("https://cdn.example/abc/chunked/init-0.mp4")
        );
        assert!(!p.complete);
    }

    #[test]
    fn selection_of_a_range() {
        let p = playlist();
        let all = p.select(None).unwrap();
        assert_eq!((all.first, all.count, all.duration_ms()), (0, 4, 35_500));
        assert_eq!(all.cut_end_ms, None);

        // 12 s .. 25 s: segments 1 and 2, output starts at 10 s.
        let part = p
            .select(Some(TimeRange {
                start_ms: 12_000,
                end_ms: 25_000,
            }))
            .unwrap();
        assert_eq!((part.first, part.count, part.start_ms), (1, 2, 10_000));
        assert_eq!(part.cut_end_ms, Some(15_000));
        assert_eq!(part.duration_ms(), 15_000);

        // Exactly on segment boundaries: no cut.
        let exact = p
            .select(Some(TimeRange {
                start_ms: 10_000,
                end_ms: 20_000,
            }))
            .unwrap();
        assert_eq!((exact.first, exact.count, exact.cut_end_ms), (1, 1, None));

        assert!(
            p.select(Some(TimeRange {
                start_ms: 40_000,
                end_ms: 50_000
            }))
            .is_err()
        );
    }

    #[test]
    fn muted_ranges_merge() {
        let p = playlist();
        let all = p.select(None).unwrap();
        assert_eq!(
            p.muted_ranges(&all),
            vec![TimeRange {
                start_ms: 10_000,
                end_ms: 30_000
            }]
        );
        let part = p
            .select(Some(TimeRange {
                start_ms: 12_000,
                end_ms: 25_000,
            }))
            .unwrap();
        assert_eq!(
            p.muted_ranges(&part),
            vec![TimeRange {
                start_ms: 0,
                end_ms: 15_000
            }]
        );
    }
}
