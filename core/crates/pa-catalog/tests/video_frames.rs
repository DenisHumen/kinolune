// SPDX-License-Identifier: MPL-2.0
//! UI-032: preview frames of the user's own videos, on real folders with
//! frames from a fake decoder (no FFmpeg needed):
//!
//! - a video of an unlocked personal library gets a frame at about 10 %
//!   of its duration, a black one is skipped, and the frame is cached in
//!   the library's own folder by the file's fingerprint;
//! - a locked library is refused before anything is read, its cached
//!   frames included, and a library locked while a frame is decoded gets
//!   nothing stored;
//! - the main library («Мои видео») has its own folder and never returns
//!   a personal file's frame;
//! - deleting the library deletes its frames.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pa_catalog::frame_search::{FrameReader, FrameSource};
use pa_catalog::{Catalog, ContentType, Error, Lock, PersonalFilter, PersonalSort};
use pa_probe::VideoFrame;
use pa_scanner::CancelToken;
use pa_vfs::Vfs;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

/// A decoder of 10-minute videos whose frame at 10 % is black.
#[derive(Default)]
struct Frames {
    opened: AtomicUsize,
    asked: Arc<Mutex<Vec<u64>>>,
}

struct Reader {
    asked: Arc<Mutex<Vec<u64>>>,
}

const DURATION_MS: u64 = 10 * 60 * 1000;

impl FrameSource for Frames {
    fn open(&self, _vfs: Arc<dyn Vfs>, _path: &str) -> Result<Box<dyn FrameReader>, String> {
        self.opened.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(Reader {
            asked: Arc::clone(&self.asked),
        }))
    }
}

impl FrameReader for Reader {
    fn duration_ms(&self) -> Option<u64> {
        Some(DURATION_MS)
    }

    fn frame_at(&mut self, at_ms: u64) -> Result<Option<VideoFrame>, String> {
        self.asked.lock().unwrap().push(at_ms);
        let (w, h) = (64u32, 36u32);
        let black = at_ms <= DURATION_MS / 10;
        let rgb = (0..w * h)
            .flat_map(|i| {
                if black {
                    [0, 0, 0]
                } else {
                    let x = u8::try_from(i % w * 4).unwrap_or(255);
                    [x, 255 - x, 90]
                }
            })
            .collect();
        Ok(Some(VideoFrame {
            position_ms: at_ms,
            width: w,
            height: h,
            rgb,
        }))
    }
}

/// Locks the library as soon as a file is opened (the app went to the
/// background while a frame was being decoded).
struct LockWhileDecoding<'a> {
    cat: &'a Catalog,
    library: i64,
    inner: Frames,
}

impl FrameSource for LockWhileDecoding<'_> {
    fn open(&self, vfs: Arc<dyn Vfs>, path: &str) -> Result<Box<dyn FrameReader>, String> {
        self.cat.lock_personal_library(self.library);
        self.inner.open(vfs, path)
    }
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_frame_is_made_for_an_unlocked_library_and_refused_for_a_locked_one() {
    let dir = tempfile::tempdir().unwrap();
    let main_root = dir.path().join("media");
    let home_root = dir.path().join("home");
    media(&main_root, "Clips/VID_20240612_101500.mp4", 1);
    media(&home_root, "2024/Birthday/clip_0042.mp4", 2);
    media(&home_root, "2024/Beach/clip_0043.mp4", 3);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let main = cat
        .add_source(None, "local", "Media", main_root.to_str().unwrap(), None)
        .unwrap()
        .id;
    let family = cat
        .create_personal_library("Семья", &Lock::Pin("2468"))
        .unwrap()
        .id;
    let home = cat
        .add_source(None, "local", "Home", home_root.to_str().unwrap(), None)
        .unwrap()
        .id;
    cat.set_source_content(home, ContentType::Personal, Some(family))
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();

    let videos = cat
        .personal_items(family, &PersonalFilter::default(), PersonalSort::Title)
        .unwrap();
    assert_eq!(videos.len(), 2);
    let (beach, birthday) = (videos[0].file_id, videos[1].file_id);
    let frames_dir = dir.path().join("frames").join(format!("personal-{family}"));

    // Unlocked (just created): a frame, from 15 % since 10 % is black.
    let frames = Frames::default();
    let jpeg = cat
        .personal_frame(family, birthday, &frames)
        .unwrap()
        .expect("a frame");
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "a JPEG");
    assert_eq!(
        *frames.asked.lock().unwrap(),
        [DURATION_MS / 10, DURATION_MS * 15 / 100],
        "about 10 % first, a black frame skipped"
    );
    // Cached in the library's own folder by the fingerprint, not the name.
    let cached = files_in(&frames_dir);
    assert_eq!(cached.len(), 1, "{cached:?}");
    assert!(
        cached[0].ends_with(".jpg") && !cached[0].contains("clip"),
        "{cached:?}"
    );
    let conn = pa_library::open(cat.path()).unwrap();
    let fingerprint: Option<String> = conn
        .query_row(
            "SELECT coalesce(fingerprint, oshash) FROM files WHERE id = ?1",
            [birthday],
            |r| r.get(0),
        )
        .unwrap();
    if let Some(fp) = fingerprint {
        assert_eq!(cached[0], format!("{fp}.jpg"));
    }
    // Asked again: from the cache, the file is not opened again.
    assert_eq!(
        cat.personal_frame(family, birthday, &frames).unwrap(),
        Some(jpeg.clone())
    );
    assert_eq!(frames.opened.load(Ordering::Relaxed), 1);

    // Locked: refused before anything is read, the cached frame included.
    cat.lock_personal_library(family);
    for file in [birthday, beach] {
        assert!(
            matches!(
                cat.personal_frame(family, file, &frames),
                Err(Error::Denied(_))
            ),
            "a locked library gives no frame"
        );
    }
    assert_eq!(frames.opened.load(Ordering::Relaxed), 1, "nothing was read");
    assert_eq!(files_in(&frames_dir).len(), 1, "nothing was made");

    // Locked while decoding: nothing stored, nothing returned.
    assert!(cat.unlock_personal_library(family, "2468").unwrap().ok);
    let racing = LockWhileDecoding {
        cat: &cat,
        library: family,
        inner: Frames::default(),
    };
    assert!(matches!(
        cat.personal_frame(family, beach, &racing),
        Err(Error::Denied(_))
    ));
    assert_eq!(files_in(&frames_dir).len(), 1, "the late frame was dropped");

    // Open again: the cached frame is back.
    assert!(cat.unlock_personal_library(family, "2468").unwrap().ok);
    assert_eq!(
        cat.personal_frame(family, birthday, &frames).unwrap(),
        Some(jpeg)
    );

    // «Мои видео» of the main library: its own folder; a personal file is
    // not found there.
    let clip: i64 = conn
        .query_row("SELECT id FROM files WHERE source_id = ?1", [main], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(cat.video_frame(clip, &frames).unwrap().is_some());
    assert_eq!(files_in(&dir.path().join("frames").join("main")).len(), 1);
    assert!(matches!(
        cat.video_frame(birthday, &frames),
        Err(Error::NotFound(_))
    ));

    // A file the decoder finds only black in: no frame, remembered.
    struct Black;
    struct BlackReader;
    impl FrameSource for Black {
        fn open(&self, _: Arc<dyn Vfs>, _: &str) -> Result<Box<dyn FrameReader>, String> {
            Ok(Box::new(BlackReader))
        }
    }
    impl FrameReader for BlackReader {
        fn duration_ms(&self) -> Option<u64> {
            Some(DURATION_MS)
        }
        fn frame_at(&mut self, at_ms: u64) -> Result<Option<VideoFrame>, String> {
            Ok(Some(VideoFrame {
                position_ms: at_ms,
                width: 8,
                height: 8,
                rgb: vec![0; 8 * 8 * 3],
            }))
        }
    }
    assert_eq!(cat.personal_frame(family, beach, &Black).unwrap(), None);
    assert_eq!(
        cat.personal_frame(family, beach, &frames).unwrap(),
        None,
        "not decoded again"
    );

    // Deleting the library deletes its frames.
    cat.delete_personal_library(family).unwrap();
    assert!(!frames_dir.exists(), "the frames went with the library");
    assert!(dir.path().join("frames").join("main").exists());
}
