-- SPDX-License-Identifier: MPL-2.0
--
-- PLAY-015: where an imported skip segment (origin 'import') comes from.
-- Expand-only.
--
-- provider      the program whose markers were imported: 'plex',
--               'jellyfin' or 'edl' (a Kodi/MPlayer edit decision list,
--               written by hand, by Comskip or by Jellyfin's intro-skipper).
--               NULL for every other origin.
-- provider_ref  the sidecar file next to the video the markers were read
--               from (its name, e.g. 'Episode 1.edl'); NULL when they were
--               handed over by a program (a media server's answer). A
--               sidecar scan replaces only rows it wrote itself.

ALTER TABLE skip_segments ADD COLUMN provider TEXT
    CHECK (provider IS NULL OR provider IN ('plex', 'jellyfin', 'edl'));
ALTER TABLE skip_segments ADD COLUMN provider_ref TEXT;
