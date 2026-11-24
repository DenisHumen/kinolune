// SPDX-License-Identifier: MPL-2.0
//! UI-049: a whole season marked watched or unwatched in one call, each
//! change in the sync outbox, and «Отменить» putting back the watched marks
//! and the positions the episodes had.

use std::path::Path;

use pa_catalog::{Catalog, ItemRef, WatchMark};
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn setup() -> (tempfile::TempDir, Catalog, Vec<ItemRef>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Shows/Dark/Season 1/Dark.S01E01.Secrets.1080p.mkv", 3);
    media(&root, "Shows/Dark/Season 1/Dark.S01E02.Lies.1080p.mkv", 4);
    media(&root, "Shows/Dark/Season 1/Dark.S01E03.Past.1080p.mkv", 5);
    media(&root, "Shows/Dark/Season 1/Dark.S01E04.Double.1080p.mkv", 6);
    let cat = Catalog::open(dir.path().join("db/library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let show = cat.shows().unwrap()[0].id;
    let eps: Vec<ItemRef> = cat.episodes(show).unwrap().iter().map(|e| e.id).collect();
    assert_eq!(eps.len(), 4);
    (dir, cat, eps)
}

/// (position, watched) of an episode; (0, false) without progress.
fn state(cat: &Catalog, item: ItemRef) -> (i64, bool) {
    cat.progress(item)
        .unwrap()
        .map_or((0, false), |p| (p.position_ms, p.watched))
}

#[test]
fn bulk_mark_watched_and_undo_restores_positions() {
    let (_dir, cat, eps) = setup();
    // E1 watched, E2 started at 40 %, E3 and E4 never played.
    cat.save_progress(eps[0], 2_990_000, 3_000_000, 1_000, false)
        .unwrap();
    cat.save_progress(eps[1], 1_200_000, 3_000_000, 1_000, false)
        .unwrap();
    assert_eq!(state(&cat, eps[0]), (2_990_000, true));

    let before = cat.set_watched_many(&eps, true, 2_000).unwrap();
    // E1 was watched already: only the other three changed.
    let changed: Vec<ItemRef> = before.iter().map(|m| m.item).collect();
    assert_eq!(changed, eps[1..].to_vec());
    assert_eq!(
        before[0],
        WatchMark {
            item: eps[1],
            position_ms: 1_200_000,
            duration_ms: 3_000_000,
            watched: false,
        }
    );
    for &e in &eps {
        assert!(state(&cat, e).1, "{e} is watched");
    }
    // Nothing of the season is left in "Continue watching".
    assert!(
        cat.continue_watching(50)
            .unwrap()
            .iter()
            .all(|c| !eps.contains(&c.item.id))
    );
    // Marking again changes nothing.
    assert!(cat.set_watched_many(&eps, true, 2_500).unwrap().is_empty());

    // «Отменить»: E2 back at 40 %, E3 and E4 not watched, E1 untouched.
    cat.restore_watch_marks(&before, 3_000).unwrap();
    assert_eq!(state(&cat, eps[0]), (2_990_000, true));
    assert_eq!(state(&cat, eps[1]), (1_200_000, false));
    assert_eq!(state(&cat, eps[2]), (0, false));
    assert_eq!(state(&cat, eps[3]), (0, false));
    // E2 is in "Continue watching" again.
    assert!(
        cat.continue_watching(50)
            .unwrap()
            .iter()
            .any(|c| c.item.id == eps[1] && !c.up_next)
    );
}

#[test]
fn bulk_mark_unwatched_and_undo_restores_watched() {
    let (_dir, cat, eps) = setup();
    cat.save_progress(eps[0], 0, 3_000_000, 1_000, true).unwrap();
    cat.save_progress(eps[1], 2_990_000, 3_000_000, 1_000, false)
        .unwrap();
    cat.save_progress(eps[2], 600_000, 3_000_000, 1_000, false)
        .unwrap();

    let before = cat.set_watched_many(&eps, false, 2_000).unwrap();
    // E4 had nothing to clear.
    let changed: Vec<ItemRef> = before.iter().map(|m| m.item).collect();
    assert_eq!(changed, eps[..3].to_vec());
    for &e in &eps {
        assert_eq!(state(&cat, e), (0, false), "{e} is cleared");
    }

    cat.restore_watch_marks(&before, 3_000).unwrap();
    assert!(state(&cat, eps[0]).1);
    assert_eq!(state(&cat, eps[1]), (2_990_000, true));
    assert_eq!(state(&cat, eps[2]), (600_000, false));
    assert_eq!(state(&cat, eps[3]), (0, false));
}

#[test]
fn bulk_mark_goes_to_sync_and_skips_items_without_progress() {
    let (_dir, cat, eps) = setup();
    cat.enable_sync("device-a").unwrap();
    let ops_before = cat.pending_ops(512).unwrap().len();
    let show = cat.shows().unwrap()[0].id;
    // A show has no progress of its own: it is skipped, its episodes not.
    let mut items = vec![show];
    items.extend_from_slice(&eps);
    let before = cat.set_watched_many(&items, true, 2_000).unwrap();
    assert_eq!(before.len(), eps.len());
    let ops = cat.pending_ops(512).unwrap();
    // At least one op per episode (position and "watched").
    assert!(
        ops.len() >= ops_before + eps.len(),
        "{} ops after {ops_before}",
        ops.len()
    );

    // «Отменить» is synced too: new ops, not just the ones of the mark.
    let ids = |ops: &[pa_catalog::pa_sync::Op]| -> std::collections::HashSet<String> {
        ops.iter().map(|o| format!("{:?}", o.op_id)).collect()
    };
    let marked = ids(&ops);
    cat.restore_watch_marks(&before, 3_000).unwrap();
    let restored = ids(&cat.pending_ops(512).unwrap());
    assert!(restored.difference(&marked).count() >= eps.len());
    for &e in &eps {
        assert!(!state(&cat, e).1, "{e} is not watched after undo");
    }
}
