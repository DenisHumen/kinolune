// SPDX-License-Identifier: MPL-2.0
//! Sidecar previews and the cache through the service, without FFmpeg.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use pa_probe::VideoFrame;
use pa_trickplay::{Crop, Media, Origin, Status, Trickplay, TrickplayOptions, bif, jpeg};
use pa_vfs::{LocalFs, Vfs, VfsPath};

fn jpeg_of(width: u32, height: u32, shade: u8) -> Vec<u8> {
    let frame = VideoFrame {
        position_ms: 0,
        width,
        height,
        rgb: vec![shade; (width * height * 3) as usize],
    };
    jpeg::encode(&frame, 80).unwrap()
}

fn media(root: &Path, rel: &str, duration_ms: Option<u64>) -> Media {
    let vfs: Arc<dyn Vfs> = Arc::new(LocalFs::new(root).unwrap());
    let path = VfsPath::parse(rel).unwrap();
    Media {
        key: format!("test:{rel}"),
        source: Arc::from(vfs.open(&path).unwrap()),
        sidecar: Some((vfs, path)),
        duration_ms,
    }
}

fn no_generation() -> TrickplayOptions {
    TrickplayOptions {
        generate: false,
        ..TrickplayOptions::default()
    }
}

#[test]
fn reads_a_bif_next_to_the_media() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("Movies");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Film (2020).mkv"), b"not really a video").unwrap();
    let frames: Vec<Vec<u8>> = (0..5).map(|i| jpeg_of(64, 36, i * 40)).collect();
    let list: Vec<(u32, &[u8])> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| (i as u32, f.as_slice()))
        .collect();
    fs::write(
        dir.join("Film (2020)-320-10.bif"),
        bif::write(10_000, &list),
    )
    .unwrap();
    // Another film's BIF is not ours.
    fs::write(dir.join("Film (2020) Extended.bif"), b"x").unwrap();

    let cache = tempfile::tempdir().unwrap();
    let service = Trickplay::open(cache.path(), no_generation()).unwrap();
    let session = service.prepare(media(root.path(), "/Movies/Film (2020).mkv", None));
    assert_eq!(
        session.wait(Duration::from_secs(10)),
        Status::Ready(Origin::SidecarBif)
    );
    assert_eq!(session.interval_ms(), Some(10_000));
    let frame = session.frame_at(25_000).unwrap();
    assert_eq!(frame.position_ms, 20_000);
    assert_eq!(&*frame.image, frames[2].as_slice());
    assert_eq!(frame.crop, None);
    assert_eq!(session.frame_at(10_000_000).unwrap().position_ms, 40_000);
    session.release();
}

#[test]
fn reads_jellyfin_trickplay_sheets() {
    let root = tempfile::tempdir().unwrap();
    let res = root.path().join("Show/Ep.trickplay/320 - 2x2");
    fs::create_dir_all(&res).unwrap();
    fs::create_dir_all(root.path().join("Show/Ep.trickplay/160 - 2x2")).unwrap();
    fs::write(root.path().join("Show/Ep.mkv"), b"video").unwrap();
    // 2x2 tiles of 320x180: two full sheets and a last one with one row.
    fs::write(res.join("0.jpg"), jpeg_of(640, 360, 10)).unwrap();
    fs::write(res.join("1.jpg"), jpeg_of(640, 360, 20)).unwrap();
    fs::write(res.join("2.jpg"), jpeg_of(640, 180, 30)).unwrap();

    let cache = tempfile::tempdir().unwrap();
    let service = Trickplay::open(cache.path(), no_generation()).unwrap();
    // 10 tiles at 10 s.
    let session = service.prepare(media(root.path(), "/Show/Ep.mkv", Some(95_000)));
    assert_eq!(
        session.wait(Duration::from_secs(10)),
        Status::Ready(Origin::Jellyfin)
    );
    let tile = session.frame_at(55_000).unwrap();
    // Tile 5: sheet 1, second tile of the first row.
    assert_eq!(tile.image_id, 1);
    assert_eq!(tile.position_ms, 50_000);
    assert_eq!(
        tile.crop,
        Some(Crop {
            x: 320,
            y: 0,
            width: 320,
            height: 180
        })
    );
    let last = session.frame_at(95_000).unwrap();
    assert_eq!(last.image_id, 2);
    assert_eq!(last.crop.unwrap().y, 0);
    session.release();
}

#[test]
fn serves_the_cache_and_reports_missing_previews() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.mkv"), b"video").unwrap();
    let cache = tempfile::tempdir().unwrap();
    let service = Trickplay::open(cache.path(), no_generation()).unwrap();

    let session = service.prepare(media(root.path(), "/a.mkv", None));
    assert!(matches!(
        session.wait(Duration::from_secs(10)),
        Status::Unavailable(_)
    ));
    assert_eq!(session.frame_at(0), None);
    session.release();

    let thumb = jpeg_of(32, 18, 99);
    service
        .cache()
        .store(
            "test:/a.mkv",
            &bif::write(5_000, &[(0, &thumb), (1, &thumb)]),
        )
        .unwrap();
    let session = service.prepare(media(root.path(), "/a.mkv", None));
    assert_eq!(
        session.wait(Duration::from_secs(10)),
        Status::Ready(Origin::Cache)
    );
    assert_eq!(session.frame_at(7_000).unwrap().position_ms, 5_000);
    // A second user joins the same session.
    let again = service.prepare(media(root.path(), "/a.mkv", None));
    assert!(Arc::ptr_eq(&session, &again));
    session.release();
    again.release();
    assert_eq!(service.usage().files, 1);
}
