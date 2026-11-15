-- SPDX-License-Identifier: MPL-2.0
--
-- NET-019: offline copies of library files on the device (pa-offline,
-- ADR-NNNN). Expand-only.
--
-- offline_copies  one downloaded library item (a movie, an episode, a
--                 track): the storage item (pa-storage, CORE-012) that holds
--                 its folder in the `offline` target, the item it belongs to
--                 and the version that was copied. The source file stays the
--                 canonical library file: the copy is linked to it, not a new
--                 item. Deleting the storage item deletes the copy.
-- offline_files   the source files the copy holds (video, the parts of a
--                 split movie, sidecar subtitles, local art), by
--                 (source_id, path) as in `files.path`. The file of a copy is
--                 `<target path>/<storage item rel_path>/<path without '/'>`.
--                 The stream opener serves it instead of the network file
--                 while the storage item is not in the trash.

CREATE TABLE offline_copies (
    id              INTEGER PRIMARY KEY,
    storage_item_id INTEGER NOT NULL UNIQUE REFERENCES storage_items (id) ON DELETE CASCADE,
    item_key        TEXT    NOT NULL CHECK (item_key <> ''),
    source_id       INTEGER NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    file_id         INTEGER REFERENCES files (id) ON DELETE SET NULL,
    title           TEXT    NOT NULL,
    version_label   TEXT,
    job_id          INTEGER,
    profile_id      INTEGER REFERENCES profiles (id) ON DELETE SET NULL,
    created_at_ms   INTEGER NOT NULL
) STRICT;

CREATE INDEX offline_copies_item ON offline_copies (item_key);

CREATE TABLE offline_files (
    copy_id    INTEGER NOT NULL REFERENCES offline_copies (id) ON DELETE CASCADE,
    source_id  INTEGER NOT NULL,
    path       TEXT    NOT NULL CHECK (substr(path, 1, 1) = '/'),
    role       TEXT    NOT NULL CHECK (role IN ('video', 'part', 'subtitle', 'art')),
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    PRIMARY KEY (copy_id, path)
) STRICT, WITHOUT ROWID;

CREATE INDEX offline_files_source_path ON offline_files (source_id, path);
