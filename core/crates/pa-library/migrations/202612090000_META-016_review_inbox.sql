-- SPDX-License-Identifier: MPL-2.0
--
-- META-016: the review inbox («На проверку»). Expand-only.
--
-- work_candidates  the provider results the metadata run ranked for a
--                  work, best first (at most a handful): what the inbox
--                  offers when the decision is `auto_review` («проверьте»)
--                  or no result was good enough (manual queue). Rewritten
--                  by every search of the work and removed by a user
--                  decision. `explanation_json` holds the signals of the
--                  candidate (title similarity, year, kind, episodes,
--                  duration, language, provider). Search data of TMDB, so
--                  it is refreshed with the work's metadata (well within
--                  TMDB's six-month cache limit) and never synced.
-- work_reviews     the user's final word on a work that is not a decision
--                  between candidates: `not_media` («это не фильм»: the
--                  files stay in the library under their own names, are
--                  never sent to a provider again and leave the inbox).

CREATE TABLE work_candidates (
    work_id          INTEGER NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    rank             INTEGER NOT NULL CHECK (rank >= 1),
    provider         TEXT    NOT NULL CHECK (provider <> '' AND provider NOT GLOB '*[^a-z0-9_]*'),
    -- tmdb `movie` / `tv`.
    ext_kind         TEXT    NOT NULL DEFAULT '',
    ext_id           TEXT    NOT NULL CHECK (ext_id <> ''),
    title            TEXT    NOT NULL,
    original_title   TEXT,
    year             INTEGER,
    -- https URL of the provider's CDN (hotlinked).
    poster_url       TEXT,
    overview         TEXT,
    score            REAL    NOT NULL,
    explanation_json TEXT    CHECK (explanation_json IS NULL OR json_valid(explanation_json)),
    PRIMARY KEY (work_id, rank),
    UNIQUE (work_id, provider, ext_kind, ext_id)
) STRICT;

CREATE TABLE work_reviews (
    work_id       INTEGER PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    decision      TEXT    NOT NULL CHECK (decision IN ('not_media')),
    decided_at_ms INTEGER NOT NULL
) STRICT;
