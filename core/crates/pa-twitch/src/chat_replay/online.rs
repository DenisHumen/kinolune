// SPDX-License-Identifier: MPL-2.0
//! The online replay of one VOD: GQL pages around the playhead, cached
//! for the life of the replay (06 §2.5).

use std::sync::{Arc, Mutex, MutexGuard};

use super::gql::{self, Page, PageStart};
use super::td::{ChatReplayFile, Streamer, VideoInfo};
use super::timeline::ReplayTimeline;
use super::{ChatReplayError, ChatReplayGql, ChatReplaySettings};
use crate::transport::Transport;

/// A seek fetches from this long before the new position, so the chat has
/// lines to show at once.
pub const LOOKBACK_MS: u64 = 20_000;

/// A gap this small after the end of what is cached is filled by going on
/// from there instead of starting anew at the playhead.
pub const CONTINUE_GAP_MS: u64 = 60_000;

/// Pages one [`OnlineReplay::ensure`] fetches at most; the caller asks
/// again on the next tick.
pub const MAX_PAGES_PER_ENSURE: usize = 8;

/// A stretch of the VOD whose comments are all cached: `[from_ms, to_ms)`,
/// `to_ms` `None` up to the end of the VOD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    /// Start, ms from the VOD start.
    pub from_ms: u64,
    /// End (exclusive); `None`: the rest of the VOD.
    pub to_ms: Option<u64>,
}

impl Coverage {
    /// `at_ms` is inside.
    pub fn contains(&self, at_ms: u64) -> bool {
        at_ms >= self.from_ms && self.to_ms.is_none_or(|t| at_ms < t)
    }
}

/// Requests made so far (diagnostics, tests).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageStats {
    /// GQL answers received.
    pub pages: usize,
    /// Comments cached.
    pub comments: usize,
    /// Comments Twitch marked deleted (never shown).
    pub deleted: usize,
}

#[derive(Default)]
struct State {
    timeline: ReplayTimeline,
    /// Sorted, non-overlapping `[from, to)`; `u64::MAX` is the end.
    covered: Vec<(u64, u64)>,
    channel_id: Option<String>,
    stats: PageStats,
}

impl State {
    fn add(&mut self, from: u64, to: u64) {
        if to <= from {
            return;
        }
        self.covered.push((from, to));
        self.covered.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(self.covered.len());
        for &(f, t) in &self.covered {
            match merged.last_mut() {
                Some(last) if f <= last.1 => last.1 = last.1.max(t),
                _ => merged.push((f, t)),
            }
        }
        self.covered = merged;
    }

    fn covering(&self, at: u64) -> Option<Coverage> {
        self.covered
            .iter()
            .find(|(f, t)| at >= *f && at < *t)
            .map(|&(f, t)| coverage(f, t))
    }

    /// End of the stretch that ends at or before `at`, if any.
    fn end_before(&self, at: u64) -> Option<u64> {
        self.covered
            .iter()
            .filter(|(_, t)| *t <= at)
            .map(|(_, t)| *t)
            .max()
    }
}

fn coverage(from: u64, to: u64) -> Coverage {
    Coverage {
        from_ms: from,
        to_ms: (to != u64::MAX).then_some(to),
    }
}

/// The chat of one VOD from GQL, fetched as the playhead needs it.
pub struct OnlineReplay {
    transport: Arc<dyn Transport>,
    url: String,
    settings: ChatReplaySettings,
    video_id: String,
    state: Mutex<State>,
    /// One fetch run at a time (a seek and a prefetch do not race).
    fetching: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for OnlineReplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnlineReplay")
            .field("video_id", &self.video_id)
            .finish_non_exhaustive()
    }
}

impl OnlineReplay {
    /// The replay of `video_id`; [`ChatReplayError::Off`] when the config
    /// does not allow it.
    pub fn new(
        gql: &ChatReplayGql,
        video_id: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, ChatReplayError> {
        let settings = match gql {
            ChatReplayGql::On(s) => s.clone(),
            ChatReplayGql::Off(off) => return Err(ChatReplayError::Off(*off)),
        };
        let video_id = video_id.trim();
        if video_id.is_empty() || !video_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ChatReplayError::Decode(format!("bad VOD id {video_id:?}")));
        }
        Ok(Self {
            transport,
            url: gql::GQL_URL.to_owned(),
            settings,
            video_id: video_id.to_owned(),
            state: Mutex::new(State::default()),
            fetching: tokio::sync::Mutex::new(()),
        })
    }

    /// Overrides the GQL endpoint (tests, mirrors).
    pub fn with_gql_endpoint(mut self, url: &str) -> Self {
        url.trim_end_matches('/').clone_into(&mut self.url);
        self
    }

    /// The VOD.
    pub fn video_id(&self) -> &str {
        &self.video_id
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cached stretch around `at_ms`, if it is cached.
    pub fn covered(&self, at_ms: u64) -> Option<Coverage> {
        self.state().covering(at_ms)
    }

    /// Reads the cached comments.
    pub fn with_timeline<R>(&self, f: impl FnOnce(&ReplayTimeline) -> R) -> R {
        f(&self.state().timeline)
    }

    /// Broadcaster id, once a page has named it.
    pub fn channel_id(&self) -> Option<String> {
        self.state().channel_id.clone()
    }

    /// Requests and comments so far.
    pub fn stats(&self) -> PageStats {
        self.state().stats
    }

    /// Fetches until `at_ms` is cached (at most [`MAX_PAGES_PER_ENSURE`]
    /// pages) and returns the cached stretch around it, or the stretch
    /// fetched so far when the limit came first. Already cached: no
    /// request.
    pub async fn ensure(&self, at_ms: u64) -> Result<Coverage, ChatReplayError> {
        let _one = self.fetching.lock().await;
        if let Some(c) = self.covered(at_ms) {
            return Ok(c);
        }
        let start_ms = match self.state().end_before(at_ms) {
            Some(end) if at_ms - end <= CONTINUE_GAP_MS => end,
            _ => at_ms.saturating_sub(LOOKBACK_MS),
        };
        let mut sec = start_ms / 1000;
        let mut run_from = sec * 1000;
        let mut next = PageStart::Offset(sec);
        for done in 1..=MAX_PAGES_PER_ENSURE {
            let page = match self.fetch(&next).await {
                Ok(p) => p,
                // A cursor the integrity check refuses: the rest of that
                // second is skipped, offsets go on.
                Err(ChatReplayError::IntegrityRequired | ChatReplayError::Gql { .. })
                    if matches!(next, PageStart::Cursor(_)) =>
                {
                    sec += 1;
                    self.state().add(run_from, sec * 1000);
                    next = PageStart::Offset(sec);
                    continue;
                }
                Err(e) => return Err(e),
            };
            let last_sec = page
                .comments
                .iter()
                .map(|c| c.offset_ms() / 1000)
                .max()
                .unwrap_or(sec);
            let has_next = page.has_next;
            let cursor = page.last_cursor.clone();
            self.merge(page);
            if !has_next {
                self.state().add(run_from, u64::MAX);
                return Ok(self.covered(at_ms).unwrap_or(coverage(run_from, u64::MAX)));
            }
            if last_sec > sec {
                // The last second may go on in the next page: it is asked
                // again (duplicates are dropped by id).
                sec = last_sec;
                next = PageStart::Offset(sec);
            } else if let Some(c) = cursor {
                // A whole page inside one second: go on after its last one.
                next = PageStart::Cursor(c);
                continue;
            } else {
                sec += 1;
                next = PageStart::Offset(sec);
            }
            self.state().add(run_from, sec * 1000);
            if let Some(c) = self.covered(at_ms) {
                return Ok(c);
            }
            // Far behind the playhead in a busy chat: start again just
            // before it rather than page through the whole gap.
            let per_page = (sec * 1000 - run_from) / done as u64;
            let left = (MAX_PAGES_PER_ENSURE - done) as u64;
            if at_ms > sec * 1000 && at_ms - sec * 1000 > per_page.max(1000) * left {
                sec = at_ms.saturating_sub(per_page.clamp(1000, LOOKBACK_MS)) / 1000;
                run_from = sec * 1000;
                next = PageStart::Offset(sec);
            }
        }
        Ok(self
            .covered(at_ms)
            .unwrap_or(coverage(run_from, (sec * 1000).max(run_from))))
    }

    async fn fetch(&self, start: &PageStart) -> Result<Page, ChatReplayError> {
        gql::fetch_page(
            &*self.transport,
            &self.url,
            &self.settings,
            &self.video_id,
            start,
        )
        .await
    }

    fn merge(&self, page: Page) {
        let mut s = self.state();
        s.stats.pages += 1;
        s.stats.deleted += page.deleted;
        if s.channel_id.is_none() {
            s.channel_id = page.comments.iter().find_map(|c| c.channel_id.clone());
        }
        s.timeline.merge(page.comments);
        s.stats.comments = s.timeline.len();
    }

    /// Fetches the whole chat from `from_ms` to the end of the VOD (or to
    /// `to_ms`), at most `max_pages` pages; for the chat file of a
    /// download (TWI-017).
    pub async fn fetch_all(
        &self,
        from_ms: u64,
        to_ms: Option<u64>,
        max_pages: usize,
    ) -> Result<(), ChatReplayError> {
        let mut at = from_ms;
        for _ in 0..max_pages.div_ceil(MAX_PAGES_PER_ENSURE).max(1) {
            let c = self.ensure(at).await?;
            match c.to_ms {
                None => return Ok(()),
                Some(end) if to_ms.is_some_and(|t| end >= t) => return Ok(()),
                Some(end) => at = end.max(at + 1),
            }
        }
        Ok(())
    }

    /// The cached chat as a TwitchDownloader file.
    pub fn to_file(&self, channel_name: &str, title: Option<&str>) -> ChatReplayFile {
        let s = self.state();
        let comments: Vec<_> = s
            .timeline
            .items()
            .iter()
            .map(|t| t.comment.clone())
            .collect();
        let end = comments
            .iter()
            .map(|c| c.content_offset_seconds)
            .fold(0.0, f64::max);
        ChatReplayFile {
            file_info: Some(serde_json::json!({
                "Version": {"Major": 1, "Minor": 4, "Patch": 0},
            })),
            streamer: Some(Streamer {
                name: channel_name.to_owned(),
                id: s.channel_id.clone().unwrap_or_default(),
                ..Streamer::default()
            }),
            video: Some(VideoInfo {
                id: Some(self.video_id.clone()),
                title: title.map(str::to_owned),
                start: 0.0,
                end,
                ..VideoInfo::default()
            }),
            comments,
            embedded_data: None,
        }
    }
}
