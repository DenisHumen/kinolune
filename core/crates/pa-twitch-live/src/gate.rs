// SPDX-License-Identifier: MPL-2.0
//! Turns "a broadcast started" into at most one notification.

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use pa_twitch::chapters::{ChapterSource, TimelineChange};

use crate::plan::WatchedChannel;

/// Stream ids remembered against duplicates.
const REMEMBERED: usize = 512;

/// Default [`NotificationGate`] age limit while the app runs: a broadcast
/// that started earlier was live before we looked (app start, a channel
/// just added) and is not news.
pub const DEFAULT_ALERT_MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// An "X is live" notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveAlert {
    /// Broadcaster id.
    pub user_id: String,
    /// Login (deep link `twitch/channel/<login>`).
    pub login: String,
    /// Display name.
    pub display_name: String,
    /// Stream id (notification id: one per broadcast).
    pub stream_id: String,
    /// Title, when known.
    pub title: Option<String>,
    /// Category name, when known.
    pub game_name: Option<String>,
    /// Broadcast start (Unix ms).
    pub started_at_ms: i64,
    /// When we noticed (Unix ms); `detected_at_ms - started_at_ms` is the
    /// delay the user sees.
    pub detected_at_ms: i64,
    /// How we noticed: EventSub or polling.
    pub source: ChapterSource,
}

impl LiveAlert {
    /// Delay between the start and the notification.
    pub fn delay(&self) -> Duration {
        Duration::from_millis(
            u64::try_from(self.detected_at_ms.saturating_sub(self.started_at_ms)).unwrap_or(0),
        )
    }
}

/// Decides which started broadcasts are notified: the channel's toggle is
/// on, the broadcast is fresh (not older than `max_age`) and was not
/// notified yet.
#[derive(Debug, Clone)]
pub struct NotificationGate {
    max_age: Duration,
    order: VecDeque<String>,
    seen: HashSet<String>,
}

impl NotificationGate {
    /// Gate with an age limit.
    pub fn new(max_age: Duration) -> Self {
        Self {
            max_age,
            order: VecDeque::new(),
            seen: HashSet::new(),
        }
    }

    /// Whether `stream_id` was notified.
    pub fn was_notified(&self, stream_id: &str) -> bool {
        self.seen.contains(stream_id)
    }

    /// Marks a broadcast as notified (e.g. by an earlier process).
    pub fn remember(&mut self, stream_id: &str) {
        if !self.seen.insert(stream_id.to_owned()) {
            return;
        }
        self.order.push_back(stream_id.to_owned());
        if self.order.len() > REMEMBERED
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
    }

    /// The alert for `change`, if any.
    pub fn consider(
        &mut self,
        change: &TimelineChange,
        channel: Option<&WatchedChannel>,
        now_ms: i64,
        source: ChapterSource,
    ) -> Option<LiveAlert> {
        let TimelineChange::Started(timeline) = change else {
            return None;
        };
        let channel = channel.filter(|c| c.notify)?;
        let age = u128::try_from(now_ms.saturating_sub(timeline.started_at_ms)).unwrap_or(0);
        if age > self.max_age.as_millis() || self.was_notified(&timeline.stream_id) {
            return None;
        }
        self.remember(&timeline.stream_id);
        let last = timeline.segments.last();
        Some(LiveAlert {
            user_id: timeline.user_id.clone(),
            login: channel.login.clone(),
            display_name: channel.display_name.clone(),
            stream_id: timeline.stream_id.clone(),
            title: timeline
                .title_last
                .clone()
                .or_else(|| last.and_then(|s| s.title.clone())),
            game_name: last
                .and_then(|s| s.game.as_ref())
                .map(|g| g.name.clone())
                .filter(|n| !n.is_empty()),
            started_at_ms: timeline.started_at_ms,
            detected_at_ms: now_ms,
            source,
        })
    }
}

impl Default for NotificationGate {
    fn default() -> Self {
        Self::new(DEFAULT_ALERT_MAX_AGE)
    }
}

#[cfg(test)]
mod tests {
    use pa_twitch::chapters::LiveTimeline;

    use super::*;

    fn started(stream: &str, at: i64) -> TimelineChange {
        TimelineChange::Started(LiveTimeline::new(stream, "7", at))
    }

    #[test]
    fn notifies_once_per_broadcast() {
        let mut gate = NotificationGate::default();
        let ch = WatchedChannel::new("7", "made_up", "Made Up");
        let a = gate.consider(
            &started("s1", 1_000),
            Some(&ch),
            31_000,
            ChapterSource::EventSub,
        );
        let a = a.expect("alert");
        assert_eq!(a.delay(), Duration::from_secs(30));
        assert_eq!(a.display_name, "Made Up");
        assert!(
            gate.consider(
                &started("s1", 1_000),
                Some(&ch),
                60_000,
                ChapterSource::Polling
            )
            .is_none()
        );
        assert!(
            gate.consider(
                &started("s2", 70_000),
                Some(&ch),
                80_000,
                ChapterSource::Polling
            )
            .is_some()
        );
    }

    #[test]
    fn toggle_off_unknown_and_old_are_silent() {
        let mut gate = NotificationGate::default();
        let off = WatchedChannel::new("7", "x", "X").with_notify(false);
        let on = WatchedChannel::new("7", "x", "X");
        assert!(
            gate.consider(&started("s", 0), Some(&off), 1, ChapterSource::EventSub)
                .is_none()
        );
        assert!(
            gate.consider(&started("s", 0), None, 1, ChapterSource::EventSub)
                .is_none()
        );
        let late = 11 * 60 * 1000;
        assert!(
            gate.consider(&started("s", 0), Some(&on), late, ChapterSource::Polling)
                .is_none()
        );
    }

    #[test]
    fn updates_and_ends_are_silent() {
        let mut gate = NotificationGate::default();
        let ch = WatchedChannel::new("7", "x", "X");
        let t = LiveTimeline::new("s", "7", 0);
        assert!(
            gate.consider(
                &TimelineChange::Updated(t.clone()),
                Some(&ch),
                1,
                ChapterSource::EventSub
            )
            .is_none()
        );
        assert!(
            gate.consider(
                &TimelineChange::Ended(t),
                Some(&ch),
                1,
                ChapterSource::EventSub
            )
            .is_none()
        );
    }

    #[test]
    fn memory_is_bounded() {
        let mut gate = NotificationGate::default();
        for i in 0..(REMEMBERED + 10) {
            gate.remember(&i.to_string());
        }
        assert!(!gate.was_notified("0"));
        assert!(gate.was_notified(&(REMEMBERED + 9).to_string()));
    }
}
