// SPDX-License-Identifier: MPL-2.0
//! The user's own categories, tags and star ratings of items (LIB-022).
//!
//! - **Categories** ([`create_category`], [`add_to_category`]): named
//!   groups a profile makes; an item can be in several.
//! - **Tags** ([`add_tag`], [`set_tags`], [`suggest_tags`]): free words on
//!   an item, several per item; a tag exists while an item has it.
//! - **Ratings** ([`set_rating`]): 1–5 stars of an item.
//!
//! Everything is per profile (ADR-NNNN). Categories and tags also belong to
//! a [`Scope::space`]: [`MAIN_SPACE`] is the main library, the protected
//! library of UI-025 has its own spaces, so its names never reach the main
//! library. Ratings are per profile only: an item belongs to one library.
//!
//! Items are [`Entry`] references as in LIB-010 lists: a work or a stable
//! media key. Which entries the active profile may see is decided by the
//! caller (`pa-catalog`); this module only stores references and never
//! filters by visibility.
//!
//! Names are compared by [`name_key`]: trimmed, inner whitespace collapsed
//! to one space, lower case; the first spelling is kept for display.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Error;
use crate::lists::Entry;

/// Space of the main library.
pub const MAIN_SPACE: &str = "";

/// Longest category name, in characters.
pub const MAX_CATEGORY_NAME: usize = 100;

/// Longest tag, in characters.
pub const MAX_TAG_NAME: usize = 64;

/// Most tags one item may have.
pub const MAX_TAGS_PER_ITEM: usize = 50;

/// Whose labels and in which library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope<'a> {
    pub profile_id: i64,
    /// [`MAIN_SPACE`] or a protected library's space (`[a-z0-9_:-]`, at
    /// most 64 characters).
    pub space: &'a str,
}

impl Scope<'static> {
    /// The main library of `profile_id`.
    pub fn main(profile_id: i64) -> Self {
        Self {
            profile_id,
            space: MAIN_SPACE,
        }
    }
}

impl Scope<'_> {
    fn check(&self) -> Result<(), Error> {
        let ok = self.space.len() <= 64
            && self
                .space
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:-".contains(&b));
        if ok {
            Ok(())
        } else {
            Err(Error::Invalid("label space"))
        }
    }
}

/// Comparison key of a category or tag name: trimmed, whitespace runs as
/// one space, lower case. Empty for a blank name.
pub fn name_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The display form and key of a name, or `what` if it is blank, too long
/// or has control characters.
fn clean_name(name: &str, max: usize, what: &'static str) -> Result<(String, String), Error> {
    let shown = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if shown.is_empty() || shown.chars().count() > max || shown.chars().any(char::is_control) {
        return Err(Error::Invalid(what));
    }
    let key = shown.to_lowercase();
    Ok((shown, key))
}

fn check_entry(entry: &Entry) -> Result<(), Error> {
    match entry {
        Entry::Work(_) => Ok(()),
        Entry::Media(k) => {
            let ok = (3..=256).contains(&k.len())
                && k.split_once(':')
                    .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty());
            if ok {
                Ok(())
            } else {
                Err(Error::Invalid("media key"))
            }
        }
    }
}

fn columns(entry: &Entry) -> (Option<i64>, Option<&str>) {
    match entry {
        Entry::Work(w) => (Some(*w), None),
        Entry::Media(k) => (None, Some(k.as_str())),
    }
}

fn entry_of(work: Option<i64>, key: Option<String>) -> rusqlite::Result<Entry> {
    match (work, key) {
        (Some(w), None) => Ok(Entry::Work(w)),
        (None, Some(k)) => Ok(Entry::Media(k)),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

// ---------------------------------------------------------------------------
// categories

/// A category of a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub profile_id: i64,
    pub space: String,
    /// Id other devices know the category by.
    pub sync_id: String,
    pub name: String,
    pub position: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Entries, whether the profile may see them now or not.
    pub item_count: i64,
}

const CATEGORY_COLUMNS: &str = "c.id, c.profile_id, c.space, c.sync_id, c.name, c.position,
    c.created_at_ms, c.updated_at_ms,
    (SELECT count(*) FROM user_category_items i WHERE i.category_id = c.id)";

fn category_row(r: &Row<'_>) -> rusqlite::Result<Category> {
    Ok(Category {
        id: r.get(0)?,
        profile_id: r.get(1)?,
        space: r.get(2)?,
        sync_id: r.get(3)?,
        name: r.get(4)?,
        position: r.get(5)?,
        created_at_ms: r.get(6)?,
        updated_at_ms: r.get(7)?,
        item_count: r.get(8)?,
    })
}

/// A category by id, of any profile.
pub fn category(conn: &Connection, id: i64) -> Result<Option<Category>, Error> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {CATEGORY_COLUMNS} FROM user_categories c WHERE c.id = ?1"
        ))?
        .query_row([id], category_row)
        .optional()?)
}

fn name_taken(
    conn: &Connection,
    table: &str,
    scope: Scope<'_>,
    key: &str,
    except: Option<i64>,
) -> Result<bool, Error> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT 1 FROM {table} WHERE profile_id = ?1 AND space = ?2 AND name_key = ?3
               AND id IS NOT ?4"
        ))?
        .query_row(params![scope.profile_id, scope.space, key, except], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

/// Creates a category at the end of the scope's list. A name that is taken
/// (regardless of case) is [`Error::Invalid`]`("duplicate category")`.
pub fn create_category(
    conn: &Connection,
    scope: Scope<'_>,
    name: &str,
    now_ms: i64,
) -> Result<Category, Error> {
    scope.check()?;
    let (name, key) = clean_name(name, MAX_CATEGORY_NAME, "category name")?;
    if name_taken(conn, "user_categories", scope, &key, None)? {
        return Err(Error::Invalid("duplicate category"));
    }
    conn.prepare_cached(
        "INSERT INTO user_categories
             (profile_id, space, sync_id, name, name_key, position, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, lower(hex(randomblob(16))), ?3, ?4,
                 (SELECT ifnull(max(position) + 1, 0) FROM user_categories
                   WHERE profile_id = ?1 AND space = ?2),
                 ?5, ?5)",
    )?
    .execute(params![scope.profile_id, scope.space, name, key, now_ms])?;
    category(conn, conn.last_insert_rowid())?.ok_or(Error::Invalid("category"))
}

/// Renames a category; `false` if it does not exist. A name another
/// category of the scope has is [`Error::Invalid`]`("duplicate category")`.
pub fn rename_category(conn: &Connection, id: i64, name: &str, now_ms: i64) -> Result<bool, Error> {
    let (name, key) = clean_name(name, MAX_CATEGORY_NAME, "category name")?;
    let Some(c) = category(conn, id)? else {
        return Ok(false);
    };
    let scope = Scope {
        profile_id: c.profile_id,
        space: &c.space,
    };
    if name_taken(conn, "user_categories", scope, &key, Some(id))? {
        return Err(Error::Invalid("duplicate category"));
    }
    let n = conn
        .prepare_cached(
            "UPDATE user_categories SET name = ?2, name_key = ?3, updated_at_ms = ?4 WHERE id = ?1",
        )?
        .execute(params![id, name, key, now_ms])?;
    Ok(n > 0)
}

/// Deletes a category (its items stay in the library); `false` if it did
/// not exist.
pub fn delete_category(conn: &Connection, id: i64) -> Result<bool, Error> {
    let n = conn
        .prepare_cached("DELETE FROM user_categories WHERE id = ?1")?
        .execute([id])?;
    Ok(n > 0)
}

/// Categories of a scope by position, then name.
pub fn categories(conn: &Connection, scope: Scope<'_>) -> Result<Vec<Category>, Error> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM user_categories c
         WHERE c.profile_id = ?1 AND c.space = ?2
         ORDER BY c.position, c.name_key, c.id"
    ))?;
    let rows = stmt.query_map(params![scope.profile_id, scope.space], category_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Categories of a scope that hold `entry`, by position.
pub fn categories_of(
    conn: &Connection,
    scope: Scope<'_>,
    entry: &Entry,
) -> Result<Vec<Category>, Error> {
    let (work, key) = columns(entry);
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM user_categories c
         WHERE c.profile_id = ?1 AND c.space = ?2
           AND EXISTS (SELECT 1 FROM user_category_items i WHERE i.category_id = c.id
                         AND i.work_id IS ?3 AND i.media_key IS ?4)
         ORDER BY c.position, c.name_key, c.id"
    ))?;
    let rows = stmt.query_map(
        params![scope.profile_id, scope.space, work, key],
        category_row,
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Puts `entry` into category `id`; `false` if it already was there.
pub fn add_to_category(conn: &Connection, id: i64, entry: &Entry, now_ms: i64) -> Result<bool, Error> {
    check_entry(entry)?;
    let (work, key) = columns(entry);
    let n = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO user_category_items (category_id, work_id, media_key, added_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![id, work, key, now_ms])?;
    Ok(n > 0)
}

/// Takes `entry` out of category `id`; `false` if it was not there.
pub fn remove_from_category(conn: &Connection, id: i64, entry: &Entry) -> Result<bool, Error> {
    let (work, key) = columns(entry);
    let n = conn
        .prepare_cached(
            "DELETE FROM user_category_items
             WHERE category_id = ?1 AND work_id IS ?2 AND media_key IS ?3",
        )?
        .execute(params![id, work, key])?;
    Ok(n > 0)
}

/// Entries of category `id`, most recently added first.
pub fn category_entries(conn: &Connection, id: i64) -> Result<Vec<Entry>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT work_id, media_key FROM user_category_items WHERE category_id = ?1
         ORDER BY added_at_ms DESC, rowid DESC",
    )?;
    let rows = stmt.query_map([id], |r| entry_of(r.get(0)?, r.get(1)?))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// `(work_id, category_id)` of every work in a category of the scope.
pub fn work_categories(conn: &Connection, scope: Scope<'_>) -> Result<Vec<(i64, i64)>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.work_id, i.category_id FROM user_category_items i
         JOIN user_categories c ON c.id = i.category_id
         WHERE c.profile_id = ?1 AND c.space = ?2 AND i.work_id IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![scope.profile_id, scope.space], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---------------------------------------------------------------------------
// tags

/// A tag of a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: i64,
    /// First spelling used.
    pub name: String,
    /// [`name_key`] of the name.
    pub key: String,
}

fn tag_row(r: &Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: r.get(0)?,
        name: r.get(1)?,
        key: r.get(2)?,
    })
}

fn tag_by_key(conn: &Connection, scope: Scope<'_>, key: &str) -> Result<Option<Tag>, Error> {
    Ok(conn
        .prepare_cached(
            "SELECT id, name, name_key FROM user_tags
             WHERE profile_id = ?1 AND space = ?2 AND name_key = ?3",
        )?
        .query_row(params![scope.profile_id, scope.space, key], tag_row)
        .optional()?)
}

/// Drops the tags of a scope no item has any more.
fn drop_unused_tags(conn: &Connection, scope: Scope<'_>) -> Result<(), Error> {
    conn.prepare_cached(
        "DELETE FROM user_tags WHERE profile_id = ?1 AND space = ?2
           AND NOT EXISTS (SELECT 1 FROM user_tag_items i WHERE i.tag_id = user_tags.id)",
    )?
    .execute(params![scope.profile_id, scope.space])?;
    Ok(())
}

/// Tags `entry` with `name` (created if new); `false` if it already had it.
/// More than [`MAX_TAGS_PER_ITEM`] tags is [`Error::Invalid`]`("too many
/// tags")`.
pub fn add_tag(
    conn: &Connection,
    scope: Scope<'_>,
    entry: &Entry,
    name: &str,
    now_ms: i64,
) -> Result<bool, Error> {
    scope.check()?;
    check_entry(entry)?;
    let (name, key) = clean_name(name, MAX_TAG_NAME, "tag")?;
    let tag = match tag_by_key(conn, scope, &key)? {
        Some(t) => t.id,
        None => {
            conn.prepare_cached(
                "INSERT INTO user_tags (profile_id, space, name, name_key, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![scope.profile_id, scope.space, name, key, now_ms])?;
            conn.last_insert_rowid()
        }
    };
    let (work, media) = columns(entry);
    let had: bool = conn
        .prepare_cached(
            "SELECT 1 FROM user_tag_items WHERE tag_id = ?1 AND work_id IS ?2 AND media_key IS ?3",
        )?
        .query_row(params![tag, work, media], |_| Ok(()))
        .optional()?
        .is_some();
    if had {
        return Ok(false);
    }
    if tags_of(conn, scope, entry)?.len() >= MAX_TAGS_PER_ITEM {
        drop_unused_tags(conn, scope)?;
        return Err(Error::Invalid("too many tags"));
    }
    conn.prepare_cached(
        "INSERT INTO user_tag_items (tag_id, work_id, media_key, added_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![tag, work, media, now_ms])?;
    Ok(true)
}

/// Removes tag `name` from `entry`; `false` if it did not have it. A tag
/// no item has any more is dropped.
pub fn remove_tag(
    conn: &Connection,
    scope: Scope<'_>,
    entry: &Entry,
    name: &str,
) -> Result<bool, Error> {
    let Some(tag) = tag_by_key(conn, scope, &name_key(name))? else {
        return Ok(false);
    };
    let (work, media) = columns(entry);
    let n = conn
        .prepare_cached(
            "DELETE FROM user_tag_items WHERE tag_id = ?1 AND work_id IS ?2 AND media_key IS ?3",
        )?
        .execute(params![tag.id, work, media])?;
    drop_unused_tags(conn, scope)?;
    Ok(n > 0)
}

/// Makes `names` the tags of `entry` (duplicates by [`name_key`] once, in
/// the given order of first use). Returns whether anything changed.
pub fn set_tags(
    conn: &Connection,
    scope: Scope<'_>,
    entry: &Entry,
    names: &[String],
    now_ms: i64,
) -> Result<bool, Error> {
    scope.check()?;
    check_entry(entry)?;
    let mut wanted: Vec<(String, String)> = Vec::new();
    for n in names {
        let (name, key) = clean_name(n, MAX_TAG_NAME, "tag")?;
        if !wanted.iter().any(|(_, k)| *k == key) {
            wanted.push((name, key));
        }
    }
    if wanted.len() > MAX_TAGS_PER_ITEM {
        return Err(Error::Invalid("too many tags"));
    }
    let mut changed = false;
    for t in tags_of(conn, scope, entry)? {
        if !wanted.iter().any(|(_, k)| *k == t.key) {
            changed |= remove_tag(conn, scope, entry, &t.key)?;
        }
    }
    for (name, _) in &wanted {
        changed |= add_tag(conn, scope, entry, name, now_ms)?;
    }
    Ok(changed)
}

/// Deletes tag `name` from every item of the scope; how many items had it.
pub fn delete_tag(conn: &Connection, scope: Scope<'_>, name: &str) -> Result<usize, Error> {
    let Some(tag) = tag_by_key(conn, scope, &name_key(name))? else {
        return Ok(0);
    };
    let n = conn
        .prepare_cached("DELETE FROM user_tag_items WHERE tag_id = ?1")?
        .execute([tag.id])?;
    drop_unused_tags(conn, scope)?;
    Ok(n)
}

/// Tags of `entry` in the scope, by name.
pub fn tags_of(conn: &Connection, scope: Scope<'_>, entry: &Entry) -> Result<Vec<Tag>, Error> {
    let (work, media) = columns(entry);
    let mut stmt = conn.prepare_cached(
        "SELECT t.id, t.name, t.name_key FROM user_tags t
         JOIN user_tag_items i ON i.tag_id = t.id
         WHERE t.profile_id = ?1 AND t.space = ?2 AND i.work_id IS ?3 AND i.media_key IS ?4
         ORDER BY t.name_key",
    )?;
    let rows = stmt.query_map(
        params![scope.profile_id, scope.space, work, media],
        tag_row,
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every tag of the scope with its entries, by name. Entries are not
/// filtered by visibility: the caller counts what the profile may see.
pub fn tags(conn: &Connection, scope: Scope<'_>) -> Result<Vec<(Tag, Vec<Entry>)>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.id, t.name, t.name_key, i.work_id, i.media_key FROM user_tags t
         JOIN user_tag_items i ON i.tag_id = t.id
         WHERE t.profile_id = ?1 AND t.space = ?2
         ORDER BY t.name_key, i.added_at_ms DESC, i.rowid DESC",
    )?;
    let mut rows = stmt.query(params![scope.profile_id, scope.space])?;
    let mut out: Vec<(Tag, Vec<Entry>)> = Vec::new();
    while let Some(r) = rows.next()? {
        let tag = tag_row(r)?;
        let entry = entry_of(r.get(3)?, r.get(4)?)?;
        match out.last_mut() {
            Some((t, entries)) if t.id == tag.id => entries.push(entry),
            _ => out.push((tag, vec![entry])),
        }
    }
    Ok(out)
}

/// Tags of the scope whose [`name_key`] starts with the key of `prefix`
/// (all tags for a blank prefix), then those that contain it, each group
/// by name. The caller drops the ones the profile may not see.
pub fn suggest_tags(
    conn: &Connection,
    scope: Scope<'_>,
    prefix: &str,
) -> Result<Vec<(Tag, Vec<Entry>)>, Error> {
    let key = name_key(prefix);
    let all = tags(conn, scope)?;
    if key.is_empty() {
        return Ok(all);
    }
    let (mut starts, rest): (Vec<_>, Vec<_>) = all
        .into_iter()
        .filter(|(t, _)| t.key.contains(&key))
        .partition(|(t, _)| t.key.starts_with(&key));
    starts.extend(rest);
    Ok(starts)
}

/// Entries tagged `name` in the scope, most recently tagged first.
pub fn tag_entries(conn: &Connection, scope: Scope<'_>, name: &str) -> Result<Vec<Entry>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.work_id, i.media_key FROM user_tag_items i
         JOIN user_tags t ON t.id = i.tag_id
         WHERE t.profile_id = ?1 AND t.space = ?2 AND t.name_key = ?3
         ORDER BY i.added_at_ms DESC, i.rowid DESC",
    )?;
    let rows = stmt.query_map(
        params![scope.profile_id, scope.space, name_key(name)],
        |r| entry_of(r.get(0)?, r.get(1)?),
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// `(work_id, tag key, tag name)` of every tagged work of the scope.
pub fn work_tags(conn: &Connection, scope: Scope<'_>) -> Result<Vec<(i64, String, String)>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.work_id, t.name_key, t.name FROM user_tag_items i
         JOIN user_tags t ON t.id = i.tag_id
         WHERE t.profile_id = ?1 AND t.space = ?2 AND i.work_id IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![scope.profile_id, scope.space], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---------------------------------------------------------------------------
// ratings

/// Sets (1–5 stars) or clears (`None`) the rating of `entry` by
/// `profile_id`. Returns whether it changed.
pub fn set_rating(
    conn: &Connection,
    profile_id: i64,
    entry: &Entry,
    stars: Option<u8>,
    now_ms: i64,
) -> Result<bool, Error> {
    check_entry(entry)?;
    let (work, key) = columns(entry);
    match stars {
        Some(s) if !(1..=5).contains(&s) => Err(Error::Invalid("rating")),
        Some(s) => {
            if rating(conn, profile_id, entry)? == Some(s) {
                return Ok(false);
            }
            conn.prepare_cached(
                "DELETE FROM user_ratings WHERE profile_id = ?1 AND work_id IS ?2 AND media_key IS ?3",
            )?
            .execute(params![profile_id, work, key])?;
            conn.prepare_cached(
                "INSERT INTO user_ratings (profile_id, work_id, media_key, stars, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![profile_id, work, key, s, now_ms])?;
            Ok(true)
        }
        None => {
            let n = conn
                .prepare_cached(
                    "DELETE FROM user_ratings
                     WHERE profile_id = ?1 AND work_id IS ?2 AND media_key IS ?3",
                )?
                .execute(params![profile_id, work, key])?;
            Ok(n > 0)
        }
    }
}

/// Stars `profile_id` gave `entry`, if any.
pub fn rating(conn: &Connection, profile_id: i64, entry: &Entry) -> Result<Option<u8>, Error> {
    let (work, key) = columns(entry);
    Ok(conn
        .prepare_cached(
            "SELECT stars FROM user_ratings
             WHERE profile_id = ?1 AND work_id IS ?2 AND media_key IS ?3",
        )?
        .query_row(params![profile_id, work, key], |r| r.get(0))
        .optional()?)
}

/// `(work_id, stars)` of every work `profile_id` rated.
pub fn work_ratings(conn: &Connection, profile_id: i64) -> Result<Vec<(i64, u8)>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT work_id, stars FROM user_ratings WHERE profile_id = ?1 AND work_id IS NOT NULL",
    )?;
    let rows = stmt.query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Rated entries of `profile_id` with at least `min_stars`, best first.
pub fn rated_entries(
    conn: &Connection,
    profile_id: i64,
    min_stars: u8,
) -> Result<Vec<(Entry, u8)>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT work_id, media_key, stars FROM user_ratings
         WHERE profile_id = ?1 AND stars >= ?2
         ORDER BY stars DESC, updated_at_ms DESC",
    )?;
    let rows = stmt.query_map(params![profile_id, min_stars], |r| {
        Ok((entry_of(r.get(0)?, r.get(1)?)?, r.get(2)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::DEFAULT_PROFILE;

    const NOW: i64 = 1_763_000_000_000;

    fn db() -> (Connection, i64, i64) {
        let conn = crate::open_in_memory().unwrap();
        for (id, title) in [(1, "Dune"), (2, "Arrival")] {
            conn.execute(
                "INSERT INTO works (id, kind, title, created_at_ms, updated_at_ms)
                 VALUES (?1, 'movie', ?2, 0, 0)",
                params![id, title],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO profiles (id, sync_id, name, created_at_ms, updated_at_ms)
             VALUES (2, 'p2', 'Kid', 0, 0)",
            [],
        )
        .unwrap();
        (conn, DEFAULT_PROFILE, 2)
    }

    #[test]
    fn names_normalize() {
        assert_eq!(name_key("  Sci   Fi "), "sci fi");
        assert_eq!(name_key("ФАНТАСТИКА"), "фантастика");
        assert!(clean_name(" ", 10, "x").is_err());
        assert!(clean_name("a\u{7}b", 10, "x").is_err());
        assert!(clean_name(&"x".repeat(11), 10, "x").is_err());
    }

    #[test]
    fn categories_are_per_profile_and_space_and_unique_by_name() {
        let (conn, me, kid) = db();
        let main = Scope::main(me);
        let vault = Scope {
            profile_id: me,
            space: "vault:1",
        };
        let watch = create_category(&conn, main, "To watch", NOW).unwrap();
        assert_eq!(watch.sync_id.len(), 32);
        assert!(matches!(
            create_category(&conn, main, " to  WATCH ", NOW),
            Err(Error::Invalid("duplicate category"))
        ));
        let fam = create_category(&conn, main, "Family", NOW).unwrap();
        assert_eq!(fam.position, 1);
        create_category(&conn, vault, "To watch", NOW).unwrap();
        create_category(&conn, Scope::main(kid), "To watch", NOW).unwrap();
        assert!(
            create_category(
                &conn,
                Scope {
                    profile_id: me,
                    space: "Bad Space"
                },
                "x",
                NOW
            )
            .is_err()
        );

        let names = |s| -> Vec<String> {
            categories(&conn, s)
                .unwrap()
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        assert_eq!(names(main), ["To watch", "Family"]);
        assert_eq!(names(vault), ["To watch"]);

        // One item in two categories.
        assert!(add_to_category(&conn, watch.id, &Entry::Work(1), NOW).unwrap());
        assert!(!add_to_category(&conn, watch.id, &Entry::Work(1), NOW).unwrap());
        assert!(add_to_category(&conn, fam.id, &Entry::Work(1), NOW).unwrap());
        let key = Entry::Media("tmdb:episode:1:1:1".into());
        assert!(add_to_category(&conn, fam.id, &key, NOW + 1).unwrap());
        assert_eq!(categories_of(&conn, main, &Entry::Work(1)).unwrap().len(), 2);
        assert_eq!(
            category_entries(&conn, fam.id).unwrap(),
            [key.clone(), Entry::Work(1)]
        );
        assert_eq!(category(&conn, fam.id).unwrap().unwrap().item_count, 2);

        assert!(matches!(
            rename_category(&conn, fam.id, "to watch", NOW),
            Err(Error::Invalid("duplicate category"))
        ));
        assert!(rename_category(&conn, fam.id, "Family night", NOW + 2).unwrap());
        assert_eq!(names(main), ["To watch", "Family night"]);
        assert!(remove_from_category(&conn, fam.id, &key).unwrap());
        assert!(delete_category(&conn, watch.id).unwrap());
        assert!(!delete_category(&conn, watch.id).unwrap());
        assert_eq!(work_categories(&conn, main).unwrap(), [(1, fam.id)]);
        // The work is gone: so are its category rows.
        conn.execute("DELETE FROM works WHERE id = 1", []).unwrap();
        assert_eq!(category(&conn, fam.id).unwrap().unwrap().item_count, 0);
    }

    #[test]
    fn tags_autocomplete_and_go_away_when_unused() {
        let (conn, me, kid) = db();
        let main = Scope::main(me);
        let dune = Entry::Work(1);
        let arrival = Entry::Work(2);
        assert!(add_tag(&conn, main, &dune, " Sci-Fi ", NOW).unwrap());
        assert!(!add_tag(&conn, main, &dune, "sci-fi", NOW).unwrap());
        add_tag(&conn, main, &arrival, "SCI-FI", NOW).unwrap();
        add_tag(&conn, main, &arrival, "Aliens", NOW).unwrap();
        add_tag(&conn, main, &dune, "Classic sci-fi", NOW).unwrap();
        add_tag(&conn, Scope::main(kid), &dune, "Scary", NOW).unwrap();

        let names = |v: Vec<Tag>| -> Vec<String> { v.into_iter().map(|t| t.name).collect() };
        assert_eq!(
            names(tags_of(&conn, main, &dune).unwrap()),
            ["Classic sci-fi", "Sci-Fi"]
        );
        let suggested: Vec<String> = suggest_tags(&conn, main, "sc")
            .unwrap()
            .into_iter()
            .map(|(t, _)| t.name)
            .collect();
        assert_eq!(suggested, ["Sci-Fi", "Classic sci-fi"]);
        let all = tags(&conn, main).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].0.name, "Sci-Fi");
        assert_eq!(all[2].1.len(), 2);
        assert_eq!(tag_entries(&conn, main, "SCI-FI").unwrap().len(), 2);

        assert!(remove_tag(&conn, main, &arrival, "aliens").unwrap());
        assert!(
            tags(&conn, main)
                .unwrap()
                .iter()
                .all(|(t, _)| t.key != "aliens")
        );
        assert!(set_tags(&conn, main, &dune, &["sci-fi".into(), "Desert".into()], NOW).unwrap());
        assert_eq!(
            names(tags_of(&conn, main, &dune).unwrap()),
            ["Desert", "Sci-Fi"]
        );
        assert!(!set_tags(&conn, main, &dune, &["Desert".into(), "SCI-FI".into()], NOW).unwrap());
        assert_eq!(delete_tag(&conn, main, "sci-fi").unwrap(), 2);
        assert_eq!(names(tags_of(&conn, main, &dune).unwrap()), ["Desert"]);
        assert_eq!(work_tags(&conn, main).unwrap().len(), 1);
        // The kid's tag is the kid's.
        assert_eq!(tags(&conn, Scope::main(kid)).unwrap().len(), 1);

        let many: Vec<String> = (0..=MAX_TAGS_PER_ITEM).map(|i| format!("t{i}")).collect();
        assert!(set_tags(&conn, main, &arrival, &many, NOW).is_err());
    }

    #[test]
    fn ratings_are_per_profile() {
        let (conn, me, kid) = db();
        let dune = Entry::Work(1);
        assert!(set_rating(&conn, me, &dune, Some(6), NOW).is_err());
        assert!(set_rating(&conn, me, &dune, Some(0), NOW).is_err());
        assert!(set_rating(&conn, me, &dune, Some(5), NOW).unwrap());
        assert!(!set_rating(&conn, me, &dune, Some(5), NOW).unwrap());
        assert!(set_rating(&conn, me, &dune, Some(4), NOW).unwrap());
        set_rating(&conn, kid, &dune, Some(1), NOW).unwrap();
        let ep = Entry::Media("file:abcd".into());
        set_rating(&conn, me, &ep, Some(3), NOW).unwrap();
        assert_eq!(rating(&conn, me, &dune).unwrap(), Some(4));
        assert_eq!(rating(&conn, kid, &dune).unwrap(), Some(1));
        assert_eq!(work_ratings(&conn, me).unwrap(), [(1, 4)]);
        assert_eq!(
            rated_entries(&conn, me, 3).unwrap(),
            [(dune.clone(), 4), (ep, 3)]
        );
        assert!(set_rating(&conn, me, &dune, None, NOW).unwrap());
        assert!(!set_rating(&conn, me, &dune, None, NOW).unwrap());
        assert_eq!(rating(&conn, me, &dune).unwrap(), None);
    }
}
