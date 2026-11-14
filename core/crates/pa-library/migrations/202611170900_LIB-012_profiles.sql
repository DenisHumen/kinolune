-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-012: profiles and parental-control filters. Expand-only.
--
-- Per-profile state (progress, favorites, collections, playlists, version,
-- view and display preferences, smart shelves, music plays, Twitch
-- favourites) is keyed by `profile_id` since schema v1, and everything
-- recorded so far belongs to the default profile (id 1). This migration
-- adds what a profile itself needs:
--
-- profiles.pin_hash          Argon2id hash of the profile PIN
--                            (`argon2id$v=19$m=…,t=…,p=…$<salt hex>$<hash hex>`,
--                            pa_library::profiles); NULL = no PIN.
-- profiles.max_age           content-rating limit as a minimum viewer age
--                            (0–18); NULL = no limit.
-- profiles.rating_country    ISO 3166-1 alpha-2 country whose certification
--                            wins when a work has several ('' = the strictest
--                            known one).
-- profiles.hide_unrated      1 = video without a known rating is hidden
--                            ("no rating = hidden").
-- profiles.hide_artwork      1 = posters, backdrops and stills are not shown
--                            to the profile.
-- profiles.restrict_folders  1 = only the folders in `profile_folders` are
--                            visible (white list); 0 = every source.
-- profiles.last_active_at_ms when the profile was last switched to; the app
--                            opens the most recent one.
-- profile_folders            white list of a profile: a source (`path` '/')
--                            or a folder of it (`/Movies/Kids`), with every
--                            subfolder.
-- work_certifications        content ratings of a work by provider and
--                            country (TMDB `release_dates` / `content_ratings`)
--                            and the parent's own rating (`provider` 'user',
--                            `country` ''), with the minimum age the rating
--                            means (`min_age`, NULL if unknown).

ALTER TABLE profiles ADD COLUMN pin_hash TEXT CHECK (pin_hash IS NULL OR pin_hash GLOB 'argon2id$*');
ALTER TABLE profiles ADD COLUMN max_age INTEGER CHECK (max_age BETWEEN 0 AND 21);
ALTER TABLE profiles ADD COLUMN rating_country TEXT NOT NULL DEFAULT ''
    CHECK (rating_country = '' OR (length(rating_country) = 2 AND rating_country NOT GLOB '*[^A-Z]*'));
ALTER TABLE profiles ADD COLUMN hide_unrated INTEGER NOT NULL DEFAULT 0 CHECK (hide_unrated IN (0, 1));
ALTER TABLE profiles ADD COLUMN hide_artwork INTEGER NOT NULL DEFAULT 0 CHECK (hide_artwork IN (0, 1));
ALTER TABLE profiles ADD COLUMN restrict_folders INTEGER NOT NULL DEFAULT 0
    CHECK (restrict_folders IN (0, 1));
ALTER TABLE profiles ADD COLUMN last_active_at_ms INTEGER;

CREATE TABLE profile_folders (
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    source_id  INTEGER NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    -- VfsPath of the folder: '/' (the whole source) or '/a/b', no trailing '/'.
    path       TEXT    NOT NULL
               CHECK (path = '/' OR (path GLOB '/?*' AND path NOT GLOB '*/')),
    PRIMARY KEY (profile_id, source_id, path)
) STRICT;

CREATE INDEX profile_folders_source ON profile_folders (source_id);

CREATE TABLE work_certifications (
    work_id       INTEGER NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- tmdb, user…
    provider      TEXT    NOT NULL CHECK (provider <> '' AND provider NOT GLOB '*[^a-z0-9_]*'),
    -- ISO 3166-1 alpha-2; '' for the user's own rating.
    country       TEXT    NOT NULL
                  CHECK (country = '' OR (length(country) = 2 AND country NOT GLOB '*[^A-Z]*')),
    -- As the provider writes it: `PG-13`, `TV-MA`, `16`, `18+`.
    certification TEXT    NOT NULL CHECK (certification <> ''),
    min_age       INTEGER CHECK (min_age BETWEEN 0 AND 21),
    PRIMARY KEY (work_id, provider, country)
) STRICT;
