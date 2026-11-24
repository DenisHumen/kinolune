// SPDX-License-Identifier: MPL-2.0
//! Sync of the profiles between paired devices (SYNC-009, ADR-0102).
//!
//! Progress, favorites and lists are already keyed by the profile's
//! `sync_id`, but a profile made on one device had no row on the others:
//! their ops had nowhere to land. A profile now travels as `profile.upsert`
//! ([`SyncedProfile`]: name, avatar, kid flag, rating limits, whether it has
//! a PIN) and, with a family key, the Argon2id hash of its PIN sealed in the
//! E2E container of [`crate::seal`] for the context `profile-pin:<sync id>`.
//! Deleting a profile sends `profile.delete`. The same profile then has the
//! same `sync_id` everywhere, so two profiles watching one film never touch
//! each other's progress.
//!
//! Like the sources of SYNC-010 (`pa_sources::sync`):
//!
//! - [`reconcile`] compares every profile with what was last sent
//!   (`profiles.sync_seen`) and records ops for the differences and for
//!   pending deletions. It runs after local edits and when sync starts.
//! - [`apply_remote`] writes an op of another device: last writer wins by
//!   `(hlc, op_id)` against `profiles.sync_stamp` and the tombstones. A PIN
//!   hash that cannot be opened (no key, another epoch) is not applied: the
//!   profile keeps its local PIN. A profile without a PIN on the sender
//!   loses it here too.
//!
//! Per device, never synced: the folder white list (`profile_folders`,
//! source ids are local), the order, the PIN attempt counter, the active
//! profile. A received profile that restricts folders shows nothing until
//! folders are allowed on this device: the safe side.

use pa_library::profiles::{self, ProfileSettings};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::op::{Op, OpBody, OpId};
use crate::seal::{self, SealKey};
use crate::{Error, Hlc, Stamp};

/// Settings of a profile in a `profile.upsert` op. Never holds the PIN.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedProfile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub is_kid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age: Option<u8>,
    #[serde(default)]
    pub rating_country: String,
    pub hide_unrated: bool,
    pub hide_artwork: bool,
    pub restrict_folders: bool,
    /// The profile has a PIN (its hash may still be missing from the op).
    pub has_pin: bool,
}

impl SyncedProfile {
    fn settings(&self) -> ProfileSettings {
        ProfileSettings {
            name: self.name.clone(),
            avatar: self.avatar.clone(),
            is_kid: self.is_kid,
            max_age: self.max_age,
            rating_country: self.rating_country.clone(),
            hide_unrated: self.hide_unrated,
            hide_artwork: self.hide_artwork,
            restrict_folders: self.restrict_folders,
        }
    }
}

/// What [`apply_remote`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// Nothing: a duplicate, an older op, or not a profile op.
    Unchanged,
    /// A profile was added or changed.
    Changed,
    /// A profile was deleted with its state.
    Deleted,
}

/// The E2E context of the PIN hash of profile `sync_id`.
pub fn pin_context(sync_id: &str) -> String {
    format!("profile-pin:{sync_id}")
}

#[derive(Serialize, Deserialize)]
struct StampJson {
    hlc: Hlc,
    op_id: OpId,
}

fn stamp_json(stamp: &Stamp) -> String {
    serde_json::to_string(&StampJson {
        hlc: stamp.0.clone(),
        op_id: stamp.1,
    })
    .unwrap_or_default()
}

fn parse_stamp(text: Option<String>) -> Option<Stamp> {
    let s: StampJson = serde_json::from_str(&text?).ok()?;
    Some((s.hlc, s.op_id))
}

fn newer(stamp: &Stamp, than: Option<&Stamp>) -> bool {
    than.is_none_or(|t| stamp > t)
}

/// A profile row as reconcile and apply see it.
struct Row {
    id: i64,
    sync_id: String,
    config: SyncedProfile,
    pin_hash: Option<String>,
    stamp: Option<Stamp>,
    seen: Option<String>,
}

const ROW_COLUMNS: &str = "id, sync_id, name, avatar, is_kid, max_age, rating_country, \
     hide_unrated, hide_artwork, restrict_folders, pin_hash, sync_stamp, sync_seen";

fn read_row(r: &pa_library::rusqlite::Row<'_>) -> pa_library::rusqlite::Result<Row> {
    let pin_hash: Option<String> = r.get(10)?;
    Ok(Row {
        id: r.get(0)?,
        sync_id: r.get(1)?,
        config: SyncedProfile {
            name: r.get(2)?,
            avatar: r.get(3)?,
            is_kid: r.get(4)?,
            max_age: r.get(5)?,
            rating_country: r.get(6)?,
            hide_unrated: r.get(7)?,
            hide_artwork: r.get(8)?,
            restrict_folders: r.get(9)?,
            has_pin: pin_hash.is_some(),
        },
        pin_hash,
        stamp: parse_stamp(r.get(11)?),
        seen: r.get(12)?,
    })
}

fn row_by_sync_id(conn: &Connection, sync_id: &str) -> Result<Option<Row>, Error> {
    Ok(conn
        .query_row(
            &format!("SELECT {ROW_COLUMNS} FROM profiles WHERE sync_id = ?1"),
            [sync_id],
            read_row,
        )
        .optional()?)
}

/// What an upsert of `config` with `pin_hash` carries, in the form this
/// device compares; the key epoch counts, so a new key sends the PIN again.
fn seen_of(config: &SyncedProfile, pin_hash: Option<&str>, seal: Option<&SealKey>) -> String {
    let config = serde_json::to_string(config).unwrap_or_default();
    let epoch = match (pin_hash, seal) {
        (Some(_), Some(k)) => k.epoch().to_string(),
        _ => String::new(),
    };
    format!(
        "{config}\u{1f}{}\u{1f}{epoch}",
        pin_hash.unwrap_or_default()
    )
}

fn tombstone(conn: &Connection, sync_id: &str) -> Result<Option<Option<Stamp>>, Error> {
    Ok(conn
        .query_row(
            "SELECT stamp FROM profile_sync_tombstones WHERE sync_id = ?1",
            [sync_id],
            |r| Ok(parse_stamp(r.get(0)?)),
        )
        .optional()?)
}

fn set_tombstone(conn: &Connection, sync_id: &str, stamp: &Stamp) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO profile_sync_tombstones (sync_id, stamp, pending) VALUES (?1, ?2, 0)
         ON CONFLICT (sync_id) DO UPDATE SET stamp = excluded.stamp, pending = 0",
        params![sync_id, stamp_json(stamp)],
    )?;
    Ok(())
}

/// Records ops for every local profile change not sent yet and for pending
/// deletions. `record` stamps an op, queues it in the outbox of `conn` and
/// returns its stamp. Without `seal` the ops carry no PIN hash. Returns the
/// number of ops recorded.
pub fn reconcile(
    conn: &Connection,
    seal: Option<&SealKey>,
    record: &mut dyn FnMut(&Connection, OpBody) -> Result<Stamp, Error>,
) -> Result<usize, Error> {
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(&format!("SELECT {ROW_COLUMNS} FROM profiles ORDER BY id"))?;
        stmt.query_map([], read_row)?.collect::<Result<_, _>>()?
    };
    let mut recorded = 0;
    for row in rows {
        let seen = seen_of(&row.config, row.pin_hash.as_deref(), seal);
        if row.seen.as_deref() == Some(seen.as_str()) {
            continue;
        }
        let pin = match (&row.pin_hash, seal) {
            (Some(hash), Some(key)) => Some(
                seal::seal(key, &pin_context(&row.sync_id), hash.as_bytes())
                    .map_err(|_| Error::Invalid("profile pin seal"))?,
            ),
            _ => None,
        };
        let body = OpBody::ProfileUpsert {
            profile: row.sync_id.clone(),
            config: serde_json::to_string(&row.config)?,
            pin,
        };
        let stamp = record(conn, body)?;
        conn.execute(
            "UPDATE profiles SET sync_seen = ?2, sync_stamp = ?3 WHERE id = ?1",
            params![row.id, seen, stamp_json(&stamp)],
        )?;
        recorded += 1;
    }
    let pending: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT sync_id FROM profile_sync_tombstones WHERE pending = 1")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    for sync_id in pending {
        let stamp = record(
            conn,
            OpBody::ProfileDelete {
                profile: sync_id.clone(),
            },
        )?;
        set_tombstone(conn, &sync_id, &stamp)?;
        recorded += 1;
    }
    Ok(recorded)
}

/// A new device set up from a phone (UI-046) takes the family's profiles:
/// its untouched first profile (`default`, never synced, no PIN — what a
/// fresh install has) is marked as sent, so it does not go out with a newer
/// clock and overwrite the phone's first profile; the phone's upsert then
/// applies here. Call it before the device joins. Returns whether a
/// profile was marked.
pub fn adopt_family(conn: &Connection) -> Result<bool, Error> {
    let Some(row) = row_by_sync_id(conn, "default")? else {
        return Ok(false);
    };
    if row.seen.is_some() || row.stamp.is_some() || row.pin_hash.is_some() {
        return Ok(false);
    }
    conn.execute(
        "UPDATE profiles SET sync_seen = ?2 WHERE id = ?1",
        params![row.id, seen_of(&row.config, None, None)],
    )?;
    Ok(true)
}

/// Forgets what was sent (the device left its family): after the next
/// pairing every profile goes out again; pending deletions are dropped.
pub fn reset(conn: &Connection) -> Result<(), Error> {
    conn.execute_batch(
        "UPDATE profiles SET sync_seen = NULL, sync_stamp = NULL
          WHERE sync_seen IS NOT NULL OR sync_stamp IS NOT NULL;
         DELETE FROM profile_sync_tombstones;",
    )?;
    Ok(())
}

/// Writes a `profile.upsert` or `profile.delete` of another device into the
/// library of `conn` (call it inside a transaction). Other ops give
/// [`Applied::Unchanged`].
pub fn apply_remote(
    conn: &Connection,
    seal: Option<&SealKey>,
    op: &Op,
    now_ms: i64,
) -> Result<Applied, Error> {
    let stamp: Stamp = (op.hlc.clone(), op.op_id);
    match &op.body {
        OpBody::ProfileUpsert {
            profile,
            config,
            pin,
        } => upsert(conn, seal, profile, config, pin.as_deref(), &stamp, now_ms),
        OpBody::ProfileDelete { profile } => delete(conn, profile, &stamp),
        _ => Ok(Applied::Unchanged),
    }
}

fn upsert(
    conn: &Connection,
    seal: Option<&SealKey>,
    sync_id: &str,
    config: &str,
    pin: Option<&str>,
    stamp: &Stamp,
    now_ms: i64,
) -> Result<Applied, Error> {
    if let Some(dead) = tombstone(conn, sync_id)?
        && !newer(stamp, dead.as_ref())
    {
        return Ok(Applied::Unchanged);
    }
    let config: SyncedProfile =
        serde_json::from_str(config).map_err(|_| Error::Invalid("profile config"))?;
    config.settings().check()?;
    let existing = row_by_sync_id(conn, sync_id)?;
    if existing
        .as_ref()
        .is_some_and(|r| !newer(stamp, r.stamp.as_ref()))
    {
        return Ok(Applied::Unchanged);
    }
    // The PIN: the sender's hash if it opens here; none if the sender has
    // none; otherwise the local one stays.
    let opened: Option<String> = match (pin, seal) {
        (Some(sealed), Some(key)) => seal::open(key, &pin_context(sync_id), sealed)
            .ok()
            .and_then(|b| String::from_utf8(b.to_vec()).ok())
            .filter(|h| h.starts_with("argon2id$")),
        _ => None,
    };
    let local_pin = existing.as_ref().and_then(|r| r.pin_hash.clone());
    let pin_hash = if !config.has_pin {
        None
    } else {
        opened.or(local_pin)
    };
    let s = config.settings();
    let id = match &existing {
        Some(r) => {
            conn.execute(
                "UPDATE profiles SET name = ?2, avatar = ?3, is_kid = ?4, max_age = ?5,
                        rating_country = ?6, hide_unrated = ?7, hide_artwork = ?8,
                        restrict_folders = ?9, pin_hash = ?10, updated_at_ms = ?11
                  WHERE id = ?1",
                params![
                    r.id,
                    s.name,
                    s.avatar,
                    s.is_kid,
                    s.max_age,
                    s.rating_country,
                    s.hide_unrated,
                    s.hide_artwork,
                    s.restrict_folders,
                    pin_hash,
                    now_ms
                ],
            )?;
            r.id
        }
        None => {
            conn.execute(
                "INSERT INTO profiles (sync_id, name, avatar, is_kid, position, max_age,
                        rating_country, hide_unrated, hide_artwork, restrict_folders, pin_hash,
                        created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, (SELECT ifnull(max(position), -1) + 1 FROM profiles),
                         ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                params![
                    sync_id,
                    s.name,
                    s.avatar,
                    s.is_kid,
                    s.max_age,
                    s.rating_country,
                    s.hide_unrated,
                    s.hide_artwork,
                    s.restrict_folders,
                    pin_hash,
                    now_ms
                ],
            )?;
            conn.last_insert_rowid()
        }
    };
    // What is stored now counts as sent: it is not echoed back.
    let seen = seen_of(&config, pin_hash.as_deref(), seal);
    conn.execute(
        "UPDATE profiles SET sync_seen = ?2, sync_stamp = ?3 WHERE id = ?1",
        params![id, seen, stamp_json(stamp)],
    )?;
    profiles::refresh(conn)?;
    Ok(Applied::Changed)
}

fn delete(conn: &Connection, sync_id: &str, stamp: &Stamp) -> Result<Applied, Error> {
    if sync_id == "default" {
        return Ok(Applied::Unchanged);
    }
    if let Some(dead) = tombstone(conn, sync_id)?
        && !newer(stamp, dead.as_ref())
    {
        return Ok(Applied::Unchanged);
    }
    let row = row_by_sync_id(conn, sync_id)?;
    if row
        .as_ref()
        .is_some_and(|r| !newer(stamp, r.stamp.as_ref()))
    {
        return Ok(Applied::Unchanged);
    }
    let deleted = match row {
        Some(r) => {
            profiles::delete(conn, r.id)?;
            true
        }
        None => false,
    };
    // Also replaces the pending tombstone the trigger wrote: not echoed.
    set_tombstone(conn, sync_id, stamp)?;
    Ok(if deleted {
        Applied::Deleted
    } else {
        Applied::Unchanged
    })
}
