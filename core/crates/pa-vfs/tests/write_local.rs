// SPDX-License-Identifier: MPL-2.0
//! The write contract (NET-023) against `LocalFs`, plus local-only checks:
//! links are never followed by a delete, read-only folders and backends.

mod common;

use common::TempDir;
use pa_vfs::files::{self, Conflict};
use pa_vfs::write_contract::{self as wc, Generated};
use pa_vfs::{ByteSource, DirEntry, LocalFs, Metadata, Result, Vfs, VfsPath};

fn source() -> (TempDir, LocalFs) {
    let dir = TempDir::new();
    let vfs = LocalFs::new(dir.path()).expect("LocalFs::new");
    (dir, vfs)
}

macro_rules! case {
    ($($name:ident),+ $(,)?) => {$(
        #[test]
        fn $name() {
            let (_dir, vfs) = source();
            wc::$name(&vfs, &VfsPath::root());
        }
    )+};
}

case!(writable, folders, upload_conflicts, upload_cancel, move_and_rename, copy, trash, delete);

#[test]
fn run_all_nested_base() {
    let (dir, vfs) = source();
    std::fs::create_dir_all(dir.path().join("media/личное")).expect("mkdir");
    wc::run_all(&vfs, &VfsPath::parse("/media/личное").expect("path"), true);
}

#[test]
fn resume_64_mib() {
    let (_dir, vfs) = source();
    let r = wc::upload_resume(&vfs, &VfsPath::root(), 64 * 1024 * 1024 + 3, true);
    assert_eq!(r.part_after_break, 32 * 1024 * 1024 + 1);
    assert_eq!(r.sent_on_resume, 32 * 1024 * 1024 + 2);
}

/// The 1 GiB upload of the NET-023 "done when", on the local disk.
/// Ignored by default (writes 1 GiB); the live-server run includes it.
#[test]
#[ignore = "writes 1 GiB to the temp dir"]
fn resume_1_gib() {
    let (_dir, vfs) = source();
    let r = wc::upload_resume(&vfs, &VfsPath::root(), 1 << 30, true);
    assert_eq!(r.part_after_break, 1 << 29);
}

#[test]
fn between_two_local_sources() {
    let (_a, src) = source();
    let (_b, dst) = source();
    wc::between(&src, &VfsPath::root(), &dst, &VfsPath::root());
}

/// A stale `.part` of another file is not resumed.
#[test]
fn foreign_part_is_not_resumed() {
    let (dir, vfs) = source();
    let other = Generated::new(4096, 1);
    let mut buf = vec![0u8; 4096];
    other.fill(0, &mut buf);
    std::fs::write(dir.path().join("a.bin.part"), &buf).expect("seed part");
    let data = Generated::new(10_000, 2);
    let mut d = data.clone();
    let mut first = None;
    let out = files::upload(&vfs, &VfsPath::root(), "a.bin", &mut d, Conflict::KeepBoth, &mut |done, _| {
        first.get_or_insert(done);
        true
    })
    .expect("upload");
    assert_eq!(first, Some(0), "started over");
    wc::assert_same(&vfs, &out.path, &data);
}

#[cfg(unix)]
#[test]
fn delete_removes_links_not_targets() {
    let (dir, vfs) = source();
    let outside = TempDir::new();
    std::fs::write(outside.path().join("keep.txt"), b"keep").expect("write");
    std::fs::create_dir(dir.path().join("tree")).expect("mkdir");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("tree/link-dir")).expect("link");
    std::os::unix::fs::symlink(
        outside.path().join("keep.txt"),
        dir.path().join("tree/link-file"),
    )
    .expect("link");
    files::delete_permanently(&vfs, &VfsPath::parse("/tree").expect("p")).expect("delete");
    assert!(!dir.path().join("tree").exists());
    assert_eq!(std::fs::read(outside.path().join("keep.txt")).expect("kept"), b"keep");

    // The same through the trash and a purge.
    std::fs::create_dir(dir.path().join("tree2")).expect("mkdir");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("tree2/link-dir")).expect("link");
    files::trash(&vfs, &VfsPath::parse("/tree2").expect("p")).expect("trash");
    files::empty_trash(&vfs).expect("empty");
    assert_eq!(std::fs::read(outside.path().join("keep.txt")).expect("kept"), b"keep");
}

/// A folder the user may read but not write (skipped as root, which may
/// write anywhere).
#[cfg(unix)]
#[test]
fn read_only_folder() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _) = source();
    let ro = dir.path().join("ro");
    std::fs::create_dir(&ro).expect("mkdir");
    std::fs::write(ro.join("film.mkv"), b"x").expect("write");
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    if std::fs::write(ro.join("probe"), b"").is_ok() {
        eprintln!("running as a user that ignores permissions; skipped");
        let _ = std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755));
        return;
    }
    let vfs = LocalFs::new(&ro).expect("LocalFs::new");
    wc::read_only(&vfs, &VfsPath::root());
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).expect("chmod back");
}

/// A backend without a write side is read-only, whatever the folder allows.
#[test]
fn backend_without_writes_is_read_only() {
    #[derive(Debug)]
    struct ReadOnly(LocalFs);
    impl Vfs for ReadOnly {
        fn list(&self, dir: &VfsPath) -> Result<Vec<DirEntry>> {
            self.0.list(dir)
        }
        fn stat(&self, path: &VfsPath) -> Result<Metadata> {
            self.0.stat(path)
        }
        fn open(&self, path: &VfsPath) -> Result<Box<dyn ByteSource>> {
            self.0.open(path)
        }
    }
    let (_dir, vfs) = source();
    wc::read_only(&ReadOnly(vfs), &VfsPath::root());
}

#[test]
fn writes_never_touch_the_root() {
    use pa_vfs::{ErrorKind, VfsWrite};
    let (_dir, vfs) = source();
    let root = VfsPath::root();
    for e in [
        vfs.create_dir(&root),
        vfs.remove_dir(&root),
        vfs.remove_file(&root),
        vfs.rename(&root, &VfsPath::parse("/x").expect("p")),
    ] {
        assert_eq!(e.expect_err("root").kind(), ErrorKind::InvalidPath);
    }
}
