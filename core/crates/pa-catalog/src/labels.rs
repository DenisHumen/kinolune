// SPDX-License-Identifier: MPL-2.0
//! The user's own categories, tags and 1–5 star ratings of items (LIB-022)
//! on the items of the catalog, for the active profile.
//!
//! [`Catalog::labels`] returns a [`Labels`] handle for the main library.
//! The personal libraries of UI-025 ("Личные медиа") get their own handles
//! over other [`LabelSpace`]s, with their own categories and tags; ratings
//! are per item and so follow the item's library. The space is the seam
//! for a library scope: a handle only ever reads and writes its own space.
//!
//! What gets labelled is a [`LabelTarget`]: a catalog item (a movie, show,
//! episode or track) or any fingerprinted file by its `file:` key, also an
//! unrecognized one with no work or provider id (personal media are never
//! recognized). The file key of a recognized file labels its item.
//!
//! What the active profile may not see (LIB-012, ADR-0055) never comes
//! out: categories count only visible items and files, tags and
//! autocomplete list only tags of visible ones, lists of a category or tag
//! hold only visible ones, and the calls about one target answer
//! [`Error::NotFound`] for a hidden one, as for one that does not exist.
//! The library queries filter and sort by categories, tags and ratings
//! ([`LibraryFilter::categories`](crate::LibraryFilter::categories),
//! [`SortField::UserRating`](crate::SortField::UserRating)) over the
//! visible works only.

use std::collections::{HashMap, HashSet};

use pa_library::labels::{self, MAIN_SPACE, Scope};
use pa_library::lists::Entry;
use pa_library::rusqlite::{Connection, OptionalExtension};
use pa_model::MediaKey;

use crate::items::{Item, ItemRef};
use crate::lists::{entry_of, item_of_entry, items_of, require_entry};
use crate::profiles::require_visible;
use crate::progress::{item_of_key, profile};
use crate::{Catalog, Error, now_ms};

/// Which library a [`Labels`] handle works on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LabelSpace {
    /// The main library.
    #[default]
    Main,
}

impl LabelSpace {
    /// Space key in the database (`user_categories.space`).
    fn key(self) -> &'static str {
        match self {
            Self::Main => MAIN_SPACE,
        }
    }
}

/// What a label is put on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LabelTarget {
    /// A movie, show, episode or track of the catalog.
    Item(ItemRef),
    /// A file by its `file:` key: the item of a recognized file, else the
    /// file itself.
    Key(MediaKey),
}

impl From<ItemRef> for LabelTarget {
    fn from(item: ItemRef) -> Self {
        Self::Item(item)
    }
}

impl From<MediaKey> for LabelTarget {
    fn from(key: MediaKey) -> Self {
        Self::Key(key)
    }
}

/// A labelled file that is not a catalog item (an unrecognized video).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabeledFile {
    /// Its `file:` key.
    pub key: MediaKey,
    pub source_id: i64,
    /// Path relative to the source root.
    pub rel_path: String,
}

/// A category as the app lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryInfo {
    pub id: i64,
    pub name: String,
    /// Items and files in it the active profile may see now.
    pub item_count: usize,
}

/// A tag with the number of visible items and files that have it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// First spelling used.
    pub name: String,
    pub item_count: usize,
}

/// A rated item.
#[derive(Debug, Clone, PartialEq)]
pub struct RatedItem {
    pub item: Item,
    /// 1–5.
    pub stars: u8,
}

/// Categories, tags and ratings of the active profile in one library.
#[derive(Debug, Clone, Copy)]
pub struct Labels<'a> {
    cat: &'a Catalog,
    space: LabelSpace,
}

/// `pa_library` argument errors as catalog ones (`invalid: duplicate
/// category`).
fn invalid(e: pa_library::Error) -> Error {
    match e {
        pa_library::Error::Invalid(what) => Error::Invalid(what.to_owned()),
        other => Error::Library(other),
    }
}

/// A visible thing an entry stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Member {
    Item(ItemRef),
    File(String),
}

/// A present file with this fingerprint the active profile may open.
const VISIBLE_FILE_SQL: &str = concat!(
    "SELECT f.source_id, f.path FROM files f
     WHERE f.fingerprint = ?1 AND f.missing_since_ms IS NULL AND ",
    pa_library::file_allowed!("f"),
    " ORDER BY f.id LIMIT 1"
);

fn visible_file(conn: &Connection, key: &MediaKey) -> Result<Option<LabeledFile>, Error> {
    let MediaKey::File { fingerprint } = key else {
        return Ok(None);
    };
    let row: Option<(i64, String)> = conn
        .prepare_cached(VISIBLE_FILE_SQL)?
        .query_row([fingerprint], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(row.map(|(source_id, path)| LabeledFile {
        key: key.clone(),
        source_id,
        rel_path: crate::items::relative(&path),
    }))
}

/// What a stored entry stands for now, if the active profile may see it.
fn member_of(conn: &Connection, entry: &Entry) -> Result<Option<Member>, Error> {
    if let Some(item) = item_of_entry(conn, entry)? {
        return Ok(Some(Member::Item(item.id)));
    }
    if let Entry::Media(k) = entry
        && let Ok(key) = k.parse::<MediaKey>()
        && visible_file(conn, &key)?.is_some()
    {
        return Ok(Some(Member::File(k.clone())));
    }
    Ok(None)
}

/// Visible items and files among `entries`, each once; answers repeat
/// lookups from `seen`.
fn visible_count(
    conn: &Connection,
    entries: &[Entry],
    seen: &mut HashMap<Entry, Option<Member>>,
) -> Result<usize, Error> {
    let mut members = HashSet::new();
    for e in entries {
        let m = match seen.get(e) {
            Some(m) => m.clone(),
            None => {
                let m = member_of(conn, e)?;
                seen.insert(e.clone(), m.clone());
                m
            }
        };
        if let Some(m) = m {
            members.insert(m);
        }
    }
    Ok(members.len())
}

/// Visible files among `entries` that are not catalog items, in order,
/// each once.
fn files_of(conn: &Connection, entries: &[Entry]) -> Result<Vec<LabeledFile>, Error> {
    let mut out: Vec<LabeledFile> = Vec::new();
    for e in entries {
        let Entry::Media(k) = e else { continue };
        if item_of_entry(conn, e)?.is_some() {
            continue;
        }
        if let Ok(key) = k.parse::<MediaKey>()
            && let Some(f) = visible_file(conn, &key)?
            && !out.iter().any(|o| o.key == f.key)
        {
            out.push(f);
        }
    }
    Ok(out)
}

/// The stored reference of a target the active profile may see, else
/// [`Error::NotFound`].
fn visible_entry(conn: &Connection, target: &LabelTarget) -> Result<Entry, Error> {
    match target {
        LabelTarget::Item(item) => {
            require_visible(conn, *item)?;
            require_entry(conn, *item)
        }
        LabelTarget::Key(key) => {
            if let Some(item) = item_of_key(conn, key)? {
                return visible_entry(conn, &LabelTarget::Item(item));
            }
            if visible_file(conn, key)?.is_some() {
                Ok(Entry::Media(key.to_string()))
            } else {
                Err(Error::NotFound(format!("file {key}")))
            }
        }
    }
}

/// The stored reference of a target for reading: `None` if it has none
/// yet (an item whose file is not fingerprinted).
fn read_entry(conn: &Connection, target: &LabelTarget) -> Result<Option<Entry>, Error> {
    match target {
        LabelTarget::Item(item) => {
            require_visible(conn, *item)?;
            entry_of(conn, *item)
        }
        LabelTarget::Key(_) => visible_entry(conn, target).map(Some),
    }
}

impl Catalog {
    /// Categories, tags and ratings of the active profile in the main
    /// library.
    pub fn labels(&self) -> Labels<'_> {
        Labels {
            cat: self,
            space: LabelSpace::Main,
        }
    }
}

impl Labels<'_> {
    /// The library of this handle.
    pub fn space(&self) -> LabelSpace {
        self.space
    }

    fn scope(&self, conn: &Connection) -> Result<Scope<'static>, Error> {
        Ok(Scope {
            profile_id: profile(conn)?,
            space: self.space.key(),
        })
    }

    /// A category of the active profile in this library.
    fn own_category(&self, conn: &Connection, id: i64) -> Result<labels::Category, Error> {
        let scope = self.scope(conn)?;
        labels::category(conn, id)?
            .filter(|c| c.profile_id == scope.profile_id && c.space == scope.space)
            .ok_or_else(|| Error::NotFound(format!("category {id}")))
    }

    fn info(conn: &Connection, c: labels::Category) -> Result<CategoryInfo, Error> {
        let entries = labels::category_entries(conn, c.id)?;
        Ok(CategoryInfo {
            id: c.id,
            name: c.name,
            item_count: visible_count(conn, &entries, &mut HashMap::new())?,
        })
    }

    // -- categories ----------------------------------------------------------

    /// Categories in the user's order.
    pub fn categories(&self) -> Result<Vec<CategoryInfo>, Error> {
        let conn = self.cat.lock();
        labels::categories(&conn, self.scope(&conn)?)?
            .into_iter()
            .map(|c| Self::info(&conn, c))
            .collect()
    }

    /// Creates an empty category at the end. A name in use (any case) is
    /// `Invalid("duplicate category")`.
    pub fn create_category(&self, name: &str) -> Result<CategoryInfo, Error> {
        let conn = self.cat.lock();
        let c =
            labels::create_category(&conn, self.scope(&conn)?, name, now_ms()).map_err(invalid)?;
        Self::info(&conn, c)
    }

    pub fn rename_category(&self, id: i64, name: &str) -> Result<(), Error> {
        let conn = self.cat.lock();
        self.own_category(&conn, id)?;
        labels::rename_category(&conn, id, name, now_ms()).map_err(invalid)?;
        Ok(())
    }

    /// Deletes a category; its items stay in the library.
    pub fn delete_category(&self, id: i64) -> Result<(), Error> {
        let conn = self.cat.lock();
        self.own_category(&conn, id)?;
        labels::delete_category(&conn, id)?;
        Ok(())
    }

    /// Visible catalog items of a category, most recently added first.
    pub fn category_items(&self, id: i64) -> Result<Vec<Item>, Error> {
        let conn = self.cat.lock();
        self.own_category(&conn, id)?;
        items_of(&conn, &labels::category_entries(&conn, id)?)
    }

    /// Visible files of a category that are not catalog items, most
    /// recently added first.
    pub fn category_files(&self, id: i64) -> Result<Vec<LabeledFile>, Error> {
        let conn = self.cat.lock();
        self.own_category(&conn, id)?;
        files_of(&conn, &labels::category_entries(&conn, id)?)
    }

    /// Categories that hold `target`.
    pub fn item_categories(
        &self,
        target: impl Into<LabelTarget>,
    ) -> Result<Vec<CategoryInfo>, Error> {
        let conn = self.cat.lock();
        let Some(entry) = read_entry(&conn, &target.into())? else {
            return Ok(Vec::new());
        };
        labels::categories_of(&conn, self.scope(&conn)?, &entry)?
            .into_iter()
            .map(|c| Self::info(&conn, c))
            .collect()
    }

    /// Puts `target` into category `id` (`on`) or takes it out. Returns
    /// whether anything changed.
    pub fn set_in_category(
        &self,
        id: i64,
        target: impl Into<LabelTarget>,
        on: bool,
    ) -> Result<bool, Error> {
        let conn = self.cat.lock();
        self.own_category(&conn, id)?;
        let entry = visible_entry(&conn, &target.into())?;
        Ok(if on {
            labels::add_to_category(&conn, id, &entry, now_ms())?
        } else {
            labels::remove_from_category(&conn, id, &entry)?
        })
    }

    // -- tags ----------------------------------------------------------------

    fn tag_infos(
        conn: &Connection,
        rows: Vec<(labels::Tag, Vec<Entry>)>,
    ) -> Result<Vec<TagInfo>, Error> {
        let mut seen = HashMap::new();
        let mut out = Vec::new();
        for (tag, entries) in rows {
            let n = visible_count(conn, &entries, &mut seen)?;
            if n > 0 {
                out.push(TagInfo {
                    name: tag.name,
                    item_count: n,
                });
            }
        }
        Ok(out)
    }

    /// Tags of visible items and files, by name.
    pub fn tags(&self) -> Result<Vec<TagInfo>, Error> {
        let conn = self.cat.lock();
        let rows = labels::tags(&conn, self.scope(&conn)?)?;
        Self::tag_infos(&conn, rows)
    }

    /// Autocomplete: up to `limit` tags of visible items and files that
    /// start with `prefix` (case-insensitive), then those that contain it;
    /// every tag for a blank prefix.
    pub fn suggest_tags(&self, prefix: &str, limit: usize) -> Result<Vec<TagInfo>, Error> {
        let conn = self.cat.lock();
        let rows = labels::suggest_tags(&conn, self.scope(&conn)?, prefix)?;
        let mut out = Self::tag_infos(&conn, rows)?;
        out.truncate(limit);
        Ok(out)
    }

    /// Tags of `target`, by name.
    pub fn item_tags(&self, target: impl Into<LabelTarget>) -> Result<Vec<String>, Error> {
        let conn = self.cat.lock();
        let Some(entry) = read_entry(&conn, &target.into())? else {
            return Ok(Vec::new());
        };
        Ok(labels::tags_of(&conn, self.scope(&conn)?, &entry)?
            .into_iter()
            .map(|t| t.name)
            .collect())
    }

    /// Tags `target` with `tag`; `false` if it already had it.
    pub fn add_tag(&self, target: impl Into<LabelTarget>, tag: &str) -> Result<bool, Error> {
        let conn = self.cat.lock();
        let entry = visible_entry(&conn, &target.into())?;
        labels::add_tag(&conn, self.scope(&conn)?, &entry, tag, now_ms()).map_err(invalid)
    }

    /// Removes `tag` from `target`; `false` if it did not have it.
    pub fn remove_tag(&self, target: impl Into<LabelTarget>, tag: &str) -> Result<bool, Error> {
        let conn = self.cat.lock();
        let entry = visible_entry(&conn, &target.into())?;
        Ok(labels::remove_tag(&conn, self.scope(&conn)?, &entry, tag)?)
    }

    /// Makes `tags` the tags of `target`. Returns whether anything changed.
    pub fn set_tags(&self, target: impl Into<LabelTarget>, tags: &[String]) -> Result<bool, Error> {
        let mut conn = self.cat.lock();
        let entry = visible_entry(&conn, &target.into())?;
        let scope = self.scope(&conn)?;
        let tx = conn.transaction()?;
        let changed = labels::set_tags(&tx, scope, &entry, tags, now_ms()).map_err(invalid)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Visible catalog items tagged `tag`, most recently tagged first.
    pub fn tag_items(&self, tag: &str) -> Result<Vec<Item>, Error> {
        let conn = self.cat.lock();
        let entries = labels::tag_entries(&conn, self.scope(&conn)?, tag)?;
        items_of(&conn, &entries)
    }

    /// Visible files tagged `tag` that are not catalog items, most recently
    /// tagged first.
    pub fn tag_files(&self, tag: &str) -> Result<Vec<LabeledFile>, Error> {
        let conn = self.cat.lock();
        let entries = labels::tag_entries(&conn, self.scope(&conn)?, tag)?;
        files_of(&conn, &entries)
    }

    // -- ratings -------------------------------------------------------------

    /// Stars (1–5) the active profile gave `target`.
    pub fn rating(&self, target: impl Into<LabelTarget>) -> Result<Option<u8>, Error> {
        let conn = self.cat.lock();
        let Some(entry) = read_entry(&conn, &target.into())? else {
            return Ok(None);
        };
        Ok(labels::rating(&conn, profile(&conn)?, &entry)?)
    }

    /// Rates `target` 1–5 stars or clears its rating (`None`). Returns
    /// whether it changed.
    pub fn set_rating(
        &self,
        target: impl Into<LabelTarget>,
        stars: Option<u8>,
    ) -> Result<bool, Error> {
        let conn = self.cat.lock();
        let entry = visible_entry(&conn, &target.into())?;
        labels::set_rating(&conn, profile(&conn)?, &entry, stars, now_ms()).map_err(invalid)
    }

    /// Visible catalog items rated at least `min_stars`, best first.
    pub fn rated_items(&self, min_stars: u8) -> Result<Vec<RatedItem>, Error> {
        let conn = self.cat.lock();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (entry, stars) in labels::rated_entries(&conn, profile(&conn)?, min_stars)? {
            if let Some(item) = item_of_entry(&conn, &entry)?
                && seen.insert(item.id)
            {
                out.push(RatedItem { item, stars });
            }
        }
        Ok(out)
    }
}
