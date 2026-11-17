// SPDX-License-Identifier: MPL-2.0
//! The library after writes in a source (NET-023): an upload, a move and a
//! trash show up through `refresh_after_write` on the folders the
//! operation reports, without a full scan.

use std::path::Path;

use pa_catalog::Catalog;
use pa_scanner::CancelToken;
use pa_vfs::files::{self, Conflict};
use pa_vfs::{LocalFs, VfsPath};

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn movie_titles(cat: &Catalog) -> Vec<String> {
    let mut t: Vec<_> = cat.movies().unwrap().into_iter().map(|m| m.title).collect();
    t.sort();
    t
}

fn p(s: &str) -> VfsPath {
    VfsPath::parse(s).unwrap()
}

#[test]
fn uploads_moves_and_trash_reach_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Movies/Dune (2021)/Dune.2021.2160p.WEB-DL.mkv", 1);
    media(&root, "Movies/The Matrix 1999.mkv", 2);
    let cat = Catalog::open(dir.path().join("db/library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let id = cat.sources().unwrap()[0].id;
    assert_eq!(movie_titles(&cat), ["Dune", "The Matrix"]);
    assert!(cat.source_writable(id).unwrap());

    let vfs = LocalFs::new(&root).unwrap();
    // Upload a film into /Movies.
    let mut data = std::io::Cursor::new(vec![7u8; 200_000]);
    let up = files::upload(
        &vfs,
        &p("/Movies"),
        "Heat.1995.1080p.mkv",
        &mut data,
        Conflict::KeepBoth,
        &mut |_, _| true,
    )
    .unwrap();
    let r = cat
        .refresh_after_write(id, &up.touched, &CancelToken::new())
        .unwrap();
    assert_eq!(r.added, 1, "{r:?}");
    assert_eq!(r.files_seen, 2, "only /Movies itself was listed: {r:?}");
    assert!(movie_titles(&cat).contains(&"Heat".to_owned()));

    // Move the Dune folder into a new folder: gone from /Movies, found
    // under /Archive with everything in it.
    let archive =
        files::create_folder(&vfs, &VfsPath::root(), "Archive", Conflict::KeepBoth).unwrap();
    let mv = files::move_within(
        &vfs,
        &p("/Movies/Dune (2021)"),
        &archive.path,
        None,
        Conflict::KeepBoth,
    )
    .unwrap();
    let r = cat
        .refresh_after_write(id, &mv.touched, &CancelToken::new())
        .unwrap();
    assert_eq!((r.added, r.removed), (1, 1), "{r:?}");
    assert!(movie_titles(&cat).contains(&"Dune".to_owned()));

    // Trash the uploaded film: it leaves the library.
    let tr = files::trash(&vfs, &up.path).unwrap();
    let r = cat
        .refresh_after_write(id, &tr.touched, &CancelToken::new())
        .unwrap();
    assert_eq!(r.removed, 1, "{r:?}");
    assert!(!movie_titles(&cat).contains(&"Heat".to_owned()));

    // A full scan finds nothing new: the trash is not scanned and the
    // incremental updates were complete.
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    assert_eq!(movie_titles(&cat), ["Dune", "The Matrix"]);
}
