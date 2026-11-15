// SPDX-License-Identifier: MPL-2.0
//! LIB-004 on a real folder: a renamed or moved file keeps its row, match
//! and progress; the ed2k job hashes local files and follows the source
//! policy.

use std::path::Path;

use pa_catalog::{Catalog, Ed2kOptions, Ed2kPolicy, Ed2kTrigger, ItemRef, fingerprint};
use pa_library::rusqlite::OptionalExtension;
use pa_scanner::CancelToken;
use pa_vfs::VfsPath;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

struct Lib {
    dir: tempfile::TempDir,
    cat: Catalog,
    source: i64,
}

impl Lib {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        media(&root, "Movies/Dune (2021)/Dune.2021.2160p.WEB-DL.mkv", 1);
        media(&root, "Movies/The Matrix 1999.mkv", 2);
        media(&root, "Music/Artist/Album/01 - Song.mp3", 6);
        let cat = Catalog::open(dir.path().join("db/library.db")).unwrap();
        let source = cat
            .add_source(None, "local", "Media", root.to_str().unwrap(), None)
            .unwrap()
            .id;
        let lib = Self { dir, cat, source };
        lib.scan();
        lib
    }

    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("media")
    }

    fn scan(&self) {
        let mut errors = Vec::new();
        self.cat
            .scan(None, &CancelToken::new(), &mut |u| errors.extend(u.error))
            .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }

    fn db(&self) -> pa_library::rusqlite::Connection {
        pa_library::open(self.cat.path()).unwrap()
    }

    /// `(id, missing)` of the row at `path`.
    fn file(&self, path: &str) -> Option<(i64, bool)> {
        self.db()
            .query_row(
                "SELECT id, missing_since_ms IS NOT NULL FROM files WHERE path = ?1",
                [path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    fn movie(&self, title: &str) -> pa_catalog::Item {
        self.cat
            .movies()
            .unwrap()
            .into_iter()
            .find(|m| m.title.contains(title))
            .unwrap_or_else(|| panic!("no movie {title}"))
    }

    fn match_state(&self, file: i64) -> Option<String> {
        self.db()
            .query_row(
                "SELECT state FROM matches WHERE file_id = ?1 AND state <> 'rejected'",
                [file],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
    }
}

const DUNE: &str = "/Movies/Dune (2021)/Dune.2021.2160p.WEB-DL.mkv";

#[test]
fn renamed_and_moved_file_keeps_row_match_and_progress() {
    let lib = Lib::new();
    let (id, _) = lib.file(DUNE).unwrap();
    let dune = lib.movie("Dune").id;
    lib.cat
        .fix_match(dune, Some("Dune"), Some(2021), Some(438_631))
        .unwrap();
    assert!(
        lib.cat
            .save_progress(dune, 600_000, 9_000_000, 1_000, false)
            .unwrap()
    );
    let ItemRef::Movie(work) = dune else {
        panic!("{dune:?}")
    };

    // Renamed and moved to another folder.
    let to = "/Movies/Sci-Fi/Dune Part One (2021).mkv";
    std::fs::create_dir_all(lib.root().join("Movies/Sci-Fi")).unwrap();
    std::fs::rename(lib.root().join(&DUNE[1..]), lib.root().join(&to[1..])).unwrap();
    lib.scan();

    assert_eq!(lib.file(DUNE), None);
    assert_eq!(lib.file(to), Some((id, false)), "the old row moved");
    assert_eq!(lib.match_state(id).as_deref(), Some("user_set"));
    let item = lib.movie("Dune");
    assert_eq!(item.id, ItemRef::Movie(work));
    assert_eq!(item.rel_path.trim_start_matches('/'), &to[1..]);
    let progress = lib.cat.progress(item.id).unwrap().unwrap();
    assert_eq!(progress.position_ms, 600_000);
    // One Dune, not two.
    assert_eq!(
        lib.cat
            .movies()
            .unwrap()
            .iter()
            .filter(|m| m.title.contains("Dune"))
            .count(),
        1
    );
}

#[test]
fn watcher_seeing_the_new_path_first_still_follows_the_move() {
    let lib = Lib::new();
    let old = "/Movies/The Matrix 1999.mkv";
    let (id, _) = lib.file(old).unwrap();
    let matrix = lib.movie("Matrix").id;
    assert!(
        lib.cat
            .save_progress(matrix, 42_000, 8_000_000, 1_000, false)
            .unwrap()
    );

    let to = "/Archive/The.Matrix.1999.1080p.mkv";
    std::fs::create_dir_all(lib.root().join("Archive")).unwrap();
    std::fs::rename(lib.root().join(&old[1..]), lib.root().join(&to[1..])).unwrap();
    // The new folder is rescanned first: the file is new and recognized.
    lib.cat
        .rescan_folders(
            lib.source,
            &[VfsPath::parse("/Archive").unwrap()],
            &CancelToken::new(),
        )
        .unwrap();
    // Then the old folder: the old row goes missing and takes the new path.
    lib.cat
        .rescan_folders(
            lib.source,
            &[VfsPath::parse("/Movies").unwrap()],
            &CancelToken::new(),
        )
        .unwrap();

    assert_eq!(lib.file(to), Some((id, false)));
    assert_eq!(lib.file(old), None);
    let matrices: Vec<_> = lib
        .cat
        .movies()
        .unwrap()
        .into_iter()
        .filter(|m| m.title.contains("Matrix"))
        .collect();
    assert_eq!(matrices.len(), 1, "{matrices:?}");
    assert_eq!(matrices[0].id, matrix);
    assert_eq!(
        lib.cat.progress(matrix).unwrap().unwrap().position_ms,
        42_000
    );
}

#[test]
fn a_copy_is_not_a_move() {
    let lib = Lib::new();
    let (id, _) = lib.file(DUNE).unwrap();
    let copy = "/Backup/Dune.2021.mkv";
    std::fs::create_dir_all(lib.root().join("Backup")).unwrap();
    std::fs::copy(lib.root().join(&DUNE[1..]), lib.root().join(&copy[1..])).unwrap();
    lib.scan();
    assert_eq!(lib.file(DUNE), Some((id, false)));
    let (copy_id, missing) = lib.file(copy).unwrap();
    assert!(!missing);
    assert_ne!(copy_id, id);
}

#[test]
fn ed2k_job_hashes_local_videos_and_follows_the_policy() {
    let lib = Lib::new();
    let ed2k_of = |path: &str| -> Option<String> {
        lib.db()
            .query_row("SELECT ed2k FROM files WHERE path = ?1", [path], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(lib.cat.ed2k_policy(lib.source).unwrap(), Ed2kPolicy::Auto);

    // `never`: a background run reads nothing.
    lib.cat
        .set_ed2k_policy(lib.source, Ed2kPolicy::Never)
        .unwrap();
    let r = lib
        .cat
        .hash_ed2k(&Ed2kOptions::default(), &CancelToken::new(), &mut |_| {})
        .unwrap();
    assert_eq!(r.hashed, 0);
    assert_eq!(ed2k_of(DUNE), None);

    // Back to `auto`: a local folder is hashed in the background.
    lib.cat
        .set_ed2k_policy(lib.source, Ed2kPolicy::Auto)
        .unwrap();
    let mut seen = Vec::new();
    let r = lib
        .cat
        .hash_ed2k(&Ed2kOptions::default(), &CancelToken::new(), &mut |p| {
            seen.push(p.path.clone());
        })
        .unwrap();
    assert_eq!(r.hashed, 2, "{r:?}");
    assert_eq!((r.failed, r.changed, r.stopped), (0, 0, false));
    assert!(seen.iter().all(|p| !p.ends_with(".mp3")), "{seen:?}");

    let fs = pa_vfs::LocalFs::new(lib.root()).unwrap();
    let source = pa_vfs::Vfs::open(&fs, &VfsPath::parse(DUNE).unwrap()).unwrap();
    let expect = fingerprint::ed2k(&*source, &|| false, &mut |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(ed2k_of(DUNE), Some(expect.hash));
    assert_eq!(ed2k_of("/Music/Artist/Album/01 - Song.mp3"), None);

    // Nothing left: a second run reads nothing.
    let r = lib
        .cat
        .hash_ed2k(&Ed2kOptions::default(), &CancelToken::new(), &mut |_| {})
        .unwrap();
    assert_eq!(r.hashed, 0);
}

#[test]
fn ed2k_on_demand_ignores_never_and_cancel_stops() {
    let lib = Lib::new();
    lib.cat
        .set_ed2k_policy(lib.source, Ed2kPolicy::Never)
        .unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let on_demand = Ed2kOptions {
        trigger: Ed2kTrigger::OnDemand,
        source_id: Some(lib.source),
        max_files: None,
        ..Ed2kOptions::default()
    };
    let r = lib.cat.hash_ed2k(&on_demand, &cancel, &mut |_| {}).unwrap();
    assert!(r.stopped);
    assert_eq!(r.hashed, 0);
    let r = lib
        .cat
        .hash_ed2k(&on_demand, &CancelToken::new(), &mut |_| {})
        .unwrap();
    assert_eq!(r.hashed, 2);
    assert!(lib.cat.set_ed2k_policy(999, Ed2kPolicy::Auto).is_err());
}

/// LIB-023: a run stopped in the middle of a big file keeps the chunks it
/// read; the next run reads only the rest and stores the same hash.
#[test]
fn ed2k_resumes_a_stopped_file() {
    let lib = Lib::new();
    let rel = "Movies/Big (2020)/Big.2020.mkv";
    let path = lib.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..30_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    std::fs::write(&path, &data).unwrap();
    lib.scan();
    let only_big = |p: &pa_catalog::Ed2kProgress| p.path.ends_with("Big.2020.mkv");
    // Hash the two small files first, then stop in the big one after
    // about 25 MB (two whole chunks of 9 500 KiB).
    let cancel = CancelToken::new();
    let r = lib
        .cat
        .hash_ed2k(&Ed2kOptions::default(), &cancel, &mut |p| {
            if only_big(p) && p.bytes_done >= 25_000_000 {
                cancel.cancel();
            }
        })
        .unwrap();
    assert!(r.stopped, "{r:?}");
    let saved: i64 = lib
        .db()
        .query_row(
            "SELECT length(chunk_md4) FROM file_ed2k_progress",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, 2 * 16, "two whole chunks kept");
    let mut first = None;
    let r = lib
        .cat
        .hash_ed2k(&Ed2kOptions::default(), &CancelToken::new(), &mut |p| {
            // The first report of a file is before any read.
            if only_big(p) && p.bytes_done > 0 {
                first.get_or_insert(p.bytes_done);
            }
        })
        .unwrap();
    assert_eq!(r.hashed, 1, "{r:?}");
    assert_eq!(first, Some(2 * fingerprint::ED2K_CHUNK), "resumed after the kept chunks");
    let fs = pa_vfs::LocalFs::new(lib.root()).unwrap();
    let source = pa_vfs::Vfs::open(&fs, &VfsPath::parse(&format!("/{rel}")).unwrap()).unwrap();
    let expect = fingerprint::ed2k(&*source, &|| false, &mut |_| {})
        .unwrap()
        .unwrap();
    let stored: Option<String> = lib
        .db()
        .query_row(
            "SELECT ed2k FROM files WHERE path = ?1",
            [format!("/{rel}")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, Some(expect.hash));
    let left: i64 = lib
        .db()
        .query_row("SELECT count(*) FROM file_ed2k_progress", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0, "progress is dropped once the hash is stored");
}
