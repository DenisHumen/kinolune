// SPDX-License-Identifier: MPL-2.0
//! Viewer-side chat filters (TWI-024, research 06 §3.3): a local block
//! list, a word filter (plain words or regular expressions, hide or mask),
//! mention highlighting and "hide bots".
//!
//! [`ChatFilters`] is compiled once from [`FilterSettings`] and applied to
//! every incoming [`ChatMessage`] on the session thread, before rendering:
//! hidden messages never reach the UI, masked words become `*` with the
//! character count unchanged (emote ranges stay valid; an emote inside a
//! masked word is dropped and shows as stars).
//!
//! Regular expressions use the `regex` crate: matching is linear in the
//! text, so a pattern a user typed cannot freeze the chat (no catastrophic
//! backtracking), and compiled programs are size-limited. Everything is
//! case-insensitive; `(?-i)` inside a pattern turns that off.

use std::collections::HashSet;

use regex::{Regex, RegexBuilder};

use crate::model::ChatMessage;

/// Bot accounts hidden by "hide bots" (lower-case logins): the common
/// channel bots and alert services. The user adds more in
/// [`FilterSettings::extra_bots`].
pub const KNOWN_BOTS: &[&str] = &[
    "nightbot",
    "streamelements",
    "streamlabs",
    "moobot",
    "fossabot",
    "wizebot",
    "botisimo",
    "deepbot",
    "coebot",
    "phantombot",
    "ankhbot",
    "sery_bot",
    "soundalerts",
    "kofistreambot",
    "streamstickers",
    "blerp",
    "pretzelrocks",
    "own3d",
    "lumiastream",
    "tangiabot",
    "commanderroot",
    "frostytoolsdotcom",
    "creatisbot",
    "mixitupbot",
    "streamcaptainbot",
];

/// Most words, keywords and blocked users kept (a pasted list cannot grow
/// without bound).
pub const MAX_FILTER_ENTRIES: usize = 1000;

/// Longest pattern accepted, in characters.
pub const MAX_PATTERN_CHARS: usize = 500;

/// Compiled size limit of one pattern (and of the combined program).
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// What a word rule does to a matching message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordAction {
    /// Hide the whole message.
    Hide,
    /// Replace the matched text with `*`.
    #[default]
    Mask,
}

/// One word-filter rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordRule {
    /// A word or phrase, or a regular expression when [`Self::regex`].
    pub pattern: String,
    /// [`Self::pattern`] is a regular expression (`regex` crate syntax).
    pub regex: bool,
    /// Hide or mask.
    pub action: WordAction,
}

/// What the viewer configured (per profile; the app stores it).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterSettings {
    /// Blocked users: logins (any case, optional `@`) or numeric user ids.
    pub blocked: Vec<String>,
    /// Word filter.
    pub words: Vec<WordRule>,
    /// Login of the signed-in user: messages naming it are mentions.
    /// `None` when signed out.
    pub own_login: Option<String>,
    /// Extra words that mark a message as a mention (plain, whole words).
    pub highlight_keywords: Vec<String>,
    /// Hide [`KNOWN_BOTS`] and [`Self::extra_bots`].
    pub hide_bots: bool,
    /// Logins the user considers bots.
    pub extra_bots: Vec<String>,
}

/// A word rule that could not be compiled; it is skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    /// Index in [`FilterSettings::words`].
    pub index: usize,
    /// Why.
    pub message: String,
}

/// Why a message is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HideReason {
    /// The author is on the block list.
    Blocked,
    /// The author is a bot and "hide bots" is on.
    Bot,
    /// A word rule with [`WordAction::Hide`] matched.
    Word,
}

/// Outcome of [`ChatFilters::apply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Verdict {
    /// The message must not be shown.
    pub hidden: Option<HideReason>,
    /// The message mentions the user (own name, a keyword, or a reply to
    /// the user's message).
    pub mention: bool,
    /// Some text was masked.
    pub masked: bool,
}

/// Compiled filters; cheap to share (`Arc`) between threads.
#[derive(Debug, Default)]
pub struct ChatFilters {
    blocked: HashSet<String>,
    bots: HashSet<String>,
    hide: Option<Regex>,
    mask: Option<Regex>,
    own_login: Option<String>,
    mention: Option<Regex>,
}

/// Normalises a login typed by a user: trimmed, without `@`, lower case.
fn login_key(s: &str) -> String {
    s.trim().trim_start_matches('@').to_lowercase()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A plain word or phrase as a pattern: escaped, with word boundaries on
/// the sides that start or end with a word character ("ass" does not
/// match "class", "!!" matches anywhere).
fn plain_pattern(word: &str) -> Option<String> {
    let word = word.trim();
    let first = word.chars().next()?;
    let last = word.chars().next_back()?;
    let mut p = String::new();
    if is_word_char(first) {
        p.push_str(r"\b");
    }
    p.push_str(&regex::escape(word));
    if is_word_char(last) {
        p.push_str(r"\b");
    }
    Some(p)
}

fn build(pattern: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| e.to_string())
}

/// The pattern of a rule, checked: what the settings screen shows next to
/// the field while the user types.
pub fn check_rule(rule: &WordRule) -> Result<(), String> {
    rule_pattern(rule).and_then(|p| build(&p).map(|_| ()))
}

fn rule_pattern(rule: &WordRule) -> Result<String, String> {
    if rule.pattern.trim().is_empty() {
        return Err("empty pattern".into());
    }
    if rule.pattern.chars().count() > MAX_PATTERN_CHARS {
        return Err(format!("longer than {MAX_PATTERN_CHARS} characters"));
    }
    if rule.regex {
        // A pattern that matches the empty string would mask nothing and
        // hide everything.
        let re = build(&rule.pattern)?;
        if re.is_match("") {
            return Err("matches empty text".into());
        }
        Ok(rule.pattern.clone())
    } else {
        plain_pattern(&rule.pattern).ok_or_else(|| "empty pattern".into())
    }
}

/// One program from several checked patterns.
fn combine(patterns: &[String]) -> Option<Regex> {
    if patterns.is_empty() {
        return None;
    }
    let joined = patterns
        .iter()
        .map(|p| format!("(?:{p})"))
        .collect::<Vec<_>>()
        .join("|");
    // Each part compiled alone; together they may still exceed the limit.
    build(&joined).ok()
}

impl ChatFilters {
    /// Compiles `settings`. Rules that do not compile are skipped and
    /// reported; the rest still work.
    pub fn new(settings: &FilterSettings) -> (Self, Vec<RuleError>) {
        let mut errors = Vec::new();
        let mut hide = Vec::new();
        let mut mask = Vec::new();
        for (index, rule) in settings.words.iter().enumerate().take(MAX_FILTER_ENTRIES) {
            match rule_pattern(rule).and_then(|p| build(&p).map(|_| p)) {
                Ok(p) => match rule.action {
                    WordAction::Hide => hide.push(p),
                    WordAction::Mask => mask.push(p),
                },
                Err(message) => errors.push(RuleError { index, message }),
            }
        }
        let own_login = settings
            .own_login
            .as_deref()
            .map(login_key)
            .filter(|l| !l.is_empty());
        let mut mention = Vec::new();
        if let Some(login) = &own_login {
            // `@name` or the bare name as a whole word.
            mention.push(format!(r"\b{}\b", regex::escape(login)));
        }
        mention.extend(
            settings
                .highlight_keywords
                .iter()
                .take(MAX_FILTER_ENTRIES)
                .filter_map(|k| plain_pattern(k)),
        );
        let bots = if settings.hide_bots {
            KNOWN_BOTS
                .iter()
                .map(|b| (*b).to_owned())
                .chain(
                    settings
                        .extra_bots
                        .iter()
                        .take(MAX_FILTER_ENTRIES)
                        .map(|b| login_key(b)),
                )
                .filter(|b| !b.is_empty())
                .collect()
        } else {
            HashSet::new()
        };
        let filters = Self {
            blocked: settings
                .blocked
                .iter()
                .take(MAX_FILTER_ENTRIES)
                .map(|b| login_key(b))
                .filter(|b| !b.is_empty())
                .collect(),
            bots,
            hide: combine(&hide),
            mask: combine(&mask),
            own_login,
            mention: combine(&mention),
        };
        (filters, errors)
    }

    /// No filter and no highlighting configured.
    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty()
            && self.bots.is_empty()
            && self.hide.is_none()
            && self.mask.is_none()
            && self.mention.is_none()
    }

    /// `true` when `login` / `user_id` is blocked.
    pub fn is_blocked(&self, user_id: &str, login: &str) -> bool {
        (!login.is_empty() && self.blocked.contains(&login.to_lowercase()))
            || (!user_id.is_empty() && self.blocked.contains(user_id))
    }

    /// Decides about `message` and masks its text in place.
    pub fn apply(&self, message: &mut ChatMessage) -> Verdict {
        let mut verdict = Verdict::default();
        let login = message.user.login.to_lowercase();
        let own = self.own_login.as_deref().is_some_and(|o| o == login);
        if !own {
            if self.is_blocked(&message.user.id, &login) {
                verdict.hidden = Some(HideReason::Blocked);
                return verdict;
            }
            if !login.is_empty() && self.bots.contains(&login) {
                verdict.hidden = Some(HideReason::Bot);
                return verdict;
            }
        }
        if self
            .hide
            .as_ref()
            .is_some_and(|re| re.is_match(&message.text))
        {
            verdict.hidden = Some(HideReason::Word);
            return verdict;
        }
        if let Some(re) = &self.mask {
            verdict.masked = mask(re, message);
        }
        if !own {
            let reply_to_me = match (&message.reply, &self.own_login) {
                (Some(r), Some(me)) => r.parent_user_login.eq_ignore_ascii_case(me),
                _ => false,
            };
            verdict.mention = reply_to_me
                || self
                    .mention
                    .as_ref()
                    .is_some_and(|re| re.is_match(&message.text));
        }
        verdict
    }
}

/// Replaces every match with `*` (one per character) and drops the emote
/// ranges it touches. Returns whether anything changed.
fn mask(re: &Regex, message: &mut ChatMessage) -> bool {
    let spans: Vec<(usize, usize)> = re
        .find_iter(&message.text)
        .filter(|m| !m.is_empty())
        .map(|m| (m.start(), m.end()))
        .collect();
    if spans.is_empty() {
        return false;
    }
    let mut out = String::with_capacity(message.text.len());
    // Masked character ranges, for the emote ranges.
    let mut masked_chars: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    let mut span = spans.iter().peekable();
    let mut char_index = 0usize;
    let mut open: Option<usize> = None;
    for (byte, c) in message.text.char_indices() {
        while span.peek().is_some_and(|(_, end)| byte >= *end) {
            span.next();
        }
        let inside = span
            .peek()
            .is_some_and(|(start, end)| byte >= *start && byte < *end);
        if inside {
            out.push('*');
            open.get_or_insert(char_index);
        } else {
            if let Some(s) = open.take() {
                masked_chars.push((s, char_index));
            }
            out.push(c);
        }
        char_index += 1;
    }
    if let Some(s) = open.take() {
        masked_chars.push((s, char_index));
    }
    message.text = out;
    message.emotes.retain(|e| {
        !masked_chars
            .iter()
            .any(|(s, end)| e.start < *end && *s < e.end)
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_patterns_respect_word_edges() {
        assert_eq!(plain_pattern("  "), None);
        assert_eq!(plain_pattern("spoiler").unwrap(), r"\bspoiler\b");
        assert_eq!(plain_pattern("!!").unwrap(), r"!!");
        assert_eq!(plain_pattern("a.b").unwrap(), r"\ba\.b\b");
    }

    #[test]
    fn check_rule_reports_bad_patterns() {
        let rule = |p: &str, regex| WordRule {
            pattern: p.into(),
            regex,
            action: WordAction::Mask,
        };
        assert!(check_rule(&rule("fine", false)).is_ok());
        assert!(check_rule(&rule("(unclosed", true)).is_err());
        assert!(check_rule(&rule("x*", true)).is_err(), "matches empty text");
        assert!(check_rule(&rule("", false)).is_err());
        assert!(check_rule(&rule(&"a".repeat(501), false)).is_err());
        // A plain word with regex characters is escaped, not parsed.
        assert!(check_rule(&rule("(unclosed", false)).is_ok());
    }
}
