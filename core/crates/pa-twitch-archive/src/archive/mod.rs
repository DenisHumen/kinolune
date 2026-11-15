// SPDX-License-Identifier: MPL-2.0
//! The downloader (compiled only with `cfg(pa_twitch_archive)`).

mod bitmap;
pub mod fetch;
pub mod import;
mod job;
pub mod library;
pub mod live;
pub mod meta;
mod mux;
pub mod naming;
pub mod playlist;
mod source;
#[cfg(feature = "testing")]
pub mod testing;

pub use bitmap::SegmentBitmap;
pub use job::{
    DEFAULT_WORKERS, DURATION_TOLERANCE, DownloadRequest, Quality, VodDownloader, kinds,
};
pub use mux::{Container, FfmpegMuxer, MuxJob, Muxer, Probed};
pub use naming::Naming;
pub use playlist::{MediaPlaylist, Segment, Selection, TimeRange};
pub use source::{
    ArchiveError, ArchiveSource, ChaptersFn, ChatReplayFn, HelixAccess, TwitchSource,
};

/// Marker string of this capability in full binaries (the artifact check
/// looks for `pa_twitch_archive`, ADR-0041).
pub const CAPABILITY: &str = "pa_twitch_archive";
