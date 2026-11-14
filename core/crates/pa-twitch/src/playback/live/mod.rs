// SPDX-License-Identifier: MPL-2.0
//! Native live playback: low latency and ads "as is" (TWI-006, 06 §2.4,
//! §6.4), full builds only.
//!
//! - [`MediaPlaylist`]: Twitch live media playlists — program date-times,
//!   prefetch segments (`#EXT-X-TWITCH-PREFETCH`), standard LL-HLS tags,
//!   ad breaks from `#EXT-X-DATERANGE` (class `twitch-stitched-ad`).
//! - [`LiveProxy`] (with the `reqwest` feature for [`ReqwestUpstream`]): a
//!   loopback server the player opens instead of the Twitch URL. It plays
//!   prefetch segments in [`LiveMode::LowLatency`], streams every segment
//!   to the player byte for byte and reports latency, live edge and the ad
//!   under the playhead ([`LiveReport`]).
//! - [`CatchUpPolicy`]: playback rate to get back to the target latency
//!   (never during an ad).
//!
//! # Ads
//!
//! Twitch stitches ads into the stream (SSAI). They are played in full, as
//! Twitch serves them: no segment is dropped, replaced, skipped or sped up,
//! no request parameter is changed to avoid them. The markers are read only
//! so the UI can show "Ad · 0:15", block seeking during the break and not
//! count it as watched content.

mod latency;
mod playlist;
mod proxy;

pub use latency::{CatchUpPolicy, latency_seconds};
pub use playlist::{
    AdBreak, DateRange, LiveMode, MediaPlaylist, Segment, TWITCH_AD_CLASS, format_date_time,
    resolve_uri,
};
#[cfg(feature = "reqwest")]
pub use proxy::ReqwestUpstream;
pub use proxy::{
    AdStatus, DeliveredSegment, LiveProxy, LiveReport, LiveTotals, LiveUpstream, LowLatencyInfo,
    SegmentBody,
};
