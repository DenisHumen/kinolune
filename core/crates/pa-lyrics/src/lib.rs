// SPDX-License-Identifier: MPL-2.0
//! Song lyrics kept with a music file (MUS-006).
//!
//! - Embedded in the file: ID3v2 `SYLT` (synchronised, per line) and `USLT`
//!   (plain) in MP3; the lyrics tag of the other formats (Vorbis comments
//!   `LYRICS` / `UNSYNCEDLYRICS`, MP4 `©lyr`, APE `Lyrics`). A plain tag
//!   often holds LRC text; the client parses it and treats it as synced
//!   when it has time tags.
//! - An `.lrc` file next to the track with the same base name
//!   (`Song.flac` → `Song.lrc`, any letter case of the extension).
//!
//! Everything is read through [`pa_vfs`], so the same code serves local
//! folders and network sources (SMB, WebDAV): nothing here mounts or
//! downloads the whole file, `lofty` reads the tag blocks only.
//!
//! LRC parsing and the online lookup (LRCLIB) live in the client, which
//! picks the best of all sources (synced first).

use std::io::{BufReader, Seek, SeekFrom};

use lofty::config::{ParseOptions, ParsingMode};
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::id3::v2::{
    Frame, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame, TimestampFormat,
};
use lofty::mpeg::MpegFile;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use pa_vfs::{ErrorKind, SourceReader, Vfs, VfsError, VfsPath};

/// Largest `.lrc` file read; anything bigger is not lyrics.
pub const MAX_LRC_BYTES: u64 = 1024 * 1024;

/// One line of synchronised lyrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedLine {
    /// When the line starts, from the start of the track.
    pub time_ms: u32,
    pub text: String,
}

/// The lyrics a music file carries with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileLyrics {
    /// `SYLT` lines by time; empty when the file has none.
    pub synced: Vec<SyncedLine>,
    /// The plain lyrics tag (`USLT`, `LYRICS`, `©lyr`); may be LRC text.
    pub plain: Option<String>,
    /// Text of the `.lrc` file next to the track.
    pub lrc: Option<String>,
}

impl FileLyrics {
    /// Whether nothing was found.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.synced.is_empty() && self.plain.is_none() && self.lrc.is_none()
    }
}

/// Reads the embedded lyrics and the `.lrc` file of the track at `path`.
///
/// A file whose tags cannot be parsed has no embedded lyrics (its `.lrc`
/// still counts). Fails only when the source cannot be read (offline,
/// access denied); a missing track or `.lrc` is not an error.
pub fn read_lyrics(vfs: &dyn Vfs, path: &VfsPath) -> Result<FileLyrics, VfsError> {
    let (synced, plain) = match read_embedded(vfs, path) {
        Ok(found) => found,
        Err(EmbeddedError::Vfs(e)) if e.kind() != ErrorKind::NotFound => return Err(e),
        Err(_) => (Vec::new(), None),
    };
    let lrc = read_sidecar_lrc(vfs, path)?;
    Ok(FileLyrics { synced, plain, lrc })
}

enum EmbeddedError {
    Vfs(VfsError),
    Parse,
}

impl From<VfsError> for EmbeddedError {
    fn from(e: VfsError) -> Self {
        Self::Vfs(e)
    }
}

impl From<std::io::Error> for EmbeddedError {
    fn from(_: std::io::Error) -> Self {
        Self::Parse
    }
}

impl From<lofty::error::FileParseError> for EmbeddedError {
    fn from(_: lofty::error::FileParseError) -> Self {
        Self::Parse
    }
}

fn options() -> ParseOptions {
    ParseOptions::new()
        .parsing_mode(ParsingMode::BestAttempt)
        .read_cover_art(false)
        .read_properties(true)
}

/// `SYLT` lines and the plain lyrics tag of the file.
fn read_embedded(
    vfs: &dyn Vfs,
    path: &VfsPath,
) -> Result<(Vec<SyncedLine>, Option<String>), EmbeddedError> {
    let source = vfs.open(path)?;
    let mut reader = BufReader::with_capacity(64 * 1024, SourceReader::new(source));
    let file_type = Probe::new(&mut reader).guess_file_type()?.file_type();
    reader.seek(SeekFrom::Start(0))?;

    if file_type == Some(FileType::Mpeg) {
        let file = MpegFile::read_from(&mut reader, options())?;
        let Some(tag) = file.id3v2() else {
            return Ok((Vec::new(), None));
        };
        let rate = file.properties().sample_rate();
        return Ok((sylt_lines(tag, rate), uslt_text(tag)));
    }

    let file = Probe::new(&mut reader)
        .options(options())
        .guess_file_type()?
        .read()?;
    let plain = file.tags().iter().find_map(|t| {
        t.get_string(ItemKey::Lyrics)
            .or_else(|| t.get_string(ItemKey::UnsyncLyrics))
            .and_then(non_blank)
    });
    Ok((Vec::new(), plain))
}

fn non_blank(s: &str) -> Option<String> {
    let s = s.trim_matches(|c: char| c == '\u{feff}' || c.is_whitespace());
    (!s.is_empty()).then(|| normalize_newlines(s))
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// The plain lyrics of an ID3v2 tag: the first non-empty `USLT` frame.
fn uslt_text(tag: &Id3v2Tag) -> Option<String> {
    tag.into_iter().find_map(|frame| match frame {
        Frame::UnsynchronizedText(f) => non_blank(&f.content),
        _ => None,
    })
}

/// Lines of the best `SYLT` frame (content type "lyrics" first), sorted by
/// time, in milliseconds.
fn sylt_lines(tag: &Id3v2Tag, sample_rate: u32) -> Vec<SyncedLine> {
    let frames: Vec<SynchronizedTextFrame<'_>> = tag
        .into_iter()
        .filter(|f| f.id().as_str() == "SYLT")
        .filter_map(|f| match f {
            Frame::Binary(b) => SynchronizedTextFrame::parse(&b.data, f.flags()).ok(),
            _ => None,
        })
        .filter(|f| f.content.iter().any(|(_, t)| !t.trim().is_empty()))
        .collect();
    let best = frames
        .iter()
        .find(|f| f.content_type == SyncTextContentType::Lyrics)
        .or_else(|| frames.first());
    let Some(frame) = best else {
        return Vec::new();
    };
    let mut lines: Vec<SyncedLine> = frame
        .content
        .iter()
        .map(|(t, text)| SyncedLine {
            time_ms: match frame.timestamp_format {
                TimestampFormat::MS => *t,
                TimestampFormat::MPEG => mpeg_frames_to_ms(*t, sample_rate),
            },
            // Many taggers start each entry with a newline.
            text: text.trim_start_matches(['\n', '\r']).trim_end().to_owned(),
        })
        .collect();
    lines.sort_by_key(|l| l.time_ms);
    lines
}

/// MPEG audio frames to milliseconds: 1152 samples per Layer III frame at
/// 32 kHz and above (MPEG-1), 576 below (MPEG-2 and 2.5).
#[must_use]
pub fn mpeg_frames_to_ms(frames: u32, sample_rate: u32) -> u32 {
    if sample_rate == 0 {
        return 0;
    }
    let samples: u64 = if sample_rate >= 32_000 { 1152 } else { 576 };
    let ms = u64::from(frames) * samples * 1000 / u64::from(sample_rate);
    u32::try_from(ms).unwrap_or(u32::MAX)
}

/// Text of the `.lrc` file next to the track at `path` (same base name,
/// extension in any case), or `None` if there is none.
pub fn read_sidecar_lrc(vfs: &dyn Vfs, path: &VfsPath) -> Result<Option<String>, VfsError> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(None);
    };
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let wanted = format!("{stem}.lrc");

    // The usual spelling first: one stat instead of listing a big folder.
    let direct = dir.join(&wanted).ok();
    let found = match direct.as_ref().map(|p| vfs.stat(p)) {
        Some(Ok(m)) if m.is_file() => direct,
        Some(Err(e)) if e.kind() != ErrorKind::NotFound => return Err(e),
        _ => match vfs.list(&dir) {
            Ok(entries) => entries
                .into_iter()
                .find(|e| e.metadata.is_file() && e.name.eq_ignore_ascii_case(&wanted))
                .map(|e| e.path),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        },
    };
    let Some(lrc) = found else {
        return Ok(None);
    };
    let source = match vfs.open(&lrc) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let size = source.size();
    if size == 0 || size > MAX_LRC_BYTES {
        return Ok(None);
    }
    let mut bytes = vec![0u8; usize::try_from(size).unwrap_or(0)];
    source.read_exact_at(0, &mut bytes)?;
    Ok(non_blank(&decode_text(&bytes)))
}

/// Text of a lyrics file: UTF-8 or UTF-16 with a byte order mark, UTF-8,
/// else Windows-1251 when the bytes look Cyrillic (more letters in its
/// Cyrillic range than ASCII letters), else Latin-1.
#[must_use]
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return utf16(rest, u16::from_be_bytes);
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    // Cyrillic text is mostly letters in 0xC0..=0xFF; a Latin-1 text has a
    // few accented letters among many ASCII ones.
    let latin = bytes.iter().filter(|b| b.is_ascii_alphabetic()).count();
    let cyrillic = bytes.iter().filter(|b| **b >= 0xc0).count();
    if cyrillic >= latin {
        bytes.iter().map(|b| cp1251(*b)).collect()
    } else {
        bytes.iter().map(|b| char::from(*b)).collect()
    }
}

fn utf16(bytes: &[u8], word: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| word(*c)).collect();
    String::from_utf16_lossy(&units)
}

/// Windows-1251: Russian, Ukrainian and Belarusian letters; the rest of the
/// upper half as Latin-1.
fn cp1251(b: u8) -> char {
    let code = match b {
        0xc0..=0xff => 0x0410 + u32::from(b - 0xc0),
        0xa8 => 0x0401,
        0xb8 => 0x0451,
        0xaa => 0x0404,
        0xba => 0x0454,
        0xaf => 0x0407,
        0xbf => 0x0457,
        0xb2 => 0x0406,
        0xb3 => 0x0456,
        0xa5 => 0x0490,
        0xb4 => 0x0491,
        0xa1 => 0x040e,
        0xa2 => 0x045e,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x85 => 0x2026,
        0x93 => 0x201c,
        0x94 => 0x201d,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0xab => 0x00ab,
        0xbb => 0x00bb,
        _ => u32::from(b),
    };
    char::from_u32(code).unwrap_or('\u{fffd}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpeg_frames() {
        // 1152 samples at 44.1 kHz ≈ 26.12 ms per frame.
        assert_eq!(mpeg_frames_to_ms(0, 44_100), 0);
        assert_eq!(mpeg_frames_to_ms(100, 44_100), 2612);
        assert_eq!(mpeg_frames_to_ms(100, 22_050), 2612);
        assert_eq!(mpeg_frames_to_ms(100, 0), 0);
    }

    #[test]
    fn decodes_bom_utf8_cp1251() {
        assert_eq!(decode_text(b"\xef\xbb\xbf[00:01.00]hi"), "[00:01.00]hi");
        assert_eq!(decode_text("Привіт".as_bytes()), "Привіт");
        // «Привет» and «їжак» in Windows-1251.
        assert_eq!(decode_text(b"\xcf\xf0\xe8\xe2\xe5\xf2"), "Привет");
        assert_eq!(decode_text(b"\xbf\xe6\xe0\xea"), "їжак");
        let utf16le: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain("ok".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode_text(&utf16le), "ok");
        assert_eq!(
            decode_text(b"caf\xe9 au lait \xe9t\xe9"),
            "café au lait été"
        );
    }
}
