-- SPDX-License-Identifier: MPL-2.0
--
-- PLAY-011: skippable segments of a file (intro, recap, credits, preview)
-- and the per-file analysis they were found with. Expand-only.
--
-- skip_segments  one row per file, kind and origin, so a chapter mark, an
--                audio fingerprint match and a user correction coexist;
--                playback takes the best origin of each kind (user,
--                chapter, import, audio, video — pa_library::skip).
--                Times are milliseconds from the start of the file.
-- skip_analysis  what the background job (`pa-skip`) found in one file:
--                audio fingerprints of the intro and outro windows
--                (little-endian u32 per 32 ms, 0 = silence) and the credits
--                seen in the picture. A replaced file (size or mtime) or a
--                newer analysis `version` is analysed again; matching the
--                episodes of a season reuses the stored fingerprints.

CREATE TABLE skip_segments (
    file_id       INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    kind          TEXT    NOT NULL CHECK (kind IN ('intro', 'recap', 'credits', 'preview')),
    origin        TEXT    NOT NULL CHECK (origin IN ('chapter', 'audio', 'video', 'import', 'user')),
    start_ms      INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms        INTEGER NOT NULL CHECK (end_ms > start_ms),
    confidence    REAL    NOT NULL DEFAULT 1 CHECK (confidence BETWEEN 0.0 AND 1.0),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (file_id, kind, origin)
) STRICT;

CREATE TABLE skip_analysis (
    file_id                INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    version                INTEGER NOT NULL CHECK (version > 0),
    size_bytes             INTEGER NOT NULL CHECK (size_bytes >= 0),
    mtime_ms               INTEGER,
    duration_ms            INTEGER CHECK (duration_ms >= 0),
    intro_fp               BLOB,
    intro_fp_start_ms      INTEGER CHECK (intro_fp_start_ms >= 0),
    outro_fp               BLOB,
    outro_fp_start_ms      INTEGER CHECK (outro_fp_start_ms >= 0),
    video_credits_start_ms INTEGER CHECK (video_credits_start_ms >= 0),
    video_credits_end_ms   INTEGER CHECK (video_credits_end_ms >= 0),
    error                  TEXT,
    analyzed_at_ms         INTEGER NOT NULL
) STRICT;
