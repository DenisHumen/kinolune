// SPDX-License-Identifier: MPL-2.0
//! Sync through a shared folder of a source (SYNC-007, ADR-0129).
//!
//! The folder is one source's: picked by the core itself (SYNC-015,
//! `folder_auto`) or by the user ([`Catalog::set_folder_sync`]);
//! from then on every local op is also kept in `sync_folder_outbox` (a
//! trigger on `sync_outbox`, so LAN acks do not take them away), and
//! [`Catalog::run_folder_sync`] appends them to this device's journal in
//! `/.kinolune/state/`, reads the journals of the other devices from where
//! it stopped and applies their ops like ops from the LAN
//! ([`Catalog::apply_remote`]). A folder turns sync of this library on
//! (with the device id of the sync service), paired or not.
//!
//! Nothing secret goes into the folder ([`pa_sync::folder::for_folder`]).
//! A source that cannot be written only receives.

use pa_library::rusqlite::{OptionalExtension, params};
use pa_sync::folder::{self, Cursors, Folder, FolderError};
use pa_sync::{Op, outbox};
use pa_vfs::ErrorKind;

use crate::{Catalog, Error, now_ms};

/// Ops written to the folder in one round at most; the rest goes next time.
const WRITE_BATCH: u32 = 2_000;

/// Who decides which folder syncs (SYNC-015).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FolderSyncMode {
    /// The core picks the folder itself (the default).
    #[default]
    Auto,
    /// The user picked [`FolderSyncStatus::source_id`].
    Manual,
    /// The user turned folder sync off.
    Off,
}

impl FolderSyncMode {
    /// `auto`, `manual` or `off`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::Off => "off",
        }
    }
}

/// What the app shows about folder sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderSyncStatus {
    /// Who picks the folder. In [`FolderSyncMode::Auto`] the source is
    /// `None` while no folder was found yet.
    pub mode: FolderSyncMode,
    /// The source whose folder syncs; `None` while there is none.
    pub source_id: Option<i64>,
    /// End of the last round that reached the folder (Unix ms).
    pub last_sync_ms: Option<i64>,
    /// Why the last round failed: `unavailable` (no connection),
    /// `denied` (the share or account refused), `full` or `other`.
    pub last_error: Option<String>,
    /// Other devices with a journal in the folder.
    pub devices: u32,
    /// The folder cannot be written: this device only receives.
    pub read_only: bool,
    /// Own changes not in the folder yet.
    pub pending: u64,
    /// Lines of the others skipped in the last round (broken, from a newer
    /// version, refused).
    pub skipped: u64,
}

/// Result of one round.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderSyncRun {
    pub status: FolderSyncStatus,
    /// Own ops written to the folder.
    pub written: usize,
    /// Ops of other devices that changed this library.
    pub changed: usize,
}

pub(crate) struct Row {
    pub(crate) source_id: i64,
    cursors: String,
    snapshot_size: i64,
    /// Picked by the core (SYNC-015), not by the user.
    pub(crate) auto: bool,
    /// Other devices the last round that reached the folder saw.
    pub(crate) devices: u32,
    /// Since when rounds fail to reach the folder.
    pub(crate) failing_since_ms: Option<i64>,
}

fn error_code(e: &FolderError) -> &'static str {
    match e {
        FolderError::Vfs(v) => match v.kind() {
            ErrorKind::Unavailable | ErrorKind::TimedOut | ErrorKind::NotFound => "unavailable",
            ErrorKind::PermissionDenied | ErrorKind::AuthenticationFailed => "denied",
            ErrorKind::StorageFull => "full",
            _ => "other",
        },
        FolderError::Sync(_) => "other",
    }
}

fn catalog_error_code(e: &Error) -> &'static str {
    match e {
        Error::NotFound(_) => "unavailable",
        Error::Denied(_) => "denied",
        _ => "unavailable",
    }
}

fn signed(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl Catalog {
    pub(crate) fn folder_row(&self) -> Result<Option<Row>, Error> {
        Ok(self
            .lock()
            .query_row(
                "SELECT source_id, cursors, snapshot_size, auto, devices, failing_since_ms
                 FROM sync_folder WHERE id = 1",
                [],
                |r| {
                    Ok(Row {
                        source_id: r.get(0)?,
                        cursors: r.get(1)?,
                        snapshot_size: r.get(2)?,
                        auto: r.get(3)?,
                        devices: r.get(4)?,
                        failing_since_ms: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// The user turned folder sync off.
    pub(crate) fn folder_off(&self) -> Result<bool, Error> {
        Ok(self
            .lock()
            .query_row(
                "SELECT mode = 'off' FROM sync_folder_choice WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Status of folder sync.
    pub fn folder_sync_status(&self) -> Result<FolderSyncStatus, Error> {
        let conn = self.lock();
        let pending: i64 =
            conn.query_row("SELECT count(*) FROM sync_folder_outbox", [], |r| r.get(0))?;
        let status = conn
            .query_row(
                "SELECT source_id, last_sync_ms, last_error, devices, read_only, skipped, auto
                 FROM sync_folder WHERE id = 1",
                [],
                |r| {
                    Ok(FolderSyncStatus {
                        mode: if r.get(6)? {
                            FolderSyncMode::Auto
                        } else {
                            FolderSyncMode::Manual
                        },
                        source_id: Some(r.get(0)?),
                        last_sync_ms: r.get(1)?,
                        last_error: r.get(2)?,
                        devices: r.get(3)?,
                        read_only: r.get(4)?,
                        pending: 0,
                        skipped: r.get::<_, i64>(5)?.unsigned_abs(),
                    })
                },
            )
            .optional()?;
        drop(conn);
        Ok(match status {
            Some(s) => FolderSyncStatus {
                pending: pending.unsigned_abs(),
                ..s
            },
            None => FolderSyncStatus {
                mode: if self.folder_off()? {
                    FolderSyncMode::Off
                } else {
                    FolderSyncMode::Auto
                },
                ..FolderSyncStatus::default()
            },
        })
    }

    /// The user picks the source whose folder syncs this library (`None`:
    /// folder sync off). The choice stays: the automatic pick (SYNC-015)
    /// leaves it alone until [`Catalog::set_folder_sync_auto`]; only if the
    /// picked source is removed does the folder pick itself again. Turns
    /// sync of this library on as `device_id` (the sync service's id) if
    /// it is not yet, and queues the changes not sent anywhere yet for the
    /// folder. Another source starts reading the new folder from scratch.
    /// Turning it off keeps library sync on (the LAN may use it) and
    /// forgets what was not written.
    pub fn set_folder_sync(
        &self,
        source_id: Option<i64>,
        device_id: &str,
    ) -> Result<FolderSyncStatus, Error> {
        let _serial = self.folder_lock();
        let Some(source_id) = source_id else {
            let conn = self.lock();
            conn.execute("DELETE FROM sync_folder", [])?;
            conn.execute("DELETE FROM sync_folder_outbox", [])?;
            conn.execute(
                "INSERT INTO sync_folder_choice (id, mode) VALUES (1, 'off')
                 ON CONFLICT (id) DO UPDATE SET mode = 'off'",
                [],
            )?;
            drop(conn);
            return self.folder_sync_status();
        };
        self.select_folder(source_id, device_id, false)?;
        self.lock().execute(
            "INSERT INTO sync_folder_choice (id, mode) VALUES (1, 'auto')
             ON CONFLICT (id) DO UPDATE SET mode = 'auto'",
            [],
        )?;
        self.folder_sync_status()
    }

    /// Lets the core pick the folder again (SYNC-015) and forgets the
    /// user's choice. A folder the user picked that the automatic rules
    /// could pick too stays, now as the automatic one (nothing is read
    /// again); any other is left. The next round
    /// ([`Catalog::run_folder_sync`]) looks around.
    pub fn set_folder_sync_auto(&self) -> Result<FolderSyncStatus, Error> {
        let _serial = self.folder_lock();
        let keep = match self.folder_row()? {
            Some(row) if !row.auto => self
                .auto_sources()?
                .iter()
                .any(|s| s.id == row.source_id),
            _ => true,
        };
        {
            let conn = self.lock();
            if keep {
                conn.execute("UPDATE sync_folder SET auto = 1", [])?;
            } else {
                conn.execute("DELETE FROM sync_folder", [])?;
                conn.execute("DELETE FROM sync_folder_outbox", [])?;
            }
            conn.execute(
                "INSERT INTO sync_folder_choice (id, mode) VALUES (1, 'auto')
                 ON CONFLICT (id) DO UPDATE SET
                     mode = 'auto', evaluated_ms = NULL, sources = ''",
                [],
            )?;
        }
        self.folder_sync_status()
    }

    /// Makes `source_id` the folder (`auto`: picked by the core). The
    /// same source keeps its cursors; another starts from scratch.
    /// Callers hold [`Self::folder_lock`].
    pub(crate) fn select_folder(
        &self,
        source_id: i64,
        device_id: &str,
        auto: bool,
    ) -> Result<(), Error> {
        if !pa_sync::hlc::is_valid_device_id(device_id) {
            return Err(Error::Invalid("device id".into()));
        }
        {
            let conn = self.lock();
            if crate::sources::get(&conn, source_id)?.is_none() {
                return Err(Error::NotFound(format!("source {source_id}")));
            }
            let same: bool = conn
                .query_row(
                    "SELECT source_id = ?1 FROM sync_folder WHERE id = 1",
                    [source_id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if same {
                conn.execute("UPDATE sync_folder SET auto = ?1", [auto])?;
            } else {
                conn.execute(
                    "INSERT INTO sync_folder (id, source_id, enabled_at_ms, auto)
                     VALUES (1, ?1, ?2, ?3)
                     ON CONFLICT (id) DO UPDATE SET
                         source_id = excluded.source_id, cursors = '{}', snapshot_size = 0,
                         enabled_at_ms = excluded.enabled_at_ms, last_sync_ms = NULL,
                         last_error = NULL, devices = 0, read_only = 0, skipped = 0,
                         auto = excluded.auto, failing_since_ms = NULL",
                    params![source_id, now_ms(), auto],
                )?;
            }
        }
        // Ops recorded from here on reach `sync_folder_outbox` by the
        // trigger; the ones still queued for the LAN are copied.
        self.enable_sync(device_id)?;
        self.lock().execute(
            "INSERT OR IGNORE INTO sync_folder_outbox (op_id, payload)
             SELECT op_id, payload FROM sync_outbox ORDER BY seq",
            [],
        )?;
        Ok(())
    }

    /// Queues `ops` (own ops read back from the folder left behind) for
    /// the folder; an op already queued is not queued twice.
    pub(crate) fn queue_for_folder(&self, ops: &[Op]) -> Result<(), Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO sync_folder_outbox (op_id, payload) VALUES (?1, ?2)",
        )?;
        for op in ops {
            let payload = outbox::encode(op).map_err(|e| Error::Sync(e.to_string()))?;
            stmt.execute(params![op.op_id.to_string(), payload])?;
        }
        Ok(())
    }

    pub(crate) fn folder_lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.folder_sync.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One round of folder sync as `device_id`: without a choice of the
    /// user first checks which folder (`folder_auto`), then
    /// writes the own changes, reads and applies the others', compacts the
    /// own journal when it grew. A folder that cannot be reached is not an
    /// error: the status says why and the next round tries again. Does
    /// nothing while folder sync is off or no folder was found.
    pub fn run_folder_sync(&self, device_id: &str) -> Result<FolderSyncRun, Error> {
        let _serial = self.folder_lock();
        self.auto_pick(device_id)?;
        let Some(row) = self.folder_row()? else {
            return Ok(FolderSyncRun {
                status: self.folder_sync_status()?,
                ..FolderSyncRun::default()
            });
        };
        self.enable_sync(device_id)?;
        let vfs = match self.connect(row.source_id) {
            Ok(vfs) => vfs,
            Err(e) => {
                self.folder_failed(catalog_error_code(&e))?;
                return Ok(FolderSyncRun {
                    status: self.folder_sync_status()?,
                    ..FolderSyncRun::default()
                });
            }
        };
        // Own ops not written yet; an unreadable payload (a newer model
        // after a downgrade) is dropped.
        let (pending, last_seq) = {
            let conn = self.lock();
            let mut stmt =
                conn.prepare("SELECT seq, payload FROM sync_folder_outbox ORDER BY seq LIMIT ?1")?;
            let rows = stmt.query_map([WRITE_BATCH], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut ops: Vec<Op> = Vec::new();
            let mut last = None;
            for row in rows {
                let (seq, payload) = row?;
                last = Some(seq);
                if let Ok(op) = outbox::decode(&payload) {
                    ops.push(op);
                }
            }
            (ops, last)
        };
        let mut cursors: Cursors = serde_json::from_str(&row.cursors).unwrap_or_default();
        let now = now_ms();
        let now_u = u64::try_from(now).unwrap_or(0);
        let f = Folder::new(vfs.as_ref());
        let written = match f.write(device_id, &pending, now_u) {
            Ok(w) => w,
            Err(e) => {
                self.folder_failed(error_code(&e))?;
                return Ok(FolderSyncRun {
                    status: self.folder_sync_status()?,
                    ..FolderSyncRun::default()
                });
            }
        };
        if !written.read_only
            && let Some(last) = last_seq
        {
            self.lock()
                .execute("DELETE FROM sync_folder_outbox WHERE seq <= ?1", [last])?;
        }
        let report = f.read(device_id, &mut cursors, now_u, &mut |op| {
            self.apply_remote(op).map_err(|e| e.to_string())
        });
        let report = match report {
            Ok(r) => r,
            Err(e) => {
                self.save_cursors(&cursors)?;
                self.folder_failed(error_code(&e))?;
                return Ok(FolderSyncRun {
                    status: self.folder_sync_status()?,
                    written: written.ops,
                    ..FolderSyncRun::default()
                });
            }
        };
        let mut snapshot = row.snapshot_size.unsigned_abs();
        if !written.read_only && folder::should_compact(written.size, snapshot) {
            // A failed compaction leaves the journal as it was.
            if let Ok(Some(size)) = f.compact(device_id, now_u) {
                snapshot = size;
            }
        }
        {
            let conn = self.lock();
            conn.execute(
                "UPDATE sync_folder SET cursors = ?1, snapshot_size = ?2, last_sync_ms = ?3,
                     last_error = NULL, devices = ?4, read_only = ?5, skipped = ?6,
                     failing_since_ms = NULL
                 WHERE id = 1",
                params![
                    serde_json::to_string(&cursors).map_err(|e| Error::Sync(e.to_string()))?,
                    signed(snapshot),
                    now_ms(),
                    i64::try_from(report.devices.len()).unwrap_or(i64::MAX),
                    written.read_only,
                    i64::try_from(report.skipped + report.newer).unwrap_or(i64::MAX),
                ],
            )?;
        }
        Ok(FolderSyncRun {
            status: self.folder_sync_status()?,
            written: written.ops,
            changed: report.changed,
        })
    }

    fn save_cursors(&self, cursors: &Cursors) -> Result<(), Error> {
        self.lock().execute(
            "UPDATE sync_folder SET cursors = ?1 WHERE id = 1",
            [serde_json::to_string(cursors).map_err(|e| Error::Sync(e.to_string()))?],
        )?;
        Ok(())
    }

    /// The round could not reach the folder: the status says why, and the
    /// first such round in a row is remembered (an automatic folder away
    /// for long gives way, SYNC-015).
    fn folder_failed(&self, code: &str) -> Result<(), Error> {
        self.lock().execute(
            "UPDATE sync_folder SET last_error = ?1,
                 failing_since_ms = ifnull(failing_since_ms, ?2)
             WHERE id = 1",
            params![code, now_ms()],
        )?;
        Ok(())
    }
}
