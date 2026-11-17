// SPDX-License-Identifier: MPL-2.0
//! The content kind of files and folders (LIB-025, ADR-0099) for Dart:
//! «Мои видео» (footage from phones and cameras, never recognized) and
//! «Это не то» on a folder. The calls are methods of [`CoreLibrary`].
//! Kinds are `video`, `own_video`, `music` and `audiobook`.

use pa_catalog as c;

use crate::api::library::{CoreLibrary, LibraryError};

type Result<T> = std::result::Result<T, LibraryError>;

/// A video of «Мои видео».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreOwnVideo {
    pub file_id: i64,
    pub source_id: i64,
    /// Path relative to the source root.
    pub rel_path: String,
    /// Absolute path for a local source.
    pub local_path: Option<String>,
    /// The file name without its extension.
    pub title: String,
    pub size_bytes: i64,
    pub duration_ms: Option<i64>,
    /// When the file was modified (shot), else added.
    pub taken_at_ms: i64,
}

impl From<c::OwnVideo> for CoreOwnVideo {
    fn from(v: c::OwnVideo) -> Self {
        Self {
            file_id: v.file_id,
            source_id: v.source_id,
            rel_path: v.rel_path,
            local_path: v.local_path,
            title: v.title,
            size_bytes: v.size_bytes,
            duration_ms: v.duration_ms,
            taken_at_ms: v.taken_at_ms,
        }
    }
}

/// What a subfolder holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFolderKind {
    /// Relative to the source root, no leading slash.
    pub path: String,
    /// `video`, `own_video`, `music` or `audiobook`: most of its files.
    pub kind: String,
    /// The user said «Это не то» on it or a folder above it.
    pub overridden: bool,
}

impl CoreLibrary {
    /// «Мои видео»: footage from phones and cameras in the main library,
    /// newest first.
    pub fn own_videos(&self) -> Result<Vec<CoreOwnVideo>> {
        Ok(self
            .catalog()
            .own_videos()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The preview frame (JPEG) of a video of «Мои видео» (UI-032), made
    /// from the file on first use and cached in the app's folder; `None`
    /// when the file has no frame worth showing.
    pub fn own_video_frame(&self, file_id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self.catalog().video_frame(
            file_id,
            &c::frame_search::FfmpegFrames {
                width: c::video_frames::FRAME_WIDTH,
            },
        )?)
    }

    /// What the subfolders of `dir` (relative, `""` for the root) of
    /// source `source_id` hold.
    pub fn folder_kinds(&self, source_id: i64, dir: String) -> Result<Vec<CoreFolderKind>> {
        Ok(self
            .catalog()
            .folder_kinds(source_id, &dir)?
            .into_iter()
            .map(|k| CoreFolderKind {
                path: k.path,
                kind: k.kind.as_str().into(),
                overridden: k.overridden,
            })
            .collect())
    }

    /// «Это не то» on folder `path` (relative, `""` for the whole source):
    /// its files become the other kind of their medium and the folder
    /// keeps it; again, and what was detected is back. Returns the new
    /// kind, `None` for a folder without media.
    pub fn not_this(&self, source_id: i64, path: String) -> Result<Option<String>> {
        Ok(self
            .catalog()
            .not_this(source_id, &path)?
            .map(|k| k.as_str().to_owned()))
    }
}
