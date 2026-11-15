-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-023: fast recognition of large network libraries (ADR-NNNN).
-- Expand-only.
--
-- work_metadata.anime_pending  the metadata run fetches titles and posters
--                     first (local anime database, TMDB) for every work,
--                     then the rate-limited anime providers (AniList,
--                     Shikimori) for anime works. A work whose TMDB data is
--                     written but whose anime stage has not run yet (the
--                     run was stopped, AniList failed) keeps 1, so the next
--                     run does it.

ALTER TABLE work_metadata ADD COLUMN anime_pending INTEGER NOT NULL DEFAULT 0
    CHECK (anime_pending IN (0, 1));

-- file_ed2k_progress  where the ed2k job (LIB-004) stopped in a file: the
--                     MD4 of every whole 9 500 KiB chunk read so far
--                     (16 bytes each, in order). The next run goes on from
--                     there if the file has the same size and mtime; a
--                     60 GB remux on a share is hashed over many idle
--                     periods instead of starting over. Goes with the file.
CREATE TABLE file_ed2k_progress (
    file_id       INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    size_bytes    INTEGER NOT NULL,
    mtime_ms      INTEGER,
    chunk_md4     BLOB    NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;
