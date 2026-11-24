// SPDX-License-Identifier: MPL-2.0
//! Lyrics kept with a music file (MUS-006): the embedded `SYLT` / `USLT`
//! or lyrics tag and the `.lrc` file next to the track, read from any
//! source through `pa-vfs` ([`pa_lyrics`]). The client parses LRC, asks
//! LRCLIB and picks the best of them.

use pa_catalog as c;
use pa_vfs::{ErrorKind, VfsPath};

use crate::api::library::{CoreLibrary, LibraryError};

type Result<T> = std::result::Result<T, LibraryError>;

/// A line of synchronised lyrics (`SYLT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLyricLine {
    pub time_ms: u32,
    pub text: String,
}

/// What a music file carries: all empty when it has no lyrics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoreTrackLyrics {
    /// `SYLT` lines by time.
    pub synced: Vec<CoreLyricLine>,
    /// The plain lyrics tag; may hold LRC text.
    pub plain: Option<String>,
    /// The `.lrc` file next to the track.
    pub lrc: Option<String>,
}

impl From<pa_lyrics::FileLyrics> for CoreTrackLyrics {
    fn from(l: pa_lyrics::FileLyrics) -> Self {
        Self {
            synced: l
                .synced
                .into_iter()
                .map(|s| CoreLyricLine {
                    time_ms: s.time_ms,
                    text: s.text,
                })
                .collect(),
            plain: l.plain,
            lrc: l.lrc,
        }
    }
}

fn error(code: &str, message: impl std::fmt::Display) -> LibraryError {
    LibraryError::new(code, format!("{code}: {message}"))
}

impl CoreLibrary {
    /// The lyrics stored with the file `rel_path` of source `source_id`
    /// (as in `CoreItem`): embedded tags and the `.lrc` next to it. A file
    /// without lyrics gives an empty result; fails with `unavailable` when
    /// the source cannot be reached.
    pub fn track_lyrics(&self, source_id: i64, rel_path: String) -> Result<CoreTrackLyrics> {
        let path = VfsPath::parse(&rel_path).map_err(|e| error("invalid", e))?;
        let vfs = self.catalog().connect(source_id).map_err(|e| match e {
            c::Error::NotFound(m) => error("not_found", m),
            e => error("unavailable", e),
        })?;
        pa_lyrics::read_lyrics(vfs.as_ref(), &path)
            .map(Into::into)
            .map_err(|e| match e.kind() {
                ErrorKind::PermissionDenied | ErrorKind::AuthenticationFailed => error("denied", e),
                _ => error("unavailable", e),
            })
    }
}
