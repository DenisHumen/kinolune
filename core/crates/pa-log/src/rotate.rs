// SPDX-License-Identifier: MPL-2.0
//! Size-based log rotation: `kinolune.log` grows to `max_bytes`, then
//! becomes `kinolune.1.log` (the older ones shift up) and a new file
//! starts. At most `max_files` files are kept, so the logs never take more
//! than `max_bytes * max_files` on disk.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Limits of the log files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rotation {
    /// Size of one file before it rotates.
    pub max_bytes: u64,
    /// Files kept, the current one included (at least 1).
    pub max_files: u32,
}

impl Default for Rotation {
    /// 1 MiB × 4: enough for a few sessions, small enough to attach.
    fn default() -> Self {
        Self {
            max_bytes: 1 << 20,
            max_files: 4,
        }
    }
}

pub(crate) const LOG_STEM: &str = "kinolune";

/// The current log file and its size.
#[derive(Debug)]
pub(crate) struct RotatingFile {
    dir: PathBuf,
    rotation: Rotation,
    file: File,
    size: u64,
}

pub(crate) fn log_path(dir: &Path, index: u32) -> PathBuf {
    if index == 0 {
        dir.join(format!("{LOG_STEM}.log"))
    } else {
        dir.join(format!("{LOG_STEM}.{index}.log"))
    }
}

impl RotatingFile {
    pub(crate) fn open(dir: &Path, rotation: Rotation) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = log_path(dir, 0);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            dir: dir.to_path_buf(),
            rotation,
            file,
            size,
        })
    }

    /// Appends `line` (with its newline), rotating first when it would not
    /// fit.
    pub(crate) fn write_line(&mut self, line: &str) -> io::Result<()> {
        let len = line.len() as u64 + 1;
        if self.size > 0 && self.size + len > self.rotation.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.size += len;
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let keep = self.rotation.max_files.max(1);
        // The oldest falls off, the others move up by one.
        let _ = fs::remove_file(log_path(&self.dir, keep - 1));
        for i in (0..keep.saturating_sub(1)).rev() {
            let from = log_path(&self.dir, i);
            if from.exists() {
                fs::rename(&from, log_path(&self.dir, i + 1))?;
            }
        }
        if keep == 1 {
            let _ = fs::remove_file(log_path(&self.dir, 0));
        }
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path(&self.dir, 0))?;
        self.size = 0;
        Ok(())
    }
}

/// Log files in `dir`, newest first.
pub(crate) fn log_files(dir: &Path, rotation: Rotation) -> Vec<PathBuf> {
    (0..rotation.max_files.max(1) + 2)
        .map(|i| log_path(dir, i))
        .filter(|p| p.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_keeps_at_most_max_files() {
        let dir = tempfile::tempdir().unwrap();
        let rotation = Rotation {
            max_bytes: 100,
            max_files: 3,
        };
        let mut f = RotatingFile::open(dir.path(), rotation).unwrap();
        for i in 0..50 {
            f.write_line(&format!("line {i:03} ............")).unwrap();
        }
        let files = log_files(dir.path(), rotation);
        assert_eq!(files.len(), 3);
        for p in &files {
            assert!(fs::metadata(p).unwrap().len() <= 100);
        }
        let newest = fs::read_to_string(&files[0]).unwrap();
        assert!(newest.contains("line 049"));
        assert!(!dir.path().join("kinolune.3.log").exists());
    }
}
