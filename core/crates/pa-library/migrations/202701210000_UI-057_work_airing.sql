-- SPDX-License-Identifier: MPL-2.0
-- UI-057: when the next episode of an ongoing show or anime airs.
--
-- One row per show work, the last answer of TMDB (`next_episode_to_air`,
-- `status`), AniList (`nextAiringEpisode`) or Shikimori (`next_episode_at`).
-- A cache of the device: refreshed when the show page opens and once a day
-- in the background; never synced. Offline the last value stays and
-- `failed_at_ms` marks it as old.

CREATE TABLE work_airing (
    work_id        INTEGER PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    -- airing: new episodes come (with or without a known date); hiatus: a
    -- break with no date; ended: finished or cancelled; unknown: no data.
    status         TEXT    NOT NULL CHECK (status IN ('airing', 'hiatus', 'ended', 'unknown')),
    -- The next episode; NULL season: the provider numbers episodes in one
    -- run (AniList, Shikimori).
    next_season    INTEGER CHECK (next_season >= 0),
    next_episode   INTEGER CHECK (next_episode >= 0),
    -- YYYY-MM-DD when only the date is known (TMDB).
    next_air_date  TEXT    CHECK (next_air_date IS NULL
                                  OR next_air_date GLOB '[0-9][0-9][0-9][0-9]-[01][0-9]-[0-3][0-9]'),
    -- The exact moment, Unix ms (AniList, Shikimori).
    next_air_at_ms INTEGER,
    -- tmdb, anilist or shikimori.
    source         TEXT    NOT NULL,
    -- Last answer of a provider.
    checked_at_ms  INTEGER NOT NULL,
    -- The last refresh after it failed (offline); NULL when it succeeded.
    failed_at_ms   INTEGER
) STRICT;
