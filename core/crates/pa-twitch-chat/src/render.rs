// SPDX-License-Identifier: MPL-2.0
//! Turning a [`ChatMessage`] into renderable fragments (TWI-011).
//!
//! The model of TWI-010 carries raw ids: emote ranges (character offsets)
//! and badge `set_id`/`version`. [`ChannelAssets::render`] resolves them
//! into text and emote fragments plus badge images, in one pass over the
//! text and without any I/O, so it can run for every message of a busy
//! chat (research 06 §3.4: 100 messages per second) on the thread that
//! receives them.

use std::sync::Arc;

use crate::assets::{BadgeAsset, ChannelAssets, EmoteAsset};
use crate::model::{ChatMessage, EmoteRange};

/// A piece of a message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fragment {
    /// Plain text (may contain spaces; the renderer wraps it).
    Text(String),
    /// An emote image.
    Emote {
        /// Shared asset: same `Arc` in every message that uses the emote.
        asset: Arc<EmoteAsset>,
        /// The code as typed (alt text, copy, and fallback while loading).
        code: String,
    },
}

/// A badge in front of the author's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedBadge {
    /// Shared asset.
    pub asset: Arc<BadgeAsset>,
    /// Extra info from the message, e.g. exact subscription months.
    pub info: Option<String>,
}

/// What the chat renderer (TWI-012) draws for one message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderedMessage {
    /// Badges in display order; unknown ones are skipped.
    pub badges: Vec<RenderedBadge>,
    /// Body: text and emotes in order. Empty for a system notice without
    /// user text.
    pub fragments: Vec<Fragment>,
}

impl RenderedMessage {
    /// Distinct emotes of this message (by key), in first-use order: what
    /// to prefetch.
    pub fn emotes(&self) -> Vec<&Arc<EmoteAsset>> {
        let mut seen: Vec<&Arc<EmoteAsset>> = Vec::new();
        for f in &self.fragments {
            if let Fragment::Emote { asset, .. } = f
                && !seen.iter().any(|s| Arc::ptr_eq(s, asset) || s.id == asset.id)
            {
                seen.push(asset);
            }
        }
        seen
    }
}

impl ChannelAssets {
    /// Resolves badges and emote ranges of `message`.
    ///
    /// Ranges that are empty, out of bounds or overlap an earlier range are
    /// ignored (their text stays text), so a malformed tag never loses
    /// characters.
    pub fn render(&self, message: &ChatMessage) -> RenderedMessage {
        let badges = message
            .user
            .badges
            .iter()
            .filter_map(|b| {
                self.badge(&b.set_id, &b.version).map(|asset| RenderedBadge {
                    asset,
                    info: b.info.clone(),
                })
            })
            .collect();
        RenderedMessage {
            badges,
            fragments: self.fragments(&message.text, &message.emotes),
        }
    }

    /// Splits `text` by emote ranges (character offsets, half-open).
    pub fn fragments(&self, text: &str, ranges: &[EmoteRange]) -> Vec<Fragment> {
        if text.is_empty() {
            return Vec::new();
        }
        if ranges.is_empty() {
            return vec![Fragment::Text(text.to_owned())];
        }
        let mut order: Vec<&EmoteRange> = ranges.iter().collect();
        order.sort_by_key(|r| (r.start, r.end));

        let char_count = text.chars().count();
        let mut cursor = CharCursor {
            chars: text.chars(),
            index: 0,
            byte: 0,
        };
        let mut out = Vec::with_capacity(order.len() * 2 + 1);
        let mut text_from = 0usize; // byte
        let mut next_char = 0usize; // first char not yet consumed
        for r in order {
            if r.start >= r.end || r.end > char_count || r.start < next_char {
                continue;
            }
            let (Some(start_b), Some(end_b)) = (cursor.byte_at(r.start), cursor.byte_at(r.end))
            else {
                continue;
            };
            if start_b > text_from {
                out.push(Fragment::Text(text[text_from..start_b].to_owned()));
            }
            out.push(Fragment::Emote {
                asset: self.emote(&r.id),
                code: text[start_b..end_b].to_owned(),
            });
            text_from = end_b;
            next_char = r.end;
        }
        if text_from < text.len() {
            out.push(Fragment::Text(text[text_from..].to_owned()));
        }
        out
    }
}

/// Forward-only map from character index to byte offset: one walk over the
/// text for all ranges of a message (asked indexes never decrease).
struct CharCursor<'a> {
    chars: std::str::Chars<'a>,
    index: usize,
    byte: usize,
}

impl CharCursor<'_> {
    fn byte_at(&mut self, want: usize) -> Option<usize> {
        while self.index < want {
            self.byte += self.chars.next()?.len_utf8();
            self.index += 1;
        }
        (self.index == want).then_some(self.byte)
    }
}
