// SPDX-License-Identifier: MPL-2.0
//! MUS-006: lyrics embedded in MP3 (SYLT, USLT) and FLAC (LYRICS), and the
//! `.lrc` file next to the track, read through a pa-vfs source.

use std::path::Path;

use lofty::TextEncoding;
use lofty::config::WriteOptions;
use lofty::id3::v2::{
    BinaryFrame, Frame, FrameId, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame,
    TimestampFormat, UnsynchronizedTextFrame,
};
use lofty::ogg::VorbisComments;
use lofty::tag::TagExt;
use pa_lyrics::{FileLyrics, SyncedLine, read_lyrics, read_sidecar_lrc};
use pa_vfs::{LocalFs, VfsPath};

fn vpath(s: &str) -> VfsPath {
    VfsPath::parse(s).unwrap()
}

/// MPEG-1 Layer III, 128 kbit/s, 44.1 kHz: 417-byte frames.
fn mp3_frames(n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        out.resize(out.len() + 413, 0);
    }
    out
}

/// A minimal FLAC: the marker and a STREAMINFO block (44.1 kHz, 16-bit,
/// stereo, no samples).
fn flac() -> Vec<u8> {
    let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36);
    let mut out = b"fLaC".to_vec();
    out.extend_from_slice(&[0x00, 0, 0, 34]);
    out.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&packed.to_be_bytes());
    out.extend_from_slice(&[0; 16]);
    out
}

fn sylt(format: TimestampFormat, kind: SyncTextContentType, lines: &[(u32, &str)]) -> Frame<'static> {
    let f = SynchronizedTextFrame::new(
        TextEncoding::UTF8,
        *b"eng",
        format,
        kind,
        None,
        lines.iter().map(|(t, s)| (*t, (*s).to_owned())).collect(),
    );
    Frame::Binary(BinaryFrame::new(
        FrameId::Valid("SYLT".into()),
        f.as_bytes(WriteOptions::default()).unwrap(),
    ))
}

fn write_mp3(path: &Path, frames: Vec<Frame<'static>>) {
    std::fs::write(path, mp3_frames(20)).unwrap();
    let mut tag = Id3v2Tag::new();
    for f in frames {
        tag.insert(f);
    }
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

#[test]
fn mp3_sylt_and_uslt() {
    let dir = tempfile::tempdir().unwrap();
    write_mp3(
        &dir.path().join("song.mp3"),
        vec![
            // A non-lyrics SYLT (events) must lose to the lyrics one.
            sylt(TimestampFormat::MS, SyncTextContentType::Events, &[(0, "intro")]),
            sylt(
                TimestampFormat::MS,
                SyncTextContentType::Lyrics,
                &[(12_340, "\nSecond"), (1_500, "First")],
            ),
            Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(
                TextEncoding::UTF8,
                *b"eng",
                "",
                "Plain line one\r\nPlain line two",
            )),
        ],
    );
    let vfs = LocalFs::new(dir.path()).unwrap();
    let got = read_lyrics(&vfs, &vpath("/song.mp3")).unwrap();
    assert_eq!(
        got.synced,
        vec![
            SyncedLine { time_ms: 1_500, text: "First".into() },
            SyncedLine { time_ms: 12_340, text: "Second".into() },
        ]
    );
    assert_eq!(got.plain.as_deref(), Some("Plain line one\nPlain line two"));
    assert_eq!(got.lrc, None);
}

#[test]
fn mp3_sylt_in_mpeg_frames() {
    let dir = tempfile::tempdir().unwrap();
    write_mp3(
        &dir.path().join("frames.mp3"),
        vec![sylt(
            TimestampFormat::MPEG,
            SyncTextContentType::Lyrics,
            &[(100, "At 2.6 s")],
        )],
    );
    let vfs = LocalFs::new(dir.path()).unwrap();
    let got = read_lyrics(&vfs, &vpath("/frames.mp3")).unwrap();
    assert_eq!(got.synced, vec![SyncedLine { time_ms: 2612, text: "At 2.6 s".into() }]);
    assert_eq!(got.plain, None);
}

#[test]
fn flac_lyrics_comment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.flac");
    std::fs::write(&path, flac()).unwrap();
    let mut tag = VorbisComments::default();
    tag.push("LYRICS".into(), "[00:01.00]Synced in a plain tag".into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();

    let vfs = LocalFs::new(dir.path()).unwrap();
    let got = read_lyrics(&vfs, &vpath("/track.flac")).unwrap();
    assert!(got.synced.is_empty());
    assert_eq!(got.plain.as_deref(), Some("[00:01.00]Synced in a plain tag"));
}

#[test]
fn sidecar_lrc_any_case_and_cp1251() {
    let dir = tempfile::tempdir().unwrap();
    let album = dir.path().join("Album");
    std::fs::create_dir(&album).unwrap();
    std::fs::write(album.join("01 Song.flac"), flac()).unwrap();
    std::fs::write(album.join("01 Song.LRC"), b"[00:05.00]\xcf\xf0\xe8\xe2\xe5\xf2\r\n").unwrap();
    std::fs::write(album.join("02 Other.mp3"), mp3_frames(4)).unwrap();
    std::fs::write(album.join("02 Other.lrc"), "\u{feff}[00:01.00]Hi\n").unwrap();
    std::fs::write(album.join("03 None.mp3"), mp3_frames(4)).unwrap();

    let vfs = LocalFs::new(dir.path()).unwrap();
    let got = read_lyrics(&vfs, &vpath("/Album/01 Song.flac")).unwrap();
    assert_eq!(got.lrc.as_deref(), Some("[00:05.00]Привет"));
    assert_eq!(
        read_sidecar_lrc(&vfs, &vpath("/Album/02 Other.mp3")).unwrap().as_deref(),
        Some("[00:01.00]Hi")
    );
    let none = read_lyrics(&vfs, &vpath("/Album/03 None.mp3")).unwrap();
    assert!(none.is_empty(), "{none:?}");
}

#[test]
fn missing_or_unreadable_track_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.mp3"), b"not audio").unwrap();
    std::fs::write(dir.path().join("broken.lrc"), "[00:00.50]Still here").unwrap();
    let vfs = LocalFs::new(dir.path()).unwrap();
    let got = read_lyrics(&vfs, &vpath("/broken.mp3")).unwrap();
    assert_eq!(
        got,
        FileLyrics {
            synced: vec![],
            plain: None,
            lrc: Some("[00:00.50]Still here".into()),
        }
    );
    assert!(read_lyrics(&vfs, &vpath("/gone.mp3")).unwrap().is_empty());
}
