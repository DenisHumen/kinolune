// SPDX-License-Identifier: MPL-2.0
//! Subsonic and OpenSubsonic sources (NET-016): a music server (Navidrome,
//! Gonic, Airsonic, …) read as a folder of music through
//! [`SubsonicFs`], with scrobbles of what the app plays.
//!
//! A Subsonic source is a login source like WebDAV: the address, the user
//! name and, in the secret store, the password. The Subsonic token auth
//! needs the password on every request (`md5(password + salt)`), so,
//! unlike a Jellyfin token, it is the password itself that is kept — only
//! in the secret store, never in SQLite. A source without a user name
//! holds an OpenSubsonic API key in its place.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

use pa_secrets::SecretString;
use pa_subsonic::{Client, Credentials, ServerInfo, SubsonicFs};
use pa_vfs::{ErrorKind, VfsError, VfsPath};

use crate::manager::Stored;
use crate::{Error, Result, SourceKind, SourceManager, scrub};

/// Cached trees: source id → (settings and secret they were built with,
/// tree).
pub(crate) type Trees = HashMap<i64, (String, Arc<SubsonicFs>)>;

/// The credentials of a source: a user name and password, or (no user
/// name) an API key.
fn credentials(username: Option<&str>, secret: &SecretString) -> Credentials {
    match username.map(str::trim).filter(|u| !u.is_empty()) {
        Some(user) => Credentials::password(user, secret.expose()),
        None => Credentials::api_key(secret.expose()),
    }
}

fn server_error(e: &pa_subsonic::Error, secret: Option<&SecretString>) -> Error {
    let v = pa_subsonic::vfs_error(e);
    Error::Server {
        kind: v.kind(),
        message: scrub(v.message(), secret),
    }
}

/// Checks a Subsonic server before it is added: the address answers and
/// the credentials work (`ping`). `username` `None` (or empty): `secret`
/// is an OpenSubsonic API key.
///
/// # Errors
///
/// [`Error::Invalid`] for a bad address; [`Error::Server`] with
/// [`ErrorKind::AuthenticationFailed`] for wrong credentials, or the
/// network error.
pub fn subsonic_probe(
    address: &str,
    username: Option<&str>,
    secret: &SecretString,
) -> Result<ServerInfo> {
    let client = Client::new(address, credentials(username, secret))
        .map_err(|e| Error::Invalid(e.to_string()))?;
    client.ping().map_err(|e| server_error(&e, Some(secret)))
}

impl SourceManager {
    /// The tree of a Subsonic source with its stored password or key;
    /// reused while the settings and the secret stay the same.
    pub(crate) fn subsonic_tree(
        &self,
        stored: &Stored,
        secret: Option<SecretString>,
    ) -> pa_vfs::Result<Arc<SubsonicFs>> {
        let s = &stored.source;
        let secret = secret.ok_or_else(|| {
            VfsError::new(
                ErrorKind::AuthenticationFailed,
                "no password or API key for the Subsonic server",
            )
        })?;
        let key = format!(
            "{}\n{}\n{}\n{}",
            s.uri,
            s.username.as_deref().unwrap_or(""),
            serde_json::to_string(&s.options).unwrap_or_default(),
            secret.expose()
        );
        {
            let trees = self.subsonic.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((k, fs)) = trees.get(&s.id)
                && *k == key
            {
                return Ok(Arc::clone(fs));
            }
        }
        let client = match s.options.timeout_secs {
            Some(t) => Client::with_timeouts(
                &s.uri,
                credentials(s.username.as_deref(), &secret),
                std::time::Duration::from_secs(5),
                std::time::Duration::from_secs(t.into()),
            ),
            None => Client::new(&s.uri, credentials(s.username.as_deref(), &secret)),
        }
        .map_err(|e| pa_subsonic::vfs_error(&e))?;
        let mut fs = SubsonicFs::new(Arc::new(client));
        if let Some(b) = s.options.read_ahead_bytes {
            fs = fs.with_read_ahead(usize::try_from(b).unwrap_or(usize::MAX));
        }
        let fs = Arc::new(fs);
        self.subsonic
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(s.id, (key, Arc::clone(&fs)));
        Ok(fs)
    }

    /// The Subsonic tree of source `id`.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if the source is not a Subsonic server;
    /// [`Error::Source`] if it has no password or key.
    pub fn subsonic(&self, id: i64) -> Result<Arc<SubsonicFs>> {
        let stored = self.stored(id)?;
        if stored.source.kind != SourceKind::Subsonic {
            return Err(Error::Invalid(format!(
                "source {id} is not a Subsonic server"
            )));
        }
        let secret = self.password_of(&stored)?;
        let masked = secret.clone();
        self.subsonic_tree(&stored, secret)
            .map_err(|e| Error::Source {
                id,
                kind: e.kind(),
                message: scrub(e.message(), masked.as_ref()),
            })
    }

    /// Scrobbles the song at `rel_path` of Subsonic source `id`:
    /// `submission = false` is "now playing", `true` "played" (at
    /// `time_ms`, Unix ms; default now). Returns `false` and does nothing
    /// for other sources and for paths that are not songs.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached or refuses.
    pub fn subsonic_scrobble(
        &self,
        id: i64,
        rel_path: &str,
        submission: bool,
        time_ms: Option<i64>,
    ) -> Result<bool> {
        if self.get(id)?.kind != SourceKind::Subsonic {
            return Ok(false);
        }
        let fs = self.subsonic(id)?;
        let secret = SecretString::new(fs.client().credentials().secret().expose());
        let fail = |e: &VfsError| Error::Source {
            id,
            kind: e.kind(),
            message: scrub(e.message(), Some(&secret)),
        };
        let path = VfsPath::parse(rel_path).map_err(|e| fail(&e))?;
        let song = match fs.song_at(&path) {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(false),
            // Gone from the server since the scan.
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(fail(&e)),
        };
        fs.client()
            .scrobble(&song.song_id, submission, time_ms)
            .map_err(|e| fail(&pa_subsonic::vfs_error(&e)))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pa_secrets::{MemoryStore, SecretString};

    use crate::{NewSource, SourceKind, SourceManager};

    #[test]
    fn subsonic_sources_keep_the_password_in_the_secret_store() {
        let m = SourceManager::new(
            pa_library::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        );
        let s = m
            .create(
                NewSource::new(SourceKind::Subsonic, "Music", "http://nas:4533/rest/")
                    .with_login("anna", SecretString::new("hunter2")),
            )
            .unwrap();
        assert_eq!(s.kind, SourceKind::Subsonic);
        assert_eq!(s.uri, "http://nas:4533");
        assert!(s.has_password);
        assert!(SourceKind::Subsonic.is_network() && !SourceKind::Subsonic.is_media_server());
        assert_eq!(
            "subsonic".parse::<SourceKind>().unwrap(),
            SourceKind::Subsonic
        );
        // The tree is built without touching the network and reused.
        let a = m.subsonic(s.id).unwrap();
        let b = m.subsonic(s.id).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.client().user(), Some("anna"));
        assert!(m.connect(s.id).is_ok());
        // Not a Subsonic source: nothing to scrobble.
        let dir = tempfile::tempdir().unwrap();
        let local = m
            .create(NewSource::new(
                SourceKind::Local,
                "Local",
                dir.path().to_string_lossy(),
            ))
            .unwrap();
        assert!(!m.subsonic_scrobble(local.id, "a.mp3", true, None).unwrap());
        assert!(m.subsonic(local.id).is_err());

        // No user name: the secret is an API key.
        let k = m
            .create(
                NewSource::new(SourceKind::Subsonic, "Keyed", "https://music.example")
                    .with_login("", SecretString::new("key-1")),
            )
            .unwrap();
        assert_eq!(k.username, None);
        assert_eq!(m.subsonic(k.id).unwrap().client().user(), None);

        // Credentials in the address are refused.
        assert!(
            m.create(NewSource::new(
                SourceKind::Subsonic,
                "Bad",
                "http://anna:hunter2@nas:4533"
            ))
            .is_err()
        );
        // No secret: an authentication error, not a panic.
        let none = m
            .create(NewSource::new(
                SourceKind::Subsonic,
                "None",
                "http://nas:4533",
            ))
            .unwrap();
        assert_eq!(
            m.connect(none.id).unwrap_err().source_kind(),
            Some(pa_vfs::ErrorKind::AuthenticationFailed)
        );
    }
}
