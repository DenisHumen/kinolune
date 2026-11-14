// SPDX-License-Identifier: MPL-2.0
//! The match of a disc follows its main title (LIB-021, ADR-0043).
//!
//! The folder analysis picks the main title of a disc: before LIB-021 the
//! largest `.m2ts` of a Blu-ray, now the clips of its longest playlist.
//! Matches (`matches`) and, for works without a TMDB id, progress, lists
//! and favorites (the `file:` media key) hang on files. When the analysis
//! moves the main title to other files, the scan runs [`follow`] after the
//! fingerprints and before recognition, for every disc of the source:
//!
//! - the disc's match — a user-set or user-confirmed one first, then the
//!   oldest live match on its old or new title files — is copied to the
//!   new title files without any match, and replaces automatic matches to
//!   another work on them;
//! - live matches on streams that are no longer part of the title (now
//!   `ignore`) are deleted: such a stream is not an item;
//! - if the media key of the work changed (a `file:` key moves to the
//!   fingerprint of another file), rows of `progress`, `favorites`,
//!   `collection_items` and `playlist_items` under the old key move to the
//!   new one, unless the new key already has its own row.
//!
//! Recognition then finds every title file matched and leaves the disc as
//! it was: same work, same state, same progress.

use pa_library::rusqlite::{Connection, OptionalExtension, Transaction, params};
use pa_model::MediaKey;

use crate::Error;
use crate::items::ItemRef;
use crate::progress::key_of;

/// A live match of a file of a disc.
#[derive(Debug, Clone)]
struct Live {
    id: i64,
    file_id: i64,
    work_id: i64,
    state: String,
    created_at_ms: i64,
}

impl Live {
    fn by_user(&self) -> bool {
        self.state.starts_with("user_")
    }
}

/// Files of disc group `group`: the title files in play order and the
/// streams the analysis ignores.
fn disc_files(tx: &Transaction<'_>, group: i64) -> Result<(Vec<i64>, Vec<i64>), Error> {
    let title = tx
        .prepare_cached(
            "SELECT file_id FROM file_group_members
             WHERE group_id = ?1 AND role IN ('main', 'part')
             ORDER BY ifnull(part_no, 0), file_id",
        )?
        .query_map([group], |r| r.get(0))?
        .collect::<Result<Vec<i64>, _>>()?;
    let ignored = tx
        .prepare_cached(
            "SELECT file_id FROM group_assets
             WHERE group_id = ?1 AND role = 'ignore' AND file_id IS NOT NULL",
        )?
        .query_map([group], |r| r.get(0))?
        .collect::<Result<Vec<i64>, _>>()?;
    Ok((title, ignored))
}

fn live_match(tx: &Transaction<'_>, file: i64) -> Result<Option<Live>, Error> {
    Ok(tx
        .prepare_cached(
            "SELECT id, file_id, work_id, state, created_at_ms FROM matches
             WHERE file_id = ?1 AND state <> 'rejected'",
        )?
        .query_row([file], |r| {
            Ok(Live {
                id: r.get(0)?,
                file_id: r.get(1)?,
                work_id: r.get(2)?,
                state: r.get(3)?,
                created_at_ms: r.get(4)?,
            })
        })
        .optional()?)
}

fn has_any_match(tx: &Transaction<'_>, file: i64) -> Result<bool, Error> {
    Ok(tx
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM matches WHERE file_id = ?1)")?
        .query_row([file], |r| r.get(0))?)
}

/// The `file:` key of a movie work, if that is how its progress is kept.
fn file_key(conn: &Connection, work: i64) -> Result<Option<String>, Error> {
    let kind: Option<String> = conn
        .query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
        .optional()?;
    if kind.as_deref() != Some("movie") {
        return Ok(None);
    }
    Ok(match key_of(conn, ItemRef::Movie(work))? {
        Some((key @ MediaKey::File { .. }, _)) => Some(key.to_string()),
        _ => None,
    })
}

/// Copies match `from` (work, state, decision, episodes) onto `file`.
fn copy_match(tx: &Transaction<'_>, from: &Live, file: i64, now: i64) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO matches (file_id, work_id, scheme_used, confidence, state, decided_by,
                              features_json, explanation_json, model_version,
                              created_at_ms, updated_at_ms)
         SELECT ?2, work_id, scheme_used, confidence, state, decided_by, features_json,
                explanation_json, model_version, ?3, ?3
         FROM matches WHERE id = ?1",
        params![from.id, file, now],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO match_episodes (match_id, position, episode_id)
         SELECT ?2, position, episode_id FROM match_episodes WHERE match_id = ?1",
        params![from.id, id],
    )?;
    Ok(())
}

/// Moves rows keyed by media key `old` to `new` (see the module docs).
fn rekey(tx: &Transaction<'_>, old: &str, new: &str) -> Result<(), Error> {
    for table in ["progress", "favorites", "collection_items"] {
        tx.execute(
            &format!("UPDATE OR IGNORE {table} SET media_key = ?2 WHERE media_key = ?1"),
            params![old, new],
        )?;
    }
    tx.execute(
        "UPDATE playlist_items SET media_key = ?2 WHERE media_key = ?1",
        params![old, new],
    )?;
    Ok(())
}

/// Makes the matches of the discs of `source_id` follow their main titles
/// (see the module docs). Returns the number of discs changed.
pub(crate) fn follow(conn: &mut Connection, source_id: i64) -> Result<usize, Error> {
    let tx = conn.transaction()?;
    let groups: Vec<i64> = tx
        .prepare("SELECT id FROM file_groups WHERE source_id = ?1 AND kind = 'disc'")?
        .query_map([source_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let now = crate::now_ms();
    let mut changed = 0;
    for group in groups {
        let (title, ignored) = disc_files(&tx, group)?;
        if title.is_empty() {
            continue;
        }
        let mut stale = Vec::new();
        for f in &ignored {
            if let Some(m) = live_match(&tx, *f)? {
                stale.push(m);
            }
        }
        let mut on_title = Vec::new();
        let mut unmatched = Vec::new();
        for f in &title {
            match live_match(&tx, *f)? {
                Some(m) => on_title.push(m),
                None if !has_any_match(&tx, *f)? => unmatched.push(*f),
                // Only rejected matches: recognition decides.
                None => {}
            }
        }
        let Some(owner) = stale
            .iter()
            .chain(&on_title)
            .min_by_key(|m| (!m.by_user(), m.created_at_ms, m.id))
            .cloned()
        else {
            continue;
        };
        let wrong: Vec<&Live> = on_title
            .iter()
            .filter(|m| m.work_id != owner.work_id && !m.by_user())
            .collect();
        if stale.is_empty() && unmatched.is_empty() && wrong.is_empty() {
            continue;
        }

        // Keys before: the works of every live match of the disc.
        let mut works: Vec<i64> = stale.iter().chain(&on_title).map(|m| m.work_id).collect();
        works.sort_unstable();
        works.dedup();
        let before: Vec<(i64, Option<String>)> = works
            .iter()
            .map(|w| Ok((*w, file_key(&tx, *w)?)))
            .collect::<Result<_, Error>>()?;

        for m in &wrong {
            tx.execute("DELETE FROM matches WHERE id = ?1", [m.id])?;
            unmatched.push(m.file_id);
        }
        for f in &unmatched {
            copy_match(&tx, &owner, *f, now)?;
        }
        for m in &stale {
            tx.execute("DELETE FROM matches WHERE id = ?1", [m.id])?;
        }

        let owner_key = file_key(&tx, owner.work_id)?;
        for (work, old) in before {
            let Some(old) = old else { continue };
            // A work that lost its last file follows the disc's work.
            let new = file_key(&tx, work)?.or_else(|| owner_key.clone());
            if let Some(new) = new.filter(|n| *n != old) {
                rekey(&tx, &old, &new)?;
            }
        }
        changed += 1;
    }
    tx.commit()?;
    Ok(changed)
}
