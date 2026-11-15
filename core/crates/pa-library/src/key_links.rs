// SPDX-License-Identifier: MPL-2.0
//! Keys other devices say belong to one item (SYNC-004, `sync_key_links`).
//!
//! The same film or episode has different keys on different devices: one
//! recognised it (`tmdb:movie:603`), another did not and keys it by the
//! content of its own copy (`file:<oshash>`), a third has another version of
//! it. Every progress op and `pb.*` message carries its `key` and the other
//! keys its sender knows for the item (`aliases`). The receiver stores the
//! progress under `key` as before, and [`learn`] remembers that the aliases
//! belong to it. An item's progress is then read over its own keys and the
//! keys [`linked`] to them ([`equivalent`]), newest row first
//! ([`crate::progress::newest`]).
//!
//! Links are one hop: each op carries all keys of its sender, so a key and
//! its aliases are always linked directly.
//!
//! ```
//! use pa_library::key_links;
//! use pa_model::MediaKey;
//!
//! let conn = pa_library::open_in_memory().unwrap();
//! let tmdb: MediaKey = "tmdb:movie:603".parse().unwrap();
//! let file: MediaKey = "file:8e245d9679d31e12".parse().unwrap();
//! key_links::learn(&conn, &tmdb, &[file.clone()], 1_000).unwrap();
//! assert_eq!(key_links::linked(&conn, &[file.clone()]).unwrap(), vec![tmdb.clone()]);
//! assert_eq!(key_links::equivalent(&conn, &[file.clone()]).unwrap(), vec![file, tmdb]);
//! ```

use std::collections::HashSet;

use pa_model::MediaKey;
use rusqlite::{Connection, params};

use crate::Error;

/// At most this many aliases are read from one op or message.
pub const MAX_ALIASES: usize = 16;

/// Remembers that `aliases` are other keys of the item `key` names. Invalid
/// keys, `key` itself and aliases past [`MAX_ALIASES`] are skipped. Returns
/// how many links are new.
pub fn learn(
    conn: &Connection,
    key: &MediaKey,
    aliases: &[MediaKey],
    at_ms: i64,
) -> Result<usize, Error> {
    if !key.is_valid() {
        return Err(Error::Invalid("media key"));
    }
    let key_text = key.to_string();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO sync_key_links (media_key, alias, learned_at_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT (media_key, alias) DO NOTHING",
    )?;
    let mut added = 0;
    for alias in aliases.iter().take(MAX_ALIASES) {
        if alias == key || !alias.is_valid() {
            continue;
        }
        added += stmt.execute(params![key_text, alias.to_string(), at_ms])?;
    }
    Ok(added)
}

/// Keys linked to any of `keys` (in either direction), not in `keys`,
/// in a stable order. Stored text that no longer parses is skipped.
pub fn linked(conn: &Connection, keys: &[MediaKey]) -> Result<Vec<MediaKey>, Error> {
    let own: HashSet<String> = keys.iter().map(ToString::to_string).collect();
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut by_alias = conn.prepare_cached(
        "SELECT media_key FROM sync_key_links WHERE alias = ?1 ORDER BY media_key",
    )?;
    let mut by_key = conn
        .prepare_cached("SELECT alias FROM sync_key_links WHERE media_key = ?1 ORDER BY alias")?;
    for key in keys {
        let text = key.to_string();
        let mut other: Vec<String> = by_alias
            .query_map([&text], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        other.extend(
            by_key
                .query_map([&text], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?,
        );
        for t in other {
            if own.contains(&t) || !seen.insert(t.clone()) {
                continue;
            }
            if let Ok(k) = t.parse::<MediaKey>() {
                found.push(k);
            }
        }
    }
    Ok(found)
}

/// `own` keys first (deduplicated, in order), then the keys [`linked`] to
/// them: every key progress of one item may be stored under.
pub fn equivalent(conn: &Connection, own: &[MediaKey]) -> Result<Vec<MediaKey>, Error> {
    let mut seen = HashSet::new();
    let mut out: Vec<MediaKey> = own.iter().filter(|k| seen.insert(*k)).cloned().collect();
    out.extend(linked(conn, &out)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> MediaKey {
        s.parse().unwrap()
    }

    #[test]
    fn links_are_one_hop_and_both_ways() {
        let conn = crate::open_in_memory().unwrap();
        let tmdb = k("tmdb:movie:603");
        let a = k("file:aaaa");
        let b = k("file:bbbb");
        let ed2k = k("ed2k:31d6cfe0d16ae931b73c59d7e0c089c0:10");
        assert_eq!(
            learn(&conn, &tmdb, &[a.clone(), b.clone(), tmdb.clone()], 1).unwrap(),
            2
        );
        assert_eq!(
            learn(&conn, &tmdb, std::slice::from_ref(&a), 2).unwrap(),
            0,
            "known"
        );
        learn(&conn, &a, std::slice::from_ref(&ed2k), 3).unwrap();
        // From `b`: its key; not `a` (two hops away).
        assert_eq!(
            linked(&conn, std::slice::from_ref(&b)).unwrap(),
            vec![tmdb.clone()]
        );
        // From the key: every alias.
        assert_eq!(
            linked(&conn, std::slice::from_ref(&tmdb)).unwrap(),
            vec![a.clone(), b.clone()]
        );
        assert_eq!(
            equivalent(&conn, &[a.clone(), a.clone()]).unwrap(),
            vec![a, tmdb, ed2k]
        );
    }

    #[test]
    fn at_most_max_aliases_are_kept() {
        let conn = crate::open_in_memory().unwrap();
        let aliases: Vec<MediaKey> = (0..40).map(|i| k(&format!("file:{i:04x}"))).collect();
        let n = learn(&conn, &k("tmdb:movie:1"), &aliases, 1).unwrap();
        assert_eq!(n, MAX_ALIASES);
    }
}
