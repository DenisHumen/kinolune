// SPDX-License-Identifier: MPL-2.0
//! Trickplay seek previews (`pa-trickplay`, PLAY-009) for Dart.
//!
//! [`CoreTrickplay::prepare`] starts the previews of one library file in the
//! background — a BIF or Jellyfin folder next to it, the disk cache, or
//! generation through FFmpeg if allowed — and returns a session at once.
//! The player asks [`CoreTrickplaySession::frame_at`] while the user
//! scrubs; it answers from memory, with the closest finished thumbnail
//! while generation is still running. Network files are read through the
//! same source connections as playback (`pa-vfs`), never mounted.

use std::sync::Arc;

use flutter_rust_bridge::frb;
use pa_catalog::SourceOpener;
use pa_trickplay as tp;
use pa_vfs::VfsPath;

use crate::api::library::CoreLibrary;

/// Error of a trickplay call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrickplayError {
    /// Stable code (CORE-009): `io` (the preview cache), `invalid` (a bad
    /// path) or `unreachable` (the media file cannot be opened).
    pub code: String,
    /// Detail for logs (no secrets).
    pub message: String,
}

/// A failure of the preview cache.
fn err(e: impl std::fmt::Display) -> TrickplayError {
    err_as("io")(e)
}

fn err_as<E: std::fmt::Display>(code: &'static str) -> impl Fn(E) -> TrickplayError {
    move |e| TrickplayError {
        code: code.into(),
        message: e.to_string(),
    }
}

/// Where the previews of a session come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreTrickplayOrigin {
    /// A BIF next to the media (Plex, Emby).
    SidecarBif,
    /// A Jellyfin trickplay folder next to the media.
    Jellyfin,
    /// Generated earlier and cached.
    Cache,
    /// Generated in this session.
    Generated,
}

/// Phase of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreTrickplayPhase {
    /// Looking for existing previews.
    Searching,
    /// Generating; frames are already served from the finished ones.
    Generating,
    /// Complete.
    Ready,
    /// No previews (generation off, no FFmpeg, no video, an error).
    Unavailable,
}

/// State of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTrickplayStatus {
    pub phase: CoreTrickplayPhase,
    /// Set when [`CoreTrickplayPhase::Ready`].
    pub origin: Option<CoreTrickplayOrigin>,
    /// Thumbnails done and planned while generating.
    pub done: u32,
    pub total: u32,
    /// Time between previews, once known.
    pub interval_ms: Option<u64>,
    /// Why previews are unavailable.
    pub reason: Option<String>,
}

/// One preview: a JPEG, or a sheet of tiles with the tile to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTrickplayFrame {
    /// JPEG bytes.
    pub jpeg: Vec<u8>,
    /// Equal ids within a session mean equal bytes: a UI may keep the
    /// decoded image (Jellyfin sheets hold many tiles).
    pub image_id: u64,
    /// The tile inside the image in pixels; all zero for the whole image.
    pub crop_x: u32,
    pub crop_y: u32,
    pub crop_width: u32,
    pub crop_height: u32,
    /// Time the preview shows.
    pub position_ms: u64,
}

/// Size of the disk cache of generated previews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreTrickplayUsage {
    pub bytes: u64,
    pub files: u64,
    pub max_bytes: u64,
}

/// The preview service of the app, over the sources of one library.
#[frb(opaque)]
pub struct CoreTrickplay {
    service: Arc<tp::Trickplay>,
    opener: Arc<SourceOpener>,
}

impl CoreTrickplay {
    /// Opens the service for `library`, caching generated previews in
    /// `cache_dir` (the platform cache directory, e.g. `<cache>/trickplay`)
    /// up to `max_bytes`. `generate`: create previews when a file has none
    /// (a user option on clients).
    pub fn open(
        library: &CoreLibrary,
        cache_dir: String,
        max_bytes: u64,
        generate: bool,
    ) -> Result<CoreTrickplay, TrickplayError> {
        let options = tp::TrickplayOptions {
            generate,
            cache_max_bytes: max_bytes,
            ..tp::TrickplayOptions::default()
        };
        Ok(Self {
            service: Arc::new(tp::Trickplay::open(cache_dir, options).map_err(err)?),
            opener: library.opener(),
        })
    }

    /// Turns generation on or off for files prepared from now on.
    #[frb(sync)]
    pub fn set_generate(&self, generate: bool) {
        self.service.set_generate(generate);
    }

    /// Starts the previews of the file `rel_path` of source `source_id` (as
    /// in `CoreItem`); `duration_ms` from the library helps with Jellyfin
    /// folders. Opening the file may take a moment on a network source.
    pub fn prepare(
        &self,
        source_id: i64,
        rel_path: String,
        duration_ms: Option<u64>,
    ) -> Result<CoreTrickplaySession, TrickplayError> {
        let path = VfsPath::parse(&rel_path).map_err(err_as("invalid"))?;
        // Through the opener first: it reconnects a stale source (and the
        // player opens the same file anyway).
        self.opener
            .open_file(source_id, &path)
            .map_err(err_as("unreachable"))?;
        let vfs = self
            .opener
            .vfs(source_id)
            .map_err(err_as("unreachable"))?;
        // A handle of its own: generation reads it without the read-ahead
        // the playback handle (stream proxy) uses.
        let source: Arc<dyn pa_vfs::ByteSource> = Arc::from(vfs.open(&path).map_err(err_as("unreachable"))?);
        // The content's identity: a replaced file gets new previews.
        let modified = vfs.stat(&path).ok().and_then(|m| m.modified_ms);
        let key = format!(
            "{source_id}\n{}\n{}\n{}",
            path.as_str(),
            source.size(),
            modified.unwrap_or_default()
        );
        let session = self.service.prepare(tp::Media {
            key,
            source,
            sidecar: Some((vfs, path)),
            duration_ms: duration_ms.filter(|d| *d > 0),
        });
        Ok(CoreTrickplaySession { inner: session })
    }

    /// Size of the cache.
    #[frb(sync)]
    pub fn usage(&self) -> CoreTrickplayUsage {
        let u = self.service.usage();
        CoreTrickplayUsage {
            bytes: u.bytes,
            files: u.files,
            max_bytes: u.max_bytes,
        }
    }

    /// Changes the size limit and trims the cache at once.
    pub fn set_max_bytes(&self, max_bytes: u64) {
        self.service.cache().set_max_bytes(max_bytes);
    }

    /// Removes all generated previews.
    pub fn clear(&self) -> Result<(), TrickplayError> {
        self.service.cache().clear().map_err(err)
    }
}

/// Previews of one file. Dropping it (Dart: `dispose`) stops an unfinished
/// generation unless another session of the same file is still open.
#[frb(opaque)]
pub struct CoreTrickplaySession {
    inner: Arc<tp::Session>,
}

impl Drop for CoreTrickplaySession {
    fn drop(&mut self) {
        self.inner.release();
    }
}

impl CoreTrickplaySession {
    /// Current state.
    #[frb(sync)]
    pub fn status(&self) -> CoreTrickplayStatus {
        let interval_ms = self.inner.interval_ms();
        let base = CoreTrickplayStatus {
            phase: CoreTrickplayPhase::Searching,
            origin: None,
            done: 0,
            total: 0,
            interval_ms,
            reason: None,
        };
        match self.inner.status() {
            tp::Status::Searching => base,
            tp::Status::Generating { done, total } => CoreTrickplayStatus {
                phase: CoreTrickplayPhase::Generating,
                done: u32::try_from(done).unwrap_or(u32::MAX),
                total: u32::try_from(total).unwrap_or(u32::MAX),
                ..base
            },
            tp::Status::Ready(origin) => CoreTrickplayStatus {
                phase: CoreTrickplayPhase::Ready,
                origin: Some(match origin {
                    tp::Origin::SidecarBif => CoreTrickplayOrigin::SidecarBif,
                    tp::Origin::Jellyfin => CoreTrickplayOrigin::Jellyfin,
                    tp::Origin::Cache => CoreTrickplayOrigin::Cache,
                    tp::Origin::Generated => CoreTrickplayOrigin::Generated,
                }),
                ..base
            },
            tp::Status::Unavailable(reason) => CoreTrickplayStatus {
                phase: CoreTrickplayPhase::Unavailable,
                reason: Some(reason),
                ..base
            },
        }
    }

    /// The preview for `position_ms`, or `None` if there is none (yet).
    pub fn frame_at(&self, position_ms: u64) -> Option<CoreTrickplayFrame> {
        let f = self.inner.frame_at(position_ms)?;
        let crop = f.crop.unwrap_or(tp::Crop {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        });
        Some(CoreTrickplayFrame {
            jpeg: f.image.to_vec(),
            image_id: f.image_id,
            crop_x: crop.x,
            crop_y: crop.y,
            crop_width: crop.width,
            crop_height: crop.height,
            position_ms: f.position_ms,
        })
    }

    /// Stops an unfinished generation now (nothing is cached).
    pub fn cancel(&self) {
        self.inner.cancel();
    }
}
