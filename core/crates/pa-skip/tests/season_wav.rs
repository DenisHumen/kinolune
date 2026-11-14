// SPDX-License-Identifier: MPL-2.0
//! The library job end to end on a generated season of WAV files: the
//! same intro music at a different offset in each episode, decoded by our
//! LGPL FFmpeg, matched, stored in `pa-library` and read back.
//!
//! ```text
//! PA_FFMPEG_LIB_DIR=tools/ffmpeg/out/linux-x86_64/lib cargo test -p pa-skip --test season_wav
//! ```
//!
//! Without FFmpeg the test is skipped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pa_library::rusqlite::params;
use pa_library::skip::{self, SkipKind, SkipOrigin};
use pa_probe::Ffmpeg;
use pa_skip::{AnalyzeOptions, Opener, analyze_season};
use pa_vfs::{ByteSource, LocalFs, Vfs, VfsPath};

const RATE: u32 = 16_000;
const EPISODE_S: f32 = 240.0;
const INTRO_S: f32 = 20.0;
/// Intro offsets of the episodes (cold opens), seconds.
const OFFSETS: [f32; 4] = [3.0, 10.4, 17.0, 25.25];

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    }
}

/// The intro: 0.2 s notes of a fixed sequence over a bass, with a hi-hat.
fn intro_sample(t: f32, noise: &mut Lcg) -> f32 {
    let note = (t * 5.0).floor() as i64;
    let semi = (note * note * 7 + note * 3).rem_euclid(19) as f32;
    let bass = ((t * 2.5).floor() as i64 * 5).rem_euclid(12) as f32;
    let tau = std::f32::consts::TAU;
    0.35 * (tau * 220.0 * 2f32.powf(semi / 12.0) * t).sin()
        + 0.2 * (tau * 110.0 * 2f32.powf(bass / 12.0) * t).sin()
        + if t.rem_euclid(0.25) < 0.04 {
            0.3 * noise.next()
        } else {
            0.0
        }
}

/// Episode content: its own note sequence and noise.
fn content_sample(seed: u32, t: f32, noise: &mut Lcg) -> f32 {
    let note = (t * 3.7).floor() as i64;
    let semi = (note * i64::from(5 + 2 * seed) + i64::from(seed) * 3).rem_euclid(23) as f32;
    0.25 * (std::f32::consts::TAU * 140.0 * 2f32.powf(semi / 12.0) * t).sin() + 0.2 * noise.next()
}

/// Writes a 16-bit stereo WAV.
fn write_wav(path: &Path, samples: &[f32]) {
    let data_len = (samples.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32_000.0) as i16;
        // Left and right differ a little: the mono mix must not care.
        out.extend_from_slice(&v.to_le_bytes());
        out.extend_from_slice(&((f32::from(v) * 0.8) as i16).to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

fn episode(seed: u32, offset: Option<f32>) -> Vec<f32> {
    let mut noise = Lcg(u64::from(seed) * 7919 + 1);
    let mut hat = Lcg(42);
    let n = (EPISODE_S * RATE as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            match offset {
                Some(o) if t >= o && t < o + INTRO_S => intro_sample(t - o, &mut hat),
                _ => content_sample(seed, t, &mut noise),
            }
        })
        .collect()
}

struct Dir(PathBuf);

impl Opener for Dir {
    fn open(&self, _source_id: i64, path: &str) -> Result<Arc<dyn ByteSource>, String> {
        let vfs = LocalFs::new(&self.0).map_err(|e| e.to_string())?;
        let path = VfsPath::parse(path).map_err(|e| e.to_string())?;
        vfs.open(&path).map(Arc::from).map_err(|e| e.to_string())
    }
}

#[test]
fn season_of_wav_files() {
    if std::env::var_os(pa_probe::LIB_DIR_ENV).is_none() {
        eprintln!("skipped: {} not set", pa_probe::LIB_DIR_ENV);
        return;
    }
    let ffmpeg = Ffmpeg::shared().expect("FFmpeg from PA_FFMPEG_LIB_DIR");
    let dir = tempfile::tempdir().unwrap();
    let season = dir.path().join("Show").join("Season 01");
    std::fs::create_dir_all(&season).unwrap();
    let conn = pa_library::open_in_memory().unwrap();
    conn.execute(
        "INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES (1, 'local', 'L', 'file:///m', 0)",
        [],
    )
    .unwrap();
    // Four episodes with the intro, a fifth without.
    let mut offsets: Vec<Option<f32>> = OFFSETS.iter().copied().map(Some).collect();
    offsets.push(None);
    for (i, offset) in offsets.iter().enumerate() {
        let name = format!("e{:02}.wav", i + 1);
        write_wav(&season.join(&name), &episode(i as u32 + 1, *offset));
        let size = std::fs::metadata(season.join(&name)).unwrap().len();
        conn.execute(
            "INSERT INTO files (id, source_id, path, size_bytes, added_at_ms) VALUES (?1, 1, ?2, ?3, 0)",
            params![i as i64 + 1, format!("/Show/Season 01/{name}"), size as i64],
        )
        .unwrap();
    }
    let opener = Dir(dir.path().to_path_buf());
    let opts = AnalyzeOptions::default();
    let report = analyze_season(&conn, ffmpeg, &opener, 2, &opts).unwrap();
    assert_eq!(report.files, 5);
    assert_eq!(report.analysed, 5);
    assert_eq!(report.failed, 0);
    assert_eq!(report.intros, 4, "{report:?}");
    for (i, offset) in offsets.iter().enumerate() {
        let segs = skip::effective_segments(&conn, i as i64 + 1).unwrap();
        let intro = segs.iter().find(|s| s.kind == SkipKind::Intro);
        match offset {
            Some(o) => {
                let s = intro.unwrap_or_else(|| panic!("no intro in e{}", i + 1));
                assert_eq!(s.origin, SkipOrigin::Audio);
                let want = (*o * 1000.0) as u64;
                assert!(s.start_ms.abs_diff(want) <= 500, "e{}: {s:?}", i + 1);
                let want_end = want + (INTRO_S * 1000.0) as u64;
                assert!(s.end_ms.abs_diff(want_end) <= 500, "e{}: {s:?}", i + 1);
            }
            None => assert_eq!(intro, None, "e{}", i + 1),
        }
    }
    // A second run reads nothing again and finds the same.
    let again = analyze_season(&conn, ffmpeg, &opener, 1, &opts).unwrap();
    assert_eq!(again.analysed, 0);
    assert_eq!(again.intros, 4);
}
