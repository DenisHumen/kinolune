// SPDX-License-Identifier: MPL-2.0
//! Blu-ray movie playlists (`BDMV/PLAYLIST/*.mpls`, LIB-021).
//!
//! A playlist lists the clips (`BDMV/STREAM/<clip>.m2ts`) a title plays,
//! in order, each with the in and out time of the part of the clip it
//! uses. The feature of a disc is usually its longest playlist: on a
//! seamless-branching disc the film is many clips joined by one playlist,
//! and no single `.m2ts` holds it ([`crate::structure`] picks the main
//! title with it).
//!
//! Only what the folder analysis needs is read: the play items, their clip
//! names, the first angle of multi-angle items and the in / out times.
//! Sub-paths, stream tables, marks and extension data are skipped. The
//! layout follows the public descriptions of the format (the field sizes
//! of `PlayList()` and `PlayItem()`); no code of other projects is used.
//! AVCHD `.mpl` files have the same layout.

use std::fmt;

/// Ticks per second of MPLS times (45 kHz).
pub const TICKS_PER_SECOND: u64 = 45_000;

/// Largest playlist file read; real ones are a few KiB.
pub const MAX_PLAYLIST_BYTES: usize = 512 * 1024;

/// One clip of a playlist: the part `in_time..out_time` of
/// `STREAM/<clip_id>.m2ts`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayItem {
    /// Clip name, five characters (`00055`); the stream file is
    /// `STREAM/<clip_id>.m2ts` (`.MTS` on AVCHD).
    pub clip_id: String,
    /// Start in the clip, 45 kHz ticks.
    pub in_time: u32,
    /// End in the clip, 45 kHz ticks.
    pub out_time: u32,
    /// Clips of angles 2, 3, … of a multi-angle item (PLAY-017); empty for
    /// an item with one angle. Each has the same times as angle 1.
    pub angles: Vec<String>,
}

impl PlayItem {
    /// Length of the item in milliseconds (0 if the times are reversed).
    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        u64::from(self.out_time.saturating_sub(self.in_time)) * 1000 / TICKS_PER_SECOND
    }

    /// The clip played for angle `angle` (0 = the first): the item's own
    /// clip if it has fewer angles.
    #[must_use]
    pub fn clip_for_angle(&self, angle: usize) -> &str {
        match angle.checked_sub(1).and_then(|i| self.angles.get(i)) {
            Some(c) => c,
            None => &self.clip_id,
        }
    }
}

/// Kind of an elementary stream of a playlist (its STN table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Video,
    Audio,
    /// Presentation graphics: PGS subtitles.
    Subtitle,
    /// Text subtitles (rare on discs).
    TextSubtitle,
}

/// One stream a playlist lists for its first play item (PLAY-017): the
/// stream table names the languages, which the transport stream does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub kind: StreamKind,
    /// Packet id in the clip (`0x1100` for the first audio stream).
    pub pid: u16,
    /// Coding type (`0x1B` H.264, `0x24` HEVC, `0x80` LPCM, `0x81` AC-3,
    /// `0x83` TrueHD, `0x86` DTS-HD MA, `0x90` PGS, …).
    pub coding: u8,
    /// ISO 639-2 language (`eng`, `rus`); empty for video.
    pub lang: String,
}

impl Stream {
    /// Short codec name for track menus (`TrueHD`, `DTS-HD MA`, `PGS`).
    #[must_use]
    pub fn codec_name(&self) -> &'static str {
        match self.coding {
            0x01 | 0x02 => "MPEG-2",
            0x1B => "H.264",
            0x20 => "MVC",
            0x24 => "HEVC",
            0xEA => "VC-1",
            0x03 | 0x04 => "MPEG Audio",
            0x80 => "LPCM",
            0x81 => "AC-3",
            0x82 => "DTS",
            0x83 => "TrueHD",
            0x84 | 0xA1 => "E-AC-3",
            0x85 => "DTS-HD",
            0x86 => "DTS-HD MA",
            0xA2 => "DTS Express",
            0x90 => "PGS",
            0x92 => "Text",
            _ => "",
        }
    }
}

/// A chapter of a playlist: an entry mark at `time` (45 kHz, clip time)
/// of play item `item`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub item: u16,
    pub time: u32,
}

/// A parsed playlist: its play items in order, the streams of the first
/// item and the chapter marks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playlist {
    pub items: Vec<PlayItem>,
    /// Video, audio and subtitle streams (PLAY-017); streams in sub-paths
    /// (other clips) are left out.
    pub streams: Vec<Stream>,
    /// Entry marks (chapters), in order.
    pub marks: Vec<Mark>,
}

impl Playlist {
    /// Total length of the items, in milliseconds.
    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        self.items.iter().map(PlayItem::duration_ms).sum()
    }

    /// Number of angles: the most any item has (1 for most discs).
    #[must_use]
    pub fn angle_count(&self) -> usize {
        1 + self.items.iter().map(|i| i.angles.len()).max().unwrap_or(0)
    }

    /// Starts of the chapters on the playlist's timeline (the items one
    /// after another), in milliseconds, increasing; marks outside their
    /// item are dropped.
    #[must_use]
    pub fn chapters_ms(&self) -> Vec<u64> {
        let mut starts = Vec::with_capacity(self.items.len());
        let mut at = 0;
        for it in &self.items {
            starts.push(at);
            at += it.duration_ms();
        }
        let mut out: Vec<u64> = Vec::new();
        for m in &self.marks {
            let Some(it) = self.items.get(usize::from(m.item)) else {
                continue;
            };
            if m.time < it.in_time || m.time > it.out_time {
                continue;
            }
            let ms = starts[usize::from(m.item)]
                + u64::from(m.time - it.in_time) * 1000 / TICKS_PER_SECOND;
            if out.last().is_none_or(|&l| ms > l) {
                out.push(ms);
            }
        }
        out
    }

    /// A minimal MPLS file with these items, angles, streams (in the first
    /// item) and marks: for tests and tools. [`parse`] reads it back.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        const PLAYLIST_AT: u32 = 58;
        let mut items = Vec::new();
        for (n, it) in self.items.iter().enumerate() {
            let mut body = Vec::new();
            body.extend_from_slice(&clip_bytes(&it.clip_id));
            body.extend_from_slice(b"M2TS");
            let multi = u16::from(!it.angles.is_empty()) << 4;
            body.extend_from_slice(&(multi | 1).to_be_bytes()); // connection condition 1
            body.push(0); // ref_to_STC_id
            body.extend_from_slice(&it.in_time.to_be_bytes());
            body.extend_from_slice(&it.out_time.to_be_bytes());
            body.extend_from_slice(&[0; 8]); // UO mask
            body.extend_from_slice(&[0, 0, 0, 0]); // random access, still mode, still time
            if !it.angles.is_empty() {
                body.push(u8::try_from(it.angles.len() + 1).expect("few angles"));
                body.push(0x01); // seamless angle change
                for a in &it.angles {
                    body.extend_from_slice(&clip_bytes(a));
                    body.extend_from_slice(b"M2TS");
                    body.push(0);
                }
            }
            let streams: &[Stream] = if n == 0 { &self.streams } else { &[] };
            body.extend_from_slice(&stn_table(streams));
            let len = u16::try_from(body.len()).expect("small item");
            items.extend_from_slice(&len.to_be_bytes());
            items.extend_from_slice(&body);
        }
        let mut list = Vec::new();
        list.extend_from_slice(&[0, 0]); // reserved
        let n = u16::try_from(self.items.len()).expect("fewer than 65536 items");
        list.extend_from_slice(&n.to_be_bytes());
        list.extend_from_slice(&0u16.to_be_bytes()); // sub-paths
        list.extend_from_slice(&items);
        let list_len = u32::try_from(list.len()).expect("small playlist");
        let marks_at = PLAYLIST_AT + 4 + list_len;

        let mut out = Vec::new();
        out.extend_from_slice(b"MPLS0200");
        out.extend_from_slice(&PLAYLIST_AT.to_be_bytes());
        out.extend_from_slice(&marks_at.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // no extension data
        out.resize(40, 0);
        // AppInfoPlayList: length 14, then 14 bytes.
        out.extend_from_slice(&14u32.to_be_bytes());
        out.resize(usize::try_from(PLAYLIST_AT).expect("fits"), 0);
        out.extend_from_slice(&list_len.to_be_bytes());
        out.extend_from_slice(&list);
        // PlayListMark: entry marks.
        let n = u16::try_from(self.marks.len()).expect("few marks");
        out.extend_from_slice(&(2 + 14 * u32::from(n)).to_be_bytes());
        out.extend_from_slice(&n.to_be_bytes());
        for m in &self.marks {
            out.extend_from_slice(&[0, 1]); // reserved, entry mark
            out.extend_from_slice(&m.item.to_be_bytes());
            out.extend_from_slice(&m.time.to_be_bytes());
            out.extend_from_slice(&0xFFFFu16.to_be_bytes()); // entry ES PID
            out.extend_from_slice(&0u32.to_be_bytes()); // duration
        }
        out
    }
}

fn clip_bytes(name: &str) -> [u8; 5] {
    let mut out = [b'0'; 5];
    for (o, b) in out.iter_mut().zip(name.bytes()) {
        *o = b;
    }
    out
}

/// An STN table with `streams` (each a type-1 entry: a stream of the
/// play item's own clip).
fn stn_table(streams: &[Stream]) -> Vec<u8> {
    let count = |k: StreamKind| {
        u8::try_from(streams.iter().filter(|s| s.kind == k).count()).expect("few streams")
    };
    let mut body = vec![0u8, 0];
    body.push(count(StreamKind::Video));
    body.push(count(StreamKind::Audio));
    body.push(count(StreamKind::Subtitle) + count(StreamKind::TextSubtitle));
    body.extend_from_slice(&[0; 9]); // IG, secondary, PiP counts, reserved
    for kind in [
        StreamKind::Video,
        StreamKind::Audio,
        StreamKind::Subtitle,
        StreamKind::TextSubtitle,
    ] {
        for s in streams.iter().filter(|s| s.kind == kind) {
            // stream_entry: type 1, PID, padding to 9 bytes.
            body.push(9);
            body.push(1);
            body.extend_from_slice(&s.pid.to_be_bytes());
            body.extend_from_slice(&[0; 6]);
            let mut lang = [b' '; 3];
            for (o, b) in lang.iter_mut().zip(s.lang.bytes()) {
                *o = b;
            }
            let attr: Vec<u8> = match kind {
                StreamKind::Video => vec![s.coding, 0x61, 0],
                StreamKind::Audio => [&[s.coding, 0x31][..], &lang].concat(),
                StreamKind::Subtitle => [&[s.coding][..], &lang].concat(),
                StreamKind::TextSubtitle => [&[s.coding, 1][..], &lang].concat(),
            };
            body.push(5);
            body.extend_from_slice(&attr);
            body.resize(body.len() + 5 - attr.len(), 0);
        }
    }
    let mut out = u16::try_from(body.len())
        .expect("small")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(&body);
    out
}

/// Why a file is not a readable playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MplsError {
    /// No `MPLS` signature (or an unknown version).
    NotMpls,
    /// A field points or reaches past the end of the file.
    Truncated,
    /// A field holds an impossible value.
    Invalid(&'static str),
}

impl fmt::Display for MplsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMpls => f.write_str("not an MPLS playlist"),
            Self::Truncated => f.write_str("truncated MPLS playlist"),
            Self::Invalid(what) => write!(f, "invalid MPLS playlist: {what}"),
        }
    }
}

impl std::error::Error for MplsError {}

/// Big-endian reads with bounds checks.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], MplsError> {
        let end = self.at.checked_add(n).ok_or(MplsError::Truncated)?;
        let out = self.data.get(self.at..end).ok_or(MplsError::Truncated)?;
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, MplsError> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MplsError> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, MplsError> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn clip_name(raw: &[u8]) -> Result<String, MplsError> {
    if raw.iter().all(u8::is_ascii_alphanumeric) {
        Ok(String::from_utf8_lossy(raw).into_owned())
    } else {
        Err(MplsError::Invalid("clip name"))
    }
}

/// The streams of an STN table starting at `r` (its length field).
fn streams(r: &mut Reader<'_>, end: usize) -> Result<Vec<Stream>, MplsError> {
    let len = usize::from(r.u16()?);
    let table_end = r.at.checked_add(len).ok_or(MplsError::Truncated)?;
    if table_end > end {
        return Err(MplsError::Invalid("stream table length"));
    }
    let _reserved = r.u16()?;
    let video = r.u8()?;
    let audio = r.u8()?;
    let pg = r.u8()?;
    let _ig = r.u8()?;
    let _secondary_audio = r.u8()?;
    let _secondary_video = r.u8()?;
    let pip_pg = r.u8()?;
    r.bytes(5)?;
    let mut out = Vec::new();
    let groups = [
        (video, Some(StreamKind::Video)),
        (audio, Some(StreamKind::Audio)),
        (pg.saturating_add(pip_pg), Some(StreamKind::Subtitle)),
    ];
    for (count, kind) in groups {
        for _ in 0..count {
            let entry_len = usize::from(r.u8()?);
            let entry = r.bytes(entry_len)?;
            let attr_len = usize::from(r.u8()?);
            let attr = r.bytes(attr_len)?;
            if r.at > table_end {
                return Err(MplsError::Invalid("stream past the table"));
            }
            // Type 1: a stream of the item's own clip; others live in
            // sub-paths (other clips) and are not played here.
            let (Some(kind), [1, hi, lo, ..], [coding, rest @ ..]) = (kind, entry, attr) else {
                continue;
            };
            let pid = u16::from_be_bytes([*hi, *lo]);
            let lang_at = match (kind, *coding) {
                (StreamKind::Video, _) => None,
                (StreamKind::Audio, _) => Some(1),
                (_, 0x92) => Some(1),
                _ => Some(0),
            };
            let kind = if kind == StreamKind::Subtitle && *coding == 0x92 {
                StreamKind::TextSubtitle
            } else {
                kind
            };
            let lang = lang_at
                .and_then(|i| rest.get(i..i + 3))
                .map(|l| {
                    l.iter()
                        .filter(|c| c.is_ascii_alphabetic())
                        .map(|c| char::from(c.to_ascii_lowercase()))
                        .collect()
                })
                .unwrap_or_default();
            out.push(Stream {
                kind,
                pid,
                coding: *coding,
                lang,
            });
        }
    }
    r.at = table_end;
    Ok(out)
}

/// Parses an MPLS (or AVCHD MPL) file.
///
/// # Errors
///
/// [`MplsError`] when the data is not a playlist or is cut short.
pub fn parse(data: &[u8]) -> Result<Playlist, MplsError> {
    if data.len() < 20 || &data[..4] != b"MPLS" {
        return Err(MplsError::NotMpls);
    }
    if !matches!(&data[4..8], b"0100" | b"0200" | b"0300") {
        return Err(MplsError::NotMpls);
    }
    let mut header = Reader { data, at: 8 };
    let playlist_at = usize::try_from(header.u32()?).map_err(|_| MplsError::Truncated)?;
    let marks_at = usize::try_from(header.u32()?).map_err(|_| MplsError::Truncated)?;
    let mut r = Reader {
        data,
        at: playlist_at,
    };
    let _length = r.u32()?;
    let _reserved = r.u16()?;
    let count = r.u16()?;
    let _sub_paths = r.u16()?;
    let mut items = Vec::with_capacity(usize::from(count));
    let mut streams_of_first = Vec::new();
    for n in 0..count {
        let len = usize::from(r.u16()?);
        let start = r.at;
        let end = start.checked_add(len).ok_or(MplsError::Truncated)?;
        if end > data.len() {
            return Err(MplsError::Truncated);
        }
        let mut it = Reader { data, at: start };
        let name = clip_name(it.bytes(5)?)?;
        let _codec = it.bytes(4)?;
        // 11 bits reserved, is_multi_angle, 4 bits connection condition;
        // the clip of angle 1 is the one named above.
        let flags = it.u16()?;
        let _stc = it.u8()?;
        let in_time = it.u32()?;
        let out_time = it.u32()?;
        if it.at > end {
            return Err(MplsError::Invalid("play item length"));
        }
        if out_time < in_time {
            return Err(MplsError::Invalid("out time before in time"));
        }
        // UO mask, random access flag, still mode and time.
        let _rest = it.bytes(12);
        let mut angles = Vec::new();
        if flags & 0x10 != 0 && it.at <= end {
            let n_angles = it.u8()?;
            let _angle_flags = it.u8()?;
            for _ in 1..n_angles {
                let clip = clip_name(it.bytes(5)?)?;
                let _codec = it.bytes(4)?;
                let _stc = it.u8()?;
                angles.push(clip);
            }
            if it.at > end {
                return Err(MplsError::Invalid("angles past the play item"));
            }
        }
        // Streams of the first item: later items repeat the same table
        // for the next clips. A broken table only loses the languages.
        if n == 0 && it.at + 2 <= end {
            streams_of_first = streams(&mut it, end).unwrap_or_default();
        }
        items.push(PlayItem {
            clip_id: name,
            in_time,
            out_time,
            angles,
        });
        r.at = end;
    }
    Ok(Playlist {
        items,
        streams: streams_of_first,
        marks: marks(data, marks_at),
    })
}

/// Entry marks (chapters) of the PlayListMark at `at`; none if it is
/// missing or broken.
fn marks(data: &[u8], at: usize) -> Vec<Mark> {
    if at < 40 {
        return Vec::new();
    }
    let mut r = Reader { data, at };
    let (Ok(len), Ok(n)) = (r.u32(), r.u16()) else {
        return Vec::new();
    };
    if u64::from(n) * 14 + 2 > u64::from(len) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let Ok(m) = r.bytes(14) else { break };
        if m[1] == 1 {
            out.push(Mark {
                item: u16::from_be_bytes([m[2], m[3]]),
                time: u32::from_be_bytes([m[4], m[5], m[6], m[7]]),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(clip: &str, secs: u32) -> PlayItem {
        PlayItem {
            clip_id: clip.into(),
            in_time: 27_000_000,
            out_time: 27_000_000 + secs * 45_000,
            angles: Vec::new(),
        }
    }

    #[test]
    fn round_trip_and_duration() {
        let pl = Playlist {
            items: vec![item("00055", 3600), item("00056", 1800), item("00060", 90)],
            ..Playlist::default()
        };
        let bytes = pl.to_bytes();
        let back = parse(&bytes).unwrap();
        assert_eq!(back, pl);
        assert_eq!(back.duration_ms(), (3600 + 1800 + 90) * 1000);
    }

    #[test]
    fn streams_angles_and_chapters_round_trip() {
        let stream = |kind, pid, coding, lang: &str| Stream {
            kind,
            pid,
            coding,
            lang: lang.into(),
        };
        let mut second = item("00002", 600);
        second.angles = vec!["00003".into()];
        let pl = Playlist {
            items: vec![item("00001", 1200), second, item("00004", 300)],
            streams: vec![
                stream(StreamKind::Video, 0x1011, 0x1B, ""),
                stream(StreamKind::Audio, 0x1100, 0x83, "eng"),
                stream(StreamKind::Audio, 0x1101, 0x81, "rus"),
                stream(StreamKind::Audio, 0x1102, 0x86, "ukr"),
                stream(StreamKind::Subtitle, 0x1200, 0x90, "eng"),
                stream(StreamKind::Subtitle, 0x1201, 0x90, "rus"),
            ],
            marks: vec![
                Mark {
                    item: 0,
                    time: 27_000_000,
                },
                Mark {
                    item: 0,
                    time: 27_000_000 + 600 * 45_000,
                },
                Mark {
                    item: 1,
                    time: 27_000_000,
                },
                Mark {
                    item: 2,
                    time: 27_000_000 + 60 * 45_000,
                },
                // Outside its item, and an item that does not exist.
                Mark { item: 2, time: 5 },
                Mark {
                    item: 9,
                    time: 27_000_000,
                },
            ],
        };
        let back = parse(&pl.to_bytes()).unwrap();
        assert_eq!(back, pl);
        assert_eq!(back.angle_count(), 2);
        assert_eq!(back.items[1].clip_for_angle(1), "00003");
        assert_eq!(back.items[0].clip_for_angle(1), "00001");
        assert_eq!(back.chapters_ms(), [0, 600_000, 1_200_000, 1_860_000]);
        assert_eq!(back.streams[3].codec_name(), "DTS-HD MA");
        assert_eq!(back.streams[4].codec_name(), "PGS");
    }

    #[test]
    fn multi_angle_items_use_angle_one() {
        // A hand-built item with is_multi_angle set and a second angle
        // after the fixed fields; the item length skips it.
        let mut body = Vec::new();
        body.extend_from_slice(b"00010M2TS");
        body.extend_from_slice(&0x0011u16.to_be_bytes()); // multi angle, cc 1
        body.push(0);
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&90_000u32.to_be_bytes());
        body.extend_from_slice(&[0; 12]);
        body.extend_from_slice(&[2, 0]); // two angles
        body.extend_from_slice(b"00011M2TS");
        body.push(0);
        let mut data = b"MPLS0200".to_vec();
        data.extend_from_slice(&20u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes()); // length
        data.extend_from_slice(&[0, 0, 0, 1, 0, 0]);
        data.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        data.extend_from_slice(&body);
        let pl = parse(&data).unwrap();
        assert_eq!(pl.items.len(), 1);
        assert_eq!(pl.items[0].clip_id, "00010");
        assert_eq!(pl.items[0].angles, ["00011"]);
        assert_eq!(pl.items[0].clip_for_angle(1), "00011");
        assert_eq!(pl.items[0].clip_for_angle(5), "00010");
        assert_eq!(pl.angle_count(), 2);
        assert!(pl.marks.is_empty());
        assert_eq!(pl.duration_ms(), 2000);
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(parse(b""), Err(MplsError::NotMpls));
        assert_eq!(parse(&[7u8; 300]), Err(MplsError::NotMpls));
        assert_eq!(parse(b"MPLS9999aaaaaaaaaaaaaaaa"), Err(MplsError::NotMpls));
        let mut bytes = Playlist {
            items: vec![item("00001", 10)],
            ..Playlist::default()
        }
        .to_bytes();
        bytes.truncate(70);
        assert_eq!(parse(&bytes), Err(MplsError::Truncated));
        // Points past the end.
        let mut far = b"MPLS0200".to_vec();
        far.extend_from_slice(&u32::MAX.to_be_bytes());
        far.extend_from_slice(&[0; 8]);
        assert_eq!(parse(&far), Err(MplsError::Truncated));
        // A clip name that is not a name.
        let mut bad = Playlist {
            items: vec![item("00001", 10)],
            ..Playlist::default()
        }
        .to_bytes();
        bad[58 + 4 + 6 + 2] = b'/';
        assert_eq!(parse(&bad), Err(MplsError::Invalid("clip name")));
    }
}
