// SPDX-License-Identifier: MPL-2.0
//! Matching the same media across devices (SYNC-004), without I/O.
//!
//! One film or episode has several keys: provider ids (`tmdb:movie:`,
//! `tmdb:episode:`, `anidb:episode:`), the `file:` key (OSHash, which mixes
//! in the size) and the `ed2k:` key (hash and size) of every version a
//! device has. Devices that recognised different things, or hold other
//! versions, therefore key the same item differently. [`ItemKeys`] is what
//! a device knows: the `key` an op or `pb.*` message is about, by priority
//!
//! 1. provider ids — they are the same for every version of the work;
//! 2. `file:` — the same copy on another path, host or source;
//! 3. `ed2k:` — the same copy, checked over the whole file;
//!
//! and the other keys as `aliases`. Protocol v1 peers only parse the old
//! kinds, so `key` is always one of them ([`MediaKey::is_v1_wire`]: a TMDB
//! id or `file:`); `anidb:` and `ed2k:` go only in `aliases`, a list of
//! strings old peers ignore and new peers read leniently ([`parse_aliases`]).
//!
//! Progress follows the work and episode, not the file. The exact position
//! of another version is only used when the durations of the two files are
//! close ([`durations_close`]: the same cut in another encoding); for a
//! different cut the position keeps its share of the work and is mapped
//! onto this file's duration ([`carry_position`]).

use pa_model::MediaKey;

/// At most this many aliases are sent with or read from one op or message.
pub const MAX_ALIASES: usize = 16;

/// Two durations this close (or this fraction of the longer one) are the
/// same cut in two encodings: a resume position carries over.
pub const DURATION_TOLERANCE_MS: u64 = 10_000;

/// Relative tolerance of [`durations_close`]: 1 %.
pub const DURATION_TOLERANCE_PERMILLE: u64 = 10;

/// Every key a device knows for one item.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemKeys {
    /// The key ops and messages are about: a v1 kind.
    pub key: MediaKey,
    /// The other keys, highest priority first, at most [`MAX_ALIASES`].
    pub aliases: Vec<MediaKey>,
}

/// Priority of a key kind (lower first): provider ids, `file:`, `ed2k:`.
pub fn priority(key: &MediaKey) -> u8 {
    match key {
        MediaKey::File { .. } => 1,
        MediaKey::Ed2k { .. } => 2,
        _ => 0,
    }
}

impl ItemKeys {
    /// Orders `keys` by [`priority`] (stable), drops duplicates and invalid
    /// keys and makes the first v1 key the `key`. `None` without one.
    pub fn new(keys: impl IntoIterator<Item = MediaKey>) -> Option<Self> {
        let mut all: Vec<MediaKey> = Vec::new();
        for k in keys {
            if k.is_valid() && !all.contains(&k) {
                all.push(k);
            }
        }
        all.sort_by_key(priority);
        let at = all.iter().position(MediaKey::is_v1_wire)?;
        let key = all.remove(at);
        all.truncate(MAX_ALIASES);
        Some(Self { key, aliases: all })
    }

    /// Only `key`, no aliases.
    pub fn single(key: MediaKey) -> Self {
        Self {
            key,
            aliases: Vec::new(),
        }
    }

    /// `key` and the aliases.
    pub fn all(&self) -> impl Iterator<Item = &MediaKey> {
        std::iter::once(&self.key).chain(&self.aliases)
    }

    /// The aliases in their text form, for an op or a message.
    pub fn alias_texts(&self) -> Vec<String> {
        self.aliases.iter().map(ToString::to_string).collect()
    }

    /// Whether a message about `key` with `aliases` names this item: any
    /// key in common.
    pub fn matches(&self, key: &MediaKey, aliases: &[String]) -> bool {
        let theirs = parse_aliases(aliases);
        std::iter::once(key)
            .chain(&theirs)
            .any(|k| self.all().any(|mine| mine == k))
    }
}

/// The aliases of an op or message that this build understands, at most
/// [`MAX_ALIASES`]; unknown kinds (from newer peers) are skipped.
pub fn parse_aliases(aliases: &[String]) -> Vec<MediaKey> {
    aliases
        .iter()
        .take(MAX_ALIASES)
        .filter_map(|a| a.parse::<MediaKey>().ok())
        .collect()
}

/// Whether two files of one work are the same cut, so a position in one is
/// the same moment in the other: within [`DURATION_TOLERANCE_MS`] or 1 % of
/// the longer one. Unknown durations count as close.
pub fn durations_close(a: Option<u64>, b: Option<u64>) -> bool {
    match (a.filter(|d| *d > 0), b.filter(|d| *d > 0)) {
        (Some(a), Some(b)) => {
            let tolerance = DURATION_TOLERANCE_MS.max(a.max(b) / 1000 * DURATION_TOLERANCE_PERMILLE);
            a.abs_diff(b) <= tolerance
        }
        _ => true,
    }
}

/// A position `position_ms` in a file of `from_ms` as a position in this
/// device's file of `to_ms`, with the duration to show: the same position
/// when the durations are close (or unknown), else the same share of the
/// work (an extended cut, another frame rate).
pub fn carry_position(
    position_ms: u64,
    from_ms: Option<u64>,
    to_ms: Option<u64>,
) -> (u64, Option<u64>) {
    let from_known = from_ms.filter(|d| *d > 0);
    match (from_known, to_ms.filter(|d| *d > 0)) {
        (Some(from), Some(to)) if !durations_close(Some(from), Some(to)) => {
            let scaled = u128::from(position_ms.min(from)) * u128::from(to) / u128::from(from);
            (u64::try_from(scaled).unwrap_or(to), Some(to))
        }
        _ => (position_ms, from_known.or(to_ms)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> MediaKey {
        s.parse().unwrap()
    }

    #[test]
    fn provider_ids_lead_and_new_kinds_stay_aliases() {
        let keys = ItemKeys::new([
            k("file:aaaa"),
            k("ed2k:31d6cfe0d16ae931b73c59d7e0c089c0:10"),
            k("tmdb:episode:1:1:2"),
            k("anidb:episode:23:2"),
            k("file:aaaa"),
        ])
        .unwrap();
        assert_eq!(keys.key, k("tmdb:episode:1:1:2"));
        assert_eq!(
            keys.aliases,
            vec![
                k("anidb:episode:23:2"),
                k("file:aaaa"),
                k("ed2k:31d6cfe0d16ae931b73c59d7e0c089c0:10"),
            ]
        );
        // Without a TMDB id the AniDB key is not the key: v1 peers would
        // drop the op. `file:` is.
        let anime = ItemKeys::new([k("anidb:episode:23:2"), k("file:bbbb")]).unwrap();
        assert_eq!(anime.key, k("file:bbbb"));
        assert_eq!(anime.aliases, vec![k("anidb:episode:23:2")]);
        assert!(ItemKeys::new([k("anidb:episode:23:2")]).is_none());
    }

    #[test]
    fn a_message_matches_through_any_common_key() {
        let mine = ItemKeys::new([k("file:aaaa"), k("file:bbbb")]).unwrap();
        assert!(mine.matches(&k("tmdb:movie:603"), &["file:bbbb".into()]));
        assert!(mine.matches(&k("file:aaaa"), &[]));
        assert!(!mine.matches(&k("tmdb:movie:603"), &["file:cccc".into()]));
        // Unknown kinds of newer peers are skipped, not fatal.
        assert!(mine.matches(&k("tmdb:movie:603"), &["imdb:tt1".into(), "file:aaaa".into()]));
    }

    #[test]
    fn same_cut_within_ten_seconds_or_one_percent() {
        let h2 = 2 * 3_600_000;
        assert!(durations_close(Some(h2), Some(h2 + 1_500)));
        assert!(durations_close(Some(h2), Some(h2 + 72_000)), "1 % of 2 h");
        assert!(!durations_close(Some(h2), Some(h2 + 20 * 60_000)), "extended cut");
        assert!(durations_close(Some(60_000), Some(69_000)));
        assert!(!durations_close(Some(60_000), Some(71_000)));
        assert!(durations_close(None, Some(h2)));
        assert!(durations_close(Some(0), Some(h2)));
    }

    #[test]
    fn a_position_carries_exactly_or_by_share() {
        let h2 = 2 * 3_600_000;
        // 4K remux and 1080p encode of one cut: the same second.
        assert_eq!(
            carry_position(600_000, Some(h2), Some(h2 + 800)),
            (600_000, Some(h2))
        );
        // Extended cut, 2 h 30 min: the same share.
        let ext = h2 + 30 * 60_000;
        assert_eq!(
            carry_position(h2 / 2, Some(h2), Some(ext)),
            (ext / 2, Some(ext))
        );
        // Unknown durations: as is.
        assert_eq!(carry_position(5, None, Some(h2)), (5, Some(h2)));
        assert_eq!(carry_position(5, Some(h2), None), (5, Some(h2)));
    }
}
