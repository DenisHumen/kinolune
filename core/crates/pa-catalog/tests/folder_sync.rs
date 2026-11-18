// SPDX-License-Identifier: MPL-2.0
//! SYNC-007: three catalogs share one folder source (as a NAS share) and
//! sync progress and "watched" through journals in it, without pairing,
//! each going offline and online in any order; LAN acks do not take ops
//! away from the folder; the journals are not scanned as media.

use std::path::Path;

use pa_catalog::{Catalog, ItemRef};
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

struct Device {
    id: String,
    catalog: Catalog,
    source: i64,
    _dir: tempfile::TempDir,
}

fn device(id: &str, nas: &Path) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(dir.path().join("library.db")).unwrap();
    let source = catalog
        .add_source(None, "local", "NAS", nas.to_str().unwrap(), None)
        .unwrap()
        .id;
    catalog
        .scan(None, &CancelToken::new(), &mut |_| {})
        .unwrap();
    Device {
        id: id.to_owned(),
        catalog,
        source,
        _dir: dir,
    }
}

fn matrix(cat: &Catalog) -> ItemRef {
    let movies = cat.movies().unwrap();
    movies
        .iter()
        .find(|m| m.title.contains("Matrix"))
        .unwrap()
        .id
}

/// (position, watched) of the movie on a device.
fn state(d: &Device) -> Option<(i64, bool)> {
    d.catalog
        .progress(matrix(&d.catalog))
        .unwrap()
        .map(|p| (p.position_ms, p.watched))
}

#[test]
fn three_catalogs_sync_through_a_shared_folder_in_any_order() {
    let nas = tempfile::tempdir().unwrap();
    media(nas.path(), "Movies/The Matrix 1999.mkv", 2);
    let devices = [
        device("phone", nas.path()),
        device("tv", nas.path()),
        device("laptop", nas.path()),
    ];
    for d in &devices {
        let status = d.catalog.set_folder_sync(Some(d.source), &d.id).unwrap();
        assert_eq!(status.source_id, Some(d.source));
        assert_eq!(d.catalog.sync_device_id().as_deref(), Some(d.id.as_str()));
    }

    // A fixed "random" order: (device, action) with action 0 = play to a
    // position, 1 = go online and sync, 2 = stay offline.
    let script: [(usize, u8); 18] = [
        (0, 0),
        (1, 1),
        (0, 1),
        (2, 0),
        (1, 1),
        (2, 2),
        (1, 0),
        (0, 0),
        (2, 1),
        (1, 2),
        (0, 1),
        (2, 0),
        (1, 1),
        (2, 1),
        (0, 2),
        (1, 0),
        (2, 1),
        (1, 2),
    ];
    // Positions are stamped with the real clock, as the player does.
    let clock = || {
        std::thread::sleep(std::time::Duration::from_millis(3));
        pa_catalog::pa_sync::hlc::system_ms() as i64
    };
    let mut now: i64;
    let mut last = None;
    for (step, (d, action)) in script.into_iter().enumerate() {
        now = clock();
        let dev = &devices[d];
        match action {
            0 => {
                let pos = 60_000 * (step as i64 + 1);
                dev.catalog
                    .save_progress(matrix(&dev.catalog), pos, 8_160_000, now, false)
                    .unwrap();
                last = Some(pos);
            }
            1 => {
                dev.catalog.run_folder_sync(&dev.id).unwrap();
            }
            _ => {}
        }
    }
    // A LAN hub acked everything the phone queued: the folder still gets it.
    let phone = &devices[0];
    now = clock();
    phone
        .catalog
        .save_progress(matrix(&phone.catalog), 8_000_000, 8_160_000, now, true)
        .unwrap();
    for op in phone.catalog.pending_ops(512).unwrap() {
        phone.catalog.ack_op(op.op_id).unwrap();
    }
    assert!(phone.catalog.folder_sync_status().unwrap().pending > 0);
    assert!(last.is_some());

    for _ in 0..2 {
        for d in &devices {
            let run = d.catalog.run_folder_sync(&d.id).unwrap();
            assert_eq!(run.status.last_error, None);
            assert_eq!(run.status.devices, 2);
            assert!(!run.status.read_only);
        }
    }
    for d in &devices {
        assert_eq!(state(d), Some((8_000_000, true)), "{}", d.id);
        assert_eq!(d.catalog.folder_sync_status().unwrap().pending, 0);
    }
    for id in ["phone", "tv", "laptop"] {
        assert!(nas.path().join(format!(".kinolune/state/{id}.jsonl")).exists());
    }

    // The journals are no media.
    let tv = &devices[1];
    tv.catalog
        .scan(None, &CancelToken::new(), &mut |_| {})
        .unwrap();
    assert_eq!(tv.catalog.movies().unwrap().len(), 1);

    // Off: nothing more is kept for the folder.
    let off = tv.catalog.set_folder_sync(None, &tv.id).unwrap();
    assert_eq!(off.source_id, None);
    now = clock();
    tv.catalog
        .save_progress(matrix(&tv.catalog), 1_000, 8_160_000, now, false)
        .unwrap();
    assert_eq!(tv.catalog.folder_sync_status().unwrap().pending, 0);
    assert_eq!(tv.catalog.run_folder_sync(&tv.id).unwrap().written, 0);
}

#[test]
fn an_unreachable_folder_is_a_status_not_an_error() {
    let nas = tempfile::tempdir().unwrap();
    media(nas.path(), "Movies/The Matrix 1999.mkv", 2);
    let d = device("phone", nas.path());
    d.catalog.set_folder_sync(Some(d.source), &d.id).unwrap();
    // The share went away.
    let gone = nas.path().to_path_buf();
    drop(nas);
    assert!(!gone.exists());
    let run = d.catalog.run_folder_sync(&d.id).unwrap();
    assert!(run.status.last_error.is_some(), "{:?}", run.status);
    assert_eq!(run.status.last_sync_ms, None);
}
