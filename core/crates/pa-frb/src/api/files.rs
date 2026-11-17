// SPDX-License-Identifier: MPL-2.0
//! Files of a writable source for Dart (UI-035, «Файлы»): the operations
//! of `pa_vfs::files` (NET-023, ADR-0104) — list, create a folder, upload
//! with resume, rename, move and copy within and between sources, the
//! trash with restore, «очистить» and the 30-day purge — and the library
//! update after a write.
//!
//! The calls are methods of [`CoreLibrary`]. Paths are relative to the
//! source root, without a leading slash (`""` is the root), as in
//! `CoreSources::list_folder`. A failed call is a [`LibraryError`] whose
//! message starts with a code and `:`: `not_found`, `denied` (the share
//! or account does not let us), `read_only` (the source cannot be
//! written at all), `exists`, `full`, `cancelled`, `unavailable`,
//! `invalid` or `other`.
//!
//! Uploads and transfers stream [`CoreTransferEvent`]s and can be
//! cancelled by their task id ([`CoreLibrary::files_cancel`]); a cancelled
//! upload keeps its `.part` file and continues where it stopped the next
//! time the same file is uploaded to the same place.
//!
//! An operation does not update the library itself: it returns the
//! folders it touched, and [`CoreLibrary::files_refresh`] re-lists only
//! those (`Catalog::refresh_after_write`), so the caller decides when.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pa_catalog as c;
use pa_vfs::files::{self as f, Conflict, Outcome, Touched};
use pa_vfs::{ErrorKind, Vfs, VfsError, VfsPath};

use crate::api::library::{CoreLibrary, LibraryError};
use crate::frb_generated::StreamSink;

type Result<T> = std::result::Result<T, LibraryError>;

fn error(code: &str, message: impl std::fmt::Display) -> LibraryError {
    LibraryError {
        message: format!("{code}: {message}"),
    }
}

/// The code of a file system error (see the module docs).
fn vfs_error(e: &VfsError) -> LibraryError {
    let code = match e.kind() {
        ErrorKind::NotFound => "not_found",
        ErrorKind::PermissionDenied | ErrorKind::AuthenticationFailed => "denied",
        ErrorKind::AlreadyExists => "exists",
        ErrorKind::StorageFull => "full",
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::Unavailable | ErrorKind::TimedOut => "unavailable",
        ErrorKind::InvalidPath | ErrorKind::NotADirectory | ErrorKind::IsADirectory => "invalid",
        ErrorKind::Unsupported => "read_only",
        ErrorKind::Other => "other",
    };
    error(code, e)
}

fn catalog_error(e: c::Error) -> LibraryError {
    match e {
        c::Error::NotFound(m) => error("not_found", m),
        c::Error::Invalid(m) => error("invalid", m),
        c::Error::Denied(m) => error("denied", m),
        e => error("unavailable", e),
    }
}

fn path(rel: &str) -> Result<VfsPath> {
    VfsPath::parse(rel).map_err(|e| vfs_error(&e))
}

/// `/a/b` → `a/b`.
fn rel(p: &VfsPath) -> String {
    p.as_str().trim_start_matches('/').to_owned()
}

fn conflict(s: &str) -> Conflict {
    match s {
        "replace" => Conflict::Replace,
        "skip" => Conflict::Skip,
        _ => Conflict::KeepBoth,
    }
}

/// Entries the file view never shows: the trash, the write probe,
/// uploads in progress and other hidden files.
fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.ends_with(f::PART_SUFFIX)
}

/// Cancel flags of running uploads and transfers by task id.
fn tasks() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static TASKS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    TASKS.get_or_init(Mutex::default)
}

/// Registers `task_id`; dropping the guard forgets it.
struct TaskGuard {
    id: String,
    cancelled: Arc<AtomicBool>,
}

impl TaskGuard {
    fn new(id: &str) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        tasks()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_owned(), Arc::clone(&cancelled));
        Self {
            id: id.to_owned(),
            cancelled,
        }
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        let mut t = tasks().lock().unwrap_or_else(|e| e.into_inner());
        if t.get(&self.id).is_some_and(|c| Arc::ptr_eq(c, &self.cancelled)) {
            t.remove(&self.id);
        }
    }
}

/// An entry of a folder in the file view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFileEntry {
    pub name: String,
    /// Relative to the source root, without a leading slash.
    pub rel_path: String,
    pub is_folder: bool,
    /// Files: size in bytes; folders: 0.
    pub size_bytes: u64,
    /// Last modification, Unix milliseconds, if the backend reports it.
    pub modified_ms: Option<i64>,
}

fn entry(name: String, p: &VfsPath, m: &pa_vfs::Metadata) -> CoreFileEntry {
    CoreFileEntry {
        name,
        rel_path: rel(p),
        is_folder: m.is_dir(),
        size_bytes: m.size,
        modified_ms: m.modified_ms,
    }
}

/// A folder whose listing changed (`pa_vfs::files::Touched`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTouched {
    /// Relative to the source root, `""` for the root.
    pub dir: String,
    /// Everything below it changed too.
    pub recursive: bool,
}

fn touched(t: &[Touched]) -> Vec<CoreTouched> {
    t.iter()
        .map(|t| CoreTouched {
            dir: rel(&t.dir),
            recursive: t.recursive,
        })
        .collect()
}

/// Result of a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFileOutcome {
    /// Where the entry is now (skipped: the entry that blocked it).
    pub rel_path: String,
    /// Nothing was written (`skip` on a taken name).
    pub skipped: bool,
    /// Folders of the (destination) source to refresh.
    pub touched: Vec<CoreTouched>,
    /// Folders of the origin source to refresh (a move between sources).
    pub touched_origin: Vec<CoreTouched>,
}

impl From<&Outcome> for CoreFileOutcome {
    fn from(o: &Outcome) -> Self {
        Self {
            rel_path: rel(&o.path),
            skipped: o.skipped,
            touched: touched(&o.touched),
            touched_origin: touched(&o.touched_origin),
        }
    }
}

/// An entry in the trash of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTrashEntry {
    /// Item id, for restore and permanent delete.
    pub id: String,
    pub name: String,
    /// Where it was, relative to the source root.
    pub original: String,
    pub is_folder: bool,
    pub size_bytes: u64,
    /// When it was deleted, Unix milliseconds.
    pub deleted_ms: i64,
}

/// Progress of an upload or a transfer. The last event has `finished`
/// set, with `outcome` on success and `error` (a coded message, see the
/// module docs) on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTransferEvent {
    /// Bytes of the current file done, and its size.
    pub done: u64,
    pub total: u64,
    pub finished: bool,
    pub outcome: Option<CoreFileOutcome>,
    pub error: Option<String>,
}

/// Sends progress at most every 150 ms, and polls the cancel flag.
fn progress<'a>(
    sink: &'a StreamSink<CoreTransferEvent>,
    cancelled: &'a AtomicBool,
) -> impl FnMut(u64, u64) -> bool + 'a {
    let mut last: Option<Instant> = None;
    move |done, total| {
        if last.is_none_or(|t| t.elapsed() >= Duration::from_millis(150)) || done == total {
            last = Some(Instant::now());
            let _ = sink.add(CoreTransferEvent {
                done,
                total,
                finished: false,
                outcome: None,
                error: None,
            });
        }
        !cancelled.load(Ordering::Relaxed)
    }
}

fn finish(sink: &StreamSink<CoreTransferEvent>, result: Result<CoreFileOutcome>) {
    let _ = sink.add(match result {
        Ok(o) => CoreTransferEvent {
            done: 0,
            total: 0,
            finished: true,
            outcome: Some(o),
            error: None,
        },
        Err(e) => CoreTransferEvent {
            done: 0,
            total: 0,
            finished: true,
            outcome: None,
            error: Some(e.message),
        },
    });
}

impl CoreLibrary {
    fn files_vfs(&self, source_id: i64) -> Result<Arc<dyn Vfs>> {
        self.catalog().connect(source_id).map_err(catalog_error)
    }

    /// Whether source `source_id` can be written now: its protocol writes
    /// and the share or account lets us. The file view hides its write
    /// actions otherwise. Fails only when the source cannot be reached.
    pub fn files_writable(&self, source_id: i64) -> Result<bool> {
        self.catalog()
            .source_writable(source_id)
            .map_err(catalog_error)
    }

    /// The entries of folder `dir`, live, without hidden files, the trash
    /// and uploads in progress; in no particular order.
    pub fn files_list(&self, source_id: i64, dir: String) -> Result<Vec<CoreFileEntry>> {
        let vfs = self.files_vfs(source_id)?;
        let list = vfs.list(&path(&dir)?).map_err(|e| vfs_error(&e))?;
        Ok(list
            .into_iter()
            .filter(|e| !hidden(&e.name) && (e.metadata.is_dir() || e.metadata.is_file()))
            .map(|e| entry(e.name, &e.path, &e.metadata))
            .collect())
    }

    /// The entry at `rel_path`, or `None` if there is none.
    pub fn files_stat(&self, source_id: i64, rel_path: String) -> Result<Option<CoreFileEntry>> {
        let vfs = self.files_vfs(source_id)?;
        let p = path(&rel_path)?;
        match vfs.stat(&p) {
            Ok(m) => Ok(Some(entry(
                p.file_name().unwrap_or_default().to_owned(),
                &p,
                &m,
            ))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(vfs_error(&e)),
        }
    }

    /// Creates folder `name` in `parent`. `conflict` is `keep_both`
    /// (`name (2)`), `replace` or `skip` (both keep an existing folder).
    pub fn files_create_folder(
        &self,
        source_id: i64,
        parent: String,
        name: String,
        conflict: String,
    ) -> Result<CoreFileOutcome> {
        let vfs = self.files_vfs(source_id)?;
        let o = f::create_folder(vfs.as_ref(), &path(&parent)?, &name, self::conflict(&conflict))
            .map_err(|e| vfs_error(&e))?;
        Ok((&o).into())
    }

    /// Folder `rel_path` is there, or is created with its missing parents
    /// (the upload folder of «Личное»). Returns whether it was created.
    pub fn files_ensure_folder(&self, source_id: i64, rel_path: String) -> Result<bool> {
        let vfs = self.files_vfs(source_id)?;
        let target = path(&rel_path)?;
        let mut at = VfsPath::root();
        let mut created = false;
        for name in target.components() {
            let next = at.join(name).map_err(|e| vfs_error(&e))?;
            match vfs.stat(&next) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => {
                    return Err(error("exists", format!("{next}: a file has this name")));
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    f::create_folder(vfs.as_ref(), &at, name, Conflict::Skip)
                        .map_err(|e| vfs_error(&e))?;
                    created = true;
                }
                Err(e) => return Err(vfs_error(&e)),
            }
            at = next;
        }
        Ok(created)
    }

    /// Renames `rel_path` inside its folder.
    pub fn files_rename(
        &self,
        source_id: i64,
        rel_path: String,
        new_name: String,
        conflict: String,
    ) -> Result<CoreFileOutcome> {
        let vfs = self.files_vfs(source_id)?;
        let o = f::rename(vfs.as_ref(), &path(&rel_path)?, &new_name, self::conflict(&conflict))
            .map_err(|e| vfs_error(&e))?;
        Ok((&o).into())
    }

    /// Moves `rel_path` to the trash of its source (kept 30 days).
    pub fn files_trash(&self, source_id: i64, rel_path: String) -> Result<CoreFileOutcome> {
        let vfs = self.files_vfs(source_id)?;
        let o = f::trash(vfs.as_ref(), &path(&rel_path)?).map_err(|e| vfs_error(&e))?;
        Ok((&o).into())
    }

    /// Deletes `rel_path` for good, without the trash.
    pub fn files_delete_permanently(
        &self,
        source_id: i64,
        rel_path: String,
    ) -> Result<CoreFileOutcome> {
        let vfs = self.files_vfs(source_id)?;
        let o =
            f::delete_permanently(vfs.as_ref(), &path(&rel_path)?).map_err(|e| vfs_error(&e))?;
        Ok((&o).into())
    }

    /// The trash of the source, newest first.
    pub fn files_list_trash(&self, source_id: i64) -> Result<Vec<CoreTrashEntry>> {
        let vfs = self.files_vfs(source_id)?;
        let mut items = f::list_trash(vfs.as_ref()).map_err(|e| vfs_error(&e))?;
        items.reverse();
        Ok(items
            .into_iter()
            .map(|i| CoreTrashEntry {
                name: i.original.file_name().unwrap_or_default().to_owned(),
                original: rel(&i.original),
                is_folder: i.metadata.is_dir(),
                size_bytes: i.metadata.size,
                deleted_ms: i.deleted_ms,
                id: i.id,
            })
            .collect())
    }

    /// Puts trash item `id` back where it was.
    pub fn files_restore(
        &self,
        source_id: i64,
        id: String,
        conflict: String,
    ) -> Result<CoreFileOutcome> {
        let vfs = self.files_vfs(source_id)?;
        let o =
            f::restore(vfs.as_ref(), &id, self::conflict(&conflict)).map_err(|e| vfs_error(&e))?;
        Ok((&o).into())
    }

    /// Deletes trash item `id` for good.
    pub fn files_delete_from_trash(&self, source_id: i64, id: String) -> Result<()> {
        let vfs = self.files_vfs(source_id)?;
        let item = VfsPath::root()
            .join(f::TRASH_DIR)
            .and_then(|t| t.join(&id))
            .map_err(|e| vfs_error(&e))?;
        f::delete_permanently(vfs.as_ref(), &item).map_err(|e| vfs_error(&e))?;
        Ok(())
    }

    /// «Очистить»: deletes everything in the trash; returns how many items.
    pub fn files_empty_trash(&self, source_id: i64) -> Result<u32> {
        let vfs = self.files_vfs(source_id)?;
        let n = f::empty_trash(vfs.as_ref()).map_err(|e| vfs_error(&e))?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Deletes trash items older than 30 days; returns how many. A source
    /// that cannot be written has no trash: `0`.
    pub fn files_purge_trash(&self, source_id: i64) -> Result<u32> {
        let vfs = self.files_vfs(source_id)?;
        if vfs.as_write().is_none() {
            return Ok(0);
        }
        let n =
            f::purge_trash(vfs.as_ref(), f::now_ms(), f::TRASH_KEEP).map_err(|e| vfs_error(&e))?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Updates the library after a write: re-lists only the `touched`
    /// folders of source `source_id` and recognizes what is new there
    /// (files in a folder of a personal library are never recognized).
    /// Returns whether the media files of the library changed.
    pub fn files_refresh(&self, source_id: i64, touched: Vec<CoreTouched>) -> Result<bool> {
        let t = touched
            .iter()
            .map(|t| {
                Ok(Touched {
                    dir: path(&t.dir)?,
                    recursive: t.recursive,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let report = self
            .catalog()
            .refresh_after_write(source_id, &t, &c::CancelToken::new())
            .map_err(catalog_error)?;
        Ok(report.has_changes())
    }

    /// Uploads the local file `local_path` as `name` into folder `dir` of
    /// source `source_id`, resuming an earlier attempt of the same file.
    /// Progress comes as events; [`files_cancel`](Self::files_cancel)
    /// with `task_id` stops it and keeps what was sent.
    pub fn files_upload(
        &self,
        task_id: String,
        local_path: String,
        source_id: i64,
        dir: String,
        name: String,
        conflict: String,
        sink: StreamSink<CoreTransferEvent>,
    ) {
        let task = TaskGuard::new(&task_id);
        let mut on = progress(&sink, &task.cancelled);
        let result = self.upload_file(&local_path, source_id, &dir, &name, &conflict, &mut on);
        finish(&sink, result);
    }

    fn upload_file(
        &self,
        local_path: &str,
        source_id: i64,
        dir: &str,
        name: &str,
        conflict: &str,
        on: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<CoreFileOutcome> {
        let vfs = self.files_vfs(source_id)?;
        let mut file = std::fs::File::open(local_path)
            .map_err(|e| error("not_found", format!("{local_path}: {e}")))?;
        let o = f::upload(
            vfs.as_ref(),
            &path(dir)?,
            name,
            &mut file,
            self::conflict(conflict),
            on,
        )
        .map_err(|e| vfs_error(&e))?;
        Ok((&o).into())
    }

    /// Moves (`mv = true`) or copies `rel_path` of source `from_source`
    /// into folder `to_dir` of source `to_source` — the same source or
    /// another one. Within a source a move is one rename where the
    /// protocol can; between sources the original is deleted only after
    /// the whole copy was checked.
    #[allow(clippy::too_many_arguments)]
    pub fn files_transfer(
        &self,
        task_id: String,
        mv: bool,
        from_source: i64,
        rel_path: String,
        to_source: i64,
        to_dir: String,
        conflict: String,
        sink: StreamSink<CoreTransferEvent>,
    ) {
        let task = TaskGuard::new(&task_id);
        let result = (|| {
            let from = path(&rel_path)?;
            let to = path(&to_dir)?;
            let conflict = self::conflict(&conflict);
            let src = self.files_vfs(from_source)?;
            let mut on = progress(&sink, &task.cancelled);
            let o = if from_source == to_source {
                if mv {
                    f::move_within(src.as_ref(), &from, &to, None, conflict)
                } else {
                    f::copy_within(src.as_ref(), &from, &to, conflict, &mut on)
                }
            } else {
                let dst = self.files_vfs(to_source)?;
                if mv {
                    f::move_between(src.as_ref(), &from, dst.as_ref(), &to, conflict, &mut on)
                } else {
                    f::copy_between(src.as_ref(), &from, dst.as_ref(), &to, conflict, &mut on)
                }
            }
            .map_err(|e| vfs_error(&e))?;
            Ok((&o).into())
        })();
        finish(&sink, result);
    }

    /// Cancels the upload or transfer `task_id`; `false` if it is not
    /// running.
    #[flutter_rust_bridge::frb(sync)]
    pub fn files_cancel(&self, task_id: String) -> bool {
        let t = tasks().lock().unwrap_or_else(|e| e.into_inner());
        match t.get(&task_id) {
            Some(c) => {
                c.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_names() {
        assert!(hidden(".kinolune-trash"));
        assert!(hidden("clip.mp4.part"));
        assert!(hidden(".DS_Store"));
        assert!(!hidden("clip.mp4"));
    }

    #[test]
    fn conflicts_default_to_keep_both() {
        assert_eq!(conflict("replace"), Conflict::Replace);
        assert_eq!(conflict("skip"), Conflict::Skip);
        assert_eq!(conflict("keep_both"), Conflict::KeepBoth);
        assert_eq!(conflict("anything"), Conflict::KeepBoth);
    }

    #[test]
    fn errors_carry_a_code() {
        let e = vfs_error(&VfsError::new(ErrorKind::AlreadyExists, "x"));
        assert!(e.message.starts_with("exists: "), "{}", e.message);
        let e = vfs_error(&VfsError::new(ErrorKind::Cancelled, "x"));
        assert!(e.message.starts_with("cancelled: "), "{}", e.message);
    }

    #[test]
    fn a_cancel_reaches_the_running_task_only() {
        let dir = tempfile::tempdir().unwrap();
        let lib = CoreLibrary::open(dir.path().join("l.db").to_string_lossy().into()).unwrap();
        let t = TaskGuard::new("t1");
        assert!(lib.files_cancel("t1".into()));
        assert!(t.cancelled.load(Ordering::Relaxed));
        drop(t);
        assert!(!lib.files_cancel("t1".into()));
    }

    fn library_with_source() -> (tempfile::TempDir, CoreLibrary, i64) {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).unwrap();
        let lib = CoreLibrary::open(dir.path().join("l.db").to_string_lossy().into()).unwrap();
        let src = lib
            .add_source(
                None,
                "local".into(),
                "M".into(),
                media.to_string_lossy().into(),
                None,
            )
            .unwrap();
        (dir, lib, src.id)
    }

    fn names(lib: &CoreLibrary, id: i64, dir: &str) -> Vec<String> {
        let mut v: Vec<String> = lib
            .files_list(id, dir.into())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        v.sort();
        v
    }

    #[test]
    fn folders_rename_move_copy_and_the_trash() {
        let (_dir, lib, id) = library_with_source();
        assert!(lib.files_writable(id).unwrap());
        let a = lib
            .files_create_folder(id, String::new(), "A".into(), "keep_both".into())
            .unwrap();
        assert_eq!(a.rel_path, "A");
        let a2 = lib
            .files_create_folder(id, String::new(), "A".into(), "keep_both".into())
            .unwrap();
        assert_eq!(a2.rel_path, "A (2)");
        assert!(lib.files_ensure_folder(id, "Kinolune/Личное".into()).unwrap());
        assert!(!lib.files_ensure_folder(id, "Kinolune/Личное".into()).unwrap());
        let r = lib
            .files_rename(id, "A (2)".into(), "B".into(), "keep_both".into())
            .unwrap();
        assert_eq!(r.rel_path, "B");
        assert_eq!(names(&lib, id, ""), ["A", "B", "Kinolune"]);

        let t = lib.files_trash(id, "B".into()).unwrap();
        assert_eq!(t.touched, [CoreTouched { dir: String::new(), recursive: false }]);
        // The trash folder is never listed.
        assert_eq!(names(&lib, id, ""), ["A", "Kinolune"]);
        let trash = lib.files_list_trash(id).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].original, "B");
        assert!(trash[0].is_folder);
        lib.files_restore(id, trash[0].id.clone(), "keep_both".into())
            .unwrap();
        assert_eq!(names(&lib, id, ""), ["A", "B", "Kinolune"]);
        lib.files_trash(id, "B".into()).unwrap();
        assert_eq!(lib.files_empty_trash(id).unwrap(), 1);
        assert!(lib.files_list_trash(id).unwrap().is_empty());
        // Nothing is 30 days old.
        assert_eq!(lib.files_purge_trash(id).unwrap(), 0);
        assert!(lib.files_stat(id, "B".into()).unwrap().is_none());
        assert!(lib.files_stat(id, "A".into()).unwrap().unwrap().is_folder);
    }

    #[test]
    fn an_upload_into_a_personal_folder_lands_unrecognized() {
        let (dir, lib, id) = library_with_source();
        let local = dir.path().join("clip.mp4");
        std::fs::write(&local, vec![7u8; 300_000]).unwrap();
        let personal = lib
            .create_personal_library("Личное".into(), "none".into(), None)
            .unwrap();
        lib.files_ensure_folder(id, "Kinolune/Личное".into()).unwrap();
        lib.add_personal_folder(personal.id, id, "/Kinolune/Личное".into())
            .unwrap();

        let mut seen = 0;
        let out = lib
            .upload_file(
                &local.to_string_lossy(),
                id,
                "Kinolune/Личное",
                "clip.mp4",
                "keep_both",
                &mut |done, total| {
                    assert_eq!(total, 300_000);
                    seen = done;
                    true
                },
            )
            .unwrap();
        assert_eq!(seen, 300_000);
        assert_eq!(out.rel_path, "Kinolune/Личное/clip.mp4");
        assert!(lib.files_refresh(id, out.touched).unwrap());
        let items = lib
            .personal_items(personal.id, Default::default(), "added".into())
            .unwrap();
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].rel_path, "Kinolune/Личное/clip.mp4");
        // Not in the main library.
        assert!(lib.movies().unwrap().is_empty());

        // The same name again: keep both by default, skip on request.
        let again = lib
            .upload_file(&local.to_string_lossy(), id, "Kinolune/Личное", "clip.mp4", "", &mut |_, _| true)
            .unwrap();
        assert_eq!(again.rel_path, "Kinolune/Личное/clip (2).mp4");
        let skipped = lib
            .upload_file(&local.to_string_lossy(), id, "Kinolune/Личное", "clip.mp4", "skip", &mut |_, _| true)
            .unwrap();
        assert!(skipped.skipped);

        // A cancelled upload keeps its part and resumes.
        let err = lib
            .upload_file(&local.to_string_lossy(), id, "", "big.mp4", "keep_both", &mut |d, _| d == 0)
            .unwrap_err();
        assert!(err.message.starts_with("cancelled"), "{}", err.message);
        let done = lib
            .upload_file(&local.to_string_lossy(), id, "", "big.mp4", "keep_both", &mut |_, _| true)
            .unwrap();
        assert_eq!(done.rel_path, "big.mp4");
        assert_eq!(lib.files_stat(id, "big.mp4".into()).unwrap().unwrap().size_bytes, 300_000);
    }

    #[test]
    fn a_move_between_sources_leaves_the_origin_only_after_the_copy() {
        let (dir, lib, a) = library_with_source();
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(dir.path().join("media/x.mkv"), b"0123456789").unwrap();
        let b = lib
            .add_source(None, "local".into(), "O".into(), other.to_string_lossy().into(), None)
            .unwrap()
            .id;
        let mut on = |_, _| true;
        let o = f::move_between(
            lib.files_vfs(a).unwrap().as_ref(),
            &path("x.mkv").unwrap(),
            lib.files_vfs(b).unwrap().as_ref(),
            &VfsPath::root(),
            Conflict::KeepBoth,
            &mut on,
        )
        .unwrap();
        assert_eq!(CoreFileOutcome::from(&o).touched_origin.len(), 1);
        assert!(lib.files_stat(a, "x.mkv".into()).unwrap().is_none());
        assert_eq!(lib.files_stat(b, "x.mkv".into()).unwrap().unwrap().size_bytes, 10);
    }
}
