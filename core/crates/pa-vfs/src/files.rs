// SPDX-License-Identifier: MPL-2.0
//! File management in sources (NET-023, ADR-0104): folders, uploads with
//! resume, rename, move and copy (inside a source and between sources),
//! the source trash and name conflicts.
//!
//! Everything here is built on the primitive [`VfsWrite`] calls, so the
//! rules are the same for every protocol:
//!
//! - **Read-only is found, not configured.** [`writability`] asks the
//!   backend ([`Vfs::as_write`]) and then tries to create and remove a
//!   probe folder: a read-only share answers "no" and the UI hides writes.
//! - **Deleting goes to the trash by default.** [`trash`] moves the entry
//!   into `/.kinolune-trash/<id>/` of the same source, next to a
//!   `.kinolune-origin` file with the original path; [`restore`] puts it
//!   back, [`purge_trash`] removes items older than [`TRASH_KEEP`]
//!   (30 days). [`delete_permanently`] is the separate, explicit call.
//! - **Nothing leaves the source root.** Paths are [`VfsPath`]s (no `..`),
//!   new names are single components without `\` or control characters,
//!   the root and the trash folder itself cannot be moved or deleted, a
//!   folder cannot go into itself, and a recursive delete removes a
//!   symbolic link instead of entering it.
//! - **Uploads resume.** [`upload`] writes `name.part` and renames it at
//!   the end; after a break it continues from the length of `name.part`
//!   (after checking that its last bytes match the data).
//! - **Moves keep data.** A move inside a source is one server-side
//!   rename where the protocol can; otherwise, and between sources, it is
//!   copy, check the size, then delete the original.
//! - **Name conflicts** follow [`Conflict`]: keep both ("name (2)"),
//!   replace (the old entry goes to the trash) or skip.
//!
//! Every operation returns the folders it changed ([`Outcome::touched`]),
//! so the library re-lists only them instead of rescanning the source.

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{ErrorKind, Result, VfsError};
use crate::path::VfsPath;
use crate::source::{ByteSource, SourceReader};
use crate::vfs::{EntryKind, Metadata, Vfs};
use crate::write::VfsWrite;

/// The trash folder at the source root.
pub const TRASH_DIR: &str = ".kinolune-trash";
/// The file in a trash item that holds the original path.
pub const TRASH_ORIGIN: &str = ".kinolune-origin";
/// How long trashed entries are kept by [`purge_trash`].
pub const TRASH_KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Suffix of an upload in progress.
pub const PART_SUFFIX: &str = ".part";

/// Folder [`writability`] creates and removes at the root.
const PROBE_DIR: &str = ".kinolune-write-probe";
/// Bytes compared at the end of a `.part` file before resuming it.
const RESUME_CHECK: u64 = 64 * 1024;
/// Deepest folder level a recursive copy or delete enters (link cycles).
const MAX_DEPTH: usize = 64;

/// What to do when the target name is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Conflict {
    /// Keep both: the new entry gets a free name, `name (2).ext`.
    #[default]
    KeepBoth,
    /// Replace: the existing entry goes to the trash first. For
    /// [`create_folder`] an existing folder is kept as is.
    Replace,
    /// Skip: nothing is written ([`Outcome::skipped`]).
    Skip,
}

/// A folder whose listing changed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Touched {
    /// The folder.
    pub dir: VfsPath,
    /// Everything below it changed too (a folder was copied or moved in);
    /// otherwise only its direct entries changed.
    pub recursive: bool,
}

impl Touched {
    fn shallow(dir: VfsPath) -> Self {
        Self {
            dir,
            recursive: false,
        }
    }

    fn deep(dir: VfsPath) -> Self {
        Self {
            dir,
            recursive: true,
        }
    }
}

/// Result of a write operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Where the entry is now (for [`Conflict::Skip`]: the existing entry
    /// that blocked it).
    pub path: VfsPath,
    /// Nothing was written because of [`Conflict::Skip`] (or an existing
    /// folder in [`create_folder`]).
    pub skipped: bool,
    /// Folders of the (destination) source whose listing changed.
    pub touched: Vec<Touched>,
    /// Folders of the origin source that changed, for operations between
    /// two sources ([`move_between`]); empty otherwise.
    pub touched_origin: Vec<Touched>,
}

impl Outcome {
    fn done(path: VfsPath, touched: Vec<Touched>) -> Self {
        Self {
            path,
            skipped: false,
            touched: dedup(touched),
            touched_origin: Vec::new(),
        }
    }

    fn skipped(path: VfsPath) -> Self {
        Self {
            path,
            skipped: true,
            touched: Vec::new(),
            touched_origin: Vec::new(),
        }
    }
}

fn dedup(mut v: Vec<Touched>) -> Vec<Touched> {
    let mut out: Vec<Touched> = Vec::with_capacity(v.len());
    for t in v.drain(..) {
        if let Some(o) = out.iter_mut().find(|o| o.dir == t.dir) {
            o.recursive |= t.recursive;
        } else {
            out.push(t);
        }
    }
    out
}

/// Progress of a transfer: `(bytes done, bytes total)` of the current
/// file. Return `false` to cancel; the operation then fails with
/// [`ErrorKind::Cancelled`] and an upload can be resumed later.
pub type OnProgress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

/// Data to upload: anything readable and seekable (a local file, a
/// [`SourceReader`]). Seeking finds the size and the resume offset.
pub trait UploadData: Read + Seek {}

impl<T: Read + Seek + ?Sized> UploadData for T {}

/// Whether this source can be written: the backend writes and the share
/// lets us (a probe folder at the root is created and removed).
///
/// `Ok(false)` for a backend without writes and for a read-only share or
/// account. Errors only when the source cannot be reached or the login
/// fails, so the answer is unknown.
pub fn writability(vfs: &dyn Vfs) -> Result<bool> {
    let Some(w) = vfs.as_write() else {
        return Ok(false);
    };
    let probe = VfsPath::root().join(PROBE_DIR)?;
    let unknown =
        |e: &VfsError| e.kind().is_transient() || e.kind() == ErrorKind::AuthenticationFailed;
    match w.create_dir(&probe) {
        Ok(()) => {
            // Leaving the probe behind is harmless; the next probe removes it.
            let _ = w.remove_dir(&probe);
            Ok(true)
        }
        Err(e) if unknown(&e) => Err(e),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => match w.remove_dir(&probe) {
            Ok(()) => Ok(true),
            Err(e) if unknown(&e) => Err(e),
            Err(_) => Ok(false),
        },
        Err(_) => Ok(false),
    }
}

/// The write side of `vfs`, or [`ErrorKind::PermissionDenied`] for a
/// backend that only reads.
pub fn writer(vfs: &dyn Vfs) -> Result<&dyn VfsWrite> {
    vfs.as_write().ok_or_else(|| {
        VfsError::new(
            ErrorKind::PermissionDenied,
            format!("{vfs:?}: this source is read-only"),
        )
    })
}

/// Creates the folder `name` in `parent`.
///
/// An existing folder of that name is kept and returned as
/// [`skipped`](Outcome::skipped), except with [`Conflict::KeepBoth`],
/// which creates `name (2)`.
pub fn create_folder(
    vfs: &dyn Vfs,
    parent: &VfsPath,
    name: &str,
    conflict: Conflict,
) -> Result<Outcome> {
    let w = writer(vfs)?;
    check_new_name(name)?;
    check_writable_dir(parent)?;
    let path = parent.join(name)?;
    let path = match stat_opt(vfs, &path)? {
        None => path,
        Some(_) if conflict == Conflict::KeepBoth => {
            parent.join(&free_name(vfs, parent, name, true)?)?
        }
        Some(_) => return Ok(Outcome::skipped(path)),
    };
    w.create_dir(&path)?;
    Ok(Outcome::done(path, vec![Touched::shallow(parent.clone())]))
}

/// Uploads `data` as the file `name` in `dir`, resuming an earlier
/// attempt.
///
/// The bytes go to `name.part`, which is renamed to `name` once its size
/// matches. If `name.part` is already there (a broken upload) and its
/// last bytes match `data` at the same offset, writing continues at its
/// end; otherwise it starts again. A backend that cannot append starts
/// again as well. `on_progress` sees the whole file, including the part
/// already uploaded.
pub fn upload(
    vfs: &dyn Vfs,
    dir: &VfsPath,
    name: &str,
    data: &mut dyn UploadData,
    conflict: Conflict,
    on_progress: OnProgress<'_>,
) -> Result<Outcome> {
    let w = writer(vfs)?;
    check_new_name(name)?;
    check_writable_dir(dir)?;
    let Some((target, replace)) = resolve_target(vfs, dir, name, false, conflict)? else {
        return Ok(Outcome::skipped(dir.join(name)?));
    };
    let total = data
        .seek(SeekFrom::End(0))
        .map_err(|e| VfsError::from_io(e, "upload data"))?;
    let part = part_of(&target)?;
    write_part(vfs, w, &part, data, total, on_progress)?;
    finish_part(vfs, w, &part, &target, replace)?;
    Ok(Outcome::done(target, vec![Touched::shallow(dir.clone())]))
}

/// Renames `path` inside its folder.
pub fn rename(
    vfs: &dyn Vfs,
    path: &VfsPath,
    new_name: &str,
    conflict: Conflict,
) -> Result<Outcome> {
    check_new_name(new_name)?;
    let parent = path.parent().ok_or_else(|| {
        VfsError::new(ErrorKind::InvalidPath, "the source root cannot be renamed")
    })?;
    move_within(vfs, path, &parent, Some(new_name), conflict)
}

/// Moves `from` into the folder `to_dir` of the same source, optionally
/// under a new name.
///
/// One server-side rename where the protocol allows it; otherwise copy,
/// check sizes, then delete the original.
pub fn move_within(
    vfs: &dyn Vfs,
    from: &VfsPath,
    to_dir: &VfsPath,
    new_name: Option<&str>,
    conflict: Conflict,
) -> Result<Outcome> {
    check_movable(from)?;
    check_writable_dir(to_dir)?;
    if let Some(n) = new_name {
        check_new_name(n)?;
    }
    move_unchecked(vfs, from, to_dir, new_name, conflict, &mut |_, _| true)
}

/// Copies `from` into the folder `to_dir` of the same source: server-side
/// where the protocol can, otherwise through this device.
pub fn copy_within(
    vfs: &dyn Vfs,
    from: &VfsPath,
    to_dir: &VfsPath,
    conflict: Conflict,
    on_progress: OnProgress<'_>,
) -> Result<Outcome> {
    let w = writer(vfs)?;
    check_movable(from)?;
    check_writable_dir(to_dir)?;
    if is_under(to_dir, from) {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{from}: a folder cannot be copied into itself"),
        ));
    }
    let meta = vfs.stat(from)?;
    let name = file_name(from)?;
    let Some((target, replace)) = resolve_target(vfs, to_dir, name, meta.is_dir(), conflict)?
    else {
        return Ok(Outcome::skipped(to_dir.join(name)?));
    };
    if replace {
        trash(vfs, &target)?;
    }
    match w.server_copy(from, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            copy_tree(vfs, from, vfs, w, &target, 0, on_progress)?;
        }
        Err(e) => return Err(e),
    }
    Ok(Outcome::done(
        target.clone(),
        vec![landed(to_dir, &target, &meta)],
    ))
}

/// Copies `from` of `src` into the folder `to_dir` of another source `dst`.
pub fn copy_between(
    src: &dyn Vfs,
    from: &VfsPath,
    dst: &dyn Vfs,
    to_dir: &VfsPath,
    conflict: Conflict,
    on_progress: OnProgress<'_>,
) -> Result<Outcome> {
    let wd = writer(dst)?;
    check_movable(from)?;
    check_writable_dir(to_dir)?;
    let meta = src.stat(from)?;
    let name = file_name(from)?;
    let Some((target, replace)) = resolve_target(dst, to_dir, name, meta.is_dir(), conflict)?
    else {
        return Ok(Outcome::skipped(to_dir.join(name)?));
    };
    if replace {
        trash(dst, &target)?;
    }
    copy_tree(src, from, dst, wd, &target, 0, on_progress)?;
    Ok(Outcome::done(
        target.clone(),
        vec![landed(to_dir, &target, &meta)],
    ))
}

/// Moves `from` of `src` into the folder `to_dir` of another source `dst`:
/// copy (with `.part` files and size checks), then delete the original.
///
/// The original is deleted only after everything was copied and checked;
/// a failure on the way leaves it untouched.
pub fn move_between(
    src: &dyn Vfs,
    from: &VfsPath,
    dst: &dyn Vfs,
    to_dir: &VfsPath,
    conflict: Conflict,
    on_progress: OnProgress<'_>,
) -> Result<Outcome> {
    // Both sides must write: the copy lands in `dst`, the original goes.
    let ws = writer(src)?;
    let mut out = copy_between(src, from, dst, to_dir, conflict, on_progress)?;
    if out.skipped {
        return Ok(out);
    }
    delete_tree(src, ws, from, 0)?;
    out.touched_origin = vec![Touched::shallow(parent_of(from))];
    Ok(out)
}

/// Moves `path` to the trash of its source and returns where it is now.
pub fn trash(vfs: &dyn Vfs, path: &VfsPath) -> Result<Outcome> {
    let w = writer(vfs)?;
    check_movable(path)?;
    let name = file_name(path)?;
    if name == TRASH_ORIGIN {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{path}: this name is reserved by the trash"),
        ));
    }
    vfs.stat(path)?;
    let root = trash_root();
    ensure_dir(vfs, w, &root)?;
    let item = new_trash_item(w, &root)?;
    let origin = item.join(TRASH_ORIGIN)?;
    let dest = item.join(name)?;
    let moved = write_bytes(w, &origin, path.as_str().as_bytes()).and_then(|()| {
        match w.rename(path, &dest) {
            Err(e) if e.kind() == ErrorKind::Unsupported => {
                copy_tree(vfs, path, vfs, w, &dest, 0, &mut |_, _| true)?;
                delete_tree(vfs, w, path, 0)
            }
            r => r,
        }
    });
    if let Err(e) = moved {
        // The entry is still where it was; drop the empty item.
        let _ = delete_tree(vfs, w, &item, 0);
        return Err(e);
    }
    Ok(Outcome::done(dest, vec![Touched::shallow(parent_of(path))]))
}

/// Deletes `path` for good (a file, or a folder with everything in it).
///
/// Symbolic links are removed, never followed. Inside the trash this
/// removes one item (`/.kinolune-trash/<id>`).
pub fn delete_permanently(vfs: &dyn Vfs, path: &VfsPath) -> Result<Outcome> {
    let w = writer(vfs)?;
    if path.is_root() || *path == trash_root() {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{path}: cannot be deleted"),
        ));
    }
    delete_tree(vfs, w, path, 0)?;
    let touched = if in_trash(path) {
        Vec::new()
    } else {
        vec![Touched::shallow(parent_of(path))]
    };
    Ok(Outcome::done(path.clone(), touched))
}

/// One entry in the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashItem {
    /// Item id: the name of its folder in the trash.
    pub id: String,
    /// Where the entry was before it was trashed.
    pub original: VfsPath,
    /// Where it is now, inside the trash.
    pub path: VfsPath,
    /// When it was trashed, Unix milliseconds.
    pub deleted_ms: i64,
    /// File or folder, and its size.
    pub metadata: Metadata,
}

/// The entries in the trash of `vfs`, oldest first.
pub fn list_trash(vfs: &dyn Vfs) -> Result<Vec<TrashItem>> {
    let mut out = Vec::new();
    for item in trash_items(vfs)? {
        let Some(deleted_ms) = item_time(&item.name) else {
            continue;
        };
        let entries = match vfs.list(&item.path) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let Some(entry) = entries.into_iter().find(|e| e.name != TRASH_ORIGIN) else {
            continue;
        };
        let original = read_origin(vfs, &item.path.join(TRASH_ORIGIN)?)
            .unwrap_or_else(|| VfsPath::root().join(&entry.name).unwrap_or_default());
        out.push(TrashItem {
            id: item.name,
            original,
            path: entry.path,
            deleted_ms,
            metadata: entry.metadata,
        });
    }
    out.sort_by(|a, b| (a.deleted_ms, &a.id).cmp(&(b.deleted_ms, &b.id)));
    Ok(out)
}

/// Puts a trashed entry back where it was (re-creating missing folders).
pub fn restore(vfs: &dyn Vfs, id: &str, conflict: Conflict) -> Result<Outcome> {
    let w = writer(vfs)?;
    let item = list_trash(vfs)?
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| VfsError::new(ErrorKind::NotFound, format!("trash item {id:?}")))?;
    if in_trash(&item.original) || item.original.is_root() {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("trash item {id:?}: bad original path"),
        ));
    }
    let to_dir = parent_of(&item.original);
    let name = file_name(&item.original)?.to_owned();
    ensure_dirs(vfs, w, &to_dir)?;
    let out = move_unchecked(
        vfs,
        &item.path,
        &to_dir,
        Some(&name),
        conflict,
        &mut |_, _| true,
    )?;
    if !out.skipped {
        let _ = delete_tree(vfs, w, &trash_root().join(id)?, 0);
    }
    Ok(out)
}

/// Deletes trash items older than `keep` (normally [`TRASH_KEEP`]) at
/// `now_ms`; returns how many. Also clears leftovers of an interrupted
/// trash operation once they are that old.
pub fn purge_trash(vfs: &dyn Vfs, now_ms: i64, keep: Duration) -> Result<usize> {
    let w = writer(vfs)?;
    let keep_ms = i64::try_from(keep.as_millis()).unwrap_or(i64::MAX);
    let mut n = 0;
    for item in trash_items(vfs)? {
        let Some(at) = item_time(&item.name) else {
            continue;
        };
        if at.saturating_add(keep_ms) <= now_ms {
            delete_tree(vfs, w, &item.path, 0)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Deletes everything in the trash; returns how many items.
pub fn empty_trash(vfs: &dyn Vfs) -> Result<usize> {
    let w = writer(vfs)?;
    let items = trash_items(vfs)?;
    for item in &items {
        delete_tree(vfs, w, &item.path, 0)?;
    }
    Ok(items.len())
}

/// Current time, Unix milliseconds.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

// ---------------------------------------------------------------------------
// Internals

fn trash_root() -> VfsPath {
    VfsPath::root().join(TRASH_DIR).unwrap_or_default()
}

/// `true` for the trash folder and anything in it.
fn in_trash(path: &VfsPath) -> bool {
    path.components().next() == Some(TRASH_DIR)
}

/// `true` if `path` is `dir` or below it.
fn is_under(path: &VfsPath, dir: &VfsPath) -> bool {
    let mut p = path.components();
    dir.components().all(|c| p.next() == Some(c))
}

fn parent_of(path: &VfsPath) -> VfsPath {
    path.parent().unwrap_or_default()
}

fn file_name(path: &VfsPath) -> Result<&str> {
    path.file_name()
        .ok_or_else(|| VfsError::new(ErrorKind::InvalidPath, "the source root has no name"))
}

/// A name for a new entry: one component, no `\` (a separator on SMB and
/// Windows) and no control characters.
fn check_new_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
        || name.trim().is_empty();
    if bad {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{name:?}: not a valid name"),
        ));
    }
    Ok(())
}

/// An entry that may be moved, copied or trashed: not the root, not the
/// trash or anything in it (use [`restore`] and [`delete_permanently`]).
fn check_movable(path: &VfsPath) -> Result<()> {
    if path.is_root() || in_trash(path) {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{path}: cannot be moved, copied or trashed"),
        ));
    }
    Ok(())
}

/// A folder that may receive entries: anything outside the trash.
fn check_writable_dir(dir: &VfsPath) -> Result<()> {
    if in_trash(dir) {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{dir}: nothing can be put into the trash this way"),
        ));
    }
    Ok(())
}

fn stat_opt(vfs: &dyn Vfs, path: &VfsPath) -> Result<Option<Metadata>> {
    match vfs.stat(path) {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The target path for `name` in `dir` under `conflict`: `None` to skip,
/// otherwise the path and whether an existing entry must be replaced.
fn resolve_target(
    vfs: &dyn Vfs,
    dir: &VfsPath,
    name: &str,
    is_dir: bool,
    conflict: Conflict,
) -> Result<Option<(VfsPath, bool)>> {
    let target = dir.join(name)?;
    match stat_opt(vfs, &target)? {
        None => Ok(Some((target, false))),
        Some(_) => match conflict {
            Conflict::Skip => Ok(None),
            Conflict::Replace => Ok(Some((target, true))),
            Conflict::KeepBoth => Ok(Some((
                dir.join(&free_name(vfs, dir, name, is_dir)?)?,
                false,
            ))),
        },
    }
}

/// `name (2).ext`, `name (3).ext`, … : the first that does not exist
/// (and whose `.part` does not either, so a parallel upload keeps its own).
fn free_name(vfs: &dyn Vfs, dir: &VfsPath, name: &str, is_dir: bool) -> Result<String> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if !is_dir && i > 0 => name.split_at(i),
        _ => (name, ""),
    };
    for n in 2..10_000 {
        let candidate = format!("{stem} ({n}){ext}");
        let path = dir.join(&candidate)?;
        if stat_opt(vfs, &path)?.is_none() && stat_opt(vfs, &part_of(&path)?)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(VfsError::new(
        ErrorKind::AlreadyExists,
        format!("{dir}: no free name for {name:?}"),
    ))
}

fn part_of(target: &VfsPath) -> Result<VfsPath> {
    let name = format!("{}{PART_SUFFIX}", file_name(target)?);
    parent_of(target).join(&name)
}

/// Where a copied or moved entry landed, for [`Outcome::touched`].
fn landed(to_dir: &VfsPath, target: &VfsPath, meta: &Metadata) -> Touched {
    if meta.is_dir() {
        Touched::deep(target.clone())
    } else {
        Touched::shallow(to_dir.clone())
    }
}

/// Creates `dir` if it is missing (its parent must exist).
fn ensure_dir(vfs: &dyn Vfs, w: &dyn VfsWrite, dir: &VfsPath) -> Result<()> {
    match w.create_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) => match stat_opt(vfs, dir)? {
            Some(m) if m.is_dir() => Ok(()),
            Some(_) => Err(VfsError::new(
                ErrorKind::NotADirectory,
                format!("{dir}: a file is in the way"),
            )),
            None => Err(e),
        },
    }
}

/// `mkdir -p`.
fn ensure_dirs(vfs: &dyn Vfs, w: &dyn VfsWrite, dir: &VfsPath) -> Result<()> {
    let mut at = VfsPath::root();
    for c in dir.components() {
        at = at.join(c)?;
        if stat_opt(vfs, &at)?.is_none_or(|m| !m.is_dir()) {
            ensure_dir(vfs, w, &at)?;
        }
    }
    Ok(())
}

fn write_bytes(w: &dyn VfsWrite, path: &VfsPath, bytes: &[u8]) -> Result<()> {
    let mut r = bytes;
    let n = w.write_file(path, 0, &mut r, bytes.len() as u64)?;
    if n != bytes.len() as u64 {
        return Err(VfsError::new(
            ErrorKind::Other,
            format!("{path}: short write ({n} of {} bytes)", bytes.len()),
        ));
    }
    Ok(())
}

/// Writes `data` (of `total` bytes) to `part`, resuming it if it holds a
/// prefix of `data`, and checks the final size.
fn write_part(
    vfs: &dyn Vfs,
    w: &dyn VfsWrite,
    part: &VfsPath,
    data: &mut dyn UploadData,
    total: u64,
    on_progress: OnProgress<'_>,
) -> Result<()> {
    let offset = match stat_opt(vfs, part)? {
        None => 0,
        Some(m) if m.is_file() => {
            if m.size > 0 && m.size <= total && tail_matches(vfs, part, m.size, data) {
                m.size
            } else {
                0
            }
        }
        Some(_) => {
            return Err(VfsError::new(
                ErrorKind::AlreadyExists,
                format!("{part}: a folder is in the way of the upload"),
            ));
        }
    };
    if offset < total || total == 0 {
        match send(w, part, offset, data, total, on_progress) {
            Err(e) if e.kind() == ErrorKind::Unsupported && offset > 0 => {
                send(w, part, 0, data, total, on_progress)?;
            }
            r => r?,
        }
    }
    let size = vfs.stat(part)?.size;
    if size != total {
        if size > total {
            // Not a prefix of this upload: start clean next time.
            let _ = w.remove_file(part);
        }
        return Err(VfsError::new(
            ErrorKind::Other,
            format!("{part}: {size} bytes on the source after the upload, expected {total}"),
        ));
    }
    Ok(())
}

/// Whether the last bytes of `part` (of `size` bytes) equal `data` there.
fn tail_matches(vfs: &dyn Vfs, part: &VfsPath, size: u64, data: &mut dyn UploadData) -> bool {
    let n = size.min(RESUME_CHECK);
    let at = size - n;
    let len = usize::try_from(n).unwrap_or(0);
    let mut theirs = vec![0u8; len];
    let mut ours = vec![0u8; len];
    let Ok(src) = vfs.open(part) else {
        return false;
    };
    if src.read_exact_at(at, &mut theirs).is_err() {
        return false;
    }
    if data.seek(SeekFrom::Start(at)).is_err() || data.read_exact(&mut ours).is_err() {
        return false;
    }
    theirs == ours
}

/// One `write_file` call from `offset`, with progress and cancellation.
fn send(
    w: &dyn VfsWrite,
    part: &VfsPath,
    offset: u64,
    data: &mut dyn UploadData,
    total: u64,
    on_progress: OnProgress<'_>,
) -> Result<()> {
    data.seek(SeekFrom::Start(offset))
        .map_err(|e| VfsError::from_io(e, "upload data"))?;
    let len = total - offset;
    let mut reader = Counting {
        inner: data,
        left: len,
        done: offset,
        total,
        on_progress,
        cancelled: false,
    };
    if !(reader.on_progress)(offset, total) {
        return Err(cancelled(part));
    }
    let r = w.write_file(part, offset, &mut reader, len);
    if reader.cancelled {
        return Err(cancelled(part));
    }
    let n = r?;
    if n != len {
        return Err(VfsError::new(
            ErrorKind::Other,
            format!("{part}: wrote {n} of {len} bytes"),
        ));
    }
    Ok(())
}

fn cancelled(path: &VfsPath) -> VfsError {
    VfsError::new(ErrorKind::Cancelled, format!("{path}: cancelled"))
}

/// Reads at most `left` bytes, reporting progress and stopping on cancel.
struct Counting<'a, 'p> {
    inner: &'a mut dyn UploadData,
    left: u64,
    done: u64,
    total: u64,
    on_progress: OnProgress<'p>,
    cancelled: bool,
}

impl Read for Counting<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let max = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let n = self.inner.read(&mut buf[..max])?;
        self.left -= n as u64;
        self.done += n as u64;
        if !(self.on_progress)(self.done, self.total) {
            self.cancelled = true;
            return Err(io::Error::other("cancelled"));
        }
        Ok(n)
    }
}

/// Renames the finished `part` to `target`, trashing what is there first
/// when replacing.
fn finish_part(
    vfs: &dyn Vfs,
    w: &dyn VfsWrite,
    part: &VfsPath,
    target: &VfsPath,
    replace: bool,
) -> Result<()> {
    if let Some(_existing) = stat_opt(vfs, target)? {
        if !replace {
            return Err(VfsError::new(
                ErrorKind::AlreadyExists,
                format!("{target}: appeared during the upload"),
            ));
        }
        trash(vfs, target)?;
    }
    match w.rename(part, target) {
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            let src = vfs.open(part)?;
            let size = src.size();
            let mut r = SourceReader::new(src);
            send(w, target, 0, &mut r, size, &mut |_, _| true)?;
            w.remove_file(part)
        }
        r => r,
    }
}

/// Move without the user-path checks (also used by [`restore`]).
fn move_unchecked(
    vfs: &dyn Vfs,
    from: &VfsPath,
    to_dir: &VfsPath,
    new_name: Option<&str>,
    conflict: Conflict,
    on_progress: OnProgress<'_>,
) -> Result<Outcome> {
    let w = writer(vfs)?;
    if is_under(to_dir, from) {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!("{from}: a folder cannot be moved into itself"),
        ));
    }
    let meta = vfs.stat(from)?;
    let name = match new_name {
        Some(n) => n,
        None => file_name(from)?,
    };
    let to_meta = vfs.stat(to_dir)?;
    if !to_meta.is_dir() {
        return Err(VfsError::new(ErrorKind::NotADirectory, to_dir.to_string()));
    }
    let same_dir = parent_of(from) == *to_dir;
    let old_name = file_name(from)?;
    if same_dir && old_name == name {
        return Ok(Outcome::skipped(from.clone()));
    }
    // A case-only rename: on case-insensitive shares (macOS, Windows,
    // many NAS) the "existing" target is the entry itself, and backends
    // refuse to rename onto an existing name. Go through a temporary name.
    if same_dir && old_name.to_lowercase() == name.to_lowercase() {
        let target = to_dir.join(name)?;
        let temp = to_dir.join(&format!(".kinolune-rename-{}", std::process::id()))?;
        w.rename(from, &temp)?;
        if let Err(e) = w.rename(&temp, &target) {
            // Put it back under its old name rather than lose it.
            let _ = w.rename(&temp, from);
            return Err(e);
        }
        return Ok(Outcome::done(
            target.clone(),
            vec![landed(to_dir, &target, &meta)],
        ));
    }
    let target = resolve_target(vfs, to_dir, name, meta.is_dir(), conflict)?;
    let Some((target, replace)) = target else {
        return Ok(Outcome::skipped(to_dir.join(name)?));
    };
    if replace {
        trash(vfs, &target)?;
    }
    match w.rename(from, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            copy_tree(vfs, from, vfs, w, &target, 0, on_progress)?;
            delete_tree(vfs, w, from, 0)?;
        }
        Err(e) => return Err(e),
    }
    let mut touched = vec![landed(to_dir, &target, &meta)];
    if !in_trash(from) {
        touched.push(Touched::shallow(parent_of(from)));
    }
    Ok(Outcome::done(target, touched))
}

/// Copies a file or a folder tree from `src` to `to` in `dst` through this
/// device. Files go through `.part` and are size-checked; a broken copy
/// resumes like an upload. Entries that are neither files nor folders
/// (devices, dangling links) are skipped.
fn copy_tree(
    src: &dyn Vfs,
    from: &VfsPath,
    dst: &dyn Vfs,
    wd: &dyn VfsWrite,
    to: &VfsPath,
    depth: usize,
    on_progress: OnProgress<'_>,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(VfsError::new(
            ErrorKind::Other,
            format!("{from}: folders nested too deep (a link loop?)"),
        ));
    }
    let meta = src.stat(from)?;
    match meta.kind {
        EntryKind::Dir => {
            ensure_dir(dst, wd, to)?;
            for e in src.list(from)? {
                if e.metadata.kind == EntryKind::Other {
                    continue;
                }
                copy_tree(
                    src,
                    &e.path,
                    dst,
                    wd,
                    &to.join(&e.name)?,
                    depth + 1,
                    on_progress,
                )?;
            }
            Ok(())
        }
        EntryKind::File => {
            let source = src.open(from)?;
            let size = source.size();
            let mut r = SourceReader::new(source);
            let part = part_of(to)?;
            write_part(dst, wd, &part, &mut r, size, on_progress)?;
            finish_part(dst, wd, &part, to, false)?;
            let landed = dst.stat(to)?.size;
            if landed != size {
                return Err(VfsError::new(
                    ErrorKind::Other,
                    format!("{to}: {landed} bytes after the copy, expected {size}"),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Deletes a file, a link or a folder tree. Links are removed, never
/// entered.
fn delete_tree(vfs: &dyn Vfs, w: &dyn VfsWrite, path: &VfsPath, depth: usize) -> Result<()> {
    if path.is_root() {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            "the source root cannot be deleted",
        ));
    }
    if depth > MAX_DEPTH {
        return Err(VfsError::new(
            ErrorKind::Other,
            format!("{path}: folders nested too deep"),
        ));
    }
    if w.is_symlink(path)? {
        return w.remove_file(path);
    }
    let meta = vfs.stat(path)?;
    if meta.is_dir() {
        for e in vfs.list(path)? {
            delete_tree(vfs, w, &e.path, depth + 1)?;
        }
        w.remove_dir(path)
    } else {
        w.remove_file(path)
    }
}

struct RawItem {
    name: String,
    path: VfsPath,
}

/// Folders directly in the trash.
fn trash_items(vfs: &dyn Vfs) -> Result<Vec<RawItem>> {
    let entries = match vfs.list(&trash_root()) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(entries
        .into_iter()
        .filter(|e| e.metadata.is_dir())
        .map(|e| RawItem {
            name: e.name,
            path: e.path,
        })
        .collect())
}

/// `<unix ms>-<nonce>`.
fn item_time(id: &str) -> Option<i64> {
    let (ms, rest) = id.split_once('-')?;
    if rest.is_empty() || ms.len() > 19 || !ms.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    ms.parse().ok()
}

fn new_trash_item(w: &dyn VfsWrite, root: &VfsPath) -> Result<VfsPath> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let mut last = None;
    for _ in 0..8 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let nonce = nanos
            ^ COUNTER
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_mul(0x9E37_79B9)
            ^ std::process::id().rotate_left(16);
        let item = root.join(&format!("{:013}-{nonce:08x}", now_ms()))?;
        match w.create_dir(&item) {
            Ok(()) => return Ok(item),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| VfsError::new(ErrorKind::AlreadyExists, "trash item")))
}

fn read_origin(vfs: &dyn Vfs, path: &VfsPath) -> Option<VfsPath> {
    let src = vfs.open(path).ok()?;
    let len = usize::try_from(src.size()).ok()?.min(64 * 1024);
    let mut buf = vec![0u8; len];
    src.read_exact_at(0, &mut buf).ok()?;
    VfsPath::parse(
        std::str::from_utf8(&buf)
            .ok()?
            .trim_end_matches(['\n', '\r']),
    )
    .ok()
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::KeepBoth => "keep both",
            Self::Replace => "replace",
            Self::Skip => "skip",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> VfsPath {
        VfsPath::parse(s).unwrap()
    }

    #[test]
    fn names() {
        for bad in ["", ".", "..", "a/b", "a\\b", "a\nb", "  ", "a\0"] {
            assert!(check_new_name(bad).is_err(), "{bad:?}");
        }
        for ok in ["a", "Дюна (2021).mkv", ".hidden", "a b"] {
            assert!(check_new_name(ok).is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn paths() {
        assert!(in_trash(&p("/.kinolune-trash")));
        assert!(in_trash(&p("/.kinolune-trash/1-a/x")));
        assert!(!in_trash(&p("/a/.kinolune-trash")));
        assert!(is_under(&p("/a/b"), &p("/a")));
        assert!(is_under(&p("/a"), &p("/a")));
        assert!(!is_under(&p("/ab"), &p("/a")));
        assert!(is_under(&p("/x"), &VfsPath::root()));
        assert!(check_movable(&VfsPath::root()).is_err());
        assert!(check_movable(&p("/.kinolune-trash")).is_err());
        assert!(check_movable(&p("/a")).is_ok());
        assert_eq!(part_of(&p("/d/a.mkv")).unwrap(), p("/d/a.mkv.part"));
    }

    #[test]
    fn trash_ids() {
        assert_eq!(
            item_time("0001700000000000-00ab12cd"),
            Some(1_700_000_000_000)
        );
        assert_eq!(item_time("1700000000000-x"), Some(1_700_000_000_000));
        assert_eq!(item_time("abc-1"), None);
        assert_eq!(item_time("17000"), None);
        assert_eq!(item_time("17000-"), None);
    }

    #[test]
    fn touched_dedup() {
        let t = dedup(vec![
            Touched::shallow(p("/a")),
            Touched::deep(p("/a")),
            Touched::shallow(p("/b")),
        ]);
        assert_eq!(t, vec![Touched::deep(p("/a")), Touched::shallow(p("/b"))]);
    }
}
