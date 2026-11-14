// SPDX-License-Identifier: MPL-2.0
//! VOD chapters and categories (TWI-008, research 06 §2.3).
//!
//! A chapter is the category (game) that was on air during a part of a
//! VOD. Three sources, from the most to the least official:
//!
//! 1. **Our own timeline** ([`ChapterSource::EventSub`],
//!    [`ChapterSource::Polling`]). For monitored channels a
//!    [`TimelineTracker`] turns `stream.online`, `channel.update` and
//!    `stream.offline` (EventSub WebSocket, [`parse_eventsub_message`]) or
//!    `GET /streams` polling ([`TimelineTracker::observe_poll`]) into a
//!    [`LiveTimeline`]: category segments with wall-clock times. After the
//!    broadcast [`HelixClient::archive_for_stream`] finds its archive VOD
//!    and [`LiveTimeline::vod_chapters`] maps the segments onto it:
//!    `offset = segment start − stream start`. Only documented Helix and
//!    EventSub; works in every build.
//! 2. **GQL `moments`** ([`ChapterSource::GqlMoments`]) for any VOD, only
//!    when the signed remote config turns `twitch.chapters_gql` on and
//!    carries the persisted-query hash of [`CHAPTERS_OPERATION`]
//!    ([`ChaptersGql`]); never a compiled-in hash or client-id. The GQL code
//!    exists only in full builds (`twitch-native` capability, ADR-0041).
//!    Best effort: a failure falls through to the next source.
//! 3. **Fallback** ([`ChapterSource::Fallback`]): one chapter with the
//!    channel's current category (`GET /channels`), only for a fresh archive
//!    ([`FALLBACK_MAX_AGE_MS`]); the UI marks it "≈".
//!
//! [`ChapterResolver::resolve`] picks the source for one VOD. Chapters from
//! our timeline or GQL are never asked for again while they are stored
//! (the `stored` argument); only a fallback chapter may be upgraded.
//!
//! With the `library` feature, [`store`] writes timelines, VODs, chapters
//! and their categories into the TWI-007 model of `pa-library`, where
//! `pa_library::twitch::channel_videos` and `category_videos` filter VODs by
//! a category of their chapters, not by the channel's current category.

#[cfg(pa_twitch_native)]
mod gql;
#[cfg(feature = "library")]
pub mod store;

use std::collections::BTreeMap;
use std::sync::Arc;

use pa_remote_config::{ClientContext, RemoteConfig};
use serde::Deserialize;

use crate::chat_api::EventSubSubscriptionRequest;
use crate::client::{HelixClient, VideosQuery};
use crate::model::{ChannelInfo, Stream, Video, VideoType};
use crate::playback::Flavor;
use crate::transport::Transport;
use crate::util::parse_rfc3339_ms;

/// Name of the GQL operation whose persisted-query hash the remote config
/// must carry in `twitch.persisted_queries` for chapters from GQL.
pub const CHAPTERS_OPERATION: &str = "VideoPlayer_ChapterSelectButtonVideo";

/// The fallback chapter (current category) is offered only for an archive
/// that ended (or is still growing) at most this long ago: 6 hours. Older
/// VODs get no chapters rather than a likely wrong one.
pub const FALLBACK_MAX_AGE_MS: i64 = 6 * 60 * 60 * 1000;

/// Where a chapter came from. [`ChapterSource::as_str`] is the
/// `twitch_category_segments.source` value in `pa-library`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChapterSource {
    /// Our timeline from EventSub notifications.
    EventSub,
    /// Our timeline from `GET /streams` polling (category changes are seen
    /// up to one polling interval late).
    Polling,
    /// GQL `moments` of the VOD.
    GqlMoments,
    /// The channel's current category: approximate ("≈").
    Fallback,
}

impl ChapterSource {
    /// Stored name: `eventsub`, `polling`, `gql_moments`, `fallback`.
    pub fn as_str(self) -> &'static str {
        match self {
            ChapterSource::EventSub => "eventsub",
            ChapterSource::Polling => "polling",
            ChapterSource::GqlMoments => "gql_moments",
            ChapterSource::Fallback => "fallback",
        }
    }

    /// Inverse of [`ChapterSource::as_str`].
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "eventsub" => ChapterSource::EventSub,
            "polling" => ChapterSource::Polling,
            "gql_moments" => ChapterSource::GqlMoments,
            "fallback" => ChapterSource::Fallback,
            _ => return None,
        })
    }

    /// The UI shows "≈" next to such a chapter.
    pub fn is_approximate(self) -> bool {
        self == ChapterSource::Fallback
    }

    /// Our own timeline (EventSub or polling): the preferred source.
    pub fn is_own_timeline(self) -> bool {
        matches!(self, ChapterSource::EventSub | ChapterSource::Polling)
    }
}

/// A category as a chapter knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterGame {
    /// Twitch category id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Box art URL or template, when the source has one.
    pub box_art_url: Option<String>,
}

impl ChapterGame {
    /// A category from an id and a name; `None` for an empty id ("no
    /// category").
    pub fn new(id: &str, name: &str) -> Option<Self> {
        let id = id.trim();
        (!id.is_empty()).then(|| ChapterGame {
            id: id.to_owned(),
            name: name.trim().to_owned(),
            box_art_url: None,
        })
    }
}

/// A chapter of a VOD (or of a live session): offsets from its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// The category on air.
    pub game: ChapterGame,
    /// Stream title when the chapter started, if known.
    pub title: Option<String>,
    /// Start offset.
    pub start_ms: u64,
    /// End offset; `None` while still on air.
    pub end_ms: Option<u64>,
    /// Where it came from.
    pub source: ChapterSource,
}

impl Chapter {
    /// Show "≈" next to it.
    pub fn is_approximate(&self) -> bool {
        self.source.is_approximate()
    }
}

// --- Own timeline ------------------------------------------------------------

/// A category segment of a live session, in wall-clock time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineSegment {
    /// Category; `None` while the channel has no category set.
    pub game: Option<ChapterGame>,
    /// Stream title at the start of the segment.
    pub title: Option<String>,
    /// Unix ms.
    pub started_at_ms: i64,
    /// Unix ms; `None` while on air.
    pub ended_at_ms: Option<i64>,
    /// EventSub or polling.
    pub source: ChapterSource,
}

/// Our category timeline of one broadcast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTimeline {
    /// Stream id (the archive VOD has the same `stream_id`).
    pub stream_id: String,
    /// Broadcaster id.
    pub user_id: String,
    /// Unix ms (`started_at` of the stream).
    pub started_at_ms: i64,
    /// Unix ms; `None` while live.
    pub ended_at_ms: Option<i64>,
    /// Latest title.
    pub title_last: Option<String>,
    /// Segments by start, not overlapping.
    pub segments: Vec<TimelineSegment>,
}

impl LiveTimeline {
    /// A broadcast that started at `started_at_ms`, category not known yet.
    pub fn new(stream_id: &str, user_id: &str, started_at_ms: i64) -> Self {
        Self {
            stream_id: stream_id.to_owned(),
            user_id: user_id.to_owned(),
            started_at_ms,
            ended_at_ms: None,
            title_last: None,
            segments: Vec::new(),
        }
    }

    /// Records the category on air from `at_ms`. The first known category
    /// covers the broadcast from its start (streamers pick it before going
    /// live). Returns whether anything changed.
    pub fn set_category(
        &mut self,
        game: Option<ChapterGame>,
        title: Option<String>,
        at_ms: i64,
        source: ChapterSource,
    ) -> bool {
        if self.ended_at_ms.is_some() {
            return false;
        }
        let mut changed = false;
        if title.is_some() && title != self.title_last {
            self.title_last.clone_from(&title);
            changed = true;
        }
        let at_ms = at_ms.max(self.started_at_ms);
        match self.segments.last_mut() {
            Some(last) if last.ended_at_ms.is_none() => {
                let same = last.game.as_ref().map(|g| &g.id) == game.as_ref().map(|g| &g.id);
                if same {
                    // Same category: keep the segment, learn a missing name or art.
                    if let (Some(old), Some(new)) = (last.game.as_mut(), game) {
                        if old.name.is_empty() && !new.name.is_empty() {
                            old.name = new.name;
                            changed = true;
                        }
                        if old.box_art_url.is_none() && new.box_art_url.is_some() {
                            old.box_art_url = new.box_art_url;
                            changed = true;
                        }
                    }
                    return changed;
                }
                let at_ms = at_ms.max(last.started_at_ms);
                last.ended_at_ms = Some(at_ms);
                self.segments.push(TimelineSegment {
                    game,
                    title,
                    started_at_ms: at_ms,
                    ended_at_ms: None,
                    source,
                });
            }
            Some(last) => {
                let at_ms = at_ms.max(last.ended_at_ms.unwrap_or(at_ms));
                self.segments.push(TimelineSegment {
                    game,
                    title,
                    started_at_ms: at_ms,
                    ended_at_ms: None,
                    source,
                });
            }
            None => self.segments.push(TimelineSegment {
                game,
                title,
                started_at_ms: self.started_at_ms,
                ended_at_ms: None,
                source,
            }),
        }
        true
    }

    /// Ends the broadcast at `at_ms` and closes the open segment.
    pub fn end(&mut self, at_ms: i64) {
        if self.ended_at_ms.is_some() {
            return;
        }
        let at_ms = at_ms.max(self.started_at_ms);
        if let Some(last) = self.segments.last_mut()
            && last.ended_at_ms.is_none()
        {
            last.ended_at_ms = Some(at_ms.max(last.started_at_ms));
        }
        self.ended_at_ms = Some(at_ms);
    }

    /// Chapters of the live session (offsets from the stream start);
    /// segments without a category are left out.
    pub fn chapters(&self) -> Vec<Chapter> {
        let base = self.started_at_ms;
        let offset = |t: i64| u64::try_from(t.saturating_sub(base)).unwrap_or(0);
        self.segments
            .iter()
            .filter_map(|s| {
                Some(Chapter {
                    game: s.game.clone()?,
                    title: s.title.clone(),
                    start_ms: offset(s.started_at_ms),
                    end_ms: s.ended_at_ms.map(offset),
                    source: s.source,
                })
            })
            .collect()
    }

    /// Chapters of the archive VOD of this broadcast, `duration_ms` long
    /// (0 = unknown, no clamping): `offset = segment start − stream start`,
    /// clamped to the VOD; the open segment runs to its end.
    pub fn vod_chapters(&self, duration_ms: u64) -> Vec<Chapter> {
        let mut out = Vec::new();
        for mut chapter in self.chapters() {
            if duration_ms > 0 {
                if chapter.start_ms >= duration_ms {
                    continue;
                }
                chapter.end_ms = Some(chapter.end_ms.map_or(duration_ms, |e| e.min(duration_ms)));
            }
            if chapter.end_ms == Some(chapter.start_ms) {
                continue;
            }
            out.push(chapter);
        }
        out
    }
}

/// What happened on a monitored channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineEvent {
    /// The channel went live (`stream.online`).
    Online {
        /// Broadcaster id.
        user_id: String,
        /// Stream id.
        stream_id: String,
        /// Unix ms.
        started_at_ms: i64,
    },
    /// Category or title set (`channel.update`, or a poll).
    Update {
        /// Broadcaster id.
        user_id: String,
        /// New category; `None` = no category.
        game: Option<ChapterGame>,
        /// New title.
        title: Option<String>,
        /// When it changed (Unix ms).
        at_ms: i64,
    },
    /// The broadcast ended (`stream.offline`).
    Offline {
        /// Broadcaster id.
        user_id: String,
        /// Unix ms.
        at_ms: i64,
    },
}

impl TimelineEvent {
    /// The channel's category from `GET /channels`, observed at `at_ms`:
    /// seeds a tracker that starts while the channel is already live or
    /// before its first `channel.update`.
    pub fn from_channel(channel: &ChannelInfo, at_ms: i64) -> Self {
        TimelineEvent::Update {
            user_id: channel.broadcaster_id.clone(),
            game: ChapterGame::new(&channel.game_id, &channel.game_name),
            title: (!channel.title.is_empty()).then(|| channel.title.clone()),
            at_ms,
        }
    }

    /// Broadcaster id.
    pub fn user_id(&self) -> &str {
        match self {
            TimelineEvent::Online { user_id, .. }
            | TimelineEvent::Update { user_id, .. }
            | TimelineEvent::Offline { user_id, .. } => user_id,
        }
    }
}

/// A timeline the caller should store (`store::store_live_timeline`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineChange {
    /// A broadcast started.
    Started(LiveTimeline),
    /// The category or title of a live broadcast changed.
    Updated(LiveTimeline),
    /// A broadcast ended: find its VOD
    /// ([`HelixClient::archive_for_stream`]) and resolve its chapters.
    Ended(LiveTimeline),
}

impl TimelineChange {
    /// The timeline in its new state.
    pub fn timeline(&self) -> &LiveTimeline {
        match self {
            TimelineChange::Started(t) | TimelineChange::Updated(t) | TimelineChange::Ended(t) => t,
        }
    }
}

#[derive(Debug, Default)]
struct ChannelState {
    /// Last known category and title, also while offline.
    category: Option<(Option<ChapterGame>, Option<String>)>,
    live: Option<LiveTimeline>,
}

/// Category timelines of the monitored channels, fed by EventSub events or
/// by polling.
#[derive(Debug, Default)]
pub struct TimelineTracker {
    channels: BTreeMap<String, ChannelState>,
}

impl TimelineTracker {
    /// Empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Continues a stored live timeline (after a restart). Ended timelines
    /// are ignored.
    pub fn resume(&mut self, timeline: LiveTimeline) {
        if timeline.ended_at_ms.is_some() {
            return;
        }
        let state = self.channels.entry(timeline.user_id.clone()).or_default();
        if let Some(last) = timeline.segments.last() {
            state.category = Some((last.game.clone(), timeline.title_last.clone()));
        }
        state.live = Some(timeline);
    }

    /// The live timeline of a channel.
    pub fn live(&self, user_id: &str) -> Option<&LiveTimeline> {
        self.channels.get(user_id)?.live.as_ref()
    }

    /// Applies one event; `source` is EventSub or polling.
    pub fn apply(&mut self, event: TimelineEvent, source: ChapterSource) -> Vec<TimelineChange> {
        let state = self.channels.entry(event.user_id().to_owned()).or_default();
        let mut changes = Vec::new();
        match event {
            TimelineEvent::Online {
                user_id,
                stream_id,
                started_at_ms,
            } => {
                if state
                    .live
                    .as_ref()
                    .is_some_and(|l| l.stream_id == stream_id)
                {
                    return changes;
                }
                if let Some(mut old) = state.live.take() {
                    old.end(started_at_ms);
                    changes.push(TimelineChange::Ended(old));
                }
                let mut timeline = LiveTimeline::new(&stream_id, &user_id, started_at_ms);
                if let Some((game, title)) = state.category.clone() {
                    timeline.set_category(game, title, started_at_ms, source);
                }
                changes.push(TimelineChange::Started(timeline.clone()));
                state.live = Some(timeline);
            }
            TimelineEvent::Update {
                game, title, at_ms, ..
            } => {
                let title = title.or_else(|| state.category.as_ref().and_then(|c| c.1.clone()));
                state.category = Some((game.clone(), title.clone()));
                if let Some(live) = state.live.as_mut()
                    && live.set_category(game, title, at_ms, source)
                {
                    changes.push(TimelineChange::Updated(live.clone()));
                }
            }
            TimelineEvent::Offline { at_ms, .. } => {
                if let Some(mut old) = state.live.take() {
                    old.end(at_ms);
                    changes.push(TimelineChange::Ended(old));
                }
            }
        }
        changes
    }

    /// Applies one poll of `GET /streams?user_id=…` for `user_ids` at
    /// `now_ms`: a stream that appeared starts a timeline (its category
    /// from its start), a changed category starts a segment now, a channel
    /// missing from `streams` ends its broadcast now.
    pub fn observe_poll(
        &mut self,
        user_ids: &[&str],
        streams: &[Stream],
        now_ms: i64,
    ) -> Vec<TimelineChange> {
        let mut changes = Vec::new();
        for user_id in user_ids {
            let stream = streams
                .iter()
                .find(|s| s.user_id == *user_id && !s.id.is_empty() && !s.kind.is_empty());
            let Some(stream) = stream else {
                changes.extend(self.apply(
                    TimelineEvent::Offline {
                        user_id: (*user_id).to_owned(),
                        at_ms: now_ms,
                    },
                    ChapterSource::Polling,
                ));
                continue;
            };
            let started_at_ms = parse_rfc3339_ms(&stream.started_at).unwrap_or(now_ms);
            let update = TimelineEvent::Update {
                user_id: stream.user_id.clone(),
                game: ChapterGame::new(&stream.game_id, &stream.game_name),
                title: (!stream.title.is_empty()).then(|| stream.title.clone()),
                at_ms: now_ms,
            };
            let is_new = self.live(user_id).is_none_or(|l| l.stream_id != stream.id);
            if is_new {
                // The category of a stream that just appeared is the one it
                // has now; set it before the start so it covers the start.
                let state = self.channels.entry(stream.user_id.clone()).or_default();
                if let TimelineEvent::Update { game, title, .. } = &update {
                    state.category = Some((game.clone(), title.clone()));
                }
                changes.extend(self.apply(
                    TimelineEvent::Online {
                        user_id: stream.user_id.clone(),
                        stream_id: stream.id.clone(),
                        started_at_ms,
                    },
                    ChapterSource::Polling,
                ));
            } else {
                changes.extend(self.apply(update, ChapterSource::Polling));
            }
        }
        changes
    }
}

// --- EventSub ----------------------------------------------------------------

/// EventSub WebSocket subscriptions (type, version) that feed a timeline.
pub const TIMELINE_SUBSCRIPTIONS: [(&str, &str); 3] = [
    ("stream.online", "1"),
    ("channel.update", "2"),
    ("stream.offline", "1"),
];

/// Subscription requests for one channel's timeline over a WebSocket
/// session ([`HelixClient::create_eventsub_subscription`]).
pub fn timeline_subscriptions(
    broadcaster_user_id: &str,
    session_id: &str,
) -> Vec<EventSubSubscriptionRequest> {
    TIMELINE_SUBSCRIPTIONS
        .iter()
        .map(|(kind, version)| EventSubSubscriptionRequest {
            kind: (*kind).to_owned(),
            version: (*version).to_owned(),
            condition: BTreeMap::from([(
                "broadcaster_user_id".to_owned(),
                broadcaster_user_id.to_owned(),
            )]),
            session_id: session_id.to_owned(),
        })
        .collect()
}

#[derive(Deserialize)]
struct WsMessage {
    metadata: WsMetadata,
    /// Parsed only for notifications (a keepalive has `{}`).
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct WsMetadata {
    message_type: String,
    #[serde(default)]
    message_timestamp: String,
}

#[derive(Deserialize)]
struct WsPayload {
    subscription: WsSubscription,
    event: serde_json::Value,
}

#[derive(Deserialize)]
struct WsSubscription {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct OnlineEvent {
    id: String,
    broadcaster_user_id: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    started_at: String,
}

#[derive(Deserialize)]
struct UpdateEvent {
    broadcaster_user_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    category_id: String,
    #[serde(default)]
    category_name: String,
}

#[derive(Deserialize)]
struct OfflineEvent {
    broadcaster_user_id: String,
}

/// Parses an EventSub WebSocket message. Non-notifications (welcome,
/// keepalive, reconnect, revocation) and other subscription types give
/// `Ok(None)`. The event time is the message's `message_timestamp`.
pub fn parse_eventsub_message(json: &[u8]) -> Result<Option<TimelineEvent>, ChaptersError> {
    let message: WsMessage =
        serde_json::from_slice(json).map_err(|e| ChaptersError::Decode(e.to_string()))?;
    if message.metadata.message_type != "notification" {
        return Ok(None);
    }
    let payload = message
        .payload
        .ok_or_else(|| ChaptersError::Decode("notification without payload".into()))?;
    let payload =
        WsPayload::deserialize(payload).map_err(|e| ChaptersError::Decode(e.to_string()))?;
    let at_ms = parse_rfc3339_ms(&message.metadata.message_timestamp)
        .ok_or_else(|| ChaptersError::Decode("bad message_timestamp".into()))?;
    parse_eventsub_event(&payload.subscription.kind, &payload.event, at_ms)
}

/// Parses the `event` of a notification of type `kind` received at `at_ms`
/// (for webhook transports, which carry the time in a header).
pub fn parse_eventsub_event(
    kind: &str,
    event: &serde_json::Value,
    at_ms: i64,
) -> Result<Option<TimelineEvent>, ChaptersError> {
    let decode = |e: serde_json::Error| ChaptersError::Decode(format!("{kind}: {e}"));
    Ok(match kind {
        "stream.online" => {
            let e = OnlineEvent::deserialize(event).map_err(decode)?;
            // Reruns, premieres and watch parties have no archive of their own.
            if e.kind != "live" {
                return Ok(None);
            }
            Some(TimelineEvent::Online {
                user_id: e.broadcaster_user_id,
                stream_id: e.id,
                started_at_ms: parse_rfc3339_ms(&e.started_at).unwrap_or(at_ms),
            })
        }
        "channel.update" => {
            let e = UpdateEvent::deserialize(event).map_err(decode)?;
            Some(TimelineEvent::Update {
                user_id: e.broadcaster_user_id,
                game: ChapterGame::new(&e.category_id, &e.category_name),
                title: (!e.title.is_empty()).then_some(e.title),
                at_ms,
            })
        }
        "stream.offline" => {
            let e = OfflineEvent::deserialize(event).map_err(decode)?;
            Some(TimelineEvent::Offline {
                user_id: e.broadcaster_user_id,
                at_ms,
            })
        }
        _ => None,
    })
}

// --- Helix -------------------------------------------------------------------

impl HelixClient {
    /// The archive VOD of a broadcast: `GET /videos?user_id=…&type=archive`
    /// (newest 20) matched by `stream_id`. `None` while Twitch has not
    /// published it or when the channel does not keep VODs.
    pub async fn archive_for_stream(
        &self,
        user_id: &str,
        stream_id: &str,
    ) -> crate::Result<Option<Video>> {
        let query = VideosQuery {
            video_type: Some(VideoType::Archive),
            first: Some(20),
            ..VideosQuery::by_user(user_id)
        };
        let page = self.get_videos(&query).await?;
        Ok(page
            .items
            .into_iter()
            .find(|v| v.stream_id.as_deref() == Some(stream_id)))
    }
}

/// Length of a VOD in ms (0 when Helix sends no parsable duration).
pub fn vod_duration_ms(video: &Video) -> u64 {
    video
        .duration()
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Whether a VOD is fresh enough for the fallback chapter: an archive that
/// ended at most [`FALLBACK_MAX_AGE_MS`] before `now_ms`.
pub fn fallback_allowed(video: &Video, now_ms: i64) -> bool {
    if video.kind != VideoType::Archive {
        return false;
    }
    let Some(created) = parse_rfc3339_ms(&video.created_at) else {
        return false;
    };
    let end = created.saturating_add(i64::try_from(vod_duration_ms(video)).unwrap_or(i64::MAX));
    end >= now_ms.saturating_sub(FALLBACK_MAX_AGE_MS)
}

/// The fallback chapter: the whole VOD in the channel's current category,
/// marked approximate. `None` if the channel is another one or has no
/// category.
pub fn fallback_chapter(video: &Video, channel: &ChannelInfo) -> Option<Chapter> {
    if channel.broadcaster_id != video.user_id {
        return None;
    }
    let duration = vod_duration_ms(video);
    Some(Chapter {
        game: ChapterGame::new(&channel.game_id, &channel.game_name)?,
        title: (!channel.title.is_empty()).then(|| channel.title.clone()),
        start_ms: 0,
        end_ms: (duration > 0).then_some(duration),
        source: ChapterSource::Fallback,
    })
}

// --- GQL settings ------------------------------------------------------------

/// GQL values from the remote config needed for chapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GqlChapterSettings {
    /// Public `Client-ID` header for `gql.twitch.tv`.
    pub client_id: String,
    /// Persisted-query hash of [`CHAPTERS_OPERATION`].
    pub moments_hash: String,
}

/// Why GQL chapters are off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChaptersGqlOff {
    /// Store build: no GQL code (ADR-0011 §5, ADR-0041).
    StoreBuild,
    /// `twitch.chapters_gql` (or `twitch.gql_playback`) is off or killed
    /// for this app version, or the config has no GQL client-id.
    FlagOff,
    /// The config has no persisted-query hash of [`CHAPTERS_OPERATION`].
    MissingQueryHash,
}

/// Whether this client may ask GQL for chapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaptersGql {
    /// No GQL request is ever made.
    Off(ChaptersGqlOff),
    /// GQL `moments` with these settings (acted on in full builds only).
    On(GqlChapterSettings),
}

impl ChaptersGql {
    /// For this build ([`Flavor::CURRENT`]).
    pub fn resolve(config: &RemoteConfig, ctx: &ClientContext) -> Self {
        Self::resolve_for(config, ctx, Flavor::CURRENT)
    }

    /// For a given flavor (tests and diagnostics).
    pub fn resolve_for(config: &RemoteConfig, ctx: &ClientContext, flavor: Flavor) -> Self {
        if flavor == Flavor::Store {
            return ChaptersGql::Off(ChaptersGqlOff::StoreBuild);
        }
        let twitch = config.twitch_flags(ctx);
        if !twitch.chapters_gql {
            return ChaptersGql::Off(ChaptersGqlOff::FlagOff);
        }
        match (
            twitch.gql_client_id,
            twitch.persisted_queries.get(CHAPTERS_OPERATION),
        ) {
            (Some(client_id), Some(hash)) if !client_id.is_empty() && !hash.is_empty() => {
                ChaptersGql::On(GqlChapterSettings {
                    client_id,
                    moments_hash: hash.clone(),
                })
            }
            (None, _) => ChaptersGql::Off(ChaptersGqlOff::FlagOff),
            _ => ChaptersGql::Off(ChaptersGqlOff::MissingQueryHash),
        }
    }

    /// GQL is allowed by the config.
    pub fn is_on(&self) -> bool {
        matches!(self, ChaptersGql::On(_))
    }
}

/// Chapter failures. GQL codes match the monitoring names of 06 §2.7.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChaptersError {
    /// A Helix call failed.
    #[error(transparent)]
    Helix(#[from] crate::Error),
    /// No response from GQL.
    #[error("network error: {0}")]
    Transport(String),
    /// GQL does not know the persisted-query hash: the config is stale.
    #[error("Twitch GQL rejected the persisted-query hash")]
    GqlHashInvalid,
    /// GQL wants a client-integrity token, which we do not produce.
    #[error("Twitch GQL requires a client-integrity token")]
    IntegrityRequired,
    /// Any other GQL failure.
    #[error("Twitch GQL error {status}: {message}")]
    Gql {
        /// HTTP status.
        status: u16,
        /// Error message (short).
        message: String,
    },
    /// A response or an EventSub message is not what we expect.
    #[error("unexpected Twitch data: {0}")]
    Decode(String),
}

impl ChaptersError {
    /// Stable code for logs and diagnostics.
    pub fn code(&self) -> &'static str {
        match self {
            ChaptersError::Helix(_) => "helix",
            ChaptersError::Transport(_) => "network",
            ChaptersError::GqlHashInvalid => "gql_hash_invalid",
            ChaptersError::IntegrityRequired => "integrity_required",
            ChaptersError::Gql { .. } => "gql_error",
            ChaptersError::Decode(_) => "decode",
        }
    }
}

// --- Resolution --------------------------------------------------------------

/// Input of [`ChapterResolver::resolve`].
#[derive(Debug, Clone, Copy)]
pub struct VodChaptersRequest<'a> {
    /// The VOD from Helix.
    pub video: &'a Video,
    /// Our timeline of its broadcast, if we monitored it.
    pub timeline: Option<&'a LiveTimeline>,
    /// Best source of the chapters already stored for it
    /// (`store::stored_source`), if any.
    pub stored: Option<ChapterSource>,
    /// Unix ms.
    pub now_ms: i64,
}

/// Answer of [`ChapterResolver::resolve`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedChapters {
    /// New chapters to store in place of the old ones; `None` = keep what
    /// is stored (or there is nothing).
    pub chapters: Option<Vec<Chapter>>,
    /// [`ChaptersError::code`] of a GQL failure that was skipped.
    pub gql_failure: Option<&'static str>,
}

/// Picks and fetches the chapters of a VOD (see the module docs).
pub struct ChapterResolver {
    gql: ChaptersGql,
    #[cfg(pa_twitch_native)]
    transport: Arc<dyn Transport>,
    #[cfg(pa_twitch_native)]
    gql_url: String,
}

impl ChapterResolver {
    /// A resolver; `transport` is used only for GQL in full builds.
    pub fn new(gql: ChaptersGql, transport: Arc<dyn Transport>) -> Self {
        #[cfg(not(pa_twitch_native))]
        let _ = transport;
        Self {
            gql,
            #[cfg(pa_twitch_native)]
            transport,
            #[cfg(pa_twitch_native)]
            gql_url: gql::GQL_URL.to_owned(),
        }
    }

    /// Overrides the GQL endpoint (tests, mirrors).
    #[cfg(pa_twitch_native)]
    pub fn with_gql_endpoint(mut self, url: &str) -> Self {
        url.trim_end_matches('/').clone_into(&mut self.gql_url);
        self
    }

    /// The GQL setting.
    pub fn gql(&self) -> &ChaptersGql {
        &self.gql
    }

    /// Whether [`ChapterResolver::resolve`] may send a GQL request.
    pub fn uses_gql(&self) -> bool {
        cfg!(pa_twitch_native) && self.gql.is_on()
    }

    /// Chapters of one VOD: our timeline, else what is stored (unless it is
    /// only a fallback), else GQL `moments` (flag on, full build), else the
    /// fallback for a fresh archive. Only Helix errors are returned; a GQL
    /// failure is reported in [`ResolvedChapters::gql_failure`].
    pub async fn resolve(
        &self,
        helix: &HelixClient,
        request: VodChaptersRequest<'_>,
    ) -> Result<ResolvedChapters, ChaptersError> {
        let video = request.video;
        let mut out = ResolvedChapters::default();
        if let Some(timeline) = request.timeline
            && video.stream_id.as_deref() == Some(timeline.stream_id.as_str())
        {
            let chapters = timeline.vod_chapters(vod_duration_ms(video));
            if !chapters.is_empty() {
                out.chapters = Some(chapters);
                return Ok(out);
            }
        }
        if request.stored.is_some_and(|s| !s.is_approximate()) {
            return Ok(out);
        }
        #[cfg(pa_twitch_native)]
        if let ChaptersGql::On(settings) = &self.gql {
            match gql::fetch_chapters(&*self.transport, &self.gql_url, settings, video).await {
                Ok(chapters) if !chapters.is_empty() => {
                    out.chapters = Some(chapters);
                    return Ok(out);
                }
                Ok(_) => {}
                Err(e) => out.gql_failure = Some(e.code()),
            }
        }
        if request.stored.is_some() || !fallback_allowed(video, request.now_ms) {
            return Ok(out);
        }
        let channels = helix.get_channels(&[video.user_id.as_str()]).await?;
        out.chapters = channels
            .iter()
            .find_map(|c| fallback_chapter(video, c))
            .map(|c| vec![c]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: &str) -> Option<ChapterGame> {
        ChapterGame::new(id, &format!("Game {id}"))
    }

    #[test]
    fn source_names_round_trip() {
        for s in [
            ChapterSource::EventSub,
            ChapterSource::Polling,
            ChapterSource::GqlMoments,
            ChapterSource::Fallback,
        ] {
            assert_eq!(ChapterSource::parse(s.as_str()), Some(s));
        }
        assert_eq!(ChapterSource::parse("other"), None);
        assert!(ChapterSource::Fallback.is_approximate());
        assert!(!ChapterSource::GqlMoments.is_approximate());
    }

    #[test]
    fn timeline_segments_and_vod_offsets() {
        let mut t = LiveTimeline::new("9", "1", 1_000_000);
        // First category covers the start even if learned later.
        assert!(t.set_category(game("10"), None, 1_005_000, ChapterSource::EventSub));
        // Same category again: no new segment.
        assert!(!t.set_category(game("10"), None, 1_010_000, ChapterSource::EventSub));
        assert!(t.set_category(None, None, 1_060_000, ChapterSource::EventSub));
        assert!(t.set_category(game("20"), None, 1_090_000, ChapterSource::EventSub));
        t.end(1_200_000);
        assert!(!t.set_category(game("30"), None, 1_300_000, ChapterSource::EventSub));
        let ch = t.chapters();
        assert_eq!(ch.len(), 2, "no-category gap is not a chapter");
        assert_eq!((ch[0].start_ms, ch[0].end_ms), (0, Some(60_000)));
        assert_eq!((ch[1].start_ms, ch[1].end_ms), (90_000, Some(200_000)));
        // VOD shorter than the broadcast: clamped.
        let vod = t.vod_chapters(150_000);
        assert_eq!(vod[1].end_ms, Some(150_000));
        let vod = t.vod_chapters(80_000);
        assert_eq!(vod.len(), 1);
    }

    #[test]
    fn open_segment_runs_to_the_vod_end() {
        let mut t = LiveTimeline::new("9", "1", 0);
        t.set_category(game("10"), None, 0, ChapterSource::Polling);
        assert_eq!(t.chapters()[0].end_ms, None);
        assert_eq!(t.vod_chapters(5_000)[0].end_ms, Some(5_000));
    }
}
