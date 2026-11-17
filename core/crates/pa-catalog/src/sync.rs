// SPDX-License-Identifier: MPL-2.0
//! Sync of progress and "watched" between paired devices (UI-019), of
//! sources (SYNC-010), profiles (SYNC-009) and labels (SYNC-013).
//!
//! The catalog does not talk to the network: a transport (the LAN node of
//! `pa-sync`) takes [`Catalog::pending_ops`], hands back acknowledgements
//! ([`Catalog::ack_op`]) and the ops of other devices
//! ([`Catalog::apply_remote`]).

use std::sync::{Arc, MutexGuard};

use pa_library::progress as lib_progress;
use pa_library::rusqlite::OptionalExtension;
use pa_model::MediaKey;
use pa_secrets::SecretStore;
use pa_sync::handoff::ResumePoint;
use pa_sync::matching::ItemKeys;
use pa_sync::seal::SealKey;
use pa_sync::{Op, OpBody, OpId, Replica, outbox};

use crate::items::ItemRef;
use crate::progress::{item_of_key, key_of, local_duration, profile};
use crate::{Catalog, Error, now_ms};

/// What the source list sync needs besides the replica (SYNC-010).
#[derive(Debug, Default)]
pub(crate) struct SourceSyncCtx {
    /// Where source passwords live (the source manager's store).
    secrets: Option<Arc<dyn SecretStore>>,
    /// The family's key for the E2E password container.
    seal: Option<SealKey>,
}

fn is_profile_op(body: &OpBody) -> bool {
    matches!(
        body,
        OpBody::ProfileUpsert { .. } | OpBody::ProfileDelete { .. }
    )
}

fn is_source_op(body: &OpBody) -> bool {
    matches!(
        body,
        OpBody::SourceUpsert { .. } | OpBody::SourceDelete { .. }
    )
}

impl Catalog {
    pub(crate) fn sync_lock(&self) -> MutexGuard<'_, Option<Replica>> {
        self.sync.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn source_sync_lock(&self) -> MutexGuard<'_, SourceSyncCtx> {
        self.source_sync.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The secret store of source passwords (the one the source manager
    /// uses): received passwords go there, synced ones are read from it.
    pub fn set_source_secrets(&self, secrets: Arc<dyn SecretStore>) {
        self.source_sync_lock().secrets = Some(secrets);
    }

    /// The family's key for sealing source passwords (SYNC-010), `None`
    /// when the device has no family. A new key (another epoch) seals the
    /// synced passwords again.
    pub fn set_seal_key(&self, key: Option<SealKey>) -> Result<(), Error> {
        let changed = {
            let mut ctx = self.source_sync_lock();
            let changed = ctx.seal != key;
            ctx.seal = key;
            changed
        };
        if changed {
            self.sync_sources()?;
            self.sync_profiles()?;
        }
        Ok(())
    }

    /// Records ops for the label changes of the main library (categories,
    /// their items, tags, ratings; SYNC-013) not sent yet, and writes the
    /// parked label ops of other devices whose profile, category or item is
    /// here now. Every label change of the main library calls it; so does
    /// sync start. Returns the number of ops queued; 0 while sync is off.
    pub fn sync_labels(&self) -> Result<usize, Error> {
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(0);
        };
        let tx = conn.transaction()?;
        let now = now_ms();
        let n = crate::label_sync::reconcile(&tx, &mut |c, body, aliases| {
            Ok(replica.record_aliased(c, body, aliases, now)?)
        })?;
        tx.commit()?;
        Ok(n)
    }

    /// Writes a label op of another device (SYNC-013).
    fn apply_remote_label(&self, op: &Op) -> Result<bool, Error> {
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(false);
        };
        // Orders this device's next ops after it; the durable last-writer-
        // wins check is against the registers in `label_sync`.
        replica.receive(op, now_ms())?;
        let tx = conn.transaction()?;
        let changed = crate::label_sync::apply_remote(&tx, op)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Records ops for local changes of synced sources (new, edited,
    /// deleted, sync switched on) that were not sent yet. Call it after
    /// the source manager changed a source; sync start calls it too.
    /// Returns the number of ops queued; 0 while sync is off.
    pub fn sync_sources(&self) -> Result<usize, Error> {
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(0);
        };
        let ctx = self.source_sync_lock();
        let tx = conn.transaction()?;
        let now = now_ms();
        let n = pa_sources::sync::reconcile(
            &tx,
            ctx.secrets.as_deref(),
            ctx.seal.as_ref(),
            &mut |c, body| replica.record(c, body, now).map(|op| (op.hlc, op.op_id)),
        )
        .map_err(|e| Error::Sync(e.to_string()))?;
        tx.commit()?;
        Ok(n)
    }

    /// Records ops for local profile changes (new, edited, PIN set or
    /// removed, deleted) not sent yet (SYNC-009); with the family key the
    /// PIN hash goes sealed. Returns the number of ops; 0 while sync is off.
    pub fn sync_profiles(&self) -> Result<usize, Error> {
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(0);
        };
        let ctx = self.source_sync_lock();
        let tx = conn.transaction()?;
        let now = now_ms();
        let n = pa_sync::profiles::reconcile(&tx, ctx.seal.as_ref(), &mut |c, body| {
            replica.record(c, body, now).map(|op| (op.hlc, op.op_id))
        })?;
        tx.commit()?;
        Ok(n)
    }

    /// Writes a `profile.*` op of another device.
    fn apply_remote_profile(&self, op: &Op) -> Result<bool, Error> {
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(false);
        };
        replica.receive(op, now_ms())?;
        let ctx = self.source_sync_lock();
        let tx = conn.transaction()?;
        let applied = pa_sync::profiles::apply_remote(&tx, ctx.seal.as_ref(), op, now_ms())?;
        if applied == pa_sync::profiles::Applied::Changed {
            // Labels of this profile that came before it.
            crate::label_sync::retry_parked(&tx)?;
        }
        tx.commit()?;
        Ok(applied != pa_sync::profiles::Applied::Unchanged)
    }

    /// Writes a `source.*` op of another device (with its password, if
    /// sealed for this family, into the secret store only).
    fn apply_remote_source(&self, op: &Op) -> Result<bool, Error> {
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(false);
        };
        // Orders this device's next ops after it; the durable last-writer-
        // wins check is against the stamps in `sources`.
        replica.receive(op, now_ms())?;
        let ctx = self.source_sync_lock();
        let tx = conn.transaction()?;
        let applied =
            pa_sources::sync::apply_remote(&tx, ctx.secrets.as_deref(), ctx.seal.as_ref(), op)
                .map_err(|e| Error::Sync(e.to_string()))?;
        if applied == pa_sources::sync::Applied::Deleted {
            crate::recognize::remove_orphans(&tx)?;
            pa_library::music::remove_orphans(&tx)?;
        }
        tx.commit()?;
        if applied == pa_sources::sync::Applied::Deleted {
            pa_library::search::sync(&conn)?;
        }
        Ok(applied != pa_sources::sync::Applied::Unchanged)
    }

    /// Turns sync on for this device: from now on progress changes are
    /// recorded as ops of `device_id` in the outbox. The clock continues
    /// after the newest queued op. Calling it again with the same id keeps
    /// the replica.
    pub fn enable_sync(&self, device_id: &str) -> Result<(), Error> {
        let conn = self.lock();
        let mut sync = self.sync_lock();
        if sync.as_ref().is_some_and(|r| r.device_id() == device_id) {
            return Ok(());
        }
        let mut replica = Replica::new(device_id)?;
        if let Some(hlc) = outbox::newest_hlc(&conn)? {
            let now = u64::try_from(now_ms()).unwrap_or(0);
            // A stamp from a clock far ahead is ignored, not fatal.
            let _ = replica.clock_mut().observe(&hlc, now);
        }
        outbox::compact(&conn)?;
        *sync = Some(replica);
        drop(sync);
        drop(conn);
        // Sources, profiles and labels added or changed while unpaired go
        // out now.
        self.sync_sources()?;
        self.sync_profiles()?;
        self.sync_labels()?;
        self.sync_folder_rules()?;
        Ok(())
    }

    /// Turns sync off (the device left its family) and drops the outbox:
    /// nobody is left to send it to.
    pub fn disable_sync(&self) -> Result<(), Error> {
        let conn = self.lock();
        *self.sync_lock() = None;
        conn.execute("DELETE FROM sync_outbox", [])?;
        pa_sources::sync::reset(&conn).map_err(|e| Error::Sync(e.to_string()))?;
        pa_sync::profiles::reset(&conn)?;
        crate::label_sync::reset(&conn)?;
        // Folder rules (META-017) go out again after the next pairing.
        conn.execute("DELETE FROM folder_rule_sync", [])?;
        Ok(())
    }

    /// Device id of the sync replica, if sync is on.
    pub fn sync_device_id(&self) -> Option<String> {
        self.sync_lock().as_ref().map(|r| r.device_id().to_owned())
    }

    /// Queued local ops, oldest first, after dropping the ones a newer op of
    /// the same register replaced.
    pub fn pending_ops(&self, limit: u32) -> Result<Vec<Op>, Error> {
        let conn = self.lock();
        outbox::compact(&conn)?;
        Ok(outbox::due(&conn, i64::MAX, limit)?
            .into_iter()
            .map(|p| p.op)
            .collect())
    }

    /// Removes an op another device holds now. `false` if it was not queued.
    pub fn ack_op(&self, op_id: OpId) -> Result<bool, Error> {
        let conn = self.lock();
        Ok(outbox::ack(&conn, op_id)?)
    }

    /// Writes an op of another device into the library. Returns whether
    /// anything changed. Without sync on, nothing is applied.
    pub fn apply_remote(&self, op: &Op) -> Result<bool, Error> {
        if is_source_op(&op.body) {
            return self.apply_remote_source(op);
        }
        if is_profile_op(&op.body) {
            return self.apply_remote_profile(op);
        }
        if crate::label_sync::is_label_op(&op.body) {
            return self.apply_remote_label(op);
        }
        if matches!(op.body, OpBody::FolderRule { .. }) {
            return self.apply_remote_folder_rule(op);
        }
        let conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(false);
        };
        Ok(replica.receive_into(&conn, op, now_ms())?)
    }

    /// Sync id of the profile progress is stored for (`profiles.sync_id`,
    /// the `profile` of ops and `pb.*` messages).
    pub fn sync_profile(&self) -> Result<String, Error> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT sync_id FROM profiles WHERE id = ?1",
                [profile(&conn)?],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| "default".to_owned()))
    }

    /// The media key progress of `item` is synced under (SYNC-003: the key
    /// of `pb.update`); `None` until its file is fingerprinted.
    pub fn sync_key(&self, item: ItemRef) -> Result<Option<MediaKey>, Error> {
        let conn = self.lock();
        Ok(key_of(&conn, item)?.map(|(key, _)| key))
    }

    /// Every key of `item` (SYNC-004): the one ops and `pb.*` messages are
    /// about ([`sync_key`](Self::sync_key)) and the others as aliases —
    /// provider ids, `file:` and `ed2k:` keys of its versions.
    pub fn sync_keys(&self, item: ItemRef) -> Result<Option<ItemKeys>, Error> {
        let conn = self.lock();
        crate::matching::item_keys(&conn, item)
    }

    /// The item of this library a synced key belongs to, if any: by the key,
    /// else by the keys other devices linked to it (SYNC-004).
    pub fn item_for_key(&self, key: &MediaKey) -> Result<Option<ItemRef>, Error> {
        let conn = self.lock();
        item_of_key(&conn, key)
    }

    /// The item a message about `key` with `aliases` (`pb.*`, SYNC-004) is
    /// about here: the key, its aliases by priority, then linked keys.
    /// Aliases of kinds this build does not know are skipped.
    pub fn item_for_keys(
        &self,
        key: &MediaKey,
        aliases: &[String],
    ) -> Result<Option<ItemRef>, Error> {
        let conn = self.lock();
        crate::matching::resolve(&conn, key, &pa_sync::matching::parse_aliases(aliases))
    }

    /// Duration of the file `item` plays here (its default version), if
    /// known: a point of another version is carried onto it with
    /// [`ResumePoint::for_version`] (SYNC-004).
    pub fn local_duration_ms(&self, item: ItemRef) -> Result<Option<u64>, Error> {
        let conn = self.lock();
        local_duration(&conn, item)
    }

    /// Stored progress of `item` as a resume point (SYNC-003): the last
    /// change on any device under any of its keys (SYNC-004), stamped with
    /// its time and device, carried onto this device's version.
    pub fn resume_point(&self, item: ItemRef) -> Result<Option<ResumePoint>, Error> {
        let conn = self.lock();
        crate::profiles::require_visible(&conn, item)?;
        let Some(p) = crate::matching::merged(&conn, profile(&conn)?, item)? else {
            return Ok(None);
        };
        if p.watched {
            return Ok(None);
        }
        Ok(Some(
            ResumePoint::from_progress(&p).for_version(local_duration(&conn, item)?),
        ))
    }

    /// Stored progress under a media key that is not a catalog item (a
    /// Twitch VOD, TWI-005) as a resume point.
    pub fn resume_point_for_key(&self, key: &MediaKey) -> Result<Option<ResumePoint>, Error> {
        let conn = self.lock();
        Ok(lib_progress::get(&conn, profile(&conn)?, key)?
            .filter(|p| !p.watched)
            .map(|p| ResumePoint::from_progress(&p)))
    }
}
