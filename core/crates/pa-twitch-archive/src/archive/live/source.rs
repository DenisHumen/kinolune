// SPDX-License-Identifier: MPL-2.0
//! Where a live recording gets its data: usher's live playlist, Helix
//! `GET /streams`, the TWI-008 timeline, the TWI-010 chat, a clock.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pa_twitch::chapters::LiveTimeline;
use pa_twitch::playback::{MasterPlaylist, PlaybackDecision, PlaybackError, PlaybackTarget};
use pa_twitch::{HttpResponse, StreamsQuery, parse_rfc3339_ms};
use pa_twitch_chat::{ChatEvent, ChatSource};
use serde::{Deserialize, Serialize};

use crate::archive::source::{ArchiveError, ArchiveSource, TwitchSource};

/// Time for the recorder: real time in the apps, virtual time in tests (a
/// 6-hour broadcast is recorded in seconds).
pub trait LiveClock: Send + Sync {
    /// Unix ms.
    fn now_ms(&self) -> i64;
    fn sleep(&self, d: Duration);
}

/// The system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl LiveClock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// A broadcast as Helix reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LiveStream {
    pub stream_id: String,
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub title: String,
    #[serde(default)]
    pub game_id: String,
    #[serde(default)]
    pub game_name: String,
    /// Unix ms.
    pub started_at_ms: i64,
    /// Thumbnail template with `{width}`/`{height}`.
    #[serde(default)]
    pub thumbnail_url: String,
}

impl LiveStream {
    fn from_helix(s: pa_twitch::Stream) -> Self {
        Self {
            started_at_ms: parse_rfc3339_ms(&s.started_at).unwrap_or(0),
            stream_id: s.id,
            user_id: s.user_id,
            login: s.user_login,
            display_name: s.user_name,
            title: s.title,
            game_id: s.game_id,
            game_name: s.game_name,
            thumbnail_url: s.thumbnail_url,
        }
    }
}

/// One chat message, as recorded next to the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRecord {
    pub id: String,
    /// Twitch's server time (`tmi-sent-ts`), Unix ms: what the replay is
    /// synchronised by.
    pub sent_ms: i64,
    /// When we received it, Unix ms.
    pub received_ms: i64,
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub text: String,
    /// Emotes: (id, first char, one past the last char).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emotes: Vec<(String, usize, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bits: Option<u32>,
    /// System notice kind (sub, raid, …), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// A live chat being recorded. [`drain`](Self::drain) is called on every
/// playlist poll and returns what arrived since the last call.
pub trait LiveChat: Send {
    fn drain(&mut self) -> Vec<ChatRecord>;
    /// Stops the connection.
    fn close(&mut self) {}
}

/// Data a live recording needs. Methods block; they are called from the
/// job's thread.
pub trait LiveSource: Send + Sync {
    /// Whether recording is allowed now: full build, remote-config
    /// `twitch.archive` on, native playback allowed. Checked at the start
    /// of every run and every minute while recording.
    fn check_allowed(&self) -> Result<(), ArchiveError>;

    /// The channel's broadcast from Helix, `None` when offline.
    fn stream(&self, user_id: &str) -> Result<Option<LiveStream>, ArchiveError>;

    /// usher's live master playlist (TWI-004); `None` when the channel is
    /// offline. A refusal (`usher_403`) is an error and never worked around.
    fn master_playlist(&self, login: &str) -> Result<Option<MasterPlaylist>, ArchiveError>;

    /// One HTTP GET (playlists, segments, images).
    fn get(&self, url: &str) -> Result<HttpResponse, ArchiveError>;

    /// The category timeline of the broadcast (TWI-008/009), for chapters.
    fn timeline(&self, stream_id: &str) -> Option<LiveTimeline> {
        let _ = stream_id;
        None
    }

    /// The channel's chat (TWI-010), recorded alongside; `None` without.
    fn chat(&self, stream: &LiveStream) -> Option<Box<dyn LiveChat>> {
        let _ = stream;
        None
    }
}

/// Timeline hook: usually `pa_twitch::chapters::store::load_live_timeline`
/// over the library.
pub type TimelineFn = dyn Fn(&str) -> Option<LiveTimeline> + Send + Sync;
/// Chat hook: a chat source for the channel (anonymous IRC is enough).
pub type ChatFn = dyn Fn(&LiveStream) -> Option<Arc<dyn ChatSource>> + Send + Sync;

/// [`LiveSource`] over Twitch: the same [`TwitchSource`] (features, Helix,
/// playback resolver, transport, runtime) as VOD downloads.
pub struct TwitchLiveSource {
    inner: Arc<TwitchSource>,
    timeline: Option<Box<TimelineFn>>,
    chat: Option<Box<ChatFn>>,
}

impl TwitchLiveSource {
    pub fn new(inner: Arc<TwitchSource>) -> Self {
        Self {
            inner,
            timeline: None,
            chat: None,
        }
    }

    /// Where chapters come from.
    pub fn with_timeline(mut self, f: Box<TimelineFn>) -> Self {
        self.timeline = Some(f);
        self
    }

    /// Where the chat comes from.
    pub fn with_chat(mut self, f: Box<ChatFn>) -> Self {
        self.chat = Some(f);
        self
    }
}

impl LiveSource for TwitchLiveSource {
    fn check_allowed(&self) -> Result<(), ArchiveError> {
        self.inner.check_allowed()
    }

    fn stream(&self, user_id: &str) -> Result<Option<LiveStream>, ArchiveError> {
        let id = user_id.to_owned();
        let page = self.inner.helix(|h| {
            let q = StreamsQuery {
                user_ids: vec![id.clone()],
                ..StreamsQuery::default()
            };
            async move { h.get_streams(&q).await }
        })?;
        Ok(page
            .items
            .into_iter()
            .find(|s| s.user_id == user_id && (s.kind.is_empty() || s.kind == "live"))
            .map(LiveStream::from_helix))
    }

    fn master_playlist(&self, login: &str) -> Result<Option<MasterPlaylist>, ArchiveError> {
        let target = PlaybackTarget::live(login)?;
        match self
            .inner
            .block_on(self.inner.playback().resolve(&target, self.inner.codecs()))
        {
            Ok(PlaybackDecision::Native(n)) => Ok(Some(n.playlist)),
            Ok(PlaybackDecision::Embed(e)) => Err(ArchiveError::fatal(
                "native_off",
                format!("native Twitch playback is off ({:?})", e.reason),
            )),
            Err(PlaybackError::Unavailable | PlaybackError::NotFound) => Ok(None),
            Err(PlaybackError::Restricted { code }) => Err(ArchiveError::fatal(
                "usher_403",
                format!(
                    "Twitch restricts this broadcast ({code}): restricted streams are not recorded"
                ),
            )),
            Err(e) => Err(e.into()),
        }
    }

    fn get(&self, url: &str) -> Result<HttpResponse, ArchiveError> {
        self.inner.get(url)
    }

    fn timeline(&self, stream_id: &str) -> Option<LiveTimeline> {
        self.timeline.as_ref().and_then(|f| f(stream_id))
    }

    fn chat(&self, stream: &LiveStream) -> Option<Box<dyn LiveChat>> {
        let source = self.chat.as_ref().and_then(|f| f(stream))?;
        Some(Box::new(ChatSourceFeed::start(
            &self.inner.runtime_handle(),
            source,
            Arc::new(SystemClock),
        )))
    }
}

/// [`LiveChat`] over a TWI-010 [`ChatSource`]: a task on `rt` collects
/// chat messages until the feed is closed or dropped.
pub struct ChatSourceFeed {
    lines: Arc<Mutex<Vec<ChatRecord>>>,
    source: Arc<dyn ChatSource>,
    task: tokio::task::JoinHandle<()>,
}

impl ChatSourceFeed {
    pub fn start(
        rt: &tokio::runtime::Handle,
        source: Arc<dyn ChatSource>,
        clock: Arc<dyn LiveClock>,
    ) -> Self {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let task = {
            let (lines, source) = (Arc::clone(&lines), Arc::clone(&source));
            rt.spawn(async move {
                while let Some(event) = source.next_event().await {
                    if let ChatEvent::Message(m) = event {
                        let line = ChatRecord {
                            id: m.id.clone(),
                            sent_ms: m.ts_ms,
                            received_ms: clock.now_ms(),
                            user_id: m.user.id.clone(),
                            login: m.user.login.clone(),
                            display_name: m.user.display_name.clone(),
                            color: m.user.color.clone(),
                            text: m.text.clone(),
                            emotes: m
                                .emotes
                                .iter()
                                .map(|e| (e.id.clone(), e.start, e.end))
                                .collect(),
                            bits: m.bits,
                            notice: m.notice.as_ref().map(|n| format!("{n:?}")),
                        };
                        lines.lock().unwrap_or_else(|e| e.into_inner()).push(line);
                    }
                }
            })
        };
        Self {
            lines,
            source,
            task,
        }
    }
}

impl LiveChat for ChatSourceFeed {
    fn drain(&mut self) -> Vec<ChatRecord> {
        std::mem::take(&mut *self.lines.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn close(&mut self) {
        self.source.close();
        self.task.abort();
    }
}

impl Drop for ChatSourceFeed {
    fn drop(&mut self) {
        self.close();
    }
}
