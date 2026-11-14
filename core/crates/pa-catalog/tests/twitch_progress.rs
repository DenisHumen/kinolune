// SPDX-License-Identifier: MPL-2.0
//! TWI-005: a Twitch VOD played in the official embed is no catalog item;
//! its position is stored under its media key `twitch:vod:<id>`, feeds the
//! resume point and, with sync on, becomes an op for the other devices.

use pa_catalog::Catalog;
use pa_library::progress::{self, ContinueQuery, Thresholds};
use pa_model::{MediaKey, MediaKind};

fn vod(id: &str) -> MediaKey {
    MediaKey::TwitchVod {
        video_id: id.to_owned(),
    }
}

#[test]
fn vod_position_is_stored_by_key_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let key = vod("2001");
    cat.save_key_progress(&key, 600_000, 3_600_000, 1_763_000_000_000, false)
        .unwrap();
    let point = cat.resume_point_for_key(&key).unwrap().expect("stored");
    assert_eq!(point.position_ms, 600_000);
    assert!(cat.resume_point_for_key(&vod("2002")).unwrap().is_none());
}

#[test]
fn vod_progress_feeds_twitch_continue_watching() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let cat = Catalog::open(&path).unwrap();
    cat.save_key_progress(&vod("2001"), 600_000, 3_600_000, 1_763_000_000_000, false)
        .unwrap();
    drop(cat);
    let conn = pa_library::rusqlite::Connection::open(&path).unwrap();
    let rows = progress::continue_watching(
        &conn,
        &Thresholds::default(),
        1,
        &ContinueQuery {
            kinds: vec![MediaKind::TwitchVod],
            up_next: false,
            limit: 10,
        },
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item.media_key, vod("2001"));
}

#[test]
fn only_twitch_vod_keys_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let movie = MediaKey::TmdbMovie { id: 603 };
    assert!(cat.save_key_progress(&movie, 1, 2, 3, false).is_err());
    let clip = MediaKey::TwitchClip {
        slug: "Some-Clip".into(),
    };
    assert!(cat.save_key_progress(&clip, 1, 2, 3, false).is_err());
}

#[test]
fn with_sync_on_the_position_becomes_an_op() {
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.enable_sync("dev-a").unwrap();
    cat.save_key_progress(&vod("2001"), 600_000, 3_600_000, 1_763_000_000_000, false)
        .unwrap();
    let ops = cat.pending_ops(16).unwrap();
    assert!(
        ops.iter().any(|op| format!("{op:?}").contains("2001")),
        "{ops:?}"
    );
}
