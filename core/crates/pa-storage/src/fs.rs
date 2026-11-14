// SPDX-License-Identifier: MPL-2.0
//! Volume space, atomic moves and the fake file system for tests.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Space of the volume that holds a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Space {
    /// Size of the volume.
    pub total: u64,
    /// Bytes this process may still write (free space minus what the
    /// file system keeps for root).
    pub available: u64,
}

/// Source of volume space. [`SystemFs`] asks the OS; tests inject
/// [`FakeFs`] to simulate a small volume without mounting one.
pub trait FsStats: Send + Sync {
    /// Space of the volume of `path`; an error if the path is not there
    /// (unmounted share, removed disk).
    fn space(&self, path: &Path) -> io::Result<Space>;
}

/// [`FsStats`] of the real file system (`statvfs`, `GetDiskFreeSpaceExW`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemFs;

impl FsStats for SystemFs {
    fn space(&self, path: &Path) -> io::Result<Space> {
        let stats = fs4::statvfs(path)?;
        Ok(Space {
            total: stats.total_space(),
            available: stats.available_space(),
        })
    }
}

/// A simulated volume for tests: a fixed size, real files under its root
/// count as used (walked on every call), plus bytes used by "other
/// programs" ([`FakeFs::set_external`]).
#[derive(Debug, Default)]
pub struct FakeFs {
    volumes: Mutex<HashMap<PathBuf, (u64, u64)>>,
}

impl FakeFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a volume of `total` bytes mounted at `root`.
    pub fn add_volume(&self, root: impl Into<PathBuf>, total: u64) {
        lock(&self.volumes).insert(root.into(), (total, 0));
    }

    /// Bytes on the volume of `root` that are not files under it.
    pub fn set_external(&self, root: &Path, bytes: u64) {
        if let Some(v) = lock(&self.volumes).get_mut(root) {
            v.1 = bytes;
        }
    }

    /// Removes a volume: its paths report an error, like an unmounted share.
    pub fn remove_volume(&self, root: &Path) {
        lock(&self.volumes).remove(root);
    }
}

impl FsStats for FakeFs {
    fn space(&self, path: &Path) -> io::Result<Space> {
        let volumes = lock(&self.volumes);
        let (root, (total, external)) = volumes
            .iter()
            .filter(|(root, _)| path.starts_with(root))
            .max_by_key(|(root, _)| root.as_os_str().len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such volume"))?;
        let used = dir_size(root)?.saturating_add(*external);
        Ok(Space {
            total: *total,
            available: total.saturating_sub(used),
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Total size of the files under `path` (not following symlinks).
pub fn dir_size(path: &Path) -> io::Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Ok(meta.len());
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(dir_size(&entry?.path())?);
    }
    Ok(total)
}

/// True if `e` says that a rename crossed volumes.
pub fn is_cross_device(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::CrossesDevices {
        return true;
    }
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
    #[cfg(unix)]
    let code = 18;
    #[cfg(windows)]
    let code = 17;
    #[cfg(not(any(unix, windows)))]
    let code = -1;
    e.raw_os_error() == Some(code)
}

/// Moves `src` to `dst` atomically: `dst` either does not exist or is the
/// complete file. A rename where possible; across volumes the file is
/// copied to `scratch` (a directory on the volume of `dst`), synced,
/// renamed into place, and `src` is removed. Fails if `dst` exists.
pub fn move_atomic(src: &Path, dst: &Path, scratch: &Path) -> io::Result<()> {
    move_with(src, dst, scratch, |a, b| fs::rename(a, b))
}

static PARTIAL: AtomicU64 = AtomicU64::new(0);

pub(crate) fn move_with(
    src: &Path,
    dst: &Path,
    scratch: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    if fs::symlink_metadata(dst).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists", dst.display()),
        ));
    }
    match rename(src, dst) {
        Ok(()) => {}
        Err(e) if is_cross_device(&e) => {
            if fs::metadata(src)?.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::CrossesDevices,
                    "cannot move a directory across volumes",
                ));
            }
            fs::create_dir_all(scratch)?;
            let partial = scratch.join(format!(
                "xdev-{}-{}",
                std::process::id(),
                PARTIAL.fetch_add(1, Ordering::Relaxed)
            ));
            let copied = fs::copy(src, &partial)
                .and_then(|_| fs::File::open(&partial)?.sync_all())
                .and_then(|()| fs::rename(&partial, dst));
            if let Err(e) = copied {
                let _ = fs::remove_file(&partial);
                return Err(e);
            }
            fs::remove_file(src)?;
        }
        Err(e) => return Err(e),
    }
    sync_parent(dst);
    Ok(())
}

/// Makes a rename durable (Unix: fsync of the directory). Best effort.
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_moves_and_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, b"one").unwrap();
        move_atomic(&a, &b, &dir.path().join(".tmp")).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(&b).unwrap(), b"one");
        fs::write(&a, b"two").unwrap();
        let err = move_atomic(&a, &b, &dir.path().join(".tmp")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&b).unwrap(), b"one");
    }

    #[test]
    fn cross_device_falls_back_to_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("sub/b"));
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        fs::write(&a, vec![7u8; 100_000]).unwrap();
        let scratch = dir.path().join(".tmp");
        let exdev = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::CrossesDevices));
        move_with(&a, &b, &scratch, exdev).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(&b).unwrap().len(), 100_000);
        // No partial copy is left behind.
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    }

    #[test]
    fn other_rename_errors_are_returned() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, b"x").unwrap();
        let denied = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::PermissionDenied));
        let err = move_with(&a, &b, dir.path(), denied).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(a.exists() && !b.exists());
    }

    #[test]
    fn system_fs_reports_the_temp_volume() {
        let dir = tempfile::tempdir().unwrap();
        let space = SystemFs.space(dir.path()).unwrap();
        assert!(space.total > 0 && space.available <= space.total);
        assert!(SystemFs.space(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn fake_fs_counts_files_and_external_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeFs::new();
        fake.add_volume(dir.path(), 10_000);
        fs::create_dir(dir.path().join("x")).unwrap();
        fs::write(dir.path().join("x/f"), vec![0u8; 3000]).unwrap();
        fake.set_external(dir.path(), 1000);
        let space = fake.space(&dir.path().join("x")).unwrap();
        assert_eq!(
            space,
            Space {
                total: 10_000,
                available: 6000
            }
        );
        fake.remove_volume(dir.path());
        assert!(fake.space(dir.path()).is_err());
    }
}
