// SPDX-License-Identifier: MPL-2.0
//! Host (OS) paths of local sources (BUG-022).
//!
//! The library stores a local source root as a `file://` URI, which always
//! uses `/` (`file:///C:/Users/me/Movies`), and the files under it as a
//! [`VfsPath`](crate::VfsPath) (`/Films/Dune.mkv`). Neither is a Windows
//! path: `PathBuf::from("C:/Users/me/Movies").join("Films/Dune.mkv")` gives
//! `C:/Users/me/Movies\Films/Dune.mkv`. The OS opens it, but the user sees
//! three kinds of separators. These helpers turn the stored forms into one
//! native path (`C:\Users\me\Movies\Films\Dune.mkv` on Windows, unchanged
//! elsewhere). The stored forms stay as they are: nothing is migrated.
//!
//! The `*_for` functions take the platform as an argument, so the Windows
//! rules are tested on every OS.

use std::path::{Path, PathBuf};

/// Whether this build runs on Windows: the platform the host paths are for.
const WINDOWS: bool = cfg!(windows);

/// `path` with the separators of the platform: on Windows every `/` becomes
/// `\`; elsewhere `path` is returned as is (`\` is a valid name character
/// there).
#[must_use]
pub fn native_separators_for(path: &str, windows: bool) -> String {
    if windows {
        path.replace('/', "\\")
    } else {
        path.to_owned()
    }
}

/// [`native_separators_for`] this platform.
#[must_use]
pub fn native_separators(path: &str) -> String {
    native_separators_for(path, WINDOWS)
}

/// A host path from a stored string (a decoded `file://` URI, a folder
/// setting): with native separators, see [`native_separators`].
#[must_use]
pub fn host_path(stored: &str) -> PathBuf {
    PathBuf::from(native_separators(stored))
}

/// `path` with native separators (a no-op outside Windows and for a path
/// that has none to fix).
#[must_use]
pub fn native(path: PathBuf) -> PathBuf {
    if !WINDOWS {
        return path;
    }
    match path.to_str() {
        Some(s) if s.contains('/') => host_path(s),
        _ => path,
    }
}

/// `rel`, a `/`-separated path relative to a root (a `VfsPath` with or
/// without the leading `/`, an offline copy's folder), joined to the host
/// path `root` one name at a time, so every separator is native. Empty
/// names (`//`, a leading or trailing `/`) are skipped. Only `/` separates
/// names of `rel`: a backslash is a valid name character outside Windows.
#[must_use]
pub fn join_relative(root: &Path, rel: &str) -> PathBuf {
    let mut out = native(root.to_path_buf());
    for name in rel.split('/').filter(|n| !n.is_empty()) {
        out.push(name);
    }
    out
}

/// [`join_relative`] as a string for the given platform, for tests and
/// display: the root's separators are made native and the names of `rel`
/// are appended with the platform's separator.
#[must_use]
pub fn join_relative_for(root: &str, rel: &str, windows: bool) -> String {
    let sep = if windows { '\\' } else { '/' };
    let mut out = native_separators_for(root, windows);
    for name in rel.split('/').filter(|n| !n.is_empty()) {
        if !out.ends_with(sep) {
            out.push(sep);
        }
        out.push_str(name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_root_gets_backslashes() {
        assert_eq!(
            native_separators_for("C:/Users/me/library", true),
            r"C:\Users\me\library"
        );
        assert_eq!(
            native_separators_for(r"C:\Users\me/Kinolune/downloads/offline", true),
            r"C:\Users\me\Kinolune\downloads\offline"
        );
        // UNC share mounted as a local folder.
        assert_eq!(
            native_separators_for("//nas/media/Films", true),
            r"\\nas\media\Films"
        );
    }

    #[test]
    fn posix_paths_are_untouched() {
        for p in ["/home/me/Фильмы", "/srv/a\\b", "/"] {
            assert_eq!(native_separators_for(p, false), p);
        }
    }

    #[test]
    fn file_of_a_windows_source_has_one_separator() {
        // The QA-017 path: root from `file:///C:/Users/me/library`, the
        // file's `VfsPath`.
        assert_eq!(
            join_relative_for(
                "C:/Users/me/library",
                "/Movies/Charge (2022)/Charge (2022) - part1.mkv",
                true
            ),
            r"C:\Users\me\library\Movies\Charge (2022)\Charge (2022) - part1.mkv"
        );
        assert_eq!(
            join_relative_for(r"C:\", "Movies/a.mkv", true),
            r"C:\Movies\a.mkv"
        );
        assert_eq!(join_relative_for("C:/lib", "", true), r"C:\lib");
    }

    #[test]
    fn file_of_a_posix_source() {
        assert_eq!(
            join_relative_for("/srv/media", "Movies//Dune (2021)/Dune.mkv", false),
            "/srv/media/Movies/Dune (2021)/Dune.mkv"
        );
        assert_eq!(join_relative_for("/", "/a.mkv", false), "/a.mkv");
        assert_eq!(
            join_relative(Path::new("/srv/media"), "/Movies/Dune.mkv"),
            if WINDOWS {
                PathBuf::from(r"\srv\media\Movies\Dune.mkv")
            } else {
                PathBuf::from("/srv/media/Movies/Dune.mkv")
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_host_paths() {
        let root = host_path("C:/Users/me/library");
        assert_eq!(root.to_str(), Some(r"C:\Users\me\library"));
        let file = join_relative(
            Path::new("C:/Users/me/library"),
            "/Movies/Charge (2022)/a.mkv",
        );
        assert_eq!(
            file.to_str(),
            Some(r"C:\Users\me\library\Movies\Charge (2022)\a.mkv")
        );
        // Same file as the mixed form the old code produced: an existing
        // library keeps matching its files.
        assert_eq!(
            file,
            PathBuf::from(r"C:/Users/me/library\Movies/Charge (2022)/a.mkv")
        );
        let downloads = join_relative(Path::new(r"C:\Users\me\Kinolune/downloads"), "offline");
        assert_eq!(
            downloads.to_str(),
            Some(r"C:\Users\me\Kinolune\downloads\offline")
        );
    }
}
