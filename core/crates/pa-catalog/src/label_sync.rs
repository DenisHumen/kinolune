// SPDX-License-Identifier: MPL-2.0
//! Sync of the user's categories, tags and ratings (SYNC-013, ADR-NNNN)
//! between paired devices.
//!
//! The labels of LIB-022 travel as ops of `pa-sync`, one last-writer-wins
//! register each ([`OpBody::register`]):
//!
//! - `category.upsert` / `category.delete`: a category of a profile by its
//!   `user_categories.sync_id`, with its name;
//! - `category.item`: an item in a category or not;
//! - `tag.set`: a tag on an item or not, the tag known by its `name_key`;
//! - `rating.set`: the existing op, stars × 2 (`0..=10`).
//!
//! Items are named by stable refs, never by local row ids (ADR-0079): the
//! media key of an item (`tmdb:movie:…`, `tmdb:episode:…`, `file:…`, the
//! other keys in the op's `aliases`) or, for a show, a work ref
//! (`tmdb:tv:1399`, `anidb::…`, `<provider>:<kind>:<id>`).
//!
//! The registers live in `label_sync` with the newest op of each, so the
//! last writer wins across restarts and an off op or a delete is the
//! tombstone that an older op cannot undo. Like the profiles of SYNC-009:
//!
//! - [`reconcile`] compares the labels of every profile with the
//!   registers and records ops for the differences. It runs after every
//!   label change of the main library and when sync starts, so labels made
//!   before pairing go out too.
//! - [`apply_remote`] writes an op of another device if it is newer than
//!   the register. An op about a profile, category or item this device
//!   does not have yet is parked and retried by the next [`reconcile`].
//!
//! Never synced: the labels of personal libraries (UI-025, their own
//! spaces) and any label or rating of a personal file or of an item with a
//! personal file. An item that leaves the library here does not take its
//! labels away from other devices.
//!
//! Two devices that made a category with the same name merge it: the one
//! with the smaller sync id stays, the other's items move into it, and ops
//! about the other land in it ([`RegState::Merged`]).

use std::collections::{BTreeMap, HashMap};

use pa_library::labels::{self, MAIN_SPACE, Scope};
use pa_library::lists::Entry;
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_model::MediaKey;
use pa_sync::matching::{MAX_ALIASES, parse_aliases};
use pa_sync::{Op, OpBody, Stamp, outbox};

use crate::items::ItemRef;
use crate::lists::entry_of;
use crate::matching::{own_keys, resolve};
use crate::{Error, now_ms};

/// Providers of a work ref, the preferred first.
const PROVIDERS: [&str; 4] = ["tmdb", "anidb", "tvdb", "imdb"];

/// Whether an op is about labels (what [`apply_remote`] writes).
pub(crate) fn is_label_op(body: &OpBody) -> bool {
    matches!(
        body,
        OpBody::CategoryUpsert { .. }
            | OpBody::CategoryDelete { .. }
            | OpBody::CategoryItem { .. }
            | OpBody::TagSet { .. }
            | OpBody::Rating { .. }
    )
}

/// State of a register row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegState {
    /// The library holds what the op says.
    Applied,
    /// Waiting for a profile, category or item this device does not have.
    Parked,
    /// A category merged into another one (`merged_into`).
    Merged,
}

impl RegState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Parked => "parked",
            Self::Merged => "merged",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "parked" => Self::Parked,
            "merged" => Self::Merged,
            _ => Self::Applied,
        }
    }
}

/// One row of `label_sync`.
#[derive(Debug, Clone)]
struct Stored {
    op: Option<Op>,
    state: RegState,
    merged_into: Option<String>,
}

fn stamp(op: &Op) -> Stamp {
    (op.hlc.clone(), op.op_id)
}

fn read_row(
    r: &pa_library::rusqlite::Row<'_>,
) -> pa_library::rusqlite::Result<(String, Option<String>, String, Option<String>)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}

fn stored_of(op: Option<String>, state: &str, merged_into: Option<String>) -> Stored {
    Stored {
        // A row this build cannot read is treated as empty: a newer op
        // replaces it.
        op: op.and_then(|o| outbox::decode(&o).ok()),
        state: RegState::parse(state),
        merged_into,
    }
}

fn load_all(conn: &Connection) -> Result<BTreeMap<String, Stored>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT register, op, state, merged_into FROM label_sync ORDER BY register",
    )?;
    let rows = stmt.query_map([], read_row)?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (register, op, state, merged_into) = row?;
        out.insert(register, stored_of(op, &state, merged_into));
    }
    Ok(out)
}

fn load_one(conn: &Connection, register: &str) -> Result<Option<Stored>, Error> {
    Ok(conn
        .prepare_cached(
            "SELECT register, op, state, merged_into FROM label_sync WHERE register = ?1",
        )?
        .query_row([register], read_row)
        .optional()?
        .map(|(_, op, state, merged_into)| stored_of(op, &state, merged_into)))
}

/// Writes the newest op of a register. A merged row stays merged.
fn save(conn: &Connection, op: &Op, state: RegState) -> Result<(), Error> {
    if state == RegState::Merged {
        // The row was marked merged (with `merged_into`) before. An insert
        // would fail the row check before the conflict is seen.
        conn.prepare_cached("UPDATE label_sync SET op = ?2 WHERE register = ?1")?
            .execute(params![op.body.register(), outbox::encode(op)?])?;
        return Ok(());
    }
    conn.prepare_cached(
        "INSERT INTO label_sync (register, profile, op, state) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (register) DO UPDATE SET op = excluded.op,
             state = CASE WHEN label_sync.state = 'merged' THEN 'merged' ELSE excluded.state END",
    )?
    .execute(params![
        op.body.register(),
        profile_of(&op.body),
        outbox::encode(op)?,
        state.as_str()
    ])?;
    Ok(())
}

fn set_state(conn: &Connection, register: &str, state: RegState) -> Result<(), Error> {
    conn.prepare_cached(
        "UPDATE label_sync SET state = ?2 WHERE register = ?1 AND state <> 'merged'",
    )?
    .execute(params![register, state.as_str()])?;
    Ok(())
}

/// The register of category `sync_id` of `profile`.
fn category_register(profile: &str, sync_id: &str) -> String {
    OpBody::CategoryDelete {
        profile: profile.to_owned(),
        category: sync_id.to_owned(),
    }
    .register()
}

/// Category `from` was merged into `into`: ops about `from` land in `into`
/// from now on, and no delete of `from` is ever sent.
fn mark_merged(conn: &Connection, profile: &str, from: &str, into: &str) -> Result<(), Error> {
    conn.prepare_cached(
        "INSERT INTO label_sync (register, profile, op, state, merged_into)
         VALUES (?1, ?2, NULL, 'merged', ?3)
         ON CONFLICT (register) DO UPDATE SET state = 'merged', merged_into = excluded.merged_into",
    )?
    .execute(params![category_register(profile, from), profile, into])?;
    // What was merged into `from` goes on into `into`.
    conn.prepare_cached("UPDATE label_sync SET merged_into = ?2 WHERE merged_into = ?1")?
        .execute(params![from, into])?;
    Ok(())
}

fn profile_of(body: &OpBody) -> &str {
    match body {
        OpBody::CategoryUpsert { profile, .. }
        | OpBody::CategoryDelete { profile, .. }
        | OpBody::CategoryItem { profile, .. }
        | OpBody::TagSet { profile, .. }
        | OpBody::Rating { profile, .. } => profile,
        _ => "",
    }
}

/// Local profiles by sync id.
fn profiles(conn: &Connection) -> Result<HashMap<String, i64>, Error> {
    let mut stmt = conn.prepare_cached("SELECT sync_id, id FROM profiles")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---------------------------------------------------------------------------
// stable refs of items

/// SQL: a file of the work `?1` is personal media.
const WORK_PERSONAL_SQL: &str = concat!(
    "SELECT 1 FROM matches m JOIN files f ON f.id = m.file_id WHERE m.work_id = ?1 AND ",
    pa_library::file_personal!("f"),
    " LIMIT 1"
);

/// SQL: a file with the fingerprint `?1` is personal media.
const FILE_PERSONAL_SQL: &str = concat!(
    "SELECT 1 FROM files f WHERE f.fingerprint = ?1 AND ",
    pa_library::file_personal!("f"),
    " LIMIT 1"
);

fn work_is_personal(conn: &Connection, work: i64) -> Result<bool, Error> {
    Ok(conn
        .prepare_cached(WORK_PERSONAL_SQL)?
        .query_row([work], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Whether `entry` is personal media here (UI-025): a personal file, or an
/// item with a personal file. Its labels and rating never leave the device.
fn is_personal(conn: &Connection, entry: &Entry) -> Result<bool, Error> {
    let key = match entry {
        Entry::Work(w) => return work_is_personal(conn, *w),
        Entry::Media(k) => match k.parse::<MediaKey>() {
            Ok(key) => key,
            Err(_) => return Ok(false),
        },
    };
    if let MediaKey::File { fingerprint } = &key
        && conn
            .prepare_cached(FILE_PERSONAL_SQL)?
            .query_row([fingerprint], |_| Ok(()))
            .optional()?
            .is_some()
    {
        return Ok(true);
    }
    let work = match resolve(conn, &key, &[])? {
        Some(ItemRef::Movie(w) | ItemRef::Show(w)) => Some(w),
        Some(ItemRef::Episode(ep)) => conn
            .prepare_cached("SELECT work_id FROM episodes WHERE id = ?1")?
            .query_row([ep], |r| r.get(0))
            .optional()?,
        Some(ItemRef::Track(_)) | None => None,
    };
    match work {
        Some(w) => work_is_personal(conn, w),
        None => Ok(false),
    }
}

/// Provider refs of a work, the preferred first: `<provider>:<kind>:<id>`.
fn work_refs(conn: &Connection, work: i64) -> Result<Vec<String>, Error> {
    let mut stmt = conn
        .prepare_cached("SELECT provider, ext_kind, ext_id FROM external_ids WHERE work_id = ?1")?;
    let rows = stmt.query_map([work], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut refs: Vec<(usize, String)> = Vec::new();
    for row in rows {
        let (provider, kind, id) = row?;
        let rank = PROVIDERS
            .iter()
            .position(|p| *p == provider)
            .unwrap_or(PROVIDERS.len());
        let r = format!("{provider}:{kind}:{id}");
        if r.len() <= pa_sync::op::MAX_REF_LEN && !r.chars().any(char::is_control) {
            refs.push((rank, r));
        }
    }
    refs.sort();
    Ok(refs.into_iter().map(|(_, r)| r).collect())
}

/// The work a provider ref names here.
fn work_of_ref(conn: &Connection, r: &str) -> Result<Option<i64>, Error> {
    let mut parts = r.splitn(3, ':');
    let (Some(provider), Some(kind), Some(id)) = (parts.next(), parts.next(), parts.next()) else {
        return Ok(None);
    };
    if provider.is_empty() || id.is_empty() {
        return Ok(None);
    }
    Ok(conn
        .prepare_cached(
            "SELECT work_id FROM external_ids WHERE provider = ?1 AND ext_kind = ?2 AND ext_id = ?3",
        )?
        .query_row(params![provider, kind, id], |r| r.get(0))
        .optional()?)
}

/// The stable ref of `entry` and the other keys of its item, `None` if it
/// has none here or is personal media.
fn target_of(conn: &Connection, entry: &Entry) -> Result<Option<(String, Vec<String>)>, Error> {
    if is_personal(conn, entry)? {
        return Ok(None);
    }
    let keys: Vec<String> = match entry {
        Entry::Work(w) => {
            let kind: Option<String> = conn
                .prepare_cached("SELECT kind FROM works WHERE id = ?1")?
                .query_row([w], |r| r.get(0))
                .optional()?;
            let mut keys = match kind.as_deref() {
                None => return Ok(None),
                Some("movie") => own_keys(conn, ItemRef::Movie(*w))?
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                Some(_) => Vec::new(),
            };
            for r in work_refs(conn, *w)? {
                if !keys.contains(&r) {
                    keys.push(r);
                }
            }
            keys
        }
        Entry::Media(k) => {
            let Ok(key) = k.parse::<MediaKey>() else {
                return Ok(None);
            };
            let mut keys = vec![k.clone()];
            if let Some(item) = resolve(conn, &key, &[])? {
                for other in own_keys(conn, item)? {
                    let other = other.to_string();
                    if !keys.contains(&other) {
                        keys.push(other);
                    }
                }
            }
            keys
        }
    };
    let mut keys = keys.into_iter();
    let Some(target) = keys.next() else {
        return Ok(None);
    };
    Ok(Some((target, keys.take(MAX_ALIASES).collect())))
}

/// The entry here of an item another device named `target` (with
/// `aliases`), `None` if this device does not have it.
fn entry_of_target(
    conn: &Connection,
    target: &str,
    aliases: &[String],
) -> Result<Option<Entry>, Error> {
    if let Ok(key) = target.parse::<MediaKey>()
        && let Some(item) = resolve(conn, &key, &parse_aliases(aliases))?
        && let Some(entry) = entry_of(conn, item)?
    {
        return Ok(Some(entry));
    }
    for r in std::iter::once(target).chain(aliases.iter().map(String::as_str)) {
        if let Some(w) = work_of_ref(conn, r)? {
            return Ok(Some(Entry::Work(w)));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// register values

/// What a register says, as compared between the library and the ops.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    /// A category's name; `None` when deleted.
    Category(Option<String>),
    Member(bool),
    Tagged(bool),
    /// Stars 1–5.
    Rating(Option<u8>),
}

/// Stars of a `rating.set` value (`0..=10`; 0 clears).
fn stars_of(rating: Option<u8>) -> Option<u8> {
    rating.filter(|r| *r > 0).map(|r| r.div_ceil(2).clamp(1, 5))
}

fn value_of(body: &OpBody) -> Option<Value> {
    Some(match body {
        OpBody::CategoryUpsert { name, .. } => Value::Category(Some(name.clone())),
        OpBody::CategoryDelete { .. } => Value::Category(None),
        OpBody::CategoryItem { member, .. } => Value::Member(*member),
        OpBody::TagSet { tagged, .. } => Value::Tagged(*tagged),
        OpBody::Rating { rating, .. } => Value::Rating(stars_of(*rating)),
        _ => return None,
    })
}

/// The op of the same register as `body` that says `value`.
fn body_with(body: &OpBody, value: Value) -> Option<OpBody> {
    Some(match (body, value) {
        (
            OpBody::CategoryUpsert {
                profile, category, ..
            }
            | OpBody::CategoryDelete { profile, category },
            Value::Category(name),
        ) => match name {
            Some(name) => OpBody::CategoryUpsert {
                profile: profile.clone(),
                category: category.clone(),
                name,
            },
            None => OpBody::CategoryDelete {
                profile: profile.clone(),
                category: category.clone(),
            },
        },
        (
            OpBody::CategoryItem {
                profile,
                category,
                target,
                ..
            },
            Value::Member(member),
        ) => OpBody::CategoryItem {
            profile: profile.clone(),
            category: category.clone(),
            target: target.clone(),
            member,
        },
        (
            OpBody::TagSet {
                profile,
                tag,
                target,
                ..
            },
            Value::Tagged(tagged),
        ) => OpBody::TagSet {
            profile: profile.clone(),
            tag: tag.clone(),
            target: target.clone(),
            tagged,
        },
        (
            OpBody::Rating {
                profile, target, ..
            },
            Value::Rating(stars),
        ) => OpBody::Rating {
            profile: profile.clone(),
            target: target.clone(),
            rating: stars.map(|s| s * 2),
        },
        _ => return None,
    })
}

/// The local category of `profile_id` in the main library that ops about
/// category `sync_id` of `profile` land in: the category itself, or the one
/// it was merged into.
fn local_category(
    conn: &Connection,
    profile_id: i64,
    profile: &str,
    sync_id: &str,
) -> Result<Option<labels::Category>, Error> {
    let mine = |c: &labels::Category| c.profile_id == profile_id && c.space == MAIN_SPACE;
    if let Some(c) = labels::category_by_sync_id(conn, sync_id)? {
        return Ok(Some(c).filter(mine));
    }
    let Some(into) = load_one(conn, &category_register(profile, sync_id))?
        .filter(|s| s.state == RegState::Merged)
        .and_then(|s| s.merged_into)
    else {
        return Ok(None);
    };
    Ok(labels::category_by_sync_id(conn, &into)?.filter(mine))
}

/// What the library here says for the register of `op`; `None` if this
/// device cannot tell (no such profile, category or item, or a personal
/// one).
fn local_value(
    conn: &Connection,
    op: &Op,
    ids: &HashMap<String, i64>,
) -> Result<Option<Value>, Error> {
    let profile = profile_of(&op.body);
    let Some(&profile_id) = ids.get(profile) else {
        return Ok(None);
    };
    let scope = Scope::main(profile_id);
    let entry = |target: &str| -> Result<Option<Entry>, Error> {
        match entry_of_target(conn, target, &op.aliases)? {
            Some(e) if !is_personal(conn, &e)? => Ok(Some(e)),
            _ => Ok(None),
        }
    };
    Ok(match &op.body {
        OpBody::CategoryUpsert { category, .. } | OpBody::CategoryDelete { category, .. } => {
            let c = labels::category_by_sync_id(conn, category)?;
            match c {
                Some(c) if c.profile_id == profile_id && c.space == MAIN_SPACE => {
                    Some(Value::Category(Some(c.name)))
                }
                Some(_) => None,
                None => Some(Value::Category(None)),
            }
        }
        OpBody::CategoryItem {
            category, target, ..
        } => {
            let Some(c) = local_category(conn, profile_id, profile, category)? else {
                return Ok(None);
            };
            let Some(e) = entry(target)? else {
                return Ok(None);
            };
            let member = labels::categories_of(conn, scope, &e)?
                .iter()
                .any(|x| x.id == c.id);
            Some(Value::Member(member))
        }
        OpBody::TagSet { tag, target, .. } => {
            let Some(e) = entry(target)? else {
                return Ok(None);
            };
            let key = labels::name_key(tag);
            let tagged = labels::tags_of(conn, scope, &e)?
                .iter()
                .any(|t| t.key == key);
            Some(Value::Tagged(tagged))
        }
        OpBody::Rating { target, .. } => {
            let Some(e) = entry(target)? else {
                return Ok(None);
            };
            Some(Value::Rating(labels::rating(conn, profile_id, &e)?))
        }
        _ => None,
    })
}

/// Every label of the main library of every profile as the op that says
/// it, by register, with the aliases of its item.
fn local_labels(
    conn: &Connection,
    ids: &HashMap<String, i64>,
) -> Result<BTreeMap<String, (OpBody, Vec<String>)>, Error> {
    let mut targets: HashMap<Entry, Option<(String, Vec<String>)>> = HashMap::new();
    let mut target = |e: &Entry| -> Result<Option<(String, Vec<String>)>, Error> {
        if let Some(t) = targets.get(e) {
            return Ok(t.clone());
        }
        let t = target_of(conn, e)?;
        targets.insert(e.clone(), t.clone());
        Ok(t)
    };
    let mut out = BTreeMap::new();
    let mut put = |body: OpBody, aliases: Vec<String>| {
        if body.validate().is_ok() {
            out.insert(body.register(), (body, aliases));
        }
    };
    let mut ids: Vec<(&String, &i64)> = ids.iter().collect();
    ids.sort();
    for (profile, &profile_id) in ids {
        let scope = Scope::main(profile_id);
        for c in labels::categories(conn, scope)? {
            put(
                OpBody::CategoryUpsert {
                    profile: profile.clone(),
                    category: c.sync_id.clone(),
                    name: c.name.clone(),
                },
                Vec::new(),
            );
            for e in labels::category_entries(conn, c.id)? {
                if let Some((t, aliases)) = target(&e)? {
                    put(
                        OpBody::CategoryItem {
                            profile: profile.clone(),
                            category: c.sync_id.clone(),
                            target: t,
                            member: true,
                        },
                        aliases,
                    );
                }
            }
        }
        for (tag, entries) in labels::tags(conn, scope)? {
            for e in entries {
                if let Some((t, aliases)) = target(&e)? {
                    put(
                        OpBody::TagSet {
                            profile: profile.clone(),
                            tag: tag.name.clone(),
                            target: t,
                            tagged: true,
                        },
                        aliases,
                    );
                }
            }
        }
        for (e, stars) in labels::rated_entries(conn, profile_id, 1)? {
            if let Some((t, aliases)) = target(&e)? {
                put(
                    OpBody::Rating {
                        profile: profile.clone(),
                        target: t,
                        rating: Some(stars * 2),
                    },
                    aliases,
                );
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// sending

/// Stamps a label op with its aliases and queues it in the outbox.
pub(crate) type Record<'a> = dyn FnMut(&Connection, OpBody, Vec<String>) -> Result<Op, Error> + 'a;

/// Records ops for every label change of the main library not sent yet:
/// new and renamed categories, deleted ones, items put in or taken out,
/// tags, ratings. Parked ops of other devices are retried first. `record`
/// stamps an op with its aliases and queues it in the outbox of `conn`.
/// Returns the number of ops recorded. Call it inside a transaction.
pub(crate) fn reconcile(conn: &Connection, record: &mut Record<'_>) -> Result<usize, Error> {
    retry_parked(conn)?;
    let ids = profiles(conn)?;
    let stored = load_all(conn)?;
    let local = local_labels(conn, &ids)?;
    let mut recorded = 0;
    for (register, (body, aliases)) in &local {
        let want = value_of(body);
        if let Some(s) = stored.get(register)
            && let Some(op) = &s.op
            && value_of(&op.body) == want
        {
            if s.state == RegState::Parked {
                set_state(conn, register, RegState::Applied)?;
            }
            continue;
        }
        let op = record(conn, body.clone(), aliases.clone())?;
        save(conn, &op, RegState::Applied)?;
        recorded += 1;
    }
    for (register, s) in &stored {
        if local.contains_key(register) || s.state != RegState::Applied {
            continue;
        }
        let Some(op) = &s.op else { continue };
        let Some(now) = local_value(conn, op, &ids)? else {
            continue;
        };
        if value_of(&op.body).as_ref() == Some(&now) {
            continue;
        }
        let Some(body) = body_with(&op.body, now) else {
            continue;
        };
        let op = record(conn, body, op.aliases.clone())?;
        save(conn, &op, RegState::Applied)?;
        recorded += 1;
    }
    Ok(recorded)
}

/// Forgets what was sent and received (the device left its family): after
/// the next pairing every label goes out again.
pub(crate) fn reset(conn: &Connection) -> Result<(), Error> {
    conn.execute("DELETE FROM label_sync", [])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// receiving

/// What writing an op did.
enum Outcome {
    /// Written (or nothing to write); whether the library changed.
    Applied(bool),
    /// About something this device does not have yet.
    Parked,
    /// A category that merged into another (`merged_into` is recorded).
    Merged,
}

/// Merges two local categories of a profile that have the same name: the
/// one with the smaller sync id stays. Returns the one that stays.
fn merge_pair(
    conn: &Connection,
    profile: &str,
    a: labels::Category,
    b: labels::Category,
) -> Result<labels::Category, Error> {
    let (win, lose) = if a.sync_id <= b.sync_id {
        (a, b)
    } else {
        (b, a)
    };
    labels::merge_category(conn, lose.id, win.id)?;
    mark_merged(conn, profile, &lose.sync_id, &win.sync_id)?;
    Ok(win)
}

fn upsert_category(
    conn: &Connection,
    profile_id: i64,
    profile: &str,
    sync_id: &str,
    name: &str,
    now: i64,
) -> Result<Outcome, Error> {
    let scope = Scope::main(profile_id);
    let same_name = labels::category_by_name(conn, scope, name)?;
    match labels::category_by_sync_id(conn, sync_id)? {
        // Not a category of this profile's main library: never touched.
        Some(c) if c.profile_id != profile_id || c.space != MAIN_SPACE => {
            Ok(Outcome::Applied(false))
        }
        Some(c) => {
            let mut target = c.clone();
            if let Some(other) = same_name.filter(|o| o.id != c.id) {
                target = merge_pair(conn, profile, c, other)?;
                if target.sync_id != sync_id {
                    // This category went into the other one.
                    return Ok(Outcome::Merged);
                }
            }
            if target.name != name {
                labels::rename_category(conn, target.id, name, now)?;
            }
            Ok(Outcome::Applied(true))
        }
        None => match same_name {
            Some(local) if local.sync_id.as_str() < sync_id => {
                // The local one stays; this one lands in it.
                mark_merged(conn, profile, sync_id, &local.sync_id)?;
                Ok(Outcome::Merged)
            }
            Some(local) => {
                // The other device's id stays: the local category takes it.
                labels::set_category_sync_id(conn, local.id, sync_id)?;
                mark_merged(conn, profile, &local.sync_id, sync_id)?;
                if local.name != name {
                    labels::rename_category(conn, local.id, name, now)?;
                }
                Ok(Outcome::Applied(true))
            }
            None => {
                labels::create_category_with_sync_id(conn, scope, sync_id, name, now)?;
                Ok(Outcome::Applied(true))
            }
        },
    }
}

/// Writes the label op `op` into the library; the register is not touched.
fn write(conn: &Connection, op: &Op, now: i64) -> Result<Outcome, Error> {
    let profile = profile_of(&op.body);
    let profile_id: Option<i64> = conn
        .prepare_cached("SELECT id FROM profiles WHERE sync_id = ?1")?
        .query_row([profile], |r| r.get(0))
        .optional()?;
    let Some(profile_id) = profile_id else {
        return Ok(Outcome::Parked);
    };
    let scope = Scope::main(profile_id);
    let entry = |target: &str| entry_of_target(conn, target, &op.aliases);
    Ok(match &op.body {
        OpBody::CategoryUpsert { category, name, .. } => {
            upsert_category(conn, profile_id, profile, category, name, now)?
        }
        OpBody::CategoryDelete { category, .. } => {
            match labels::category_by_sync_id(conn, category)? {
                Some(c) if c.profile_id == profile_id && c.space == MAIN_SPACE => {
                    Outcome::Applied(labels::delete_category(conn, c.id)?)
                }
                // Merged into another category or never here: a delete of
                // the old id does not touch the merged one.
                _ => Outcome::Applied(false),
            }
        }
        OpBody::CategoryItem {
            category,
            target,
            member,
            ..
        } => {
            let Some(c) = local_category(conn, profile_id, profile, category)? else {
                return Ok(Outcome::Parked);
            };
            let Some(e) = entry(target)? else {
                return Ok(Outcome::Parked);
            };
            Outcome::Applied(if *member {
                labels::add_to_category(conn, c.id, &e, now)?
            } else {
                labels::remove_from_category(conn, c.id, &e)?
            })
        }
        OpBody::TagSet {
            tag,
            target,
            tagged,
            ..
        } => {
            let Some(e) = entry(target)? else {
                return Ok(Outcome::Parked);
            };
            if *tagged {
                match labels::add_tag(conn, scope, &e, tag, now) {
                    Ok(changed) => Outcome::Applied(changed),
                    // More tags than an item may have here: the rest wait
                    // for a tag to go.
                    Err(pa_library::Error::Invalid(_)) => Outcome::Applied(false),
                    Err(e) => return Err(e.into()),
                }
            } else {
                Outcome::Applied(labels::remove_tag(conn, scope, &e, tag)?)
            }
        }
        OpBody::Rating { target, rating, .. } => {
            let Some(e) = entry(target)? else {
                return Ok(Outcome::Parked);
            };
            Outcome::Applied(labels::set_rating(
                conn,
                profile_id,
                &e,
                stars_of(*rating),
                now,
            )?)
        }
        _ => Outcome::Applied(false),
    })
}

/// Writes a label op of another device into the library of `conn` if it
/// is newer than its register (call it inside a transaction). Returns
/// whether the library changed.
pub(crate) fn apply_remote(conn: &Connection, op: &Op) -> Result<bool, Error> {
    if !is_label_op(&op.body) {
        return Ok(false);
    }
    let register = op.body.register();
    if let Some(s) = load_one(conn, &register)?
        && s.op.as_ref().is_some_and(|old| stamp(old) >= stamp(op))
    {
        return Ok(false);
    }
    if newer_here(conn, op)? {
        return Ok(false);
    }
    let merged = load_one(conn, &register)?.is_some_and(|s| s.state == RegState::Merged);
    let outcome = if merged {
        // A category merged into another: renames and deletes of the old id
        // no longer apply.
        Outcome::Merged
    } else {
        write(conn, op, now_ms())?
    };
    let (state, changed) = match outcome {
        Outcome::Applied(changed) => (RegState::Applied, changed),
        Outcome::Parked => (RegState::Parked, false),
        Outcome::Merged => (RegState::Merged, true),
    };
    save(conn, op, state)?;
    if state == RegState::Applied {
        save_local_form(conn, op)?;
    }
    if changed && matches!(op.body, OpBody::CategoryUpsert { .. }) {
        // Items that came before their category.
        retry_parked(conn)?;
    }
    Ok(changed)
}

/// Whether this device holds a newer write of the label `op` is about under
/// its own ids (the same item under another key, ADR-0079, or a merged
/// category). Then `op` lost: the library keeps the newer value, which is
/// also recorded under the sender's ids so it is not sent back as a change.
fn newer_here(conn: &Connection, op: &Op) -> Result<bool, Error> {
    let Some(body) = local_form(conn, op)? else {
        return Ok(false);
    };
    let Some(here) = load_one(conn, &body.register())?.and_then(|s| s.op) else {
        return Ok(false);
    };
    if stamp(&here) < stamp(op) {
        return Ok(false);
    }
    if let Some(value) = value_of(&here.body)
        && let Some(body) = body_with(&op.body, value)
    {
        let mirrored = Op {
            op_id: here.op_id,
            hlc: here.hlc.clone(),
            aliases: op.aliases.clone(),
            body,
        };
        let register = mirrored.body.register();
        let older = load_one(conn, &register)?
            .and_then(|s| s.op)
            .is_none_or(|old| stamp(&old) < stamp(&mirrored));
        if older {
            save(conn, &mirrored, RegState::Applied)?;
        }
    }
    Ok(true)
}

/// The op this device would send for the label `op` wrote: the same label
/// under this device's category id and item ref. `None` if they are the
/// ones `op` has.
fn local_form(conn: &Connection, op: &Op) -> Result<Option<OpBody>, Error> {
    let profile = profile_of(&op.body);
    let profile_id: Option<i64> = conn
        .prepare_cached("SELECT id FROM profiles WHERE sync_id = ?1")?
        .query_row([profile], |r| r.get(0))
        .optional()?;
    let Some(profile_id) = profile_id else {
        return Ok(None);
    };
    let local_target = |target: &str| -> Result<Option<String>, Error> {
        let Some(e) = entry_of_target(conn, target, &op.aliases)? else {
            return Ok(None);
        };
        Ok(target_of(conn, &e)?.map(|(t, _)| t))
    };
    let body = match &op.body {
        OpBody::CategoryItem {
            category,
            target,
            member,
            ..
        } => {
            let Some(c) = local_category(conn, profile_id, profile, category)? else {
                return Ok(None);
            };
            let Some(t) = local_target(target)? else {
                return Ok(None);
            };
            OpBody::CategoryItem {
                profile: profile.to_owned(),
                category: c.sync_id,
                target: t,
                member: *member,
            }
        }
        OpBody::TagSet {
            tag,
            target,
            tagged,
            ..
        } => {
            let Some(t) = local_target(target)? else {
                return Ok(None);
            };
            OpBody::TagSet {
                profile: profile.to_owned(),
                tag: tag.clone(),
                target: t,
                tagged: *tagged,
            }
        }
        OpBody::Rating { target, rating, .. } => {
            let Some(t) = local_target(target)? else {
                return Ok(None);
            };
            OpBody::Rating {
                profile: profile.to_owned(),
                target: t,
                rating: *rating,
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(body).filter(|b| b.register() != op.body.register()))
}

/// Records what a received label op wrote under this device's own ids too
/// (with the op's stamp), so [`reconcile`] does not send the same label
/// back under another key. A newer register is left alone.
fn save_local_form(conn: &Connection, op: &Op) -> Result<(), Error> {
    let Some(body) = local_form(conn, op)? else {
        return Ok(());
    };
    let local = Op {
        op_id: op.op_id,
        hlc: op.hlc.clone(),
        aliases: op.aliases.clone(),
        body,
    };
    let register = local.body.register();
    if let Some(s) = load_one(conn, &register)?
        && s.op.as_ref().is_some_and(|old| stamp(old) >= stamp(&local))
    {
        return Ok(());
    }
    save(conn, &local, RegState::Applied)
}

/// Writes the parked ops whose profile, category or item is here now.
/// Returns how many were written.
pub(crate) fn retry_parked(conn: &Connection) -> Result<usize, Error> {
    let parked: Vec<(String, Option<String>)> = {
        let mut stmt = conn.prepare_cached(
            "SELECT register, op FROM label_sync WHERE state = 'parked' ORDER BY register",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let now = now_ms();
    let mut written = 0;
    // Categories first: their items may be parked behind them.
    let mut ops: Vec<(String, Op)> = parked
        .into_iter()
        .filter_map(|(r, op)| Some((r, outbox::decode(&op?).ok()?)))
        .collect();
    ops.sort_by_key(|(_, op)| !matches!(op.body, OpBody::CategoryUpsert { .. }));
    for (register, op) in ops {
        if newer_here(conn, &op)? {
            // Lost to a label made here meanwhile.
            set_state(conn, &register, RegState::Applied)?;
            written += 1;
            continue;
        }
        match write(conn, &op, now)? {
            Outcome::Applied(_) => {
                set_state(conn, &register, RegState::Applied)?;
                save_local_form(conn, &op)?;
                written += 1;
            }
            Outcome::Merged => {
                conn.prepare_cached("UPDATE label_sync SET state = 'merged' WHERE register = ?1 AND merged_into IS NOT NULL")?
                    .execute([&register])?;
                written += 1;
            }
            Outcome::Parked => {}
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_round_up_from_any_rating() {
        assert_eq!(stars_of(None), None);
        assert_eq!(stars_of(Some(0)), None);
        assert_eq!(stars_of(Some(1)), Some(1));
        assert_eq!(stars_of(Some(2)), Some(1));
        assert_eq!(stars_of(Some(7)), Some(4));
        assert_eq!(stars_of(Some(10)), Some(5));
        assert_eq!(stars_of(Some(200)), Some(5));
    }
}
