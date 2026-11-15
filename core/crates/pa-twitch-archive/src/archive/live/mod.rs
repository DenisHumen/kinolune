// SPDX-License-Identifier: MPL-2.0
//! Live recording (TWI-018, research 06 §4.2), full builds only.
//!
//! A recording is a `pa-jobs` job of type [`TWITCH_LIVE_RECORD`] at
//! priority `LIVE` with a `target_storage`. The handler, [`LiveRecorder`]:
//!
//! 1. checks the remote-config `twitch.archive` flag and native playback
//!    ([`LiveSource::check_allowed`]) at the start and every minute; off →
//!    nothing is recorded, or what was recorded is finalised;
//! 2. resolves usher's live master playlist (TWI-004) — a refusal
//!    (`usher_403`) is never worked around — and polls the media playlist
//!    every half target duration;
//! 3. appends every new complete segment, deduplicated by
//!    `EXT-X-MEDIA-SEQUENCE`, to part files of `part_minutes` of media;
//!    prefetch segments are left for the next poll. Ad segments are kept as
//!    Twitch serves them and only marked (ADR-0052); `#EXT-X-DISCONTINUITY`
//!    is noted. Sequence numbers that went by unrecorded (an outage longer
//!    than the playlist window, a pause) are a gap in the journal;
//! 4. survives network errors (backoff; the playlist window covers short
//!    blips), asks usher again on `403`/`404` or a stale playlist, and ends
//!    when usher reports the channel offline for `offline_grace`;
//! 5. records the chat alongside (TWI-010) with Twitch's message time and
//!    the live edge's program date-time;
//! 6. finalises: chapters from the TWI-008 timeline (or the categories it
//!    saw in Helix), `TWITCH_*` tags (ads, gaps, recorded span), one remux
//!    into MKV with timestamps folded across discontinuities and gaps
//!    (`RemuxRequest::fold_gaps`), `.chat.json.gz` with offsets on the
//!    recording's timeline, `.info.json`; commits the folder
//!    `<login> [<user_id>]/<year>/<date> LIVE <title> [s<stream_id>]/`.
//!
//! On low space the storage monitor (CORE-012) empties the trash and
//! pauses other jobs first; a live recording is paused only at the hard
//! reserve, and the segments it misses become a gap (`paused`).
//!
//! [`AutoRecord`] queues recordings from TWI-009 live events by the
//! household's record rules.

mod journal;
mod recorder;
mod rules;
mod source;
#[cfg(feature = "testing")]
pub mod testing;
mod timeline;

pub use journal::{AdEntry, CatEntry, ChatEntry, GapEntry, SegEntry};
pub use recorder::{INFO_FORMAT, LiveRecorder, LiveRequest, LiveSettings, TWITCH_LIVE_RECORD};
pub use rules::{AutoDecision, AutoRecord, group_of, quality_of};
pub use source::{
    ChatFn, ChatRecord, ChatSourceFeed, LiveChat, LiveClock, LiveSource, LiveStream, SystemClock,
    TimelineFn, TwitchLiveSource,
};
pub use timeline::{Placed, RecTimeline};
