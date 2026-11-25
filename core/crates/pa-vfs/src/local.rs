// SPDX-License-Identifier: MPL-2.0
//! Local file system backend: a folder on this device or a mounted share.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{ErrorKind, Result, VfsError};
use crate::path::VfsPath;
use crate::source::ByteSource;
use crate::vfs::{DirEntry, EntryKind, Metadata, Vfs};
use crate::write::VfsWrite;

/// Copy buffer of [`LocalFs::write_file`].
const WRITE_BUF: usize = 1024 * 1024;

/// [`Vfs`] over a local directory.
///
/// Every [`VfsPath`] resolves under `root`; `..` cannot be expressed, so a
/// path never leaves the root. Symbolic links are followed like on a
/// network share, including links that point outside the root: the user
/// chose the folder and its links.
///
/// On Windows, names that the platform cannot hold (`\`, `:`, `*`, `?`,
/// `"`, `<`, `>`, `|`, control characters, reserved device names such as
/// `CON` or `nul.txt`) are rejected with [`ErrorKind::InvalidPath`].
#[derive(Debug, Clone)]
pub struct LocalFs {
    root: PathBuf,
}

impl LocalFs {
    /// Opens a local source rooted at `root`, which must be an existing directory.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        // BUG-022: a root from a `file://` URI has `/` on Windows.
        let root = crate::host::native(root.into());
        let meta = fs::metadata(&root).map_err(|e| VfsError::from_io(e, "source root"))?;
        if !meta.is_dir() {
            return Err(VfsError::new(
                ErrorKind::NotADirectory,
                "source root is not a directory",
            ));
        }
        Ok(Self { root })
    }

    /// The directory this source is rooted at.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Maps a [`VfsPath`] to a host path under the root.
    pub fn host_path(&self, path: &VfsPath) -> Result<PathBuf> {
        let mut out = self.root.clone();
        for name in path.components() {
            check_host_name(name)
                .map_err(|why| VfsError::new(ErrorKind::InvalidPath, format!("{path}: {why}")))?;
            out.push(name);
        }
        Ok(out)
    }
}

impl Vfs for LocalFs {
    fn list(&self, dir: &VfsPath) -> Result<Vec<DirEntry>> {
        let host = self.host_path(dir)?;
        let meta = fs::metadata(&host).map_err(|e| VfsError::from_io(e, dir))?;
        if !meta.is_dir() {
            return Err(VfsError::new(ErrorKind::NotADirectory, dir.to_string()));
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&host).map_err(|e| VfsError::from_io(e, dir))? {
            let entry = entry.map_err(|e| VfsError::from_io(e, dir))?;
            // Non-UTF-8 names cannot become a VfsPath; skip them (documented on `Vfs::list`).
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(path) = dir.join(&name) else {
                continue;
            };
            // Entries removed between read_dir and stat are skipped.
            let Some(metadata) = entry_metadata(&entry.path()) else {
                continue;
            };
            out.push(DirEntry {
                name,
                path,
                metadata,
            });
        }
        out.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn stat(&self, path: &VfsPath) -> Result<Metadata> {
        let host = self.host_path(path)?;
        match fs::metadata(&host) {
            Ok(m) => Ok(convert(&m)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(&host) {
                // A dangling symlink exists but cannot be followed.
                Ok(m) => Ok(Metadata {
                    kind: EntryKind::Other,
                    size: 0,
                    modified_ms: m.modified().ok().map(to_unix_ms),
                }),
                Err(_) => Err(VfsError::from_io(e, path)),
            },
            Err(e) => Err(VfsError::from_io(e, path)),
        }
    }

    fn open(&self, path: &VfsPath) -> Result<Box<dyn ByteSource>> {
        let host = self.host_path(path)?;
        // Check the type before opening: opening a FIFO would block.
        let meta = fs::metadata(&host).map_err(|e| VfsError::from_io(e, path))?;
        if meta.is_dir() {
            return Err(VfsError::new(ErrorKind::IsADirectory, path.to_string()));
        }
        if !meta.is_file() {
            return Err(VfsError::new(
                ErrorKind::Unsupported,
                format!("{path}: not a regular file"),
            ));
        }
        let file = File::open(&host).map_err(|e| VfsError::from_io(e, path))?;
        let size = file
            .metadata()
            .map_err(|e| VfsError::from_io(e, path))?
            .len();
        Ok(Box::new(LocalSource {
            file: imp::Handle::new(file),
            size,
            path: path.clone(),
        }))
    }

    fn as_write(&self) -> Option<&dyn VfsWrite> {
        Some(self)
    }
}

impl LocalFs {
    /// Host path of an entry that a write creates or removes: never the root.
    fn write_path(&self, path: &VfsPath) -> Result<PathBuf> {
        if path.is_root() {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                "the source root cannot be written",
            ));
        }
        self.host_path(path)
    }

    /// Fails with [`ErrorKind::AlreadyExists`] if `path` exists (even as a
    /// dangling link): `rename(2)` and `fs::copy` would replace it.
    fn check_free(host: &Path, path: &VfsPath) -> Result<()> {
        match fs::symlink_metadata(host) {
            Ok(_) => Err(VfsError::new(ErrorKind::AlreadyExists, path.to_string())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(VfsError::from_io(e, path)),
        }
    }
}

impl VfsWrite for LocalFs {
    fn create_dir(&self, path: &VfsPath) -> Result<()> {
        let host = self.write_path(path)?;
        fs::create_dir(&host).map_err(|e| VfsError::from_io(e, path))
    }

    fn write_file(
        &self,
        path: &VfsPath,
        offset: u64,
        data: &mut dyn Read,
        len: u64,
    ) -> Result<u64> {
        let host = self.write_path(path)?;
        let mut file = if offset == 0 {
            File::create(&host)
        } else {
            OpenOptions::new().append(true).open(&host)
        }
        .map_err(|e| VfsError::from_io(e, path))?;
        if offset > 0 {
            let size = file
                .metadata()
                .map_err(|e| VfsError::from_io(e, path))?
                .len();
            if size != offset {
                return Err(VfsError::new(
                    ErrorKind::Other,
                    format!("{path}: {size} bytes, cannot append at {offset}"),
                ));
            }
        }
        let mut buf = vec![0u8; WRITE_BUF.min(usize::try_from(len).unwrap_or(WRITE_BUF).max(1))];
        let mut done = 0u64;
        while done < len {
            let want = usize::try_from(len - done)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            let n = match data.read(&mut buf[..want]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(VfsError::from_io(e, "upload data")),
            };
            file.write_all(&buf[..n])
                .map_err(|e| VfsError::from_io(e, path))?;
            done += n as u64;
        }
        file.flush().map_err(|e| VfsError::from_io(e, path))?;
        Ok(done)
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        let src = self.write_path(from)?;
        let dst = self.write_path(to)?;
        Self::check_free(&dst, to)?;
        fs::rename(&src, &dst).map_err(|e| VfsError::from_io(e, from))
    }

    fn server_copy(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        let src = self.write_path(from)?;
        let dst = self.write_path(to)?;
        // Files only (the OS may clone them); folders go entry by entry.
        if !fs::metadata(&src)
            .map_err(|e| VfsError::from_io(e, from))?
            .is_file()
        {
            return Err(VfsError::new(
                ErrorKind::Unsupported,
                format!("{from}: not a regular file"),
            ));
        }
        Self::check_free(&dst, to)?;
        fs::copy(&src, &dst).map_err(|e| VfsError::from_io(e, to))?;
        Ok(())
    }

    fn remove_file(&self, path: &VfsPath) -> Result<()> {
        let host = self.write_path(path)?;
        // `remove_file` removes a link itself, never its target.
        match fs::remove_file(&host) {
            Err(e) if fs::symlink_metadata(&host).is_ok_and(|m| m.file_type().is_symlink()) => {
                // A link to a folder on Windows is a directory entry.
                fs::remove_dir(&host).map_err(|_| VfsError::from_io(e, path))
            }
            r => r.map_err(|e| VfsError::from_io(e, path)),
        }
    }

    fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        let host = self.write_path(path)?;
        fs::remove_dir(&host).map_err(|e| VfsError::from_io(e, path))
    }

    fn is_symlink(&self, path: &VfsPath) -> Result<bool> {
        let host = self.host_path(path)?;
        match fs::symlink_metadata(&host) {
            Ok(m) => Ok(m.file_type().is_symlink()),
            Err(e) => Err(VfsError::from_io(e, path)),
        }
    }
}

/// An opened local file.
#[derive(Debug)]
struct LocalSource {
    file: imp::Handle,
    size: u64,
    path: VfsPath,
}

impl ByteSource for LocalSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= self.size || buf.is_empty() {
            return Ok(0);
        }
        // Never return bytes past the size fixed at open, even if the file grew.
        let left = usize::try_from(self.size - offset).unwrap_or(usize::MAX);
        let len = buf.len().min(left);
        loop {
            match self.file.read_at(&mut buf[..len], offset) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                r => return r.map_err(|e| VfsError::from_io(e, &self.path)),
            }
        }
    }
}

fn entry_metadata(host: &Path) -> Option<Metadata> {
    match fs::metadata(host) {
        Ok(m) => Some(convert(&m)),
        // Dangling link or an entry we may not stat: list it as Other.
        Err(_) => fs::symlink_metadata(host).ok().map(|m| Metadata {
            kind: EntryKind::Other,
            size: 0,
            modified_ms: m.modified().ok().map(to_unix_ms),
        }),
    }
}

fn convert(m: &fs::Metadata) -> Metadata {
    let kind = if m.is_file() {
        EntryKind::File
    } else if m.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::Other
    };
    Metadata {
        kind,
        size: if kind == EntryKind::File { m.len() } else { 0 },
        modified_ms: m.modified().ok().map(to_unix_ms),
    }
}

fn to_unix_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
    }
}

#[cfg(windows)]
fn check_host_name(name: &str) -> std::result::Result<(), &'static str> {
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err("character not allowed on Windows");
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let reserved = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|r| stem.eq_ignore_ascii_case(r))
        || ["COM", "LPT"].iter().any(|r| {
            stem.len() == 4
                && stem[..3].eq_ignore_ascii_case(r)
                && matches!(stem.as_bytes()[3], b'1'..=b'9')
        });
    if reserved {
        return Err("reserved device name on Windows");
    }
    Ok(())
}

#[cfg(not(windows))]
fn check_host_name(_name: &str) -> std::result::Result<(), &'static str> {
    Ok(())
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::unix::fs::FileExt;

    #[derive(Debug)]
    pub(super) struct Handle(File);

    impl Handle {
        pub(super) fn new(file: File) -> Self {
            Self(file)
        }

        pub(super) fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
            self.0.read_at(buf, offset)
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::windows::fs::FileExt;

    #[derive(Debug)]
    pub(super) struct Handle(File);

    impl Handle {
        pub(super) fn new(file: File) -> Self {
            Self(file)
        }

        /// `seek_read` passes the offset in OVERLAPPED, so concurrent calls
        /// on a synchronous handle do not race on a shared cursor.
        pub(super) fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
            self.0.seek_read(buf, offset)
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::fs::File;
    use std::io::{self, Read, Seek, SeekFrom};
    use std::sync::Mutex;

    #[derive(Debug)]
    pub(super) struct Handle(Mutex<File>);

    impl Handle {
        pub(super) fn new(file: File) -> Self {
            Self(Mutex::new(file))
        }

        pub(super) fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
            let mut f = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            f.seek(SeekFrom::Start(offset))?;
            f.read(buf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_ms_conversion() {
        assert_eq!(to_unix_ms(UNIX_EPOCH), 0);
        assert_eq!(
            to_unix_ms(UNIX_EPOCH + std::time::Duration::from_millis(1_700_000_000_123)),
            1_700_000_000_123
        );
        assert_eq!(
            to_unix_ms(UNIX_EPOCH - std::time::Duration::from_millis(5)),
            -5
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_names() {
        for bad in ["a:b", "a\\b", "CON", "con.txt", "LPT1", "x?", "nul .mkv"] {
            assert!(check_host_name(bad).is_err(), "{bad:?}");
        }
        for ok in ["CONSOLE", "COM0", "Дюна.mkv", "a b"] {
            assert!(check_host_name(ok).is_ok(), "{ok:?}");
        }
    }
}
