-- SPDX-License-Identifier: MPL-2.0
--
-- UI-033: playback positions of personal media (ADR-0098). Expand-only.
--
-- personal_progress  where a video of a personal library was left, per
--                    library. `media_key` is the item key of a video moved
--                    from the main library (`tmdb:…`, else `file:`) and
--                    the `file:` key (ed2k fingerprint) of any other file.
--                    Never in `progress`, never in the sync outbox:
--                    the main "Continue watching", Watch Next, Top Shelf
--                    and other devices do not see these rows. A library
--                    belongs to one profile, so other profiles do not
--                    either; deleting the library deletes its positions.

CREATE TABLE personal_progress (
    library_id    INTEGER NOT NULL REFERENCES personal_libraries (id) ON DELETE CASCADE,
    media_key     TEXT    NOT NULL CHECK (media_key <> ''),
    position_ms   INTEGER NOT NULL CHECK (position_ms >= 0),
    duration_ms   INTEGER CHECK (duration_ms IS NULL OR duration_ms > 0),
    watched       INTEGER NOT NULL DEFAULT 0 CHECK (watched IN (0, 1)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (library_id, media_key)
) WITHOUT ROWID;

CREATE INDEX personal_progress_recent ON personal_progress (library_id, updated_at_ms);
