-- SPDX-License-Identifier: MPL-2.0
--
-- SYNC-015: the sync folder picks itself (ADR-0129, "Автоматический выбор
-- папки"). Without a choice of the user the device joins the folder where
-- the other devices keep their journals, or makes one on the first
-- network source it can write; the user can still pick another folder or
-- turn it off, and that choice stays.
--
-- sync_folder.auto              1: the folder was picked automatically and
--                               may be replaced by the same rules; 0: the
--                               user picked it (rows of SYNC-007 builds
--                               were picked by hand, so they stay 0).
-- sync_folder.failing_since_ms  start of the current run of rounds that
--                               could not reach the folder; NULL after a
--                               round that reached it. An automatic folder
--                               away for long gives way to the next one.
-- sync_folder_choice            the user's choice when there is no row in
--                               sync_folder: `auto` (also when there is no
--                               row here) or `off`. `evaluated_ms` and
--                               `sources` (the ids of the sources that
--                               could carry a folder, comma-separated) tell
--                               when the automatic pick last looked around
--                               and what it saw, so a new source is looked
--                               at at once and the rest not every round.
--
-- Expand only (LIB-002).

ALTER TABLE sync_folder ADD COLUMN auto INTEGER NOT NULL DEFAULT 0 CHECK (auto IN (0, 1));
ALTER TABLE sync_folder ADD COLUMN failing_since_ms INTEGER;

CREATE TABLE sync_folder_choice (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    mode          TEXT    NOT NULL DEFAULT 'auto' CHECK (mode IN ('auto', 'off')),
    evaluated_ms  INTEGER,
    sources       TEXT    NOT NULL DEFAULT ''
) STRICT;
