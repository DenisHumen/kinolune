-- SPDX-License-Identifier: MPL-2.0
--
-- MUS-012 (ADR-0130): audio playlists mirrored from a music server
-- (Subsonic/Navidrome, Jellyfin, Emby). Written by the scan of the source
-- (`pa_catalog::Catalog::scan`), read by `Catalog::music_playlists`.
-- Expand-only (LIB-002).
--
-- A mirrored playlist is an ordinary `playlists` row of kind 'audio' (one
-- per profile that may see the source), so the music UI, the queue and
-- offline downloads of a playlist (NET-019) work on it unchanged. This
-- table marks it as the server's: the app does not edit it (the next scan
-- would overwrite the edit), and it goes away with the source or when the
-- server no longer has it.
--
-- remote_id  the server's playlist id.

CREATE TABLE server_playlists (
    playlist_id   INTEGER PRIMARY KEY REFERENCES playlists (id) ON DELETE CASCADE,
    source_id     INTEGER NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    remote_id     TEXT    NOT NULL CHECK (remote_id <> ''),
    synced_at_ms  INTEGER NOT NULL
) STRICT;

CREATE INDEX server_playlists_source ON server_playlists (source_id, remote_id);

-- A removed source takes its mirrored playlists along (the cascade above
-- removes only the marks).
CREATE TRIGGER server_playlists_gone AFTER DELETE ON server_playlists
BEGIN
    DELETE FROM playlists WHERE id = OLD.playlist_id;
END;
