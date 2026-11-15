-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-015: progress of the pa_library v0 import that waits for its file's
-- fingerprint (ADR-NNNN). Expand-only.
--
-- import_v0_progress  progress of an unrecognized v0 item whose file could
--                     not be hashed during the import (an SMB share that was
--                     not mounted, a disk that was offline). Its media key is
--                     `file:<fingerprint>`, so the row waits here; the scan
--                     that fingerprints the file moves it into `progress`
--                     (newer state wins) and deletes it. Goes with the file.

CREATE TABLE import_v0_progress (
    file_id       INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    source_type   TEXT    NOT NULL CHECK (source_type IN ('movie', 'episode', 'track', 'video')),
    position_ms   INTEGER NOT NULL CHECK (position_ms >= 0),
    duration_ms   INTEGER CHECK (duration_ms >= 0),
    watched       INTEGER NOT NULL DEFAULT 0 CHECK (watched IN (0, 1)),
    updated_at_ms INTEGER NOT NULL
) STRICT;
