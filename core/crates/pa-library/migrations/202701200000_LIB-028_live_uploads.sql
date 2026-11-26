-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-028 (ADR-NNNN): files dropped into the library on one device show up
-- at once on every device as a grey card with a moving bar. The bar moves
-- on ephemeral `up` frames that never reach a database; what is durable is
-- the last step of every upload, synced as an `upload.event` op:
-- started, recognized, done, cancelled.
--
-- live_uploads.upload_id   the upload id (the `i` of `up` ticks)
-- live_uploads.phase       `started` | `recognized` | `done` | `cancelled`
--                          (a newer phase of another build is kept as text)
-- live_uploads.info        JSON object the apps read: file name, size,
--                          kind, the guess before copying, the source and
--                          folder, the sending device's name
-- live_uploads.device_id   the device that uploads (its HLC device)
-- live_uploads.stamp       JSON `{hlc, op_id}` of the step that won (last
--                          writer wins); NULL while sync is off
-- live_uploads.updated_ms  local time of the last change, for clean-up
--
-- Expand only (LIB-002).

CREATE TABLE live_uploads (
    upload_id   TEXT    PRIMARY KEY,
    phase       TEXT    NOT NULL,
    info        TEXT    NOT NULL,
    device_id   TEXT    NOT NULL,
    stamp       TEXT,
    updated_ms  INTEGER NOT NULL
) STRICT;
