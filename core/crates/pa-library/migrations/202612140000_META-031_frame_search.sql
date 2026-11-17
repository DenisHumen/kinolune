-- SPDX-License-Identifier: MPL-2.0
--
-- META-031 (ADR-0104): anime recognised by its frames through trace.moe,
-- only with the user's consent. Expand-only (LIB-002).
--
-- frame_searches  the outcome of one file's frame search, by the file's
--                 content fingerprint (`files.fingerprint`): a file is
--                 never searched twice, also after a rescan, a move or a
--                 new library database. `outcome`: 'matched' (one AniList
--                 entry had the agreement; `anilist_id`, the agreed
--                 `episode` in AniList order if any), 'review' (the frames
--                 did not agree; `candidates_json` the entries they named,
--                 offered in the review inbox) or 'no_frames' (no usable
--                 frame). `agreement` is the share of frames of the best
--                 entry, `frames` the searches spent. No image, no
--                 trace.moe URL and nothing of the user's file name is
--                 stored.

CREATE TABLE frame_searches (
    fingerprint     TEXT    PRIMARY KEY
                    CHECK (fingerprint <> '' AND fingerprint NOT GLOB '*[^0-9a-f]*'),
    outcome         TEXT    NOT NULL CHECK (outcome IN ('matched', 'review', 'no_frames')),
    anilist_id      INTEGER CHECK (anilist_id > 0),
    episode         INTEGER CHECK (episode > 0),
    agreement       REAL    NOT NULL CHECK (agreement BETWEEN 0.0 AND 1.0),
    frames          INTEGER NOT NULL CHECK (frames >= 0),
    candidates_json TEXT    NOT NULL CHECK (json_valid(candidates_json)),
    searched_at_ms  INTEGER NOT NULL,
    CHECK ((outcome = 'matched') = (anilist_id IS NOT NULL))
) STRICT;
