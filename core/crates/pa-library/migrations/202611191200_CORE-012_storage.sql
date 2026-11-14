-- SPDX-License-Identifier: MPL-2.0
--
-- CORE-012: storage targets, the files the core keeps in them and retention
-- rules (pa-storage, 04 §1.10). Expand-only.
--
-- storage_targets    where jobs write: a folder or a mounted share, its
--                    kind, a quota (bytes and/or percent of the volume, the
--                    smaller wins) and a free-space reserve (bytes and/or
--                    percent, the larger wins). `jobs.target_storage` holds
--                    the id (a soft reference: jobs keep their history after
--                    a target is removed).
-- storage_items      files the core put into a target (downloads,
--                    recordings, offline copies): path relative to the
--                    target, retention group (channel or rule), size,
--                    favourite and watched marks. A trashed item keeps its
--                    row with `trashed_at_ms` and `trash_path` for 7 days.
-- storage_retention  rules per target and group (NULL group: the default for
--                    every group of the target): keep the last N, delete
--                    after X days, delete watched after Y days, keep
--                    favourites.

CREATE TABLE storage_targets (
    id              TEXT    PRIMARY KEY CHECK (id <> '' AND id NOT GLOB '*[^a-z0-9_-]*'),
    kind            TEXT    NOT NULL
                    CHECK (kind IN ('library', 'downloads', 'recordings', 'transcode_cache')),
    path            TEXT    NOT NULL CHECK (path <> ''),
    quota_bytes     INTEGER CHECK (quota_bytes > 0),
    quota_percent   REAL    CHECK (quota_percent > 0 AND quota_percent <= 100),
    reserve_bytes   INTEGER NOT NULL DEFAULT 0 CHECK (reserve_bytes >= 0),
    reserve_percent REAL    NOT NULL DEFAULT 0 CHECK (reserve_percent >= 0 AND reserve_percent < 100),
    created_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL
) STRICT;

CREATE TABLE storage_items (
    id            INTEGER PRIMARY KEY,
    target_id     TEXT    NOT NULL REFERENCES storage_targets (id) ON DELETE CASCADE,
    rel_path      TEXT    NOT NULL CHECK (rel_path <> '' AND substr(rel_path, 1, 1) <> '/'),
    group_key     TEXT    CHECK (group_key <> ''),
    size_bytes    INTEGER NOT NULL CHECK (size_bytes >= 0),
    profile_id    INTEGER REFERENCES profiles (id) ON DELETE SET NULL,
    favorite      INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    watched_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    trashed_at_ms INTEGER,
    trash_path    TEXT,
    CHECK ((trashed_at_ms IS NULL) = (trash_path IS NULL))
) STRICT;

CREATE INDEX storage_items_group ON storage_items (target_id, group_key, created_at_ms);
CREATE UNIQUE INDEX storage_items_live_path ON storage_items (target_id, rel_path)
    WHERE trashed_at_ms IS NULL;
CREATE INDEX storage_items_trash ON storage_items (trashed_at_ms) WHERE trashed_at_ms IS NOT NULL;
CREATE INDEX storage_items_profile ON storage_items (profile_id) WHERE profile_id IS NOT NULL;

CREATE TABLE storage_retention (
    id                 INTEGER PRIMARY KEY,
    target_id          TEXT    NOT NULL REFERENCES storage_targets (id) ON DELETE CASCADE,
    group_key          TEXT    CHECK (group_key <> ''),
    keep_last          INTEGER CHECK (keep_last >= 0),
    max_age_days       INTEGER CHECK (max_age_days > 0),
    watched_after_days INTEGER CHECK (watched_after_days >= 0),
    keep_favorites     INTEGER NOT NULL DEFAULT 1 CHECK (keep_favorites IN (0, 1)),
    updated_at_ms      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX storage_retention_scope ON storage_retention (target_id, coalesce(group_key, ''));
