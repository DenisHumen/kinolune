-- SPDX-License-Identifier: MPL-2.0
--
-- SYNC-010: sync of the source list between paired devices (ADR-NNNN).
-- Expand-only.
--
-- sources.sync_id     stable id of the source in sync ops (`source.upsert`,
--                     `source.delete`); the row id differs per device.
-- sources.sync_mode   per-source opt-in: 0 not synced, 1 settings without
--                     the password, 2 settings and the password in the E2E
--                     container (pa-sync `seal`). Network sources only.
-- sources.sync_stamp  JSON `{"hlc":…,"op_id":…}` of the newest op recorded
--                     or applied for the source: last writer wins by HLC.
-- sources.sync_seen   what that op carried, as this device compares it
--                     (mode, settings, the local key of the password, the
--                     family key epoch). A different value means a local
--                     change not sent yet. Holds no secret.
--
-- source_sync_tombstones  deleted synced sources: a late, older upsert does
--                         not bring them back. `pending = 1`: deleted here,
--                         the delete op is not recorded yet (`stamp` NULL).

ALTER TABLE sources ADD COLUMN sync_id TEXT CHECK (sync_id <> '');
ALTER TABLE sources ADD COLUMN sync_mode INTEGER NOT NULL DEFAULT 0
    CHECK (sync_mode IN (0, 1, 2));
ALTER TABLE sources ADD COLUMN sync_stamp TEXT CHECK (json_valid(sync_stamp));
ALTER TABLE sources ADD COLUMN sync_seen TEXT;

UPDATE sources SET sync_id = lower(hex(randomblob(16))) WHERE sync_id IS NULL;

CREATE UNIQUE INDEX sources_sync_id ON sources (sync_id) WHERE sync_id IS NOT NULL;

CREATE TABLE source_sync_tombstones (
    sync_id TEXT    PRIMARY KEY CHECK (sync_id <> ''),
    stamp   TEXT    CHECK (json_valid(stamp)),
    pending INTEGER NOT NULL DEFAULT 1 CHECK (pending IN (0, 1))
) STRICT;

CREATE TRIGGER sources_sync_tombstone AFTER DELETE ON sources
WHEN old.sync_mode > 0 AND old.sync_id IS NOT NULL
BEGIN
    INSERT INTO source_sync_tombstones (sync_id, stamp, pending)
    VALUES (old.sync_id, NULL, 1)
    ON CONFLICT (sync_id) DO UPDATE SET stamp = NULL, pending = 1;
END;
