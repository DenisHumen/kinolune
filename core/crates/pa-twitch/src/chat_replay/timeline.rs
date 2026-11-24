// SPDX-License-Identifier: MPL-2.0
//! Comments on the VOD's timeline (06 §3.4): sorted by offset, found by
//! binary search.

use std::collections::HashSet;
use std::ops::Range;

use super::td::Comment;

/// A comment and the moment it is shown, ms from the VOD start.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedComment {
    /// When the replay shows it.
    pub at_ms: u64,
    /// The comment.
    pub comment: Comment,
}

/// Comments sorted by the moment they are shown (ties keep the order they
/// came in), without duplicates and without deleted ones.
///
/// GQL gives whole seconds, so a busy second would arrive as one burst
/// every second. Comments that share a whole-second offset are spread
/// evenly over that second (`n`-th of `k` at `s + n/k`), the way the
/// Twitch player shows them; offsets with a fraction (our recordings,
/// TwitchDownloader files written from them) are kept as they are.
#[derive(Debug, Clone, Default)]
pub struct ReplayTimeline {
    items: Vec<TimedComment>,
    ids: HashSet<String>,
}

impl ReplayTimeline {
    /// An empty timeline.
    pub fn new() -> Self {
        Self::default()
    }

    /// A timeline of `comments`.
    pub fn from_comments(comments: impl IntoIterator<Item = Comment>) -> Self {
        let mut t = Self::new();
        t.merge(comments);
        t
    }

    /// Adds the comments not seen yet; returns how many were added.
    pub fn merge(&mut self, comments: impl IntoIterator<Item = Comment>) -> usize {
        let before = self.items.len();
        for c in comments {
            if c.is_deleted() || (!c.id.is_empty() && !self.ids.insert(c.id.clone())) {
                continue;
            }
            self.items.push(TimedComment {
                at_ms: c.offset_ms(),
                comment: c,
            });
        }
        let added = self.items.len() - before;
        if added > 0 {
            self.spread();
        }
        added
    }

    /// Re-sorts and spreads the whole-second groups (stable: equal raw
    /// offsets keep their arrival order).
    fn spread(&mut self) {
        self.items.sort_by_key(|t| t.comment.offset_ms());
        let mut i = 0;
        while i < self.items.len() {
            let raw = self.items[i].comment.offset_ms();
            let mut j = i + 1;
            while j < self.items.len() && self.items[j].comment.offset_ms() == raw {
                j += 1;
            }
            let k = (j - i) as u64;
            let whole = raw % 1000 == 0;
            for (n, item) in self.items[i..j].iter_mut().enumerate() {
                item.at_ms = if whole && k > 1 {
                    raw + n as u64 * 1000 / k
                } else {
                    raw
                };
            }
            i = j;
        }
    }

    /// Number of comments.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// No comments.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// All comments in order.
    pub fn items(&self) -> &[TimedComment] {
        &self.items
    }

    /// Whether a comment with this id is in the timeline.
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Binary search: how many comments are shown at or before `at_ms`.
    pub fn count_until(&self, at_ms: u64) -> usize {
        self.items.partition_point(|t| t.at_ms <= at_ms)
    }

    /// Comments shown in `(after_ms, until_ms]` (both by binary search).
    pub fn between(&self, after_ms: Option<u64>, until_ms: u64) -> Range<usize> {
        let start = after_ms.map_or(0, |a| self.count_until(a));
        let end = self.count_until(until_ms).max(start);
        start..end
    }

    /// The last `max` comments shown at or before `at_ms`: what the chat
    /// shows after a seek to `at_ms`.
    pub fn window(&self, at_ms: u64, max: usize) -> Range<usize> {
        let end = self.count_until(at_ms);
        end.saturating_sub(max)..end
    }

    /// Moment of the last comment.
    pub fn last_ms(&self) -> Option<u64> {
        self.items.last().map(|t| t.at_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, secs: f64) -> Comment {
        Comment {
            id: id.into(),
            content_offset_seconds: secs,
            ..Comment::default()
        }
    }

    #[test]
    fn sorted_deduplicated_and_spread() {
        let mut t = ReplayTimeline::from_comments([c("b", 2.0), c("a", 1.0), c("c", 2.0)]);
        assert_eq!(t.merge([c("a", 1.0), c("d", 2.0), c("e", 3.25)]), 2);
        let at: Vec<(String, u64)> = t
            .items()
            .iter()
            .map(|x| (x.comment.id.clone(), x.at_ms))
            .collect();
        assert_eq!(
            at,
            [
                ("a".into(), 1000),
                ("b".into(), 2000),
                ("c".into(), 2333),
                ("d".into(), 2666),
                ("e".into(), 3250),
            ]
        );
    }

    #[test]
    fn binary_search_window() {
        let t = ReplayTimeline::from_comments((0..100).map(|i| c(&i.to_string(), f64::from(i))));
        assert_eq!(t.count_until(0), 1);
        assert_eq!(t.count_until(49_999), 50);
        assert_eq!(t.count_until(50_000), 51);
        assert_eq!(t.window(50_000, 10), 41..51);
        assert_eq!(t.window(2_000, 10), 0..3);
        assert_eq!(t.between(Some(10_000), 12_000), 11..13);
        assert_eq!(t.between(Some(12_000), 10_000), 13..13);
        assert_eq!(t.between(None, 1_500), 0..2);
    }
}
