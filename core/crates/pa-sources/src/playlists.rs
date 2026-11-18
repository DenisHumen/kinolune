// SPDX-License-Identifier: MPL-2.0
//! Music playlists of music servers (MUS-012): what a Subsonic server
//! (Navidrome, Gonic, …) or a Jellyfin or Emby server keeps as the user's
//! playlists, with each song as its path in the source's file tree — the
//! same path the scan gave the song's file, so the library can mirror the
//! playlist as an audio playlist of its tracks.
//!
//! Other source kinds have no playlists of their own (an M3U file in a
//! folder is a file, not a server playlist).

use pa_vfs::VfsError;

use crate::{Error, Result, SourceKind, SourceManager, scrub};

/// A music playlist of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPlaylist {
    /// The server's id of the playlist.
    pub remote_id: String,
    /// Its name on the server.
    pub name: String,
    /// Its songs in order, as paths of the source's tree (`/Artist/Album/
    /// 01 - Song.flac`). Songs the tree does not hold (another library,
    /// deleted) and videos are left out.
    pub paths: Vec<String>,
}

fn fail(id: i64, e: &VfsError) -> Error {
    Error::Source {
        id,
        kind: e.kind(),
        message: scrub(e.message(), None),
    }
}

impl SourceManager {
    /// The music playlists of source `id` with their songs as paths of
    /// the source's tree; `None` for kinds that have no server playlists.
    /// Uses the tree the scan built (cached), so it adds only the
    /// playlist requests.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached or refuses.
    pub fn server_playlists(&self, id: i64) -> Result<Option<Vec<ServerPlaylist>>> {
        match self.get(id)?.kind {
            SourceKind::Subsonic => self.subsonic_playlists(id).map(Some),
            kind if kind.is_media_server() => self.jellyfin_playlists(id).map(Some),
            _ => Ok(None),
        }
    }

    fn subsonic_playlists(&self, id: i64) -> Result<Vec<ServerPlaylist>> {
        let fs = self.subsonic(id)?;
        let secret = pa_secrets::SecretString::new(fs.client().credentials().secret().expose());
        let fail = |e: &VfsError| Error::Source {
            id,
            kind: e.kind(),
            message: scrub(e.message(), Some(&secret)),
        };
        let client = fs.client();
        let lists = client
            .playlists()
            .map_err(|e| fail(&pa_subsonic::vfs_error(&e)))?;
        let mut out = Vec::with_capacity(lists.len());
        for p in lists {
            let full = match client.playlist(&p.id) {
                Ok(full) => full,
                // Deleted between the two calls.
                Err(pa_subsonic::Error::NotFound) => continue,
                Err(e) => return Err(fail(&pa_subsonic::vfs_error(&e))),
            };
            let mut paths = Vec::with_capacity(full.entries.len());
            for song in &full.entries {
                if let Some(path) = fs.path_of(&song.id).map_err(|e| fail(&e))? {
                    paths.push(path.as_str().to_owned());
                }
            }
            out.push(ServerPlaylist {
                remote_id: p.id,
                name: name_or_id(&p.name, &full.name),
                paths,
            });
        }
        Ok(out)
    }

    fn jellyfin_playlists(&self, id: i64) -> Result<Vec<ServerPlaylist>> {
        let fs = self.jellyfin(id)?;
        let jf = |e: &pa_jellyfin::Error| fail(id, &pa_jellyfin::vfs_error(e));
        let client = fs.client();
        let user = fs.user_id();
        let lists = client.playlists(user).map_err(|e| jf(&e))?;
        let mut out = Vec::new();
        for p in lists {
            // Video playlists stay on the server: this is music.
            if p.media_type.as_deref().is_some_and(|t| t != "Audio") {
                continue;
            }
            let items = match client.playlist_items(user, &p.id) {
                Ok(items) => items,
                Err(pa_jellyfin::Error::NotFound) => continue,
                Err(e) => return Err(jf(&e)),
            };
            let mut paths = Vec::with_capacity(items.len());
            for item in items.iter().filter(|i| i.kind == "Audio") {
                if let Some(path) = fs.path_of(&item.id).map_err(|e| fail(id, &e))? {
                    paths.push(path.as_str().to_owned());
                }
            }
            // A playlist of untyped items that held no song is a video one.
            if paths.is_empty() && p.media_type.is_none() && !items.is_empty() {
                continue;
            }
            out.push(ServerPlaylist {
                remote_id: p.id,
                name: name_or_id(&p.name, ""),
                paths,
            });
        }
        Ok(out)
    }
}

/// The name of a playlist; a server that sent none gets "Playlist".
fn name_or_id(name: &str, other: &str) -> String {
    [name, other]
        .iter()
        .map(|n| n.trim())
        .find(|n| !n.is_empty())
        .unwrap_or("Playlist")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pa_secrets::MemoryStore;

    use super::*;
    use crate::NewSource;

    #[test]
    fn names_fall_back() {
        assert_eq!(name_or_id(" Road ", ""), "Road");
        assert_eq!(name_or_id("", "Full"), "Full");
        assert_eq!(name_or_id(" ", ""), "Playlist");
    }

    #[test]
    fn folders_have_no_server_playlists() {
        let dir = tempfile::tempdir().unwrap();
        let m = SourceManager::new(
            pa_library::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        );
        let s = m
            .create(NewSource::new(
                SourceKind::Local,
                "Music",
                dir.path().to_str().unwrap(),
            ))
            .unwrap();
        assert_eq!(m.server_playlists(s.id).unwrap(), None);
    }
}
