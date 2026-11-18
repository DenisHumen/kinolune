-- SPDX-License-Identifier: MPL-2.0
--
-- META-012 (ADR-0132): Fanart.tv clear art. `artwork.kind` gains
-- 'clearart' (transparent character art of a work); the clear logo stays
-- 'logo'. Expand-only: the CHECK of a STRICT table cannot be altered, so
-- the table is rebuilt with every row and both indexes as they were.
-- Nothing references `artwork`.

CREATE TABLE artwork_new (
    id         INTEGER PRIMARY KEY,
    work_id    INTEGER REFERENCES works (id) ON DELETE CASCADE,
    episode_id INTEGER REFERENCES episodes (id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL
               CHECK (kind IN ('poster', 'backdrop', 'logo', 'still', 'clearart')),
    -- tmdb, anilist, shikimori, fanart, local…
    provider   TEXT    NOT NULL CHECK (provider <> '' AND provider NOT GLOB '*[^a-z0-9_]*'),
    url        TEXT    NOT NULL CHECK (url <> ''),
    -- ISO 639-1 of text on the image; '' if none or unknown.
    lang       TEXT    NOT NULL DEFAULT '',
    -- Lower is preferred.
    priority   INTEGER NOT NULL DEFAULT 0,
    CHECK ((work_id IS NULL) <> (episode_id IS NULL))
) STRICT;

INSERT INTO artwork_new (id, work_id, episode_id, kind, provider, url, lang, priority)
SELECT id, work_id, episode_id, kind, provider, url, lang, priority FROM artwork;

DROP TABLE artwork;
ALTER TABLE artwork_new RENAME TO artwork;

CREATE UNIQUE INDEX artwork_work ON artwork (work_id, kind, provider) WHERE work_id IS NOT NULL;
CREATE UNIQUE INDEX artwork_episode ON artwork (episode_id, kind, provider)
    WHERE episode_id IS NOT NULL;
