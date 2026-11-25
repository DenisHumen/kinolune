// SPDX-License-Identifier: MPL-2.0
//! Generation through our LGPL FFmpeg over the QA-002 test media.
//!
//! ```text
//! just tests::media
//! PA_FFMPEG_LIB_DIR=tools/ffmpeg/out/linux-x86_64/lib cargo test -p pa-trickplay --test generate
//! ```
//!
//! `PA_TEST_MEDIA_DIR` overrides the media root (default `tools/qa/media`).
//! Without FFmpeg or the media the tests are skipped.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use pa_probe::{Ffmpeg, Grab, GrabOptions};
use pa_trickplay::{
    GenerateOptions, Media, Origin, Status, Trickplay, TrickplayOptions, generate, jpeg,
};
use pa_vfs::{ByteSource, LocalFs, Vfs, VfsPath};

/// The synthetic videos (3 s each) the decoders must handle.
const VIDEOS: &[&str] = &[
    "video/h264-aac.mp4",
    "video/h264-aac-moov-at-end.mp4",
    "video/h264-hi10p-flac.mkv",
    "video/hevc-main10-opus.mkv",
    "video/hevc-hvc1-aac.mp4",
    "video/av1-opus.mkv",
    "video/vp9-opus.webm",
    "video/mpeg2-interlaced-ac3.ts",
    "video/h264-ac3.m2ts",
    "video/mpeg4-asp-mp3.avi",
    "video/hevc-hdr10.mkv",
];

fn setup() -> Option<(&'static Ffmpeg, PathBuf)> {
    if std::env::var_os(pa_probe::LIB_DIR_ENV).is_none() {
        eprintln!("skipped: {} not set", pa_probe::LIB_DIR_ENV);
        return None;
    }
    let ffmpeg = Ffmpeg::shared().expect("FFmpeg from PA_FFMPEG_LIB_DIR");
    let root = std::env::var_os("PA_TEST_MEDIA_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/qa/media"),
        PathBuf::from,
    );
    let synthetic = root.join("synthetic");
    if !synthetic.is_dir() {
        eprintln!(
            "skipped: no {} (run `just tests::media`)",
            synthetic.display()
        );
        return None;
    }
    Some((ffmpeg, synthetic))
}

fn open(root: &Path, rel: &str) -> Arc<dyn ByteSource> {
    let vfs = LocalFs::new(root).unwrap();
    Arc::from(
        vfs.open(&VfsPath::parse(&format!("/{rel}")).unwrap())
            .unwrap(),
    )
}

#[test]
fn grabs_keyframes_of_every_codec() {
    let Some((ffmpeg, root)) = setup() else {
        return;
    };
    for rel in VIDEOS {
        if !root.join(rel).is_file() {
            eprintln!("missing {rel}");
            continue;
        }
        let mut grabber = ffmpeg
            .frame_grabber(open(&root, rel), GrabOptions::default())
            .unwrap_or_else(|e| panic!("{rel}: {e}"));
        let (w, h) = grabber.thumbnail_size();
        assert!(w > 0 && w <= 320 && h > 0, "{rel}: {w}x{h}");
        let duration = grabber.duration_ms().expect(rel);
        for at in [0, duration / 2, duration.saturating_sub(100)] {
            match grabber.grab(at, |_| false) {
                Ok(Grab::Frame(frame)) => {
                    assert_eq!(frame.rgb.len(), (frame.width * frame.height * 3) as usize);
                    // At or before `at`; MPEG-TS may start with frames
                    // shown before its first keyframe.
                    assert!(frame.position_ms <= at + 1000, "{rel} at {at}: {frame:?}");
                    // Not a blank picture: testsrc has colour bars.
                    let distinct = frame.rgb.chunks(3).filter(|p| p[0] > 200).count();
                    assert!(distinct > 0, "{rel} at {at}: blank");
                    let encoded = jpeg::encode(&frame, 70).unwrap();
                    assert_eq!(
                        jpeg::dimensions(&encoded),
                        Some((frame.width, frame.height))
                    );
                }
                other => panic!("{rel} at {at}: {other:?}"),
            }
        }
    }
}

#[test]
fn generates_caches_and_reuses_previews() {
    let Some((_, root)) = setup() else { return };
    let rel = "video/h264-aac.mp4";
    let cache = tempfile::tempdir().unwrap();
    let options = TrickplayOptions {
        generation: GenerateOptions {
            interval_ms: 500,
            workers: 3,
            ..GenerateOptions::default()
        },
        ..TrickplayOptions::default()
    };
    let service = Trickplay::open(cache.path(), options.clone()).unwrap();
    let media = || Media {
        key: format!("test:{rel}"),
        source: open(&root, rel),
        sidecar: None,
        duration_ms: None,
    };
    let session = service.prepare(media());
    assert_eq!(
        session.wait(Duration::from_secs(60)),
        Status::Ready(Origin::Generated)
    );
    assert_eq!(session.interval_ms(), Some(500));
    let started = Instant::now();
    let frame = session.frame_at(1_600).unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(frame.position_ms <= 1_600);
    assert!(jpeg::dimensions(&frame.image).is_some());
    session.release();
    assert_eq!(service.usage().files, 1);

    // A new service over the same directory serves the cache.
    let again = Trickplay::open(cache.path(), options).unwrap();
    let session = again.prepare(media());
    assert_eq!(
        session.wait(Duration::from_secs(10)),
        Status::Ready(Origin::Cache)
    );
    assert!(session.frame_at(2_900).is_some());
    session.release();
}

#[test]
fn cancellation_stops_generation() {
    let Some((ffmpeg, root)) = setup() else {
        return;
    };
    let source = open(&root, "video/hevc-main10-opus.mkv");
    let cancel = Arc::new(AtomicBool::new(true));
    let opts = GenerateOptions {
        interval_ms: 100,
        cancel: Some(Arc::clone(&cancel)),
        ..GenerateOptions::default()
    };
    // Cancelled before it starts: the first grabber refuses.
    assert!(matches!(
        generate::prepare(ffmpeg, &source, &opts),
        Err(pa_trickplay::Error::Cancelled)
    ));
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let live = generate::prepare(ffmpeg, &source, &opts).unwrap();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(
        generate::run(ffmpeg, &source, &opts, &live),
        Err(pa_trickplay::Error::Cancelled)
    ));
}
