-- SPDX-License-Identifier: MPL-2.0
--
-- SYNC-009: profiles travel between paired devices as `profile.upsert` /
-- `profile.delete` ops (pa-sync `profiles`), the same way sources do
-- (SYNC-010).
--
-- profiles.sync_stamp   stamp `{hlc, op_id}` of the newest op applied or
--                       sent for the profile (last writer wins)
-- profiles.sync_seen    what was last sent (settings, PIN hash, key epoch):
--                       reconcile sends again only when it changed
-- profile_sync_tombstones  deleted profiles: `pending` = a delete still to
--                       send; `stamp` = the delete's stamp, so an older
--                       upsert cannot bring the profile back
--
-- Expand only (LIB-002).

ALTER TABLE profiles ADD COLUMN sync_stamp TEXT CHECK (json_valid(sync_stamp));
ALTER TABLE profiles ADD COLUMN sync_seen TEXT;

CREATE TABLE profile_sync_tombstones (
    sync_id TEXT    PRIMARY KEY CHECK (sync_id <> ''),
    stamp   TEXT    CHECK (json_valid(stamp)),
    pending INTEGER NOT NULL DEFAULT 1 CHECK (pending IN (0, 1))
) STRICT;

CREATE TRIGGER profiles_sync_tombstone AFTER DELETE ON profiles
WHEN old.sync_id <> 'default'
BEGIN
    INSERT INTO profile_sync_tombstones (sync_id, stamp, pending)
    VALUES (old.sync_id, NULL, 1)
    ON CONFLICT (sync_id) DO UPDATE SET stamp = NULL, pending = 1;
END;
