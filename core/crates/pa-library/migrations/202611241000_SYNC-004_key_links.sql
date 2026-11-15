-- SPDX-License-Identifier: MPL-2.0
--
-- SYNC-004: matching the same media across devices (ADR-NNNN).
-- Expand-only.
--
-- sync_key_links  keys another device says belong to one item: an op or a
--                 `pb.*` message carries its `key` and the item's other keys
--                 (`aliases`: provider ids, `file:` keys of its versions,
--                 `ed2k:`). Progress stays under the key it was written with
--                 (journals and old peers keep working); this device reads
--                 an item's progress over its own keys and the keys linked
--                 to them. Facts about content, not per profile.

CREATE TABLE sync_key_links (
    media_key     TEXT    NOT NULL CHECK (media_key <> ''),
    alias         TEXT    NOT NULL CHECK (alias <> '' AND alias <> media_key),
    learned_at_ms INTEGER NOT NULL,
    PRIMARY KEY (media_key, alias)
) STRICT, WITHOUT ROWID;

CREATE INDEX sync_key_links_alias ON sync_key_links (alias);
