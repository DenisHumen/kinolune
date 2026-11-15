-- SPDX-License-Identifier: MPL-2.0
--
-- META-021: MusicBrainz metadata of albums and artists, Cover Art Archive
-- covers and normalized genres (docs/dev/db.md "Музыкальные метаданные").
-- Expand-only migration.
--
-- Written by `pa_catalog::Catalog::fetch_music_metadata`, one row per album
-- and per album artist; the tag data in `music_albums` / `music_artists`
-- stays as read, so a rescan never loses what was fetched and a reset only
-- deletes these rows. Rows go with their album or artist.

CREATE TABLE music_album_metadata (
    album_id           INTEGER PRIMARY KEY REFERENCES music_albums (id) ON DELETE CASCADE,
    outcome            TEXT    NOT NULL CHECK (outcome IN ('matched', 'not_found', 'failed')),
    -- How the release was found: the release or release-group MBID of the
    -- tags, or a search by album artist and title ranked by track count,
    -- year and track durations.
    decided_by         TEXT    CHECK (decided_by IN ('release_mbid', 'release_group_mbid', 'search')),
    confidence         REAL    CHECK (confidence BETWEEN 0 AND 1),
    -- Lowercase UUIDs. A match always has a release group; a release only
    -- when one edition was identified.
    release_mbid       TEXT    CHECK (release_mbid IS NULL
                                      OR (length(release_mbid) = 36 AND release_mbid NOT GLOB '*[^0-9a-f-]*')),
    release_group_mbid TEXT    CHECK (release_group_mbid IS NULL
                                      OR (length(release_group_mbid) = 36
                                          AND release_group_mbid NOT GLOB '*[^0-9a-f-]*')),
    title              TEXT,
    -- Artist credit as one string (`A & B feat. C`).
    artist_credit      TEXT,
    -- Release date as MusicBrainz gives it (`1997`, `1997-05-21`).
    release_date       TEXT,
    -- Year of the first release of the release group.
    original_year      INTEGER CHECK (original_year BETWEEN 1000 AND 3000),
    -- Release-group type: `Album`, `EP`, `Single`… and secondary types
    -- (`["Live", "Compilation"]`).
    primary_type       TEXT,
    secondary_types    TEXT    NOT NULL DEFAULT '[]'
                               CHECK (json_valid(secondary_types) AND json_type(secondary_types) = 'array'),
    country            TEXT,
    label              TEXT,
    -- Normalized genre names, most votes first (`pa_library::genre`).
    genres             TEXT    NOT NULL DEFAULT '[]'
                               CHECK (json_valid(genres) AND json_type(genres) = 'array'),
    -- Front cover of the release group in the Cover Art Archive
    -- (`https://coverartarchive.org/release-group/<id>/front-500`). The
    -- image itself is downloaded by the image cache on the device only.
    cover_url          TEXT    CHECK (cover_url IS NULL OR cover_url GLOB 'https://*'),
    -- 0: the Cover Art Archive could not be asked yet (network); the next
    -- run asks again without asking MusicBrainz.
    cover_checked      INTEGER NOT NULL DEFAULT 0 CHECK (cover_checked IN (0, 1)),
    -- Why the last fetch failed (`failed`).
    error              TEXT,
    fetched_at_ms      INTEGER NOT NULL,
    CHECK (outcome = 'matched' OR (release_mbid IS NULL AND release_group_mbid IS NULL
                                    AND cover_url IS NULL)),
    CHECK (outcome <> 'matched' OR (release_group_mbid IS NOT NULL AND decided_by IS NOT NULL))
) STRICT;

CREATE INDEX music_album_metadata_release_group ON music_album_metadata (release_group_mbid)
    WHERE release_group_mbid IS NOT NULL;

CREATE TABLE music_artist_metadata (
    artist_id      INTEGER PRIMARY KEY REFERENCES music_artists (id) ON DELETE CASCADE,
    outcome        TEXT    NOT NULL CHECK (outcome IN ('matched', 'not_found', 'failed')),
    -- `mbid`: the artist MBID of the tags; `album`: the credit of a matched
    -- album; `search`: the only exact name match of an artist search.
    decided_by     TEXT    CHECK (decided_by IN ('mbid', 'album', 'search')),
    mbid           TEXT    CHECK (mbid IS NULL OR (length(mbid) = 36 AND mbid NOT GLOB '*[^0-9a-f-]*')),
    name           TEXT,
    sort_name      TEXT,
    -- `Person`, `Group`, `Orchestra`, `Choir`, `Character`, `Other`.
    artist_type    TEXT,
    country        TEXT,
    disambiguation TEXT,
    -- Begin and end dates as MusicBrainz gives them (`1985`, `1985-03-01`).
    begin_date     TEXT,
    end_date       TEXT,
    genres         TEXT    NOT NULL DEFAULT '[]'
                           CHECK (json_valid(genres) AND json_type(genres) = 'array'),
    error          TEXT,
    fetched_at_ms  INTEGER NOT NULL,
    CHECK (outcome = 'matched' OR mbid IS NULL),
    CHECK (outcome <> 'matched' OR (mbid IS NOT NULL AND decided_by IS NOT NULL))
) STRICT;
