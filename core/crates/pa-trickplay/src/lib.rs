// SPDX-License-Identifier: MPL-2.0
//! Trickplay previews: the picture under the finger while scrubbing
//! (PLAY-009, `docs/research/01` §1.5).
//!
//! A [`Session`] per media file answers [`Session::frame_at`] — the JPEG
//! for a position — from the best origin available, found on a background
//! thread by [`Trickplay::prepare`]:
//!
//! 1. **Sidecars** next to the media, read through `pa-vfs` (so local,
//!    SMB and WebDAV alike): a BIF of Plex/Emby/Roku or a Jellyfin
//!    trickplay folder ([`sidecar`]).
//! 2. **Our cache**: a BIF per file in an LRU directory with a size limit
//!    ([`cache`]).
//! 3. **Generation**, if enabled: keyframes every 10 s (longer for very
//!    long media), decoded by FFmpeg — LGPL, loaded at run time through
//!    `pa-probe` — straight from the [`pa_vfs::ByteSource`], several
//!    workers in parallel, coarse to fine so previews exist across the
//!    whole timeline within seconds ([`generate`]); encoded as 320 px JPEG
//!    and cached when complete.
//!
//! Lookups are memory reads (a Jellyfin sheet is read once, then kept),
//! well under the 100 ms a scrubbing UI allows.
//!
//! Clients generate lazily (when a file is opened) and only if the user
//! allows it ([`TrickplayOptions::generate`]); a server generates ahead as
//! a background job ([`Trickplay::ensure`], job kind [`JOB_KIND`]).
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::time::Duration;
//! use pa_trickplay::{Media, Trickplay, TrickplayOptions};
//! use pa_vfs::{LocalFs, Vfs, VfsPath};
//!
//! let vfs: Arc<dyn Vfs> = Arc::new(LocalFs::new("/media")?);
//! let path = VfsPath::parse("/Movies/Dune (2021).mkv")?;
//! let service = Trickplay::open("/tmp/trickplay", TrickplayOptions::default())?;
//! let session = service.prepare(Media {
//!     key: format!("local:{}", path.as_str()),
//!     source: Arc::from(vfs.open(&path)?),
//!     sidecar: Some((vfs, path)),
//!     duration_ms: None,
//! });
//! session.wait(Duration::from_secs(5));
//! if let Some(frame) = session.frame_at(42 * 60 * 1000) {
//!     println!("{} bytes of JPEG at {} ms", frame.image.len(), frame.position_ms);
//! }
//! session.release();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod bif;
pub mod cache;
pub mod generate;
pub mod jpeg;
pub mod service;
pub mod sidecar;

pub use cache::{TrickplayCache, Usage};
pub use generate::{GenerateOptions, Generated, LiveFrames};
pub use service::{Frame, Media, Origin, Session, Status, Trickplay, TrickplayOptions};
pub use sidecar::Crop;

/// `pa-jobs` job type of server-side generation (params: the file).
pub const JOB_KIND: &str = "trickplay";

/// Error of preview work.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// FFmpeg is missing or cannot open or decode the file.
    #[error(transparent)]
    Probe(#[from] pa_probe::ProbeError),
    /// Reading the source failed.
    #[error("source: {0}")]
    Source(#[from] pa_vfs::VfsError),
    /// The disk cache failed.
    #[error("cache: {0}")]
    Io(#[from] std::io::Error),
    /// JPEG encoding failed.
    #[error("jpeg: {0}")]
    Encode(String),
    /// The file cannot have previews (no duration, oversized sidecar…).
    #[error("{0}")]
    Invalid(String),
    /// Stopped by [`Session::cancel`].
    #[error("cancelled")]
    Cancelled,
}
