-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-022: the user's own categories, tags and star ratings of items.
-- Expand-only. All three are per profile (ADR-NNNN): one person's labels
-- and opinions are not another's, and a supervised profile never sees the
-- names another profile gave to items it may not see.
--
-- `space` is the library the labels belong to: '' is the main library;
-- the protected library of UI-025 uses its own spaces, so its categories
-- and tags never show up in the main library's lists and autocomplete.
--
-- Items are referenced as in LIB-010 lists: a work (`works.id`, a movie or
-- a whole show) or a playable item by its stable media key (an episode, a
-- track, an unrecognized video).
--
-- user_categories      named categories of a profile; an item can be in
--                      several. `sync_id` (random, 32 hex digits) is the
--                      id other devices know it by (SYNC, follow-up).
--                      `name_key` is the normalized name (trimmed, single
--                      spaces, lower case): names are unique per profile
--                      and space regardless of case.
-- user_category_items  items of a category.
-- user_tags            tags of a profile; a tag exists while an item has
--                      it. Tags are known to other devices by `name_key`.
-- user_tag_items       tags of items.
-- user_ratings         1–5 stars of an item by a profile.

CREATE TABLE user_categories (
    id            INTEGER PRIMARY KEY,
    profile_id    INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    space         TEXT    NOT NULL DEFAULT ''
                  CHECK (length(space) <= 64 AND space NOT GLOB '*[^a-z0-9_:-]*'),
    sync_id       TEXT    NOT NULL UNIQUE
                  CHECK (length(sync_id) BETWEEN 1 AND 64 AND sync_id NOT GLOB '*[^0-9a-z-]*'),
    name          TEXT    NOT NULL CHECK (name <> '' AND length(name) <= 100),
    name_key      TEXT    NOT NULL CHECK (name_key <> ''),
    position      INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (profile_id, space, name_key)
) STRICT;

CREATE TABLE user_category_items (
    category_id INTEGER NOT NULL REFERENCES user_categories (id) ON DELETE CASCADE,
    work_id     INTEGER REFERENCES works (id) ON DELETE CASCADE,
    media_key   TEXT    CHECK (length(media_key) BETWEEN 3 AND 256 AND media_key GLOB '?*:?*'),
    added_at_ms INTEGER NOT NULL,
    CHECK ((media_key IS NULL) <> (work_id IS NULL))
) STRICT;

CREATE UNIQUE INDEX user_category_items_work ON user_category_items (category_id, work_id)
    WHERE work_id IS NOT NULL;
CREATE UNIQUE INDEX user_category_items_media ON user_category_items (category_id, media_key)
    WHERE media_key IS NOT NULL;
CREATE INDEX user_category_items_work_ref ON user_category_items (work_id) WHERE work_id IS NOT NULL;
CREATE INDEX user_category_items_media_ref ON user_category_items (media_key)
    WHERE media_key IS NOT NULL;

CREATE TABLE user_tags (
    id            INTEGER PRIMARY KEY,
    profile_id    INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    space         TEXT    NOT NULL DEFAULT ''
                  CHECK (length(space) <= 64 AND space NOT GLOB '*[^a-z0-9_:-]*'),
    name          TEXT    NOT NULL CHECK (name <> '' AND length(name) <= 64),
    name_key      TEXT    NOT NULL CHECK (name_key <> ''),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (profile_id, space, name_key)
) STRICT;

CREATE TABLE user_tag_items (
    tag_id      INTEGER NOT NULL REFERENCES user_tags (id) ON DELETE CASCADE,
    work_id     INTEGER REFERENCES works (id) ON DELETE CASCADE,
    media_key   TEXT    CHECK (length(media_key) BETWEEN 3 AND 256 AND media_key GLOB '?*:?*'),
    added_at_ms INTEGER NOT NULL,
    CHECK ((media_key IS NULL) <> (work_id IS NULL))
) STRICT;

CREATE UNIQUE INDEX user_tag_items_work ON user_tag_items (tag_id, work_id) WHERE work_id IS NOT NULL;
CREATE UNIQUE INDEX user_tag_items_media ON user_tag_items (tag_id, media_key)
    WHERE media_key IS NOT NULL;
CREATE INDEX user_tag_items_work_ref ON user_tag_items (work_id) WHERE work_id IS NOT NULL;
CREATE INDEX user_tag_items_media_ref ON user_tag_items (media_key) WHERE media_key IS NOT NULL;

CREATE TABLE user_ratings (
    profile_id    INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    work_id       INTEGER REFERENCES works (id) ON DELETE CASCADE,
    media_key     TEXT    CHECK (length(media_key) BETWEEN 3 AND 256 AND media_key GLOB '?*:?*'),
    stars         INTEGER NOT NULL CHECK (stars BETWEEN 1 AND 5),
    updated_at_ms INTEGER NOT NULL,
    CHECK ((media_key IS NULL) <> (work_id IS NULL))
) STRICT;

CREATE UNIQUE INDEX user_ratings_work ON user_ratings (profile_id, work_id) WHERE work_id IS NOT NULL;
CREATE UNIQUE INDEX user_ratings_media ON user_ratings (profile_id, media_key)
    WHERE media_key IS NOT NULL;
CREATE INDEX user_ratings_work_ref ON user_ratings (work_id) WHERE work_id IS NOT NULL;
