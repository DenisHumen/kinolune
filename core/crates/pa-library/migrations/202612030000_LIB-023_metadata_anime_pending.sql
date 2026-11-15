-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-023: the metadata run fetches titles and posters first (local anime
-- database, TMDB) for every work, then the rate-limited anime providers
-- (AniList, Shikimori) for anime works (ADR-NNNN). A work whose TMDB
-- metadata is written but whose anime enrichment has not run yet (the run
-- was stopped, AniList failed) keeps anime_pending = 1, so the next run
-- does it. Expand-only.

ALTER TABLE work_metadata ADD COLUMN anime_pending INTEGER NOT NULL DEFAULT 0
    CHECK (anime_pending IN (0, 1));
