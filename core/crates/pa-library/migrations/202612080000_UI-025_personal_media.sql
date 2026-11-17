-- SPDX-License-Identifier: MPL-2.0
--
-- UI-025: personal media ("Личные медиа"). Expand-only.
--
-- sources.content_type     what the user said a source holds: 'video'
--                          (movies and shows, recognized; the default and
--                          what every existing source was), 'music' (no
--                          video recognition) or 'personal' (own videos:
--                          never recognized, never sent to a provider,
--                          hidden from the main library).
-- personal_libraries       libraries of a profile in the "Личные медиа" tab.
--                          `lock_kind`: 'none', 'pin' (4–8 digits) or
--                          'password'; `lock_hash` is the Argon2id hash of
--                          the PIN or password (as profile PINs, ADR-0055),
--                          never the secret. `sync_id` (32 hex) is the id
--                          other devices would know it by (no sync yet).
-- personal_library_folders folders (VfsPath, '/' = the whole source) whose
--                          files belong to a library. A folder is in at
--                          most one library; the longest folder wins.
-- personal_library_files   files moved from the main library into a
--                          library. They keep their matches, so moving one
--                          back restores the item as it was.
--
-- A file of a personal library, or of a 'personal' source without one, is
-- not part of the main library: `file_visible!` (pa_library::profiles)
-- leaves it out unless the connection's `temp.personal_scope` names its
-- library, which the catalog sets only while the library is unlocked.

ALTER TABLE sources ADD COLUMN content_type TEXT NOT NULL DEFAULT 'video'
    CHECK (content_type IN ('video', 'music', 'personal'));

CREATE TABLE personal_libraries (
    id            INTEGER PRIMARY KEY,
    profile_id    INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    sync_id       TEXT    NOT NULL UNIQUE
                  CHECK (length(sync_id) = 32 AND sync_id NOT GLOB '*[^0-9a-f]*'),
    name          TEXT    NOT NULL CHECK (name <> '' AND length(name) <= 100),
    lock_kind     TEXT    NOT NULL DEFAULT 'none'
                  CHECK (lock_kind IN ('none', 'pin', 'password')),
    lock_hash     TEXT,
    position      INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    CHECK ((lock_kind = 'none') = (lock_hash IS NULL))
);

CREATE INDEX personal_libraries_profile ON personal_libraries (profile_id, position);

CREATE TABLE personal_library_folders (
    source_id   INTEGER NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    path        TEXT    NOT NULL CHECK (substr(path, 1, 1) = '/'),
    library_id  INTEGER NOT NULL REFERENCES personal_libraries (id) ON DELETE CASCADE,
    added_at_ms INTEGER NOT NULL,
    PRIMARY KEY (source_id, path)
) WITHOUT ROWID;

CREATE INDEX personal_library_folders_library ON personal_library_folders (library_id);

CREATE TABLE personal_library_files (
    file_id     INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    library_id  INTEGER NOT NULL REFERENCES personal_libraries (id) ON DELETE CASCADE,
    moved_at_ms INTEGER NOT NULL
);

CREATE INDEX personal_library_files_library ON personal_library_files (library_id);
