// SPDX-License-Identifier: MPL-2.0
//! What a Blu-ray or DVD plays (PLAY-017, ADR-0140): the main title with
//! all its audio and subtitle streams and their languages, chapters and
//! angles, the extras, and whether the disc is protected.
//!
//! [`plan`] reads a disc folder (`BDMV`, `VIDEO_TS`) through any [`Vfs`]: a
//! folder of a source, or a disc image opened with
//! [`pa_vfs::iso::IsoFs`]. Only the navigation files are read (playlists,
//! clip infos, IFOs, a few sectors of a VOB); nothing is decrypted.
//!
//! - **Blu-ray:** the longest playlist whose clips are all present is the
//!   main title; the others longer than [`EXTRA_MIN_MS`] that do not just
//!   repeat clips of the main title are extras. A play item that uses part
//!   of a clip (seamless branching) becomes a segment with a start and
//!   length taken from the clip's presentation start (`.clpi`). Streams
//!   and languages come from the playlist's stream table, chapters from its
//!   entry marks; angles 2… are clip lists of their own. 3D (MVC) plays as
//!   2D: the dependent view is a separate stream that the player leaves
//!   out.
//! - **DVD:** the title set with the most VOB data is the main title, its
//!   VOBs played back to back; audio and subpicture languages and the
//!   chapters of its longest program chain come from `VTS_nn_0.IFO`.
//! - **Protection:** `AACS/` (AACS), `BDSVM/` (BD+) or scrambled VOB packs
//!   (CSS) make the disc protected; it is not played (ADR-0139).

use std::collections::{BTreeMap, HashSet};

use pa_vfs::{ByteSource, EntryKind, Result, Vfs, VfsPath};

use crate::mpls::{self, MAX_PLAYLIST_BYTES, Playlist, StreamKind, TICKS_PER_SECOND};

/// Shortest playlist listed as an extra (two minutes): shorter ones are
/// menus, logos and warnings.
pub const EXTRA_MIN_MS: u64 = 120_000;

/// Most extras listed for one disc.
const MAX_EXTRAS: usize = 30;

/// Most playlists read from one disc.
const MAX_PLAYLISTS: usize = 2000;

/// Kind of disc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscKind {
    BluRay,
    Dvd,
}

/// Copy protection found on a disc (read only, never removed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// `AACS/` on a Blu-ray.
    Aacs,
    /// `BDSVM/` (BD+) on a Blu-ray.
    BdPlus,
    /// Scrambled packs in the VOBs of a DVD.
    Css,
}

impl Protection {
    /// Stable code for clients.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Aacs => "aacs",
            Self::BdPlus => "bdplus",
            Self::Css => "css",
        }
    }
}

/// One file of a title, or the part `start_ms..start_ms + length_ms` of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscSegment {
    /// Path relative to the disc root (`/BDMV/STREAM/00010.m2ts`).
    pub path: VfsPath,
    pub size: u64,
    /// Start in the file, from its first timestamp; `None` from the start.
    pub start_ms: Option<u64>,
    /// Length; `None` to the end.
    pub length_ms: Option<u64>,
}

/// An audio, subtitle or video stream of a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscStream {
    pub kind: StreamKind,
    /// Id of the stream in the file as demuxers report it: the PID of a
    /// Blu-ray stream (`0x1100`), the substream of a DVD stream (`0x80`
    /// AC-3, `0x20` subpicture).
    pub id: u16,
    /// Codec for track menus (`TrueHD`, `PGS`, `AC-3`, `VobSub`).
    pub codec: String,
    /// ISO 639-2 language (`eng`) for Blu-ray, ISO 639-1 (`en`) for DVD;
    /// empty if not known.
    pub lang: String,
}

/// A title of a disc: the main film or an extra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscTitle {
    /// Playlist (`00800`) or title set (`VTS_01`).
    pub id: String,
    pub duration_ms: u64,
    /// The segments to play for each angle; at least one angle.
    pub angles: Vec<Vec<DiscSegment>>,
    /// Chapter starts on the title's timeline, increasing.
    pub chapters_ms: Vec<u64>,
    pub streams: Vec<DiscStream>,
    /// Ultra HD Blu-ray (HEVC video).
    pub uhd: bool,
    /// Stereoscopic 3D (MVC); played as 2D.
    pub is_3d: bool,
}

/// What a disc plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscPlan {
    pub kind: DiscKind,
    pub protection: Option<Protection>,
    /// `None` if no title is playable (all clips missing).
    pub main: Option<DiscTitle>,
    /// Blu-ray extras, longest first.
    pub extras: Vec<DiscTitle>,
}

/// Children of `dir` by upper-cased name.
fn children(vfs: &dyn Vfs, dir: &VfsPath) -> BTreeMap<String, (VfsPath, EntryKind, u64)> {
    vfs.list(dir)
        .map(|entries| {
            entries
                .into_iter()
                .map(|e| {
                    (
                        e.name.to_ascii_uppercase(),
                        (e.path, e.metadata.kind, e.metadata.size),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read(vfs: &dyn Vfs, path: &VfsPath, limit: usize) -> Option<Vec<u8>> {
    let file = vfs.open(path).ok()?;
    let len = usize::try_from(file.size())
        .unwrap_or(usize::MAX)
        .min(limit);
    let mut buf = vec![0u8; len];
    file.read_exact_at(0, &mut buf).ok()?;
    Some(buf)
}

/// The disc folder `root` (holding `BDMV` or `VIDEO_TS`) as a plan; `None`
/// if it holds neither.
///
/// # Errors
///
/// Listing `root` fails.
pub fn plan(vfs: &dyn Vfs, root: &VfsPath) -> Result<Option<DiscPlan>> {
    let top = {
        let entries = vfs.list(root)?;
        entries
            .into_iter()
            .map(|e| (e.name.to_ascii_uppercase(), (e.path, e.metadata.kind)))
            .collect::<BTreeMap<_, _>>()
    };
    let dir = |name: &str| {
        top.get(name)
            .filter(|(_, k)| *k == EntryKind::Dir)
            .map(|(p, _)| p.clone())
    };
    if let Some(bdmv) = dir("BDMV") {
        let protection = if dir("AACS").is_some() {
            Some(Protection::Aacs)
        } else if dir("BDSVM").is_some() {
            Some(Protection::BdPlus)
        } else {
            None
        };
        return Ok(Some(blu_ray(vfs, &bdmv, protection)));
    }
    if let Some(video_ts) = dir("VIDEO_TS") {
        return Ok(Some(dvd(vfs, &video_ts)));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Blu-ray

/// Presentation start and end (45 kHz) of a clip, from its `.clpi`.
fn clip_times(
    vfs: &dyn Vfs,
    clipinf: &BTreeMap<String, (VfsPath, EntryKind, u64)>,
    clip: &str,
) -> Option<(u32, u32)> {
    let (path, _, _) = clipinf.get(&format!("{clip}.CLPI"))?;
    parse_clpi(&read(vfs, path, MAX_PLAYLIST_BYTES)?)
}

/// Presentation start of the first and end of the last STC sequence of a
/// clip info file.
#[must_use]
pub fn parse_clpi(b: &[u8]) -> Option<(u32, u32)> {
    if b.get(..4)? != b"HDMV" {
        return None;
    }
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
    };
    let seq = usize::try_from(be32(8)?).ok()?;
    let n_atc = *b.get(seq + 5)?;
    let mut at = seq + 6;
    let mut first = None;
    let mut last = None;
    for _ in 0..n_atc {
        let n_stc = *b.get(at + 4)?;
        at += 6;
        for _ in 0..n_stc {
            let start = be32(at + 6)?;
            let end = be32(at + 10)?;
            first.get_or_insert(start);
            last = Some(end);
            at += 14;
        }
    }
    Some((first?, last?))
}

/// A clip info file with one STC sequence `start..end` (for tests and
/// tools).
#[must_use]
pub fn clpi_bytes(start: u32, end: u32) -> Vec<u8> {
    let mut b = b"HDMV0200".to_vec();
    b.extend_from_slice(&40u32.to_be_bytes()); // SequenceInfo
    b.resize(40, 0);
    b.extend_from_slice(&22u32.to_be_bytes()); // length
    b.extend_from_slice(&[0, 1]); // reserved, one ATC sequence
    b.extend_from_slice(&0u32.to_be_bytes()); // SPN_ATC_start
    b.extend_from_slice(&[1, 0]); // one STC sequence, offset id
    b.extend_from_slice(&0x1001u16.to_be_bytes()); // PCR PID
    b.extend_from_slice(&0u32.to_be_bytes()); // SPN_STC_start
    b.extend_from_slice(&start.to_be_bytes());
    b.extend_from_slice(&end.to_be_bytes());
    b
}

fn ticks_ms(t: u32) -> u64 {
    u64::from(t) * 1000 / TICKS_PER_SECOND
}

fn blu_ray(vfs: &dyn Vfs, bdmv: &VfsPath, protection: Option<Protection>) -> DiscPlan {
    let sub = children(vfs, bdmv);
    let dir = |name: &str| {
        sub.get(name)
            .filter(|(_, k, _)| *k == EntryKind::Dir)
            .map(|(p, _, _)| p.clone())
    };
    let streams = dir("STREAM").map(|d| children(vfs, &d)).unwrap_or_default();
    let clipinf = dir("CLIPINF")
        .map(|d| children(vfs, &d))
        .unwrap_or_default();
    let has_ssif = streams
        .get("SSIF")
        .is_some_and(|(_, k, _)| *k == EntryKind::Dir);
    let clip_file = |clip: &str| {
        streams
            .get(&format!("{clip}.M2TS"))
            .or_else(|| streams.get(&format!("{clip}.MTS")))
            .filter(|(_, k, _)| *k == EntryKind::File)
    };

    let mut playlists: Vec<(String, Playlist)> = Vec::new();
    if let Some(pdir) = dir("PLAYLIST") {
        for (name, (path, kind, _)) in children(vfs, &pdir) {
            if playlists.len() >= MAX_PLAYLISTS {
                break;
            }
            let Some(stem) = name
                .strip_suffix(".MPLS")
                .or_else(|| name.strip_suffix(".MPL"))
            else {
                continue;
            };
            if kind != EntryKind::File {
                continue;
            }
            let Some(pl) = read(vfs, &path, MAX_PLAYLIST_BYTES).and_then(|b| mpls::parse(&b).ok())
            else {
                continue;
            };
            let usable = !pl.items.is_empty()
                && pl.items.iter().all(|it| {
                    std::iter::once(&it.clip_id)
                        .chain(&it.angles)
                        .all(|c| clip_file(c).is_some())
                });
            if usable && pl.duration_ms() > 0 {
                playlists.push((stem.to_owned(), pl));
            }
        }
    }
    // Longest first; equal lengths by name.
    playlists.sort_by(|a, b| {
        b.1.duration_ms()
            .cmp(&a.1.duration_ms())
            .then(a.0.cmp(&b.0))
    });

    let mut times = BTreeMap::new();
    let mut title = |id: &str, pl: &Playlist| -> DiscTitle {
        let angles = (0..pl.angle_count())
            .map(|angle| {
                pl.items
                    .iter()
                    .filter_map(|it| {
                        let clip = it.clip_for_angle(angle);
                        let (path, _, size) = clip_file(clip)?;
                        let range = *times
                            .entry(clip.to_owned())
                            .or_insert_with(|| clip_times(vfs, &clipinf, clip));
                        // A part of the clip: from the item's in time,
                        // counted from the clip's first timestamp.
                        let whole = range.is_none_or(|(s, e)| {
                            it.in_time <= s.saturating_add(45_000)
                                && it.out_time.saturating_add(45_000) >= e
                        });
                        let (start_ms, length_ms) = match range {
                            Some((s, _)) if !whole => (
                                Some(ticks_ms(it.in_time.saturating_sub(s))),
                                Some(it.duration_ms()),
                            ),
                            _ => (None, None),
                        };
                        Some(DiscSegment {
                            path: path.clone(),
                            size: *size,
                            start_ms,
                            length_ms,
                        })
                    })
                    .collect()
            })
            .collect();
        let video = pl.streams.iter().find(|s| s.kind == StreamKind::Video);
        DiscTitle {
            id: id.to_owned(),
            duration_ms: pl.duration_ms(),
            angles,
            chapters_ms: pl.chapters_ms(),
            streams: pl
                .streams
                .iter()
                .map(|s| DiscStream {
                    kind: s.kind,
                    id: s.pid,
                    codec: s.codec_name().to_owned(),
                    lang: s.lang.clone(),
                })
                .collect(),
            uhd: video.is_some_and(|v| v.coding == 0x24),
            is_3d: has_ssif || pl.streams.iter().any(|s| s.coding == 0x20),
        }
    };

    let Some((main_id, main_pl)) = playlists.first() else {
        return DiscPlan {
            kind: DiscKind::BluRay,
            protection,
            main: None,
            extras: Vec::new(),
        };
    };
    let main = title(main_id, main_pl);
    let main_clips: HashSet<&str> = main_pl.items.iter().map(|i| i.clip_id.as_str()).collect();
    let mut seen: HashSet<Vec<&str>> = HashSet::new();
    seen.insert(main_pl.items.iter().map(|i| i.clip_id.as_str()).collect());
    let mut extras = Vec::new();
    for (id, pl) in &playlists[1..] {
        if extras.len() >= MAX_EXTRAS || pl.duration_ms() <= EXTRA_MIN_MS {
            continue;
        }
        let clips: Vec<&str> = pl.items.iter().map(|i| i.clip_id.as_str()).collect();
        // Parts of the film and copies of one playlist are not extras.
        if clips.iter().all(|c| main_clips.contains(c)) || !seen.insert(clips) {
            continue;
        }
        extras.push(title(id, pl));
    }
    DiscPlan {
        kind: DiscKind::BluRay,
        protection,
        main: Some(main),
        extras,
    }
}

// ---------------------------------------------------------------------------
// DVD

/// `VTS_nn_k.VOB` → (`nn`, `k`).
fn vob_number(name: &str) -> Option<(u8, u8)> {
    let rest = name.strip_prefix("VTS_")?.strip_suffix(".VOB")?;
    let (set, part) = rest.split_once('_')?;
    Some((set.parse().ok()?, part.parse().ok()?))
}

fn dvd(vfs: &dyn Vfs, video_ts: &VfsPath) -> DiscPlan {
    let files = children(vfs, video_ts);
    let mut sets: BTreeMap<u8, Vec<(u8, VfsPath, u64)>> = BTreeMap::new();
    for (name, (path, kind, size)) in &files {
        if *kind != EntryKind::File {
            continue;
        }
        if let Some((set, part)) = vob_number(name)
            && part >= 1
        {
            sets.entry(set)
                .or_default()
                .push((part, path.clone(), *size));
        }
    }
    let Some((set, mut vobs)) = sets.into_iter().max_by_key(|(set, v)| {
        (
            v.iter().map(|(_, _, s)| *s).sum::<u64>(),
            std::cmp::Reverse(*set),
        )
    }) else {
        return DiscPlan {
            kind: DiscKind::Dvd,
            protection: None,
            main: None,
            extras: Vec::new(),
        };
    };
    vobs.sort();
    let protection = vobs
        .iter()
        .max_by_key(|(_, _, s)| *s)
        .and_then(|(_, p, _)| vfs.open(p).ok())
        .is_some_and(|f| scrambled(f.as_ref()))
        .then_some(Protection::Css);
    let ifo = files
        .get(&format!("VTS_{set:02}_0.IFO"))
        .and_then(|(p, _, _)| read(vfs, p, 4 * 1024 * 1024))
        .and_then(|b| parse_ifo(&b))
        .unwrap_or_default();
    let segments = vobs
        .into_iter()
        .map(|(_, path, size)| DiscSegment {
            path,
            size,
            start_ms: None,
            length_ms: None,
        })
        .collect();
    DiscPlan {
        kind: DiscKind::Dvd,
        protection,
        main: Some(DiscTitle {
            id: format!("VTS_{set:02}"),
            duration_ms: ifo.duration_ms,
            angles: vec![segments],
            chapters_ms: ifo.chapters_ms,
            streams: ifo.streams,
            uhd: false,
            is_3d: false,
        }),
        extras: Vec::new(),
    }
}

/// Whether sampled packs of a VOB have the PES scrambling bits set (CSS).
fn scrambled(f: &dyn ByteSource) -> bool {
    const SAMPLES: u64 = 16;
    let packs = f.size() / 2048;
    if packs == 0 {
        return false;
    }
    let mut buf = [0u8; 2048];
    for i in 0..SAMPLES.min(packs) {
        let pack = packs * i / SAMPLES.min(packs);
        if f.read_exact_at(pack * 2048, &mut buf).is_err() {
            return false;
        }
        if buf[..4] != [0, 0, 1, 0xBA] || buf[4] >> 6 != 1 {
            continue; // not an MPEG-2 pack
        }
        let mut at = 14 + usize::from(buf[13] & 7);
        // Skip a system header, then look at the first PES packet.
        for _ in 0..2 {
            let Some(h) = buf.get(at..at + 7) else { break };
            if h[..3] != [0, 0, 1] {
                break;
            }
            let id = h[3];
            let len = usize::from(u16::from_be_bytes([h[4], h[5]]));
            if id == 0xBB || id == 0xBE {
                at += 6 + len;
                continue;
            }
            if (id == 0xBD || id & 0xE0 == 0xC0 || id & 0xF0 == 0xE0) && (h[6] >> 4) & 3 != 0 {
                return true;
            }
            break;
        }
    }
    false
}

/// What the IFO of a title set says.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IfoInfo {
    pub streams: Vec<DiscStream>,
    /// Length of the longest program chain.
    pub duration_ms: u64,
    pub chapters_ms: Vec<u64>,
}

/// A DVD time: BCD hours, minutes, seconds and frames (with the rate in
/// the top bits of the last byte).
fn dvd_time_ms(t: &[u8]) -> u64 {
    let bcd = |b: u8| u64::from(b >> 4) * 10 + u64::from(b & 0x0F);
    let fps = if t[3] >> 6 == 3 { 30 } else { 25 };
    ((bcd(t[0]) * 60 + bcd(t[1])) * 60 + bcd(t[2])) * 1000 + bcd(t[3] & 0x3F) * 1000 / fps
}

fn lang(b: &[u8], flag: bool) -> String {
    if flag && b.iter().all(u8::is_ascii_alphabetic) {
        String::from_utf8_lossy(b).to_ascii_lowercase()
    } else {
        String::new()
    }
}

/// Parses a `VTS_nn_0.IFO`: audio and subpicture streams with languages,
/// and the chapters of the longest program chain.
#[must_use]
pub fn parse_ifo(b: &[u8]) -> Option<IfoInfo> {
    if b.get(..12)? != b"DVDVIDEO-VTS" {
        return None;
    }
    let be16 = |at: usize| -> Option<u16> {
        Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
    };
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
    };
    let mut streams = Vec::new();
    let n_audio = usize::from(be16(0x202)?).min(8);
    for i in 0..n_audio {
        let a = b.get(0x204 + 8 * i..0x204 + 8 * i + 8)?;
        let n = u16::try_from(i).unwrap_or(0);
        let (id, codec) = match a[0] >> 5 {
            0 => (0x80 + n, "AC-3"),
            2 | 3 => (0x1C0 + n, "MPEG Audio"),
            4 => (0xA0 + n, "LPCM"),
            6 => (0x88 + n, "DTS"),
            _ => (0x80 + n, ""),
        };
        streams.push(DiscStream {
            kind: StreamKind::Audio,
            id,
            codec: codec.to_owned(),
            lang: lang(&a[2..4], (a[0] >> 2) & 3 == 1),
        });
    }
    let n_sub = usize::from(be16(0x254)?).min(32);
    for i in 0..n_sub {
        let s = b.get(0x256 + 6 * i..0x256 + 6 * i + 6)?;
        streams.push(DiscStream {
            kind: StreamKind::Subtitle,
            id: 0x20 + u16::try_from(i).unwrap_or(0),
            codec: "VobSub".to_owned(),
            lang: lang(&s[2..4], s[0] & 3 == 1),
        });
    }
    // Program chains: the longest one is the film.
    let mut info = IfoInfo {
        streams,
        ..IfoInfo::default()
    };
    let pgci = usize::try_from(be32(0xCC)?).ok()? * 2048;
    let count = usize::from(be16(pgci).unwrap_or(0)).min(999);
    for i in 0..count {
        let Some(off) = be32(pgci + 8 + 8 * i + 4) else {
            break;
        };
        let pgc = pgci + usize::try_from(off).ok()?;
        let (Some(&programs), Some(&cells), Some(time)) =
            (b.get(pgc + 2), b.get(pgc + 3), b.get(pgc + 4..pgc + 8))
        else {
            break;
        };
        let duration = dvd_time_ms(time);
        if duration <= info.duration_ms {
            continue;
        }
        let (Some(map), Some(cell_at)) = (be16(pgc + 0xE6), be16(pgc + 0xE8)) else {
            break;
        };
        let entries = b.get(pgc + usize::from(map)..pgc + usize::from(map) + usize::from(programs));
        let cell_times: Option<Vec<u64>> = (0..usize::from(cells))
            .map(|c| {
                let at = pgc + usize::from(cell_at) + 24 * c + 4;
                b.get(at..at + 4).map(dvd_time_ms)
            })
            .collect();
        let (Some(entries), Some(cell_times)) = (entries, cell_times) else {
            continue;
        };
        let mut chapters: Vec<u64> = Vec::new();
        for &first in entries {
            let start: u64 = cell_times
                .iter()
                .take(usize::from(first).saturating_sub(1))
                .sum();
            if chapters.last().is_none_or(|&l| start > l) {
                chapters.push(start);
            }
        }
        info.duration_ms = duration;
        info.chapters_ms = chapters;
    }
    Some(info)
}

/// Tools and tests: navigation files of generated discs.
pub mod fixture {
    /// A DVD time in BCD at 25 fps.
    fn bcd_time(ms: u64) -> [u8; 4] {
        let bcd = |v: u64| u8::try_from((v / 10) << 4 | (v % 10)).unwrap_or(0);
        let s = ms / 1000;
        [
            bcd(s / 3600 % 100),
            bcd(s / 60 % 60),
            bcd(s % 60),
            0x40 | bcd((ms % 1000) * 25 / 1000),
        ]
    }

    /// A `VTS_nn_0.IFO` with AC-3 audio streams in `audio` languages
    /// (ISO 639-1), subpictures in `subs` languages and one program chain
    /// of `cells_ms` cells, each a chapter.
    #[must_use]
    pub fn ifo_bytes(audio: &[&str], subs: &[&str], cells_ms: &[u64]) -> Vec<u8> {
        let mut b = vec![0u8; 2 * 2048];
        b[..12].copy_from_slice(b"DVDVIDEO-VTS");
        b[0xCC..0xD0].copy_from_slice(&1u32.to_be_bytes()); // VTS_PGCI in sector 1
        b[0x202..0x204].copy_from_slice(&u16::try_from(audio.len()).unwrap_or(0).to_be_bytes());
        for (i, l) in audio.iter().enumerate() {
            let a = 0x204 + 8 * i;
            b[a] = 0b0000_0100; // AC-3, language present
            b[a + 2..a + 4].copy_from_slice(&l.as_bytes()[..2]);
        }
        b[0x254..0x256].copy_from_slice(&u16::try_from(subs.len()).unwrap_or(0).to_be_bytes());
        for (i, l) in subs.iter().enumerate() {
            let s = 0x256 + 6 * i;
            b[s] = 1;
            b[s + 2..s + 4].copy_from_slice(&l.as_bytes()[..2]);
        }
        let p = 2048;
        b[p..p + 2].copy_from_slice(&1u16.to_be_bytes());
        b[p + 8 + 4..p + 8 + 8].copy_from_slice(&16u32.to_be_bytes());
        let pgc = p + 16;
        let n = u8::try_from(cells_ms.len()).unwrap_or(0);
        b[pgc + 2] = n;
        b[pgc + 3] = n;
        b[pgc + 4..pgc + 8].copy_from_slice(&bcd_time(cells_ms.iter().sum()));
        let map = 0xEC_u16;
        let cells = map + 2 * u16::from(n);
        b[pgc + 0xE6..pgc + 0xE8].copy_from_slice(&map.to_be_bytes());
        b[pgc + 0xE8..pgc + 0xEA].copy_from_slice(&cells.to_be_bytes());
        for i in 0..usize::from(n) {
            b[pgc + usize::from(map) + i] = u8::try_from(i + 1).unwrap_or(0);
            let c = pgc + usize::from(cells) + 24 * i;
            b[c + 4..c + 8].copy_from_slice(&bcd_time(cells_ms[i]));
        }
        b
    }
}

#[cfg(test)]
mod tests;
