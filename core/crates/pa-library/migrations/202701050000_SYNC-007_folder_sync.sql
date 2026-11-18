-- SPDX-License-Identifier: MPL-2.0
--
-- SYNC-007: sync through a shared folder of a source (ADR-0129). Every
-- device appends its ops to `/.kinolune/state/<device_id>.jsonl` of the
-- source the user picked and reads the journals of the others
-- (`pa_sync::folder`).
--
-- sync_folder          the picked source; at most one row. `cursors` is the
--                      JSON of `pa_sync::folder::Cursors` (where this
--                      device stopped in each other journal);
--                      `snapshot_size` the size of the own journal after
--                      its last compaction; the rest is the status the
--                      app shows. Gone with its source.
-- sync_folder_outbox   own ops not in the folder yet. Filled by a trigger
--                      from `sync_outbox` while a folder is picked, so the
--                      LAN acks that empty `sync_outbox` do not take ops
--                      away from the folder.
--
-- Expand only (LIB-002).

CREATE TABLE sync_folder (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    source_id      INTEGER NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    cursors        TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(cursors)),
    snapshot_size  INTEGER NOT NULL DEFAULT 0 CHECK (snapshot_size >= 0),
    enabled_at_ms  INTEGER NOT NULL,
    last_sync_ms   INTEGER,
    last_error     TEXT,
    devices        INTEGER NOT NULL DEFAULT 0 CHECK (devices >= 0),
    read_only      INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1)),
    skipped        INTEGER NOT NULL DEFAULT 0 CHECK (skipped >= 0)
) STRICT;

CREATE INDEX sync_folder_source ON sync_folder (source_id);

CREATE TABLE sync_folder_outbox (
    seq      INTEGER PRIMARY KEY,
    op_id    TEXT    NOT NULL UNIQUE CHECK (length(op_id) = 36),
    -- pa_model::Versioned<pa_sync::Op> as JSON, as in sync_outbox.
    payload  TEXT    NOT NULL CHECK (json_valid(payload))
) STRICT;

CREATE TRIGGER sync_folder_outbox_fill AFTER INSERT ON sync_outbox
WHEN EXISTS (SELECT 1 FROM sync_folder)
BEGIN
    INSERT OR IGNORE INTO sync_folder_outbox (op_id, payload)
    VALUES (NEW.op_id, NEW.payload);
END;
