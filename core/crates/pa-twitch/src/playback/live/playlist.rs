// SPDX-License-Identifier: MPL-2.0
//! Twitch live media playlists (06 §2.1 step 4, §2.4), full builds only.
//!
//! Reads what the player needs for low latency and ad labelling:
//! segments with their program date-time, Twitch prefetch segments
//! (`#EXT-X-TWITCH-PREFETCH`), the standard LL-HLS tags (`#EXT-X-PART`,
//! `#EXT-X-PRELOAD-HINT`, only counted) and `#EXT-X-DATERANGE` ad markers.
//!
//! [`MediaPlaylist::rewrite`] renders the playlist for the player. It never
//! drops a segment: ad segments, ad markers and every unknown tag go to the
//! player as Twitch sent them. In low-latency mode the prefetch segments are
//! added as ordinary segments (what streamlink's `--twitch-low-latency`
//! does), because FFmpeg's HLS demuxer does not know the Twitch tag.

use std::collections::HashMap;

use crate::playback::PlaybackError;

/// `#EXT-X-DATERANGE` class Twitch uses for server-side stitched ads.
pub const TWITCH_AD_CLASS: &str = "twitch-stitched-ad";

/// One media segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Media sequence number.
    pub sequence: u64,
    /// Duration in seconds (`#EXTINF`; for prefetch segments, the duration
    /// of the last regular segment).
    pub duration: f64,
    /// URI as written in the playlist (relative or absolute).
    pub uri: String,
    /// `#EXTINF` title (`live` for content on Twitch, `Amazon|…` for ads).
    pub title: String,
    /// Program date-time of the first frame, Unix ms (from
    /// `#EXT-X-PROGRAM-DATE-TIME`, or inferred from the previous segment).
    pub program_date_time: Option<i64>,
    /// `#EXT-X-DISCONTINUITY` before this segment.
    pub discontinuity: bool,
    /// A Twitch prefetch segment: announced before it is complete.
    pub prefetch: bool,
    /// Part of an ad break (inside an ad `DATERANGE` or titled as an ad).
    pub ad: bool,
}

impl Segment {
    /// Program date-time of the end of the segment, Unix ms.
    pub fn end_date_time(&self) -> Option<i64> {
        self.program_date_time
            .map(|start| start + secs_to_ms(self.duration))
    }
}

/// One `#EXT-X-DATERANGE`.
#[derive(Debug, Clone, PartialEq)]
pub struct DateRange {
    /// `ID`.
    pub id: String,
    /// `CLASS`, if any.
    pub class: Option<String>,
    /// `START-DATE`, Unix ms.
    pub start: i64,
    /// `DURATION` (or `PLANNED-DURATION`) in seconds.
    pub duration: Option<f64>,
    /// All attributes as written (including `X-…` client attributes).
    pub attributes: HashMap<String, String>,
}

impl DateRange {
    /// Whether this range marks a Twitch ad break.
    pub fn is_ad(&self) -> bool {
        self.class.as_deref() == Some(TWITCH_AD_CLASS) || self.id.starts_with("stitched-ad-")
    }

    /// End, Unix ms, when the duration is known.
    pub fn end(&self) -> Option<i64> {
        self.duration.map(|d| self.start + secs_to_ms(d))
    }
}

/// An ad break announced by the playlist. Only for labelling: the ad is
/// played in full and never skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct AdBreak {
    /// `DATERANGE` id.
    pub id: String,
    /// Start, Unix ms (program date-time).
    pub start: i64,
    /// Planned length in seconds, if announced.
    pub duration: Option<f64>,
    /// Position of this ad in its pod (`X-TV-TWITCH-AD-POD-POSITION`, 0-based).
    pub pod_position: Option<u32>,
    /// Number of ads in the pod (`X-TV-TWITCH-AD-POD-LENGTH`).
    pub pod_length: Option<u32>,
}

impl AdBreak {
    /// End, Unix ms, when the duration is known.
    pub fn end(&self) -> Option<i64> {
        self.duration.map(|d| self.start + secs_to_ms(d))
    }

    /// Whether the program date-time `at` (Unix ms) is inside the break.
    /// A break without a duration covers only its start.
    pub fn contains(&self, at: i64) -> bool {
        at >= self.start && at < self.end().unwrap_or(self.start + 1)
    }
}

/// A parsed media playlist.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaPlaylist {
    /// `#EXT-X-TARGETDURATION`.
    pub target_duration: f64,
    /// `#EXT-X-MEDIA-SEQUENCE`.
    pub media_sequence: u64,
    /// `#EXT-X-ENDLIST` present (the stream ended).
    pub ended: bool,
    /// `#EXT-X-PLAYLIST-TYPE:EVENT`: segments are only ever appended (a
    /// Twitch VOD, also the archive of a stream still on air, TWI-025).
    pub event: bool,
    /// Regular segments, then the prefetch segments, in order.
    pub segments: Vec<Segment>,
    /// All `#EXT-X-DATERANGE` tags.
    pub date_ranges: Vec<DateRange>,
    /// `#EXT-X-PART-INF:PART-TARGET` (standard LL-HLS), if present.
    pub part_target: Option<f64>,
    /// Number of `#EXT-X-PART` tags (standard LL-HLS partial segments).
    pub parts: usize,
    /// `#EXT-X-PRELOAD-HINT` present.
    pub preload_hint: bool,
}

/// How the playlist is rendered for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveMode {
    /// Prefetch segments are played as soon as Twitch announces them and
    /// the player keeps a small buffer (target ≈ 2–4 s behind the edge).
    #[default]
    LowLatency,
    /// Only complete segments, a larger buffer.
    Stable,
}

impl MediaPlaylist {
    /// Parses a media playlist.
    pub fn parse(text: &str) -> Result<Self, PlaybackError> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some("#EXTM3U") {
            return Err(PlaybackError::Decode("not an HLS playlist".into()));
        }
        let mut out = MediaPlaylist::default();
        let mut pending: Option<(f64, String)> = None;
        let mut pdt: Option<i64> = None;
        let mut discontinuity = false;
        let mut next_seq: Option<u64> = None;
        for line in lines {
            if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
                out.target_duration = v.trim().parse().unwrap_or(0.0);
            } else if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                out.media_sequence = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("#EXTINF:") {
                let (duration, title) = v.split_once(',').unwrap_or((v, ""));
                let duration = duration
                    .trim()
                    .parse()
                    .map_err(|_| PlaybackError::Decode("bad #EXTINF duration".into()))?;
                pending = Some((duration, title.trim().to_owned()));
            } else if let Some(v) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
                pdt = crate::util::parse_rfc3339_ms(v);
            } else if line == "#EXT-X-DISCONTINUITY" {
                discontinuity = true;
            } else if line == "#EXT-X-ENDLIST" {
                out.ended = true;
            } else if let Some(v) = line.strip_prefix("#EXT-X-PLAYLIST-TYPE:") {
                out.event = v.trim() == "EVENT";
            } else if let Some(v) = line.strip_prefix("#EXT-X-DATERANGE:") {
                if let Some(range) = date_range(v) {
                    out.date_ranges.push(range);
                }
            } else if let Some(v) = line.strip_prefix("#EXT-X-PART-INF:") {
                out.part_target = attributes(v)
                    .get("PART-TARGET")
                    .and_then(|t| t.parse().ok());
            } else if line.starts_with("#EXT-X-PART:") {
                out.parts += 1;
            } else if line.starts_with("#EXT-X-PRELOAD-HINT:") {
                out.preload_hint = true;
            } else if let Some(uri) = line.strip_prefix("#EXT-X-TWITCH-PREFETCH:") {
                let seq = *next_seq.get_or_insert(out.media_sequence);
                let previous = out.segments.last();
                let duration = previous.map_or(out.target_duration, |s| s.duration);
                let start = previous.and_then(Segment::end_date_time);
                out.segments.push(Segment {
                    sequence: seq,
                    duration,
                    uri: uri.trim().to_owned(),
                    title: String::new(),
                    program_date_time: start,
                    discontinuity: false,
                    prefetch: true,
                    ad: false,
                });
                next_seq = Some(seq + 1);
            } else if line.starts_with('#') {
                // Other tags: kept by `rewrite`, not needed here.
            } else if let Some((duration, title)) = pending.take() {
                let seq = *next_seq.get_or_insert(out.media_sequence);
                let start = pdt.take().or_else(|| {
                    out.segments
                        .last()
                        .filter(|_| !discontinuity)
                        .and_then(Segment::end_date_time)
                });
                out.segments.push(Segment {
                    sequence: seq,
                    duration,
                    uri: line.to_owned(),
                    title,
                    program_date_time: start,
                    discontinuity: std::mem::take(&mut discontinuity),
                    prefetch: false,
                    ad: false,
                });
                next_seq = Some(seq + 1);
            }
        }
        let breaks = out.ad_breaks();
        for s in &mut out.segments {
            let in_break = s.program_date_time.is_some_and(|start| {
                let mid = start + secs_to_ms(s.duration) / 2;
                breaks.iter().any(|b| b.contains(mid))
            });
            s.ad = in_break || s.title.starts_with("Amazon");
        }
        Ok(out)
    }

    /// Ad breaks announced by `#EXT-X-DATERANGE` (class
    /// [`TWITCH_AD_CLASS`]), in playlist order.
    pub fn ad_breaks(&self) -> Vec<AdBreak> {
        let num = |r: &DateRange, k: &str| r.attributes.get(k).and_then(|v| v.parse().ok());
        self.date_ranges
            .iter()
            .filter(|r| r.is_ad())
            .map(|r| AdBreak {
                id: r.id.clone(),
                start: r.start,
                duration: r.duration,
                pod_position: num(r, "X-TV-TWITCH-AD-POD-POSITION"),
                pod_length: num(r, "X-TV-TWITCH-AD-POD-LENGTH"),
            })
            .collect()
    }

    /// Regular (complete) segments.
    pub fn complete(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter().filter(|s| !s.prefetch)
    }

    /// Prefetch segments.
    pub fn prefetch(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter().filter(|s| s.prefetch)
    }

    /// Live edge: program date-time of the end of the newest complete
    /// segment, Unix ms.
    pub fn edge(&self) -> Option<i64> {
        self.complete().last().and_then(Segment::end_date_time)
    }

    /// Renders the playlist for the player.
    ///
    /// Every line is kept as Twitch sent it, except that segment URIs are
    /// replaced by `segment_uri(segment)` (a loopback URL that streams the
    /// same bytes) and, in [`LiveMode::LowLatency`], each
    /// `#EXT-X-TWITCH-PREFETCH` line becomes `#EXTINF` + URI. In
    /// [`LiveMode::Stable`] the prefetch lines are left out, so the player
    /// waits for complete segments. The `URI` of `#EXT-X-MAP` (fMP4 init
    /// segment) is replaced by `map_uri(uri)`. No segment is ever removed,
    /// ads included.
    pub fn rewrite(
        &self,
        original: &str,
        mode: LiveMode,
        segment_uri: impl Fn(&Segment) -> String,
        map_uri: impl Fn(&str) -> String,
    ) -> String {
        let mut out = String::with_capacity(original.len() + 256);
        let mut segments = self.segments.iter();
        let mut expect_uri = false;
        for line in original.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if line.starts_with("#EXT-X-TWITCH-PREFETCH:") {
                let Some(s) = segments.next() else { continue };
                if mode == LiveMode::LowLatency {
                    out.push_str(&format!("#EXTINF:{:.3},prefetch\n", s.duration));
                    out.push_str(&segment_uri(s));
                    out.push('\n');
                }
                continue;
            }
            if let Some(list) = line.strip_prefix("#EXT-X-MAP:")
                && let Some(uri) = attributes(list).get("URI")
            {
                let quoted = format!("URI=\"{uri}\"");
                let mapped = format!("URI=\"{}\"", map_uri(uri));
                out.push_str(&line.replacen(&quoted, &mapped, 1));
                out.push('\n');
                continue;
            }
            if line.starts_with("#EXTINF:") {
                expect_uri = true;
            } else if !line.starts_with('#') && expect_uri {
                expect_uri = false;
                if let Some(s) = segments.next() {
                    out.push_str(&segment_uri(s));
                    out.push('\n');
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

/// Seconds → milliseconds, rounded.
pub(crate) fn secs_to_ms(secs: f64) -> i64 {
    (secs * 1000.0).round() as i64
}

fn date_range(list: &str) -> Option<DateRange> {
    let attributes = attributes(list);
    let id = attributes.get("ID")?.clone();
    let start = crate::util::parse_rfc3339_ms(attributes.get("START-DATE")?)?;
    let duration = attributes
        .get("DURATION")
        .or_else(|| attributes.get("PLANNED-DURATION"))
        .and_then(|d| d.parse().ok());
    Some(DateRange {
        id,
        class: attributes.get("CLASS").cloned(),
        start,
        duration,
        attributes,
    })
}

/// Parses an HLS attribute list (`KEY=value,KEY="quoted, value"`).
pub(crate) fn attributes(list: &str) -> HashMap<String, String> {
    super::super::hls::attributes(list)
}

/// Resolves a playlist URI against the playlist URL.
pub fn resolve_uri(base: &str, uri: &str) -> String {
    if uri.contains("://") {
        return uri.to_owned();
    }
    let Some(scheme_end) = base.find("://") else {
        return uri.to_owned();
    };
    let after = scheme_end + 3;
    let host_end = base[after..].find('/').map_or(base.len(), |i| after + i);
    if uri.starts_with('/') {
        return format!("{}{uri}", &base[..host_end]);
    }
    let path_end = base[host_end..]
        .find(['?', '#'])
        .map_or(base.len(), |i| host_end + i);
    let dir_end = base[host_end..path_end]
        .rfind('/')
        .map_or(host_end, |i| host_end + i);
    format!("{}/{uri}", &base[..dir_end])
}

/// Formats Unix ms as an HLS program date-time (`2026-11-14T12:00:00.000Z`).
pub fn format_date_time(unix_ms: i64) -> String {
    let secs = unix_ms.div_euclid(1000);
    let ms = unix_ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // H. Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-11-14T12:00:00.000Z";

    fn t0() -> i64 {
        crate::util::parse_rfc3339_ms(T0).unwrap()
    }

    /// A made-up Twitch-style low-latency playlist: two content segments,
    /// two ad segments under a stitched-ad range, one content segment, two
    /// prefetch segments.
    pub(crate) const LL: &str = "#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:100
#EXT-X-TWITCH-ELAPSED-SECS:200.000
#EXT-X-DATERANGE:ID=\"stitched-ad-1-4\",CLASS=\"twitch-stitched-ad\",START-DATE=\"2026-11-14T12:00:04.000Z\",DURATION=4.000,X-TV-TWITCH-AD-POD-LENGTH=\"1\",X-TV-TWITCH-AD-POD-POSITION=\"0\"
#EXT-X-PROGRAM-DATE-TIME:2026-11-14T12:00:00.000Z
#EXTINF:2.000,live
seg100.ts
#EXT-X-PROGRAM-DATE-TIME:2026-11-14T12:00:02.000Z
#EXTINF:2.000,live
seg101.ts
#EXT-X-PROGRAM-DATE-TIME:2026-11-14T12:00:04.000Z
#EXTINF:2.000,Amazon|fake-ad-1
seg102.ts
#EXT-X-PROGRAM-DATE-TIME:2026-11-14T12:00:06.000Z
#EXTINF:2.000,Amazon|fake-ad-1
seg103.ts
#EXT-X-PROGRAM-DATE-TIME:2026-11-14T12:00:08.000Z
#EXTINF:2.000,live
seg104.ts
#EXT-X-TWITCH-PREFETCH:https://edge.example.invalid/v1/segment/seg105.ts
#EXT-X-TWITCH-PREFETCH:https://edge.example.invalid/v1/segment/seg106.ts
";

    #[test]
    fn parses_segments_prefetch_and_ads() {
        let p = MediaPlaylist::parse(LL).unwrap();
        assert_eq!(p.target_duration, 2.0);
        assert_eq!(p.segments.len(), 7);
        let seqs: Vec<u64> = p.segments.iter().map(|s| s.sequence).collect();
        assert_eq!(seqs, [100, 101, 102, 103, 104, 105, 106]);
        let ads: Vec<u64> = p
            .segments
            .iter()
            .filter(|s| s.ad)
            .map(|s| s.sequence)
            .collect();
        assert_eq!(ads, [102, 103]);
        assert_eq!(p.prefetch().count(), 2);
        assert_eq!(p.segments[5].program_date_time, Some(t0() + 10_000));
        assert_eq!(p.segments[6].program_date_time, Some(t0() + 12_000));
        assert_eq!(p.edge(), Some(t0() + 10_000));
        let breaks = p.ad_breaks();
        assert_eq!(breaks.len(), 1);
        assert_eq!(breaks[0].start, t0() + 4000);
        assert_eq!(breaks[0].end(), Some(t0() + 8000));
        assert_eq!(breaks[0].pod_length, Some(1));
        assert!(breaks[0].contains(t0() + 7999));
        assert!(!breaks[0].contains(t0() + 8000));
    }

    #[test]
    fn rewrite_keeps_every_segment_and_marker() {
        let p = MediaPlaylist::parse(LL).unwrap();
        let ll = p.rewrite(
            LL,
            LiveMode::LowLatency,
            |s| format!("s/{}.ts", s.sequence),
            str::to_owned,
        );
        for seq in 100..=106 {
            assert!(
                ll.contains(&format!("s/{seq}.ts\n")),
                "{seq} missing:\n{ll}"
            );
        }
        assert!(ll.contains("CLASS=\"twitch-stitched-ad\""));
        assert!(ll.contains("#EXTINF:2.000,Amazon|fake-ad-1"));
        assert!(ll.contains("#EXT-X-TWITCH-ELAPSED-SECS:200.000"));
        assert!(!ll.contains("TWITCH-PREFETCH"));
        assert!(!ll.contains("seg10"), "upstream URIs must be mapped");
        // Reparsing the rendered playlist yields the same segments, ads
        // included, now all complete.
        let again = MediaPlaylist::parse(&ll).unwrap();
        assert_eq!(again.segments.len(), 7);
        assert_eq!(again.segments.iter().filter(|s| s.ad).count(), 2);

        let stable = p.rewrite(
            LL,
            LiveMode::Stable,
            |s| format!("s/{}.ts", s.sequence),
            str::to_owned,
        );
        for seq in 100..=104 {
            assert!(stable.contains(&format!("s/{seq}.ts\n")));
        }
        assert!(!stable.contains("s/105.ts"));
        assert_eq!(MediaPlaylist::parse(&stable).unwrap().segments.len(), 5);
    }

    #[test]
    fn standard_ll_hls_tags_are_counted_and_kept() {
        let text = "#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-PART-INF:PART-TARGET=0.5
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5
#EXT-X-MEDIA-SEQUENCE:7
#EXT-X-PROGRAM-DATE-TIME:2026-11-14T12:00:00Z
#EXTINF:2.0,
a.ts
#EXT-X-PART:DURATION=0.5,URI=\"b.0.ts\",INDEPENDENT=YES
#EXT-X-PART:DURATION=0.5,URI=\"b.1.ts\"
#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"b.2.ts\"
";
        let p = MediaPlaylist::parse(text).unwrap();
        assert_eq!(p.part_target, Some(0.5));
        assert_eq!(p.parts, 2);
        assert!(p.preload_hint);
        assert_eq!(p.segments.len(), 1);
        let out = p.rewrite(text, LiveMode::LowLatency, |_| "x.ts".into(), str::to_owned);
        assert!(out.contains("#EXT-X-PRELOAD-HINT:TYPE=PART"));
        assert!(out.contains("\nx.ts\n"));
    }

    #[test]
    fn init_segment_uri_is_mapped() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,live\na.mp4\n";
        let p = MediaPlaylist::parse(text).unwrap();
        let out = p.rewrite(
            text,
            LiveMode::LowLatency,
            |_| "s/0.mp4".into(),
            |u| format!("http://o/{u}"),
        );
        assert!(
            out.contains("#EXT-X-MAP:URI=\"http://o/init.mp4\"\n"),
            "{out}"
        );
    }

    #[test]
    fn inferred_date_time_stops_at_discontinuity() {
        let text = "#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-PROGRAM-DATE-TIME:2026-11-14T12:00:00Z
#EXTINF:2.0,live
a.ts
#EXTINF:2.0,live
b.ts
#EXT-X-DISCONTINUITY
#EXTINF:2.0,live
c.ts
";
        let p = MediaPlaylist::parse(text).unwrap();
        assert_eq!(p.segments[1].program_date_time, Some(t0() + 2000));
        assert!(p.segments[2].discontinuity);
        assert_eq!(p.segments[2].program_date_time, None);
    }

    #[test]
    fn uri_resolution() {
        let base = "https://h.example.invalid/a/b/index.m3u8?x=1";
        assert_eq!(
            resolve_uri(base, "s.ts"),
            "https://h.example.invalid/a/b/s.ts"
        );
        assert_eq!(resolve_uri(base, "/s.ts"), "https://h.example.invalid/s.ts");
        assert_eq!(resolve_uri(base, "http://o/s.ts"), "http://o/s.ts");
        assert_eq!(resolve_uri("http://h:1/p.m3u8", "s.ts"), "http://h:1/s.ts");
    }

    #[test]
    fn date_time_round_trip() {
        for ms in [0, t0(), t0() + 1234, 951_782_400_000 /* 2000-02-29 */] {
            let text = format_date_time(ms);
            assert_eq!(crate::util::parse_rfc3339_ms(&text), Some(ms), "{text}");
        }
        assert_eq!(format_date_time(t0()), T0);
    }

    #[test]
    fn event_playlist_type() {
        let text = "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-TARGETDURATION:10\n#EXTINF:10.0,\n0.ts\n";
        let p = MediaPlaylist::parse(text).unwrap();
        assert!(p.event && !p.ended);
        assert!(!MediaPlaylist::parse(LL).unwrap().event);
    }

    #[test]
    fn rejects_garbage() {
        assert!(MediaPlaylist::parse("hello").is_err());
        assert!(MediaPlaylist::parse("#EXTM3U\n#EXTINF:abc,\nx.ts").is_err());
    }
}
