// SPDX-License-Identifier: MPL-2.0
//! One poll without a running monitor: the Android WorkManager job (and
//! iOS BGAppRefresh later) wakes up, asks `GET /streams` once, stores the
//! timelines and returns what to notify. The tracker is resumed from the
//! store, so a broadcast seen by an earlier run is not announced again.

use std::time::Duration;

use pa_twitch::Stream;
use pa_twitch::chapters::{ChapterSource, TimelineChange, TimelineTracker};

use crate::api::{LiveApi, LiveError};
use crate::gate::{LiveAlert, NotificationGate};
use crate::plan::WatchedChannel;
use crate::store::TimelineStore;

/// Result of [`check_once`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Notifications to show.
    pub alerts: Vec<LiveAlert>,
    /// Timeline changes (already stored).
    pub changes: Vec<TimelineChange>,
    /// Channels live now.
    pub live: Vec<Stream>,
}

/// Polls `channels` once at `now_ms`. `max_age` should cover the time
/// since the previous run (the WorkManager period plus the delays Android
/// adds), so a broadcast that started between two runs is still news.
pub async fn check_once(
    api: &dyn LiveApi,
    channels: &[WatchedChannel],
    store: &mut dyn TimelineStore,
    max_age: Duration,
    now_ms: i64,
) -> Result<CheckReport, LiveError> {
    let mut report = CheckReport::default();
    if channels.is_empty() {
        return Ok(report);
    }
    let mut tracker = TimelineTracker::new();
    let mut gate = NotificationGate::new(max_age);
    for timeline in store.open_timelines(now_ms) {
        gate.remember(&timeline.stream_id);
        tracker.resume(timeline);
    }
    let ids: Vec<String> = channels.iter().map(|c| c.user_id.clone()).collect();
    let streams = api.live_streams(ids.clone()).await?;
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    for change in tracker.observe_poll(&refs, &streams, now_ms) {
        store.save(&change, now_ms);
        let channel = channels
            .iter()
            .find(|c| c.user_id == change.timeline().user_id);
        if let Some(alert) = gate.consider(&change, channel, now_ms, ChapterSource::Polling) {
            report.alerts.push(alert);
        }
        if let TimelineChange::Ended(timeline) = &change
            && let Ok(Some(video)) = api
                .archive_for_stream(timeline.user_id.clone(), timeline.stream_id.clone())
                .await
        {
            store.save_vod(&video, timeline, now_ms);
        }
        report.changes.push(change);
    }
    report.live = streams;
    Ok(report)
}
