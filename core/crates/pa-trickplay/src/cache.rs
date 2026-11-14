// SPDX-License-Identifier: MPL-2.0
//! Disk cache of generated previews: one BIF per media file, LRU with a
//! size limit.
//!
//! As in the artwork cache (LIB-013, ADR-0036), the last access is the
//! file's modification time, so the order survives restarts; when the
//! limit is exceeded the least recently used files go first, down to 90 %
//! of it. Files are named by a hash of the caller's media key and written
//! atomically (temporary file, then rename).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

/// Default size limit: 512 MiB, about 60 two-hour films at 10 s.
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// A hit refreshes the modification time at most this often.
const TOUCH_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Size of the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub bytes: u64,
    pub files: u64,
    pub max_bytes: u64,
}

/// See the module docs.
#[derive(Debug)]
pub struct TrickplayCache {
    dir: PathBuf,
    max_bytes: AtomicU64,
    lock: Mutex<()>,
}

impl TrickplayCache {
    /// Opens (creating) the cache in `dir` and removes leftovers of
    /// interrupted writes.
    ///
    /// # Errors
    ///
    /// If the directory cannot be created or listed.
    pub fn open(dir: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "tmp") {
                let _ = fs::remove_file(path);
            }
        }
        Ok(Self {
            dir,
            max_bytes: AtomicU64::new(max_bytes),
            lock: Mutex::new(()),
        })
    }

    /// The cache directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> PathBuf {
        let hash = Sha256::digest(key.as_bytes());
        let name: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{name}.bif"))
    }

    /// The cached BIF of `key`, marking it used.
    #[must_use]
    pub fn load(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.path(key);
        let data = fs::read(&path).ok()?;
        touch(&path);
        Some(data)
    }

    /// `true` if `key` is cached (without marking it used).
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.path(key).is_file()
    }

    /// Stores the BIF of `key`, then trims the cache to its limit.
    ///
    /// # Errors
    ///
    /// If the file cannot be written.
    pub fn store(&self, key: &str, bif: &[u8]) -> io::Result<()> {
        let path = self.path(key);
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&tmp, bif)?;
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        self.trim(Some(&path));
        Ok(())
    }

    /// Size of the cached files.
    #[must_use]
    pub fn usage(&self) -> Usage {
        let files = self.files();
        Usage {
            bytes: files.iter().map(|f| f.1).sum(),
            files: files.len() as u64,
            max_bytes: self.max_bytes.load(Ordering::Relaxed),
        }
    }

    /// Changes the limit and trims at once.
    pub fn set_max_bytes(&self, max_bytes: u64) {
        self.max_bytes.store(max_bytes, Ordering::Relaxed);
        self.trim(None);
    }

    /// Removes every cached preview.
    ///
    /// # Errors
    ///
    /// If a file cannot be removed.
    pub fn clear(&self) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        for (path, _, _) in self.files() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// `(path, size, last use)` of the cached files.
    fn files(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        dir.filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "bif"))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                Some((
                    e.path(),
                    meta.len(),
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                ))
            })
            .collect()
    }

    /// Removes least recently used files down to 90 % of the limit when it
    /// is exceeded; `keep` (just written) goes last.
    fn trim(&self, keep: Option<&Path>) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let max = self.max_bytes.load(Ordering::Relaxed);
        let mut files = self.files();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= max {
            return;
        }
        let target = max / 10 * 9;
        files.sort_by_key(|(p, _, t)| (Some(p.as_path()) == keep, *t));
        for (path, size, _) in files {
            if total <= target {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }
}

/// Sets the modification time to now, unless it was set recently.
fn touch(path: &Path) {
    let now = SystemTime::now();
    let Ok(file) = fs::File::options().write(true).open(path) else {
        return;
    };
    let recent = file
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age < TOUCH_INTERVAL);
    if !recent {
        let _ = file.set_modified(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn age(cache: &TrickplayCache, key: &str, secs: u64) {
        let file = fs::File::options()
            .write(true)
            .open(cache.path(key))
            .unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn store_load_and_evict_least_recent() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrickplayCache::open(dir.path(), 3000).unwrap();
        assert_eq!(cache.load("a"), None);
        cache.store("a", &[1; 1000]).unwrap();
        cache.store("b", &[2; 1000]).unwrap();
        age(&cache, "a", 3600);
        age(&cache, "b", 7200);
        // A hit makes "b" recent; "a" is now the oldest.
        assert_eq!(cache.load("b").unwrap().len(), 1000);
        cache.store("c", &[3; 1500]).unwrap();
        assert!(!cache.contains("a"));
        assert!(cache.contains("b") && cache.contains("c"));
        let usage = cache.usage();
        assert_eq!((usage.bytes, usage.files, usage.max_bytes), (2500, 2, 3000));

        cache.set_max_bytes(1000);
        assert!(cache.usage().bytes <= 1000);
        cache.clear().unwrap();
        assert_eq!(cache.usage().files, 0);
    }

    #[test]
    fn removes_leftover_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("x.123.tmp"), b"partial").unwrap();
        let cache = TrickplayCache::open(dir.path(), DEFAULT_MAX_BYTES).unwrap();
        assert_eq!(fs::read_dir(cache.dir()).unwrap().count(), 0);
    }
}
