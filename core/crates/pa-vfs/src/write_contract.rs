// SPDX-License-Identifier: MPL-2.0
//! Contract suite of the write side (NET-023): every backend that
//! implements [`VfsWrite`](crate::VfsWrite) runs it against a scratch
//! folder on a writable source, and [`read_only`] against a read-only one.
//!
//! The cases go through [`files`](crate::files), so they check the
//! backend primitives and the shared rules together. Each case works in a
//! fresh folder under `base` and panics with a descriptive message on
//! failure. Large uploads use [`Generated`] data, so no test file is
//! needed on disk.

use std::io::{self, Read, Seek, SeekFrom};

use crate::error::ErrorKind;
use crate::files::{self, Conflict, PART_SUFFIX, TRASH_KEEP};
use crate::path::VfsPath;
use crate::source::ByteSource;
use crate::vfs::Vfs;

/// Deterministic pseudo-random bytes of any size, readable and seekable
/// (the byte at an offset depends only on the offset and the seed).
#[derive(Debug, Clone)]
pub struct Generated {
    size: u64,
    pos: u64,
    seed: u64,
}

impl Generated {
    /// `size` bytes for `seed`.
    #[must_use]
    pub fn new(size: u64, seed: u64) -> Self {
        Self { size, pos: 0, seed }
    }

    /// Fills `buf` with the bytes at `offset`.
    pub fn fill(&self, offset: u64, buf: &mut [u8]) {
        for (i, b) in buf.iter_mut().enumerate() {
            let at = offset + i as u64;
            *b = word(self.seed, at >> 3).to_le_bytes()[(at & 7) as usize];
        }
    }
}

fn word(seed: u64, n: u64) -> u64 {
    // splitmix64
    let mut z = n.wrapping_add(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Read for Generated {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.size.saturating_sub(self.pos);
        let n = usize::try_from(left).unwrap_or(usize::MAX).min(buf.len());
        self.fill(self.pos, &mut buf[..n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Generated {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.size.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = next.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.pos)
    }
}

/// Data that breaks (an I/O error, like a dropped connection or a killed
/// app) after `left` bytes have been read through it.
#[derive(Debug)]
pub struct BreakAfter<R> {
    inner: R,
    left: u64,
}

impl<R> BreakAfter<R> {
    /// Breaks `inner` after `left` bytes.
    pub fn new(inner: R, left: u64) -> Self {
        Self { inner, left }
    }
}

impl<R: Read> Read for BreakAfter<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, "test break"));
        }
        let max = usize::try_from(self.left).unwrap_or(usize::MAX).min(buf.len());
        let n = self.inner.read(&mut buf[..max])?;
        self.left -= n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for BreakAfter<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// A fresh folder for one case: `base/<name>-<time>`.
#[must_use]
pub fn scratch(vfs: &dyn Vfs, base: &VfsPath, name: &str) -> VfsPath {
    let name = format!("{name}-{}", files::now_ms());
    files::create_folder(vfs, base, &name, Conflict::KeepBoth)
        .unwrap_or_else(|e| panic!("create scratch folder {name} in {base}: {e}"))
        .path
}

fn put(vfs: &dyn Vfs, dir: &VfsPath, name: &str, bytes: &[u8], c: Conflict) -> files::Outcome {
    let mut data = io::Cursor::new(bytes.to_vec());
    files::upload(vfs, dir, name, &mut data, c, &mut |_, _| true)
        .unwrap_or_else(|e| panic!("upload {dir}/{name}: {e}"))
}

/// Whole content of a file.
#[must_use]
pub fn read_all(vfs: &dyn Vfs, path: &VfsPath) -> Vec<u8> {
    let src = vfs.open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let mut buf = vec![0u8; usize::try_from(src.size()).expect("small file")];
    src.read_exact_at(0, &mut buf)
        .unwrap_or_else(|e| panic!("read {path}: {e}"));
    buf
}

fn exists(vfs: &dyn Vfs, path: &VfsPath) -> bool {
    match vfs.stat(path) {
        Ok(_) => true,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => panic!("stat {path}: {e}"),
    }
}

fn names(vfs: &dyn Vfs, dir: &VfsPath) -> Vec<String> {
    let mut n: Vec<_> = vfs
        .list(dir)
        .unwrap_or_else(|e| panic!("list {dir}: {e}"))
        .into_iter()
        .map(|e| e.name)
        .collect();
    n.sort();
    n
}

fn at(dir: &VfsPath, name: &str) -> VfsPath {
    dir.join(name).expect("valid name")
}

/// Checks that `path` holds exactly `data` (streamed in 4 MiB pieces).
pub fn assert_same(vfs: &dyn Vfs, path: &VfsPath, data: &Generated) {
    let src = vfs.open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    assert_eq!(src.size(), data.size, "{path}: size");
    let mut theirs = vec![0u8; 4 << 20];
    let mut ours = vec![0u8; 4 << 20];
    let mut off = 0u64;
    while off < data.size {
        let n = usize::try_from(data.size - off).unwrap_or(usize::MAX).min(theirs.len());
        src.read_exact_at(off, &mut theirs[..n])
            .unwrap_or_else(|e| panic!("read {path} at {off}: {e}"));
        data.fill(off, &mut ours[..n]);
        assert!(theirs[..n] == ours[..n], "{path}: content differs in [{off}, +{n})");
        off += n as u64;
    }
}

/// The source reports writes as possible.
pub fn writable(vfs: &dyn Vfs, _base: &VfsPath) {
    assert_eq!(files::writability(vfs).map_err(|e| e.to_string()), Ok(true));
}

/// Folders: create, name conflicts, nothing left of the probe.
pub fn folders(vfs: &dyn Vfs, base: &VfsPath) {
    let dir = scratch(vfs, base, "folders");
    let a = files::create_folder(vfs, &dir, "Фильмы 2024", Conflict::KeepBoth).expect("mkdir");
    assert_eq!(a.path, at(&dir, "Фильмы 2024"));
    assert!(!a.skipped);
    assert_eq!(a.touched.len(), 1);
    assert_eq!(a.touched[0].dir, dir);
    assert!(vfs.stat(&a.path).expect("stat").is_dir());
    let b = files::create_folder(vfs, &dir, "Фильмы 2024", Conflict::KeepBoth).expect("mkdir 2");
    assert_eq!(b.path, at(&dir, "Фильмы 2024 (2)"));
    let c = files::create_folder(vfs, &dir, "Фильмы 2024", Conflict::Skip).expect("mkdir skip");
    assert!(c.skipped);
    let d = files::create_folder(vfs, &a.path, "sub", Conflict::KeepBoth).expect("nested");
    assert!(vfs.stat(&d.path).expect("stat").is_dir());
    assert_eq!(names(vfs, &dir), ["Фильмы 2024", "Фильмы 2024 (2)"]);
}

/// Small uploads and every conflict policy.
pub fn upload_conflicts(vfs: &dyn Vfs, base: &VfsPath) {
    let dir = scratch(vfs, base, "conflicts");
    let name = "clip #1 & 100%.mp4";
    let a = put(vfs, &dir, name, b"first", Conflict::KeepBoth);
    assert_eq!(a.path, at(&dir, name));
    assert_eq!(read_all(vfs, &a.path), b"first");
    let b = put(vfs, &dir, name, b"second", Conflict::KeepBoth);
    assert_eq!(b.path, at(&dir, "clip #1 & 100% (2).mp4"));
    assert_eq!(read_all(vfs, &b.path), b"second");
    let c = put(vfs, &dir, name, b"third", Conflict::Skip);
    assert!(c.skipped);
    assert_eq!(read_all(vfs, &a.path), b"first");
    let d = put(vfs, &dir, name, b"replaced", Conflict::Replace);
    assert_eq!(d.path, a.path);
    assert_eq!(read_all(vfs, &d.path), b"replaced");
    // The replaced file went to the trash, not away.
    let trashed: Vec<_> = files::list_trash(vfs)
        .expect("list trash")
        .into_iter()
        .filter(|i| i.original == a.path)
        .collect();
    assert_eq!(trashed.len(), 1, "replaced file in the trash");
    assert_eq!(read_all(vfs, &trashed[0].path), b"first");
    let empty = put(vfs, &dir, "empty.bin", b"", Conflict::KeepBoth);
    assert_eq!(vfs.stat(&empty.path).expect("stat").size, 0);
    // No `.part` left behind.
    assert!(
        !names(vfs, &dir).iter().any(|n| n.ends_with(PART_SUFFIX)),
        "{:?}",
        names(vfs, &dir)
    );
}

/// What [`upload_resume`] saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeReport {
    /// Size of `.part` after the break.
    pub part_after_break: u64,
    /// Bytes the second attempt read from the data (less than the size
    /// when it resumed).
    pub sent_on_resume: u64,
}

/// Upload of `size` bytes that breaks in the middle and is resumed.
///
/// With `expect_resume` the second attempt must continue from the `.part`
/// (not send everything again); otherwise it may start over (a WebDAV
/// server without partial `PUT`). Either way the result must be exact.
pub fn upload_resume(vfs: &dyn Vfs, base: &VfsPath, size: u64, expect_resume: bool) -> ResumeReport {
    let dir = scratch(vfs, base, "resume");
    let name = "big.bin";
    let data = Generated::new(size, 23);
    let mut broken = BreakAfter::new(data.clone(), size / 2);
    let err = files::upload(vfs, &dir, name, &mut broken, Conflict::KeepBoth, &mut |_, _| true)
        .expect_err("the broken upload must fail");
    assert_ne!(err.kind(), ErrorKind::AlreadyExists, "{err}");
    assert!(!exists(vfs, &at(&dir, name)), "no final file after a break");
    let part = at(&dir, &format!("{name}{PART_SUFFIX}"));
    let part_after_break = vfs.stat(&part).map(|m| m.size).unwrap_or(0);
    assert!(part_after_break <= size / 2, "part {part_after_break} > {}", size / 2);

    // Bytes read on this attempt: progress starts at the resume offset
    // and goes back to 0 if the backend has to start over.
    let mut last: Option<u64> = None;
    let mut sent = 0u64;
    let mut fresh = data.clone();
    let out = files::upload(vfs, &dir, name, &mut fresh, Conflict::KeepBoth, &mut |done, _| {
        if let Some(l) = last {
            sent += if done >= l { done - l } else { done };
        }
        last = Some(done);
        true
    })
    .unwrap_or_else(|e| panic!("resumed upload: {e}"));
    assert_eq!(out.path, at(&dir, name), "same name after the resume");
    if expect_resume {
        assert!(part_after_break > 0, "something was written before the break");
        assert!(
            sent <= size - part_after_break,
            "resumed upload sent {sent} bytes, {part_after_break} were already there"
        );
    }
    assert!(!exists(vfs, &part), ".part renamed");
    assert_same(vfs, &out.path, &data);
    let report = ResumeReport {
        part_after_break,
        sent_on_resume: sent,
    };
    files::delete_permanently(vfs, &dir).expect("clean up");
    report
}

/// A cancelled upload stops with `Cancelled` and resumes later.
pub fn upload_cancel(vfs: &dyn Vfs, base: &VfsPath) {
    let dir = scratch(vfs, base, "cancel");
    let data = Generated::new(3 << 20, 5);
    let mut d = data.clone();
    let err = files::upload(vfs, &dir, "c.bin", &mut d, Conflict::KeepBoth, &mut |done, _| {
        done < (1 << 20)
    })
    .expect_err("cancelled");
    assert_eq!(err.kind(), ErrorKind::Cancelled, "{err}");
    assert!(!exists(vfs, &at(&dir, "c.bin")));
    let mut d = data.clone();
    let out = files::upload(vfs, &dir, "c.bin", &mut d, Conflict::KeepBoth, &mut |_, _| true)
        .expect("upload after cancel");
    assert_same(vfs, &out.path, &data);
}

/// Rename and move inside the source, with conflicts and guards.
pub fn move_and_rename(vfs: &dyn Vfs, base: &VfsPath) {
    let dir = scratch(vfs, base, "move");
    let a = files::create_folder(vfs, &dir, "A", Conflict::KeepBoth).expect("A").path;
    let b = files::create_folder(vfs, &dir, "B", Conflict::KeepBoth).expect("B").path;
    let f = put(vfs, &a, "film.mkv", b"film", Conflict::KeepBoth).path;
    put(vfs, &a, "inner.txt", b"inner", Conflict::KeepBoth);

    // Rename a file, then a case-only rename.
    let r = files::rename(vfs, &f, "Фильм.mkv", Conflict::KeepBoth).expect("rename");
    assert_eq!(r.path, at(&a, "Фильм.mkv"));
    assert!(!exists(vfs, &f));
    let r = files::rename(vfs, &r.path, "фильм.mkv", Conflict::KeepBoth).expect("case rename");
    assert_eq!(r.path, at(&a, "фильм.mkv"));
    assert_eq!(read_all(vfs, &r.path), b"film");
    let film = r.path;

    // File into another folder.
    let m = files::move_within(vfs, &film, &b, None, Conflict::KeepBoth).expect("move file");
    assert_eq!(m.path, at(&b, "фильм.mkv"));
    assert!(m.touched.iter().any(|t| t.dir == a && !t.recursive), "{:?}", m.touched);
    assert!(m.touched.iter().any(|t| t.dir == b && !t.recursive), "{:?}", m.touched);
    assert_eq!(read_all(vfs, &m.path), b"film");

    // Conflict on move: keep both, skip, replace.
    put(vfs, &a, "фильм.mkv", b"other", Conflict::KeepBoth);
    let k = files::move_within(vfs, &at(&a, "фильм.mkv"), &b, None, Conflict::KeepBoth)
        .expect("keep both");
    assert_eq!(k.path, at(&b, "фильм (2).mkv"));
    put(vfs, &a, "фильм.mkv", b"third", Conflict::KeepBoth);
    let s = files::move_within(vfs, &at(&a, "фильм.mkv"), &b, None, Conflict::Skip).expect("skip");
    assert!(s.skipped);
    assert!(exists(vfs, &at(&a, "фильм.mkv")));
    let p = files::move_within(vfs, &at(&a, "фильм.mkv"), &b, None, Conflict::Replace)
        .expect("replace");
    assert_eq!(read_all(vfs, &p.path), b"third");

    // A folder with its content into another folder.
    let mf = files::move_within(vfs, &a, &b, None, Conflict::KeepBoth).expect("move folder");
    assert_eq!(mf.path, at(&b, "A"));
    assert!(mf.touched.iter().any(|t| t.dir == mf.path && t.recursive), "{:?}", mf.touched);
    assert_eq!(read_all(vfs, &at(&mf.path, "inner.txt")), b"inner");
    assert!(!exists(vfs, &a));

    // Guards.
    let into_self = files::move_within(vfs, &b, &mf.path, None, Conflict::KeepBoth)
        .expect_err("into itself");
    assert_eq!(into_self.kind(), ErrorKind::InvalidPath, "{into_self}");
    for bad in ["a/b", "..", "a\\b", ""] {
        let e = files::rename(vfs, &b, bad, Conflict::KeepBoth).expect_err(bad);
        assert_eq!(e.kind(), ErrorKind::InvalidPath, "{bad:?}: {e}");
    }
    let root = files::move_within(vfs, &VfsPath::root(), &b, None, Conflict::KeepBoth)
        .expect_err("root");
    assert_eq!(root.kind(), ErrorKind::InvalidPath);
    let into_trash = files::move_within(
        vfs,
        &b,
        &VfsPath::parse("/.kinolune-trash").expect("path"),
        None,
        Conflict::KeepBoth,
    )
    .expect_err("into the trash");
    assert_eq!(into_trash.kind(), ErrorKind::InvalidPath);
}

/// Copy of a file and of a folder tree inside the source.
pub fn copy(vfs: &dyn Vfs, base: &VfsPath) {
    let dir = scratch(vfs, base, "copy");
    let src = files::create_folder(vfs, &dir, "src", Conflict::KeepBoth).expect("src").path;
    let sub = files::create_folder(vfs, &src, "Season 1", Conflict::KeepBoth).expect("sub").path;
    let data = Generated::new(1_500_001, 9);
    let mut d = data.clone();
    files::upload(vfs, &sub, "e01.mkv", &mut d, Conflict::KeepBoth, &mut |_, _| true)
        .expect("upload");
    put(vfs, &src, "a.nfo", b"nfo", Conflict::KeepBoth);
    let dst = files::create_folder(vfs, &dir, "dst", Conflict::KeepBoth).expect("dst").path;

    let c = files::copy_within(vfs, &at(&src, "a.nfo"), &dst, Conflict::KeepBoth, &mut |_, _| true)
        .expect("copy file");
    assert_eq!(read_all(vfs, &c.path), b"nfo");
    assert_eq!(read_all(vfs, &at(&src, "a.nfo")), b"nfo", "original kept");
    let c2 = files::copy_within(vfs, &at(&src, "a.nfo"), &dst, Conflict::KeepBoth, &mut |_, _| true)
        .expect("copy file again");
    assert_eq!(c2.path, at(&dst, "a (2).nfo"));

    let t = files::copy_within(vfs, &src, &dst, Conflict::KeepBoth, &mut |_, _| true)
        .expect("copy tree");
    assert_eq!(t.path, at(&dst, "src"));
    assert_same(vfs, &at(&at(&t.path, "Season 1"), "e01.mkv"), &data);
    assert_same(vfs, &at(&sub, "e01.mkv"), &data);
    let e = files::copy_within(vfs, &src, &sub, Conflict::KeepBoth, &mut |_, _| true)
        .expect_err("into itself");
    assert_eq!(e.kind(), ErrorKind::InvalidPath);
}

/// Trash: files and folders go in, are listed with their origin, come
/// back (also into a folder that was removed meanwhile), and old items are
/// purged.
pub fn trash(vfs: &dyn Vfs, base: &VfsPath) {
    let dir = scratch(vfs, base, "trash");
    let f = put(vfs, &dir, "home video.mp4", b"video", Conflict::KeepBoth).path;
    let folder = files::create_folder(vfs, &dir, "Отпуск", Conflict::KeepBoth).expect("dir").path;
    put(vfs, &folder, "1.mp4", b"one", Conflict::KeepBoth);

    let t = files::trash(vfs, &f).expect("trash file");
    assert!(!exists(vfs, &f));
    assert!(t.touched.iter().any(|x| x.dir == dir), "{:?}", t.touched);
    files::trash(vfs, &folder).expect("trash folder");
    assert!(!exists(vfs, &folder));
    assert!(!names(vfs, &dir).iter().any(|n| n.starts_with('.')), "nothing else in the folder");

    let items: Vec<_> = files::list_trash(vfs)
        .expect("list")
        .into_iter()
        .filter(|i| i.original == f || i.original == folder)
        .collect();
    assert_eq!(items.len(), 2, "{items:?}");
    let file_item = items.iter().find(|i| i.original == f).expect("file item");
    assert!(file_item.metadata.is_file());
    assert!(items.iter().find(|i| i.original == folder).expect("dir item").metadata.is_dir());
    let age = files::now_ms() - file_item.deleted_ms;
    assert!((0..600_000).contains(&age), "deleted_ms {}", file_item.deleted_ms);
    // The trash cannot be trashed or moved.
    let e = files::trash(vfs, &file_item.path).expect_err("trash in trash");
    assert_eq!(e.kind(), ErrorKind::InvalidPath);

    // Restore the file next to a new file of the same name: keep both.
    put(vfs, &dir, "home video.mp4", b"new", Conflict::KeepBoth);
    let r = files::restore(vfs, &file_item.id, Conflict::KeepBoth).expect("restore");
    assert_eq!(r.path, at(&dir, "home video (2).mp4"));
    assert_eq!(read_all(vfs, &r.path), b"video");
    assert!(
        !files::list_trash(vfs).expect("list").iter().any(|i| i.id == file_item.id),
        "restored item left the trash"
    );

    // Restore a folder whose parent is gone: the parent is re-created.
    let dir_item = items.into_iter().find(|i| i.original == folder).expect("dir item");
    files::delete_permanently(vfs, &dir).expect("drop parent");
    let r = files::restore(vfs, &dir_item.id, Conflict::KeepBoth).expect("restore folder");
    assert_eq!(r.path, folder);
    assert_eq!(read_all(vfs, &at(&folder, "1.mp4")), b"one");

    // Purge: nothing is old enough now; 31 days later everything is.
    let t2 = files::trash(vfs, &folder).expect("trash again");
    assert_eq!(files::purge_trash(vfs, files::now_ms(), TRASH_KEEP).expect("purge now"), 0);
    let later = files::now_ms() + i64::try_from(TRASH_KEEP.as_millis()).expect("ms") + 86_400_000;
    assert!(files::purge_trash(vfs, later, TRASH_KEEP).expect("purge later") >= 1);
    assert!(!exists(vfs, &t2.path), "purged");
    files::empty_trash(vfs).expect("empty");
    assert!(files::list_trash(vfs).expect("list").is_empty());
}

/// Permanent delete of a file and a tree; the root and the trash itself
/// are refused.
pub fn delete(vfs: &dyn Vfs, base: &VfsPath) {
    let dir = scratch(vfs, base, "delete");
    let f = put(vfs, &dir, "x.bin", b"x", Conflict::KeepBoth).path;
    let tree = files::create_folder(vfs, &dir, "tree", Conflict::KeepBoth).expect("tree").path;
    let deep = files::create_folder(vfs, &tree, "deep", Conflict::KeepBoth).expect("deep").path;
    put(vfs, &deep, "y.bin", b"y", Conflict::KeepBoth);
    files::delete_permanently(vfs, &f).expect("delete file");
    files::delete_permanently(vfs, &tree).expect("delete tree");
    assert!(names(vfs, &dir).is_empty(), "{:?}", names(vfs, &dir));
    assert_eq!(
        files::delete_permanently(vfs, &VfsPath::root()).expect_err("root").kind(),
        ErrorKind::InvalidPath
    );
    assert_eq!(
        files::delete_permanently(vfs, &VfsPath::parse("/.kinolune-trash").expect("p"))
            .expect_err("trash root")
            .kind(),
        ErrorKind::InvalidPath
    );
    let missing = files::delete_permanently(vfs, &at(&dir, "missing")).expect_err("missing");
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");
}

/// Move and copy from `src` to another source `dst`.
pub fn between(src: &dyn Vfs, src_base: &VfsPath, dst: &dyn Vfs, dst_base: &VfsPath) {
    let from_dir = scratch(src, src_base, "between-src");
    let to_dir = scratch(dst, dst_base, "between-dst");
    let data = Generated::new(2_000_003, 77);
    let mut d = data.clone();
    let f = files::upload(src, &from_dir, "move me.mkv", &mut d, Conflict::KeepBoth, &mut |_, _| true)
        .expect("upload")
        .path;
    let folder = files::create_folder(src, &from_dir, "Show", Conflict::KeepBoth).expect("dir").path;
    put(src, &folder, "s01e01.mkv", b"episode", Conflict::KeepBoth);

    let c = files::copy_between(src, &f, dst, &to_dir, Conflict::KeepBoth, &mut |_, _| true)
        .expect("copy between");
    assert_same(dst, &c.path, &data);
    assert!(exists(src, &f), "copy keeps the original");

    let m = files::move_between(src, &f, dst, &to_dir, Conflict::KeepBoth, &mut |_, _| true)
        .expect("move between");
    assert_eq!(m.path, at(&to_dir, "move me (2).mkv"));
    assert_same(dst, &m.path, &data);
    assert!(!exists(src, &f), "move removes the original");
    assert!(m.touched_origin.iter().any(|t| t.dir == from_dir), "{:?}", m.touched_origin);
    assert!(m.touched.iter().any(|t| t.dir == to_dir), "{:?}", m.touched);

    let mf = files::move_between(src, &folder, dst, &to_dir, Conflict::KeepBoth, &mut |_, _| true)
        .expect("move folder between");
    assert_eq!(read_all(dst, &at(&mf.path, "s01e01.mkv")), b"episode");
    assert!(!exists(src, &folder));
}

/// A source that must not be written: writability says so and writes fail
/// without leaving anything.
pub fn read_only(vfs: &dyn Vfs, dir: &VfsPath) {
    assert_eq!(files::writability(vfs).map_err(|e| e.to_string()), Ok(false));
    let before = names(vfs, dir);
    let mut d = io::Cursor::new(b"nope".to_vec());
    let e = files::upload(vfs, dir, "nope.bin", &mut d, Conflict::KeepBoth, &mut |_, _| true)
        .expect_err("upload to a read-only source");
    assert!(
        matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::Unsupported | ErrorKind::NotFound),
        "{e}"
    );
    let e = files::create_folder(vfs, dir, "nope", Conflict::KeepBoth).expect_err("mkdir");
    assert!(
        matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::Unsupported | ErrorKind::NotFound),
        "{e}"
    );
    assert_eq!(names(vfs, dir), before, "nothing was created");
}

/// One write case: a writable source and a scratch base folder in it.
pub type Case = fn(&dyn Vfs, &VfsPath);

/// Every single-source case, by name (not the 1 GiB resume, not
/// [`between`] or [`read_only`], which need other inputs).
pub const CASES: &[(&str, Case)] = &[
    ("writable", writable),
    ("folders", folders),
    ("upload_conflicts", upload_conflicts),
    ("upload_cancel", upload_cancel),
    ("move_and_rename", move_and_rename),
    ("copy", copy),
    ("trash", trash),
    ("delete", delete),
];

/// Runs [`CASES`] and a 5 MiB [`upload_resume`].
pub fn run_all(vfs: &dyn Vfs, base: &VfsPath, expect_resume: bool) {
    for (name, case) in CASES {
        eprintln!("write contract: {name}");
        case(vfs, base);
    }
    eprintln!("write contract: upload_resume");
    upload_resume(vfs, base, 5 * 1024 * 1024 + 7, expect_resume);
}
