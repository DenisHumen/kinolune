// SPDX-License-Identifier: MPL-2.0
//! Sync of the source list between paired devices (SYNC-010, ADR-NNNN).
//!
//! A network source with [`SourceSync`] other than `Off` travels as
//! `source.upsert` ops of `pa-sync`: its settings as a JSON object
//! ([`SyncedConfig`]: kind, name, address, user name, options, toggles,
//! the sync mode) and, with [`SourceSync::WithPassword`], the password in
//! the E2E container of [`pa_sync::seal`], sealed with the family key for
//! the context `source:<sync id>`. Deleting a synced source sends
//! `source.delete`.
//!
//! The functions here work on a connection the caller holds (inside its
//! transaction) and a recorder that stamps and queues ops (the catalog's
//! `pa_sync::Replica`):
//!
//! - [`reconcile`] compares every synced source with what was last sent
//!   (`sources.sync_seen`) and records ops for the differences and for
//!   pending deletions. It runs after local edits and when sync starts, so
//!   sources added before pairing go out too, and after a key rotation
//!   (the password is sealed again with the new epoch).
//! - [`apply_remote`] writes an op of another device: last writer wins by
//!   `(hlc, op_id)` against `sources.sync_stamp` and the tombstones, a
//!   received password goes only into the [`SecretStore`] (the row gets a
//!   fresh `credential_ref`), never into SQLite or a log.
//!
//! A local source of the same kind and address that never synced is
//! adopted by an incoming upsert instead of being duplicated.

use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_secrets::{SecretKey, SecretStore, SecretString};
use pa_sync::seal::{self, SealKey};
use pa_sync::{Hlc, Op, OpBody, OpId, Stamp};
use serde::{Deserialize, Serialize};

use crate::manager::{new_secret_key, non_empty, options_json};
use crate::{Error, Result, SourceKind, SourceOptions, SourceSync, address, now_ms};

/// Settings of a source in a `source.upsert` op. Never holds credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedConfig {
    /// `smb` or `webdav`.
    pub kind: String,
    pub name: String,
    /// Normalized URI without credentials.
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default)]
    pub options: SourceOptions,
    pub scan_into_library: bool,
    pub auto_scan: bool,
    pub enabled: bool,
    /// `settings` or `password` ([`SourceSync::as_str`]): the receiver
    /// syncs the source the same way, so its edits come back.
    pub sync: String,
}

/// What [`apply_remote`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// Nothing: a duplicate, an older op, or not a source op.
    Unchanged,
    /// A source was added or changed.
    Changed,
    /// A source was deleted: the caller removes what was left without
    /// files (works, music) in the same transaction.
    Deleted,
}

/// The E2E context of the password of source `sync_id`.
pub fn seal_context(sync_id: &str) -> String {
    format!("source:{sync_id}")
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

/// A synced row as reconcile and apply see it.
struct Row {
    id: i64,
    sync_id: String,
    mode: SourceSync,
    config: SyncedConfig,
    credential_ref: Option<String>,
    stamp: Option<Stamp>,
    seen: Option<String>,
}

const ROW_COLUMNS: &str = "id, sync_id, sync_mode, kind, name, uri, username, options_json, \
     scan_into_library, auto_scan, enabled, credential_ref, sync_stamp, sync_seen";

fn read_row(r: &pa_library::rusqlite::Row<'_>) -> pa_library::rusqlite::Result<Row> {
    let mode = SourceSync::from_i64(r.get(2)?);
    let options: String = r.get(7)?;
    Ok(Row {
        id: r.get(0)?,
        sync_id: r.get(1)?,
        mode,
        config: SyncedConfig {
            kind: r.get(3)?,
            name: r.get(4)?,
            uri: r.get(5)?,
            username: r.get(6)?,
            options: serde_json::from_str(&options).unwrap_or_default(),
            scan_into_library: r.get(8)?,
            auto_scan: r.get(9)?,
            enabled: r.get(10)?,
            sync: mode.as_str().to_owned(),
        },
        credential_ref: r.get(11)?,
        stamp: parse_stamp(r.get(12)?),
        seen: r.get(13)?,
    })
}

fn row_by_sync_id(conn: &Connection, sync_id: &str) -> Result<Option<Row>> {
    Ok(conn
        .query_row(
            &format!("SELECT {ROW_COLUMNS} FROM sources WHERE sync_id = ?1"),
            [sync_id],
            read_row,
        )
        .optional()?)
}

/// What a sent (or applied) op carried, as this device compares it: mode,
/// settings, the local key of the password and, when the password syncs,
/// the epoch it was sealed with. No secret in it.
fn seen_of(row: &Row, seal: Option<&SealKey>) -> String {
    let config = serde_json::to_string(&row.config).unwrap_or_default();
    let cred = row.credential_ref.as_deref().unwrap_or("");
    let epoch = match (row.mode, row.credential_ref.is_some(), seal) {
        (SourceSync::WithPassword, true, Some(k)) => k.epoch().to_string(),
        _ => String::new(),
    };
    format!(
        "{}\u{1f}{config}\u{1f}{cred}\u{1f}{epoch}",
        row.mode.as_str()
    )
}

fn tombstone(conn: &Connection, sync_id: &str) -> Result<Option<(Option<Stamp>, bool)>> {
    Ok(conn
        .query_row(
            "SELECT stamp, pending FROM source_sync_tombstones WHERE sync_id = ?1",
            [sync_id],
            |r| Ok((parse_stamp(r.get(0)?), r.get::<_, bool>(1)?)),
        )
        .optional()?)
}

fn set_tombstone(conn: &Connection, sync_id: &str, stamp: &Stamp) -> Result<()> {
    conn.execute(
        "INSERT INTO source_sync_tombstones (sync_id, stamp, pending) VALUES (?1, ?2, 0)
         ON CONFLICT (sync_id) DO UPDATE SET stamp = excluded.stamp, pending = 0",
        params![sync_id, stamp_json(stamp)],
    )?;
    Ok(())
}

/// Records ops for every local change of a synced source not sent yet and
/// for pending deletions. `record` stamps an op, queues it in the outbox of
/// `conn` and returns its stamp. Returns the number of ops recorded.
///
/// With [`SourceSync::WithPassword`] the password is read from `secrets`
/// and sealed with `seal`; without a key (not paired) or a store, the op
/// carries no password and the receivers keep theirs.
pub fn reconcile(
    conn: &Connection,
    secrets: Option<&dyn SecretStore>,
    seal: Option<&SealKey>,
    record: &mut dyn FnMut(&Connection, OpBody) -> std::result::Result<Stamp, pa_sync::Error>,
) -> Result<usize> {
    let mut rec = |conn: &Connection, body: OpBody| {
        record(conn, body).map_err(|e| Error::Invalid(format!("sync: {e}")))
    };
    let mut recorded = 0;
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM sources
              WHERE sync_mode > 0 AND sync_id IS NOT NULL AND kind IN ('smb', 'webdav', 'subsonic')
                -- UI-025: personal media never leaves the device.
                AND content_type <> 'personal'"
        ))?;
        stmt.query_map([], read_row)?
            .collect::<std::result::Result<_, _>>()?
    };
    for row in rows {
        let seen = seen_of(&row, seal);
        if row.seen.as_deref() == Some(seen.as_str()) {
            continue;
        }
        let secret = match (row.mode, &row.credential_ref, secrets, seal) {
            (SourceSync::WithPassword, Some(cred), Some(store), Some(key)) => {
                match store.get(&SecretKey::new(cred.clone())?)? {
                    Some(password) => Some(
                        seal::seal(
                            key,
                            &seal_context(&row.sync_id),
                            password.expose().as_bytes(),
                        )
                        .map_err(|e| Error::Invalid(format!("seal: {e}")))?,
                    ),
                    None => None,
                }
            }
            _ => None,
        };
        let body = OpBody::SourceUpsert {
            source: row.sync_id.clone(),
            config: serde_json::to_string(&row.config)
                .map_err(|e| Error::Invalid(format!("config: {e}")))?,
            secret,
        };
        let stamp = rec(conn, body)?;
        conn.execute(
            "UPDATE sources SET sync_seen = ?2, sync_stamp = ?3 WHERE id = ?1",
            params![row.id, seen, stamp_json(&stamp)],
        )?;
        recorded += 1;
    }
    let pending: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT sync_id FROM source_sync_tombstones WHERE pending = 1")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?
    };
    for sync_id in pending {
        let body = OpBody::SourceDelete {
            source: sync_id.clone(),
        };
        let stamp = rec(conn, body)?;
        set_tombstone(conn, &sync_id, &stamp)?;
        recorded += 1;
    }
    Ok(recorded)
}

/// Forgets what was sent (the device left its family): after the next
/// pairing every synced source goes out again. Pending deletions are
/// dropped, nobody is left to tell.
pub fn reset(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE sources SET sync_seen = NULL, sync_stamp = NULL WHERE sync_seen IS NOT NULL
            OR sync_stamp IS NOT NULL",
        [],
    )?;
    conn.execute("DELETE FROM source_sync_tombstones", [])?;
    Ok(())
}

/// Checks a received config the way [`crate::SourceManager::create`] checks
/// input, and returns it normalized.
fn checked(config: &str) -> Result<(SourceKind, SyncedConfig, SourceSync)> {
    let mut c: SyncedConfig =
        serde_json::from_str(config).map_err(|e| Error::Invalid(format!("config: {e}")))?;
    let kind: SourceKind = c.kind.parse()?;
    if !kind.is_network() {
        return Err(Error::Invalid(format!("a {kind} source does not sync")));
    }
    let sync: SourceSync = c.sync.parse()?;
    if sync == SourceSync::Off {
        return Err(Error::Invalid("a synced source with sync off".into()));
    }
    c.name = non_empty(&c.name, "name")?;
    c.uri = address::normalize(kind, &c.uri)?;
    c.username = c
        .username
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty());
    options_json(&c.options)?;
    Ok((kind, c, sync))
}

/// Writes a `source.upsert` or `source.delete` of another device into the
/// library of `conn` (call it inside a transaction). A password in the op
/// is opened with `seal` and stored in `secrets` only; if it cannot be
/// opened (another key epoch, no key) the source keeps its local password.
/// Other ops give [`Applied::Unchanged`].
pub fn apply_remote(
    conn: &Connection,
    secrets: Option<&dyn SecretStore>,
    seal: Option<&SealKey>,
    op: &Op,
) -> Result<Applied> {
    let stamp: Stamp = (op.hlc.clone(), op.op_id);
    match &op.body {
        OpBody::SourceUpsert {
            source,
            config,
            secret,
        } => upsert(
            conn,
            secrets,
            seal,
            source,
            config,
            secret.as_deref(),
            &stamp,
        ),
        OpBody::SourceDelete { source } => delete(conn, secrets, source, &stamp),
        _ => Ok(Applied::Unchanged),
    }
}

fn newer(stamp: &Stamp, than: Option<&Stamp>) -> bool {
    than.is_none_or(|t| stamp > t)
}

fn upsert(
    conn: &Connection,
    secrets: Option<&dyn SecretStore>,
    seal: Option<&SealKey>,
    sync_id: &str,
    config: &str,
    secret: Option<&str>,
    stamp: &Stamp,
) -> Result<Applied> {
    let mut row = row_by_sync_id(conn, sync_id)?;
    if let Some(r) = &row {
        if !newer(stamp, r.stamp.as_ref()) {
            return Ok(Applied::Unchanged);
        }
    } else if let Some((tomb, pending)) = tombstone(conn, sync_id)? {
        // Deleted here and not sent yet: that delete is newer than anything
        // already in flight.
        if pending || !newer(stamp, tomb.as_ref()) {
            return Ok(Applied::Unchanged);
        }
    }
    let (kind, c, mode) = checked(config)?;
    if row.is_none() {
        // The same share added by hand before pairing: adopt it.
        let adopted: Option<i64> = conn
            .query_row(
                "SELECT id FROM sources WHERE kind = ?1 AND uri = ?2 AND sync_stamp IS NULL
                   ORDER BY id LIMIT 1",
                params![kind.as_str(), c.uri],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = adopted {
            conn.execute(
                "UPDATE sources SET sync_id = ?2 WHERE id = ?1",
                params![id, sync_id],
            )?;
            row = row_by_sync_id(conn, sync_id)?;
        }
    }

    // The password, opened only on this device and stored only in its
    // secret store under a fresh key.
    let password: Option<SecretString> = match (secret, seal) {
        (Some(sealed), Some(key)) => {
            match seal::open(key, &seal_context(sync_id), sealed) {
                Ok(bytes) => Some(SecretString::new(
                    String::from_utf8(bytes.to_vec())
                        .map_err(|_| Error::Invalid("sealed password is not UTF-8".into()))?,
                )),
                // Another epoch or a broken container: keep the local one.
                Err(_) => None,
            }
        }
        _ => None,
    };
    let old_cred = row.as_ref().and_then(|r| r.credential_ref.clone());
    let (cred, new_secret) = match (&password, secrets) {
        (Some(p), Some(_)) if !p.is_empty() => (Some(new_secret_key()?), Some(p)),
        _ => (None, None),
    };
    let credential_ref = cred
        .as_ref()
        .map(|k| k.as_str().to_owned())
        .or(old_cred.clone());
    let options = options_json(&c.options)?;
    let now = now_ms();
    let id = match &row {
        Some(r) => {
            conn.execute(
                "UPDATE sources SET name = ?2, uri = ?3, username = ?4, credential_ref = ?5,
                     options_json = ?6, scan_into_library = ?7, auto_scan = ?8, enabled = ?9,
                     sync_mode = ?10, updated_at_ms = ?11
                 WHERE id = ?1",
                params![
                    r.id,
                    c.name,
                    c.uri,
                    c.username,
                    credential_ref,
                    options,
                    c.scan_into_library,
                    c.auto_scan,
                    c.enabled,
                    mode.as_i64(),
                    now,
                ],
            )?;
            r.id
        }
        None => {
            conn.execute(
                "INSERT INTO sources (kind, name, uri, username, credential_ref, options_json,
                     scan_into_library, auto_scan, enabled, position, created_at_ms,
                     updated_at_ms, sync_id, sync_mode)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                     (SELECT ifnull(max(position), 0) + 1 FROM sources), ?10, ?10, ?11, ?12)",
                params![
                    kind.as_str(),
                    c.name,
                    c.uri,
                    c.username,
                    credential_ref,
                    options,
                    c.scan_into_library,
                    c.auto_scan,
                    c.enabled,
                    now,
                    sync_id,
                    mode.as_i64(),
                ],
            )?;
            conn.last_insert_rowid()
        }
    };
    if let (Some(key), Some(p), Some(store)) = (&cred, new_secret, secrets) {
        store.set(key, p)?;
        if let Some(old) = old_cred {
            // The row no longer points at it; a failure leaves an orphan.
            let _ = SecretKey::new(old).map(|k| store.delete(&k));
        }
    }
    conn.execute(
        "DELETE FROM source_sync_tombstones WHERE sync_id = ?1",
        [sync_id],
    )?;
    // Mark it as seen in the form this device compares, so reconcile does
    // not send it back.
    let applied = row_by_sync_id(conn, sync_id)?.ok_or(Error::NotFound(id))?;
    conn.execute(
        "UPDATE sources SET sync_seen = ?2, sync_stamp = ?3 WHERE id = ?1",
        params![id, seen_of(&applied, seal), stamp_json(stamp)],
    )?;
    Ok(Applied::Changed)
}

fn delete(
    conn: &Connection,
    secrets: Option<&dyn SecretStore>,
    sync_id: &str,
    stamp: &Stamp,
) -> Result<Applied> {
    if let Some((tomb, pending)) = tombstone(conn, sync_id)?
        && !pending
        && !newer(stamp, tomb.as_ref())
    {
        return Ok(Applied::Unchanged);
    }
    let row = row_by_sync_id(conn, sync_id)?;
    let applied = match &row {
        Some(r) if newer(stamp, r.stamp.as_ref()) => {
            conn.execute("DELETE FROM sources WHERE id = ?1", [r.id])?;
            if let (Some(cred), Some(store)) = (&r.credential_ref, secrets) {
                let _ = SecretKey::new(cred.clone()).map(|k| store.delete(&k));
            }
            Applied::Deleted
        }
        Some(_) => return Ok(Applied::Unchanged),
        None => Applied::Unchanged,
    };
    // Also replaces the pending tombstone the delete trigger just wrote.
    set_tombstone(conn, sync_id, stamp)?;
    Ok(applied)
}
