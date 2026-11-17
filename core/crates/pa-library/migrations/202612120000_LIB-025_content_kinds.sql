-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-025 (ADR-0099): the content kind of every media file is decided by
-- the core; the user never picks it. Expand-only (LIB-002).
--
-- file_content_kinds        what a file holds: 'video' (a movie, show or
--                           anime: recognized), 'own_video' (footage from a
--                           phone or camera, «Мои видео»: never recognized,
--                           never sent to a provider, not hidden), 'music'
--                           or 'audiobook'. `detected` is what the
--                           classifier (pa_recognition::content) said,
--                           `kind` what counts after the user's «Это не
--                           то» and the user's own matches. `version` is
--                           the classifier version; `reasons_json` the
--                           signals that decided (explanation).
-- folder_content_overrides  «Это не то» on a folder (VfsPath, '/' = the
--                           whole source): its files of the same medium
--                           (video or audio) are `kind`. The longest folder
--                           wins.

CREATE TABLE file_content_kinds (
    file_id          INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    kind             TEXT    NOT NULL
                     CHECK (kind IN ('video', 'own_video', 'music', 'audiobook')),
    detected         TEXT    NOT NULL
                     CHECK (detected IN ('video', 'own_video', 'music', 'audiobook')),
    version          INTEGER NOT NULL CHECK (version > 0),
    score            REAL    NOT NULL,
    reasons_json     TEXT    NOT NULL CHECK (json_valid(reasons_json)),
    classified_at_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX file_content_kinds_kind ON file_content_kinds (kind);

CREATE TABLE folder_content_overrides (
    source_id INTEGER NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    path      TEXT    NOT NULL CHECK (substr(path, 1, 1) = '/'),
    kind      TEXT    NOT NULL
              CHECK (kind IN ('video', 'own_video', 'music', 'audiobook')),
    set_at_ms INTEGER NOT NULL,
    PRIMARY KEY (source_id, path)
) WITHOUT ROWID;
