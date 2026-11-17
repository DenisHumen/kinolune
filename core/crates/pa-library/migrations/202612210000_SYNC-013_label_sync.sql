-- SPDX-License-Identifier: MPL-2.0
--
-- SYNC-013: user categories, tags and ratings (LIB-022) travel between
-- paired devices as `category.upsert`, `category.delete`, `category.item`,
-- `tag.set` and `rating.set` ops (pa-catalog `label_sync`, ADR-NNNN).
-- Only the main library's labels sync; personal libraries (UI-025) never.
--
-- label_sync   one last-writer-wins register of a profile's labels per
--              row, keyed by the op's register (`pa_sync::OpBody::register`):
--              `op` is the newest op of it, sent or received (its stamp
--              orders later ones; a delete or an `off` op is the tombstone).
--              `state`:
--                'applied'  the library holds what `op` says
--                'parked'   `op` is about a profile, category or item this
--                           device does not have yet; retried when labels
--                           sync again
--                'merged'   a category merged into the one with the same
--                           name (`merged_into`, a category sync id): ops
--                           about it land there; `op` may be empty
--              `profile` is `profiles.sync_id`.
--
-- Expand only (LIB-002).

CREATE TABLE label_sync (
    register    TEXT NOT NULL PRIMARY KEY CHECK (register <> ''),
    profile     TEXT NOT NULL CHECK (profile <> ''),
    op          TEXT CHECK (op IS NULL OR json_valid(op)),
    state       TEXT NOT NULL DEFAULT 'applied'
                CHECK (state IN ('applied', 'parked', 'merged')),
    merged_into TEXT,
    CHECK ((state = 'merged') = (merged_into IS NOT NULL)),
    CHECK (state = 'merged' OR op IS NOT NULL)
) STRICT;

CREATE INDEX label_sync_parked ON label_sync (state) WHERE state = 'parked';
