// SPDX-License-Identifier: MPL-2.0
//! Automatic recording: the household's record rules (`twitch_record_rules`
//! of the library, one per profile and favourite channel) applied to the
//! live events of TWI-009.

use std::sync::Arc;

use pa_jobs::{JobId, JobQueue, JobState};
use pa_library::twitch::RecordRule;
use pa_storage::{RetentionRule, StorageManager};
use pa_twitch::chapters::{LiveTimeline, TimelineChange};
use pa_twitch_live::MonitorEvent;

use super::recorder::{LiveRequest, TWITCH_LIVE_RECORD};
use super::source::LiveSource;
use crate::archive::job::Quality;

/// What [`AutoRecord::on_change`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoDecision {
    /// A recording was queued.
    Started(JobId),
    /// The channel is already being recorded (or queued).
    AlreadyRecording(JobId),
    /// No enabled rule for the channel.
    NoRule,
    /// The rule records other categories only; an `Updated` event into one
    /// of them starts it.
    CategoryFiltered,
    /// Archiving is not allowed now (flavor, remote config, native
    /// playback off): the error code. Nothing is queued.
    NotAllowed(&'static str),
    /// Not a start or category change.
    Ignored,
}

/// `best`/`source`/`chunked` → source; `720p60`, `720p` → at most 720
/// lines; `audio_only` → audio.
pub fn quality_of(rule: &str) -> Quality {
    let q = rule.trim().to_ascii_lowercase();
    if q == "audio_only" || q == "audio" {
        return Quality::AudioOnly;
    }
    let digits: String = q.chars().take_while(char::is_ascii_digit).collect();
    match digits.parse::<u32>() {
        Ok(h) if h > 0 && q[digits.len()..].starts_with('p') => Quality::MaxHeight(h),
        _ => Quality::Source,
    }
}

/// The retention group of a channel's files (`twitch:<user_id>`, as the
/// downloads).
pub fn group_of(user_id: &str) -> String {
    format!("twitch:{user_id}")
}

/// Turns live events into recordings, by the rules.
pub struct AutoRecord {
    source: Arc<dyn LiveSource>,
    storage: StorageManager,
    queue: JobQueue,
    target: String,
}

impl AutoRecord {
    /// Recordings go to storage target `target` (kind `recordings`).
    pub fn new(
        source: Arc<dyn LiveSource>,
        storage: StorageManager,
        queue: JobQueue,
        target: impl Into<String>,
    ) -> Self {
        Self {
            source,
            storage,
            queue,
            target: target.into(),
        }
    }

    /// Applies the rules' retention (`retention_days`, `keep_last_n`) to the
    /// channels' groups of the target. With several profiles recording one
    /// channel the most generous limit wins.
    pub fn apply_retention(&self, rules: &[RecordRule]) -> Result<usize, pa_storage::Error> {
        let mut n = 0;
        let mut users: Vec<&str> = rules.iter().map(|r| r.user_id.as_str()).collect();
        users.sort_unstable();
        users.dedup();
        for user in users {
            let of_user: Vec<&RecordRule> = rules.iter().filter(|r| r.user_id == user).collect();
            let max = |f: fn(&RecordRule) -> Option<u32>| -> Option<u32> {
                // A rule without the limit keeps everything.
                if of_user.iter().any(|r| f(r).is_none()) {
                    None
                } else {
                    of_user.iter().filter_map(|r| f(r)).max()
                }
            };
            let mut rule = RetentionRule::new(self.target.clone(), Some(group_of(user)));
            rule.keep_last = max(|r| r.keep_last_n);
            rule.max_age_days = max(|r| r.retention_days);
            if rule.keep_last.is_none() && rule.max_age_days.is_none() {
                continue;
            }
            self.storage.set_retention(&rule)?;
            n += 1;
        }
        Ok(n)
    }

    /// The active recording of a channel, if any.
    pub fn active(&self, user_id: &str) -> Result<Option<JobId>, pa_jobs::Error> {
        Ok(self
            .queue
            .list(&[
                JobState::Queued,
                JobState::WaitingResources,
                JobState::Running,
                JobState::Paused,
                JobState::Retrying,
            ])?
            .into_iter()
            .find(|j| j.kind == TWITCH_LIVE_RECORD && j.params["user_id"] == user_id)
            .map(|j| j.id))
    }

    /// Handles one TWI-009 monitor event (`MonitorEvent::Timeline`).
    pub fn on_event(
        &self,
        event: &MonitorEvent,
        rules: &[RecordRule],
        login: Option<&str>,
    ) -> Result<AutoDecision, pa_storage::Error> {
        match event {
            MonitorEvent::Timeline(change) => self.on_change(change, rules, login),
            _ => Ok(AutoDecision::Ignored),
        }
    }

    /// A broadcast started (or changed category): queue its recording if
    /// an enabled rule of any profile covers the channel and its current
    /// category, and the channel is not already being recorded.
    pub fn on_change(
        &self,
        change: &TimelineChange,
        rules: &[RecordRule],
        login: Option<&str>,
    ) -> Result<AutoDecision, pa_storage::Error> {
        let timeline = match change {
            TimelineChange::Started(t) | TimelineChange::Updated(t) => t,
            TimelineChange::Ended(_) => return Ok(AutoDecision::Ignored),
        };
        let rules: Vec<&RecordRule> = rules
            .iter()
            .filter(|r| r.enabled && r.user_id == timeline.user_id)
            .collect();
        if rules.is_empty() {
            return Ok(AutoDecision::NoRule);
        }
        let game = current_game(timeline);
        let Some(rule) = rules.iter().find(|r| {
            r.categories
                .as_ref()
                .is_none_or(|ids| game.as_deref().is_some_and(|g| ids.iter().any(|i| i == g)))
        }) else {
            return Ok(AutoDecision::CategoryFiltered);
        };
        if let Some(id) = self.active(&timeline.user_id)? {
            return Ok(AutoDecision::AlreadyRecording(id));
        }
        if let Err(e) = self.source.check_allowed() {
            return Ok(AutoDecision::NotAllowed(e.code));
        }
        let mut req = LiveRequest::new(timeline.user_id.clone());
        req.login = login.map(str::to_owned);
        req.stream_id = Some(timeline.stream_id.clone());
        req.quality = quality_of(&rule.quality);
        req.with_chat = rules.iter().any(|r| r.with_chat);
        let job = req.to_job(&self.target).owner(rule.profile_id);
        Ok(AutoDecision::Started(self.storage.enqueue(&self.queue, &job)?))
    }
}

/// Category id on air now.
fn current_game(t: &LiveTimeline) -> Option<String> {
    t.segments
        .iter()
        .rev()
        .find(|s| s.ended_at_ms.is_none())
        .or(t.segments.last())
        .and_then(|s| s.game.as_ref())
        .map(|g| g.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualities() {
        assert_eq!(quality_of("best"), Quality::Source);
        assert_eq!(quality_of("source"), Quality::Source);
        assert_eq!(quality_of("720p60"), Quality::MaxHeight(720));
        assert_eq!(quality_of("480p"), Quality::MaxHeight(480));
        assert_eq!(quality_of("audio_only"), Quality::AudioOnly);
        assert_eq!(quality_of("weird"), Quality::Source);
    }
}
