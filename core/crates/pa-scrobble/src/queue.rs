// SPDX-License-Identifier: MPL-2.0
//! The offline queue of one service: scrobbles wait here, in order, until
//! the service takes them. Kept in a JSON file (written whole after every
//! change; a few hundred bytes per scrobble), so they survive a restart.
//!
//! After a failed attempt the queue waits [`backoff_ms`] (30 s doubling up
//! to 1 h, or what the service asked for) before it tries again; an
//! account that must sign in again holds the queue until it is set anew.
//! At most [`MAX_LEN`] scrobbles are kept; the oldest go first.

use std::collections::VecDeque;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Listen, Outcome};

/// Most scrobbles kept per service.
pub const MAX_LEN: usize = 5_000;

/// Pause before attempt `attempts + 1` after `attempts` failures, ms.
pub fn backoff_ms(attempts: u32) -> u64 {
    let base: u64 = 30_000;
    base.saturating_mul(1u64 << attempts.saturating_sub(1).min(7))
        .min(3_600_000)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    listens: VecDeque<Listen>,
    #[serde(default)]
    attempts: u32,
    #[serde(default)]
    next_try_ms: u64,
    #[serde(default)]
    needs_auth: bool,
}

/// One service's queue.
#[derive(Debug)]
pub struct Queue {
    path: Option<PathBuf>,
    s: Stored,
}

impl Queue {
    /// A queue kept in `path` (`None`: in memory only); an unreadable file
    /// starts empty.
    pub fn open(path: Option<PathBuf>) -> Self {
        let s = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { path, s }
    }

    pub fn len(&self) -> usize {
        self.s.listens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.s.listens.is_empty()
    }

    /// The account must sign in again before the queue goes out.
    pub fn needs_auth(&self) -> bool {
        self.s.needs_auth
    }

    /// Failed attempts in a row.
    pub fn attempts(&self) -> u32 {
        self.s.attempts
    }

    /// When the next attempt is due, Unix ms (0: now).
    pub fn next_try_ms(&self) -> u64 {
        self.s.next_try_ms
    }

    /// Adds a scrobble at the end.
    pub fn push(&mut self, listen: Listen) {
        self.s.listens.push_back(listen);
        while self.s.listens.len() > MAX_LEN {
            self.s.listens.pop_front();
        }
        self.save();
    }

    /// The first `max` scrobbles, if an attempt is due at `now_ms`.
    pub fn due(&self, max: usize, now_ms: u64) -> Option<Vec<Listen>> {
        if self.s.listens.is_empty() || self.s.needs_auth || now_ms < self.s.next_try_ms {
            return None;
        }
        Some(self.s.listens.iter().take(max).cloned().collect())
    }

    /// The answer to the first `sent` scrobbles at `now_ms`.
    pub fn answer(&mut self, sent: usize, outcome: &Outcome, now_ms: u64) {
        match outcome {
            Outcome::Done | Outcome::Drop => {
                let n = sent.min(self.s.listens.len());
                self.s.listens.drain(..n);
                self.s.attempts = 0;
                self.s.next_try_ms = 0;
            }
            Outcome::Retry { after_ms } => {
                self.s.attempts = self.s.attempts.saturating_add(1);
                let wait = after_ms.unwrap_or_else(|| backoff_ms(self.s.attempts));
                self.s.next_try_ms = now_ms.saturating_add(wait);
            }
            Outcome::Auth => self.s.needs_auth = true,
        }
        self.save();
    }

    /// The account was set again: try at once.
    pub fn reset(&mut self) {
        self.s.needs_auth = false;
        self.s.attempts = 0;
        self.s.next_try_ms = 0;
        self.save();
    }

    /// Forgets every scrobble (the user disconnected the account).
    pub fn clear(&mut self) {
        self.s = Stored::default();
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(text) = serde_json::to_string(&self.s) else {
            return;
        };
        let tmp = path.with_extension("tmp");
        let written = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            tracing::warn!(error = %e.kind(), "scrobble queue not saved");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Track;

    fn listen(n: u64) -> Listen {
        Listen {
            track: Track {
                artist: "A".into(),
                title: format!("T{n}"),
                ..Track::default()
            },
            started_at: n,
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_ms(1), 30_000);
        assert_eq!(backoff_ms(2), 60_000);
        assert_eq!(backoff_ms(3), 120_000);
        assert_eq!(backoff_ms(100), 3_600_000);
    }

    #[test]
    fn retries_wait_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.json");
        let mut q = Queue::open(Some(path.clone()));
        for n in 1..=3 {
            q.push(listen(n));
        }
        let batch = q.due(2, 1_000).unwrap();
        assert_eq!(batch, vec![listen(1), listen(2)]);
        q.answer(2, &Outcome::Retry { after_ms: None }, 1_000);
        assert_eq!(q.due(2, 30_999), None, "waits 30 s");
        let q = Queue::open(Some(path.clone()));
        assert_eq!(q.len(), 3, "kept on disk");
        assert_eq!(q.attempts(), 1);
        let mut q = q;
        let batch = q.due(2, 31_000).unwrap();
        q.answer(batch.len(), &Outcome::Done, 31_000);
        assert_eq!(q.due(50, 31_000).unwrap(), vec![listen(3)]);
        q.answer(1, &Outcome::Auth, 31_000);
        assert_eq!(q.due(50, 99_000), None, "waits for a new sign-in");
        assert!(Queue::open(Some(path.clone())).needs_auth());
        q.reset();
        q.answer(1, &Outcome::Drop, 99_000);
        assert!(q.is_empty());
        assert!(Queue::open(Some(path)).is_empty());
    }

    #[test]
    fn oldest_go_first_when_full() {
        let mut q = Queue::open(None);
        for n in 0..(MAX_LEN as u64 + 2) {
            q.push(listen(n));
        }
        assert_eq!(q.len(), MAX_LEN);
        assert_eq!(q.due(1, 0).unwrap(), vec![listen(2)]);
        q.answer(1, &Outcome::Retry { after_ms: Some(10) }, 0);
        assert_eq!(q.due(1, 9), None);
        assert!(q.due(1, 10).is_some());
    }
}
