// SPDX-License-Identifier: MPL-2.0
//! The remux and the duration check, over the runtime-loaded LGPL FFmpeg.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pa_probe::{Ffmpeg, ProbeErrorKind, ProbeOptions, RemuxAttachment, RemuxFormat, RemuxRequest};
use pa_vfs::{LocalFs, Vfs, VfsPath};
use serde::{Deserialize, Serialize};

use super::source::ArchiveError;

/// Output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    /// Matroska: chapters, tags, attachments; plays everywhere our player
    /// runs (06 §4.1).
    #[default]
    Mkv,
    /// MP4 with `+faststart`, for AVPlayer and old TVs.
    Mp4,
}

impl Container {
    pub fn extension(self) -> &'static str {
        self.format().extension()
    }

    fn format(self) -> RemuxFormat {
        match self {
            Container::Mkv => RemuxFormat::Matroska,
            Container::Mp4 => RemuxFormat::Mp4,
        }
    }
}

/// One remux.
pub struct MuxJob<'a> {
    /// Segment files in order (an fMP4 init segment first).
    pub inputs: &'a [PathBuf],
    pub output: &'a Path,
    pub container: Container,
    /// The `ffmetadata` file.
    pub metadata: &'a Path,
    pub attachments: &'a [RemuxAttachment],
    /// Cut the output here (ms), for a range ending inside a segment.
    pub cut_end_ms: Option<u64>,
    pub cancel: &'a dyn Fn() -> bool,
    /// Media time written so far, ms.
    pub progress: &'a dyn Fn(u64),
}

/// What the check of a finished file saw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Probed {
    pub duration_ms: Option<u64>,
    pub chapters: usize,
    pub attachments: usize,
    /// Container tags, keys in lower case.
    pub tags: BTreeMap<String, String>,
}

/// Remux and verification. [`FfmpegMuxer`] in the apps; tests without
/// FFmpeg use a double (`testing::ConcatMuxer`).
pub trait Muxer: Send + Sync {
    /// Remuxes; returns the written duration in ms.
    fn remux(&self, job: &MuxJob<'_>) -> Result<u64, ArchiveError>;

    /// Reads the finished file back (the `ffprobe` step of 06 §4.1).
    fn probe(&self, path: &Path) -> Result<Probed, ArchiveError>;
}

/// [`Muxer`] over `pa_probe::Ffmpeg` (LGPL FFmpeg loaded at run time from
/// `$PA_FFMPEG_LIB_DIR` or the platform loader; never linked).
#[derive(Debug, Default, Clone, Copy)]
pub struct FfmpegMuxer;

fn probe_error(e: &pa_probe::ProbeError) -> ArchiveError {
    match e.kind() {
        ProbeErrorKind::LibraryUnavailable => ArchiveError::fatal("ffmpeg_unavailable", e.message()),
        ProbeErrorKind::Cancelled => ArchiveError::retry("cancelled", e.message()),
        ProbeErrorKind::InvalidData => ArchiveError::fatal("remux_failed", e.message()),
        _ => ArchiveError::retry("remux_io", e.message()),
    }
}

impl Muxer for FfmpegMuxer {
    fn remux(&self, job: &MuxJob<'_>) -> Result<u64, ArchiveError> {
        let ff = Ffmpeg::shared().map_err(|e| probe_error(&e))?;
        let report = ff
            .remux(&RemuxRequest {
                inputs: job.inputs,
                output: job.output,
                format: job.container.format(),
                metadata: Some(job.metadata),
                attachments: job.attachments,
                end_ms: job.cut_end_ms,
                cancel: Some(job.cancel),
                progress: Some(job.progress),
            })
            .map_err(|e| probe_error(&e))?;
        Ok(report.duration_ms)
    }

    fn probe(&self, path: &Path) -> Result<Probed, ArchiveError> {
        let io = |e: pa_vfs::VfsError| ArchiveError::retry("io", e.to_string());
        let (dir, name) = (
            path.parent().unwrap_or(Path::new("/")),
            path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        );
        let fs = LocalFs::new(dir).map_err(io)?;
        let file = fs
            .open(&VfsPath::parse(&format!("/{name}")).map_err(io)?)
            .map_err(io)?;
        let info = pa_probe::probe(Arc::from(file), &ProbeOptions::default())
            .map_err(|e| probe_error(&e))?;
        Ok(Probed {
            duration_ms: info.duration_ms,
            chapters: info.chapters.len(),
            attachments: info.attachments.len() + info.video.iter().filter(|v| v.cover_art).count(),
            tags: info.tags,
        })
    }
}
