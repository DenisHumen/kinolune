-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-004: file fingerprints (docs/dev/db.md "Отпечатки файлов").
-- Expand-only migration.
--
-- `files.oshash` and `files.ed2k` exist since LIB-001. This adds what the
-- background ed2k job and move detection need:
-- - `sources.ed2k_policy`: when the ed2k job reads the files of a source.
--   `auto` hashes local folders in background runs and network sources only
--   in runs the app starts when the device is idle on Wi-Fi (or on demand);
--   `always` hashes network sources in every background run too; `never`
--   hashes only on demand.
-- - `files.ed2k_failed_at_ms`: the last failed ed2k attempt, so a file that
--   cannot be read is retried a day later, not in every run.
-- - an index over missing files by OSHash: a new file with the fingerprint
--   of a missing one is the same file, moved or renamed.

ALTER TABLE sources ADD COLUMN ed2k_policy TEXT NOT NULL DEFAULT 'auto'
    CHECK (ed2k_policy IN ('auto', 'always', 'never'));

ALTER TABLE files ADD COLUMN ed2k_failed_at_ms INTEGER;

CREATE INDEX files_missing_oshash ON files (source_id, oshash, size_bytes)
    WHERE missing_since_ms IS NOT NULL AND oshash IS NOT NULL;
