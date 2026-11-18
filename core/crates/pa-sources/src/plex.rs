// SPDX-License-Identifier: MPL-2.0
//! Plex sources (NET-012, ADR-0131): the plex.tv PIN sign-in
//! ([`PlexSignIn`]: a code entered at plex.tv/link, then the Plex Home
//! user and the server), the automatic choice of the connection (local,
//! then remote, then relay) and the cached [`PlexFs`] tree of a source.
//!
//! The secret store holds the **server token** of the chosen user (what
//! plex.tv hands out for that server), never a password: no password is
//! typed into the app at all. The account token is used during the sign-in
//! only and dropped with the [`PlexSignIn`].

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use pa_plex::{
    Account, ClientInfo, ConnectionKind, HomeUser, MediaRef, Pin, PlexFs, PlexTv, Resource, Server,
    Token, order_connections, probe_connections,
};
use pa_secrets::SecretString;
use pa_vfs::{ErrorKind, VfsError};

use crate::manager::Stored;
use crate::model::PlexOptions;
use crate::{Error, NewSource, Result, Source, SourceKind, SourceManager, SourceOptions, scrub};

/// Cached trees: source id → (settings and token they were built with,
/// tree).
pub(crate) type Trees = HashMap<i64, (String, Arc<PlexFs>)>;

/// Where the user enters the code.
pub const PLEX_LINK_URL: &str = "https://plex.tv/link";

/// How long each connection of a server is given to answer during the
/// sign-in.
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

fn server_error(e: &pa_plex::Error) -> Error {
    let v = pa_plex::vfs_error(e);
    Error::Server {
        kind: v.kind(),
        message: scrub(v.message(), None),
    }
}

fn source_error(id: i64, e: &VfsError) -> Error {
    Error::Source {
        id,
        kind: e.kind(),
        message: scrub(e.message(), None),
    }
}

fn not_signed_in() -> Error {
    Error::Server {
        kind: ErrorKind::AuthenticationFailed,
        message: "the code has not been entered at plex.tv/link yet".into(),
    }
}

/// A plex.tv sign-in in progress: show [`code`](Self::code) and
/// [`PLEX_LINK_URL`], [`poll`](Self::poll) until it says the code was
/// entered, let the user pick a Home user ([`users`](Self::users),
/// [`switch_user`](Self::switch_user); skipped when there is one) and a
/// server ([`servers`](Self::servers); skipped when there is one), then
/// [`finish`](Self::finish). `Debug` hides the tokens.
pub struct PlexSignIn {
    tv: PlexTv,
    pin: Pin,
    account_token: Option<Token>,
    account: Option<Account>,
    users: Vec<HomeUser>,
    /// The chosen Home user (uuid) and their token; `None`: the account.
    user: Option<(String, Token)>,
    servers: Vec<Resource>,
}

impl std::fmt::Debug for PlexSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlexSignIn")
            .field("code", &self.pin.code)
            .field("linked", &self.account_token.is_some())
            .field("users", &self.users.len())
            .field("servers", &self.servers.len())
            .finish_non_exhaustive()
    }
}

/// A finished sign-in, ready for [`SourceManager::create_plex`]. `Debug`
/// hides the token.
#[derive(Debug, Clone)]
pub struct PlexAccount {
    /// The server.
    pub server_name: String,
    /// Its machine identifier.
    pub server_id: String,
    /// Its connections, best first (local, remote, relay).
    pub addresses: Vec<String>,
    /// The connection that answered during the sign-in and its kind.
    pub connected: (String, ConnectionKind),
    /// The user whose libraries the source shows.
    pub user_title: String,
    /// The server token of that user.
    pub token: Token,
    /// This app as plex.tv knows it.
    pub client: ClientInfo,
}

impl PlexSignIn {
    /// Starts a sign-in at plex.tv as a new device named `device_name`.
    ///
    /// # Errors
    ///
    /// [`Error::Server`] if plex.tv cannot be reached.
    pub fn start(device_name: &str) -> Result<Self> {
        Self::start_at(pa_plex::PLEX_TV, device_name)
    }

    /// Like [`start`](Self::start) with plex.tv at `base` (tests).
    ///
    /// # Errors
    ///
    /// As [`start`](Self::start).
    pub fn start_at(base: &str, device_name: &str) -> Result<Self> {
        let client = ClientInfo::kinolune(
            device_name.trim(),
            uuid::Uuid::now_v7().simple().to_string(),
        );
        let tv = PlexTv::with_base(client, base);
        let pin = tv.create_pin().map_err(|e| server_error(&e))?;
        Ok(Self {
            tv,
            pin,
            account_token: None,
            account: None,
            users: Vec::new(),
            user: None,
            servers: Vec::new(),
        })
    }

    /// The code to enter at [`PLEX_LINK_URL`].
    #[must_use]
    pub fn code(&self) -> &str {
        &self.pin.code
    }

    /// Whether the code has been entered. Once it has, the account, its
    /// Home users and the servers are read.
    ///
    /// # Errors
    ///
    /// [`Error::Server`]; [`ErrorKind::NotFound`] once the code expired
    /// (start again).
    pub fn poll(&mut self) -> Result<bool> {
        if self.account_token.is_some() {
            return Ok(true);
        }
        let id = self.pin.id.unwrap_or_default();
        let pin = self.tv.check_pin(id).map_err(|e| server_error(&e))?;
        let Some(token) = pin.auth_token.clone().filter(|t| !t.is_empty()) else {
            return Ok(false);
        };
        let token = Token::new(token);
        let account = self.tv.account(&token).map_err(|e| server_error(&e))?;
        let users = if account.home {
            self.tv.home_users(&token).map_err(|e| server_error(&e))?
        } else {
            Vec::new()
        };
        self.servers = self.tv.servers(&token).map_err(|e| server_error(&e))?;
        self.users = users;
        self.account = Some(account);
        self.account_token = Some(token);
        Ok(true)
    }

    /// The signed-in account, once [`poll`](Self::poll) said yes.
    #[must_use]
    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// The users of the account's Plex Home (empty without a Home).
    #[must_use]
    pub fn users(&self) -> &[HomeUser] {
        &self.users
    }

    /// The uuid of the user whose servers are listed: the chosen Home
    /// user, else the account.
    #[must_use]
    pub fn current_user(&self) -> Option<&str> {
        self.user
            .as_ref()
            .map(|(u, _)| u.as_str())
            .or(self.account.as_ref().map(|a| a.uuid.as_str()))
    }

    /// Switches to Home user `uuid` (with their `pin` when protected) and
    /// lists the servers as that user. Picking the account itself without
    /// a PIN goes back to the account.
    ///
    /// # Errors
    ///
    /// [`Error::Server`] with [`ErrorKind::AuthenticationFailed`] or
    /// [`ErrorKind::PermissionDenied`] for a wrong PIN; network errors.
    pub fn switch_user(&mut self, uuid: &str, pin: Option<&str>) -> Result<()> {
        let account_token = self.account_token.clone().ok_or_else(not_signed_in)?;
        let pin = pin.map(str::trim).filter(|p| !p.is_empty());
        let is_account = self.account.as_ref().is_some_and(|a| a.uuid == uuid);
        let protected = self.users.iter().any(|u| u.uuid == uuid && u.protected);
        let token = if is_account && (pin.is_none() || !protected) {
            None
        } else {
            Some(
                self.tv
                    .switch_user(&account_token, uuid, pin)
                    .map_err(|e| server_error(&e))?,
            )
        };
        let list_with = token.clone().unwrap_or_else(|| account_token.clone());
        self.servers = self.tv.servers(&list_with).map_err(|e| server_error(&e))?;
        self.user = token.map(|t| (uuid.to_owned(), t));
        Ok(())
    }

    /// The servers the current user can reach.
    #[must_use]
    pub fn servers(&self) -> &[Resource] {
        &self.servers
    }

    /// The server to offer first: the one with a connection on `hint`'s
    /// host (a server found on the LAN), else the only one, else an owned
    /// one that plex.tv sees online.
    #[must_use]
    pub fn suggested_server(&self, hint: Option<&str>) -> Option<&Resource> {
        let host = hint.and_then(host_of);
        if let Some(h) = host
            && let Some(s) = self
                .servers
                .iter()
                .find(|s| s.connections.iter().any(|c| c.address == h))
        {
            return Some(s);
        }
        if self.servers.len() == 1 {
            return self.servers.first();
        }
        self.servers
            .iter()
            .find(|s| s.owned && s.presence)
            .or(self.servers.first())
    }

    /// The display name of the current user.
    fn user_title(&self) -> String {
        let account = self.account.as_ref();
        match &self.user {
            Some((uuid, _)) => self
                .users
                .iter()
                .find(|u| u.uuid == *uuid)
                .map(|u| u.display_name().to_owned())
                .unwrap_or_default(),
            None => account
                .map(|a| a.display_name().to_owned())
                .unwrap_or_default(),
        }
    }

    /// Picks the connection of server `server_id` that answers — local
    /// first, then remote, then relay — and returns what the source needs.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for an unknown server; [`Error::Server`] with
    /// [`ErrorKind::Unavailable`] if no connection answers.
    pub fn finish(&self, server_id: &str) -> Result<PlexAccount> {
        if self.account_token.is_none() {
            return Err(not_signed_in());
        }
        let server = self
            .servers
            .iter()
            .find(|s| s.client_identifier == server_id)
            .ok_or_else(|| Error::Invalid(format!("no Plex server {server_id:?}")))?;
        let token = server
            .access_token
            .clone()
            .filter(|t| !t.is_empty())
            .map(Token::new)
            .ok_or_else(|| Error::Server {
                kind: ErrorKind::PermissionDenied,
                message: "plex.tv gave no access to this server".into(),
            })?;
        let order = order_connections(server);
        let uris: Vec<String> = order.iter().map(|(u, _)| u.clone()).collect();
        let ok = probe_connections(
            &uris,
            self.tv.info(),
            Some(&token),
            &server.client_identifier,
            PROBE_TIMEOUT,
        );
        let connected = order
            .iter()
            .zip(&ok)
            .find(|(_, ok)| **ok)
            .map(|(c, _)| c.clone())
            .ok_or_else(|| Error::Server {
                kind: ErrorKind::Unavailable,
                message: format!("no connection of the Plex server {:?} answers", server.name),
            })?;
        Ok(PlexAccount {
            server_name: server.name.clone(),
            server_id: server.client_identifier.clone(),
            addresses: uris,
            connected,
            user_title: self.user_title(),
            token,
            client: self.tv.info().clone(),
        })
    }
}

/// The host of `http(s)://host[:port]/…`.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let hostport = rest.split('/').next()?;
    let host = match hostport.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => hostport.split(':').next()?,
    };
    (!host.is_empty()).then_some(host)
}

impl SourceManager {
    /// Adds the server of `account` as a Plex source named `name`
    /// (default: the server's name). The server token goes to the secret
    /// store; `libraries` (section keys) limits the source to those
    /// libraries.
    ///
    /// # Errors
    ///
    /// As [`create`](Self::create).
    pub fn create_plex(
        &self,
        account: PlexAccount,
        name: Option<&str>,
        libraries: Vec<String>,
    ) -> Result<Source> {
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(
            || {
                let s = account.server_name.trim();
                if s.is_empty() {
                    "Plex".to_owned()
                } else {
                    s.to_owned()
                }
            },
            str::to_owned,
        );
        let (main, alt) = account
            .addresses
            .split_first()
            .ok_or_else(|| Error::Invalid("no server address".into()))?;
        let options = SourceOptions {
            plex: Some(PlexOptions {
                server_id: account.server_id.clone(),
                client_id: account.client.client_id.clone(),
                device_name: account.client.device.clone(),
                user: account.user_title.clone(),
                alt_addresses: alt.to_vec(),
                libraries,
            }),
            ..SourceOptions::default()
        };
        let user = Some(account.user_title.clone()).filter(|u| !u.trim().is_empty());
        let mut new = NewSource::new(SourceKind::Plex, name, main.as_str()).with_options(options);
        new.username = user;
        new.password = Some(SecretString::new(account.token.expose()));
        self.create(new)
    }

    /// The tree of a Plex source with its stored token; reused while
    /// settings and token stay the same.
    pub(crate) fn plex_tree(
        &self,
        stored: &Stored,
        token: Option<SecretString>,
    ) -> pa_vfs::Result<Arc<PlexFs>> {
        let s = &stored.source;
        let unauthorized = |m: &str| VfsError::new(ErrorKind::AuthenticationFailed, m.to_owned());
        let token = token.ok_or_else(|| unauthorized("not signed in to the Plex server"))?;
        let o = s
            .options
            .plex
            .clone()
            .filter(|o| !o.client_id.is_empty())
            .ok_or_else(|| unauthorized("no Plex sign-in: sign in again"))?;
        let key = format!(
            "{}\n{}\n{}",
            s.uri,
            serde_json::to_string(&s.options).unwrap_or_default(),
            token.expose()
        );
        {
            let trees = self.plex.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((k, fs)) = trees.get(&s.id)
                && *k == key
            {
                return Ok(Arc::clone(fs));
            }
        }
        let mut addresses = vec![s.uri.clone()];
        for a in &o.alt_addresses {
            if let Ok(a) = pa_plex::normalize_address(a)
                && !addresses.contains(&a)
            {
                addresses.push(a);
            }
        }
        let client = ClientInfo::kinolune(o.device_name.clone(), o.client_id.clone());
        let server = Server::new(&addresses, client, Token::new(token.expose()))
            .map_err(|e| pa_plex::vfs_error(&e))?;
        let mut fs = PlexFs::new(Arc::new(server)).with_libraries(o.libraries);
        if let Some(b) = s.options.read_ahead_bytes {
            fs = fs.with_read_ahead(usize::try_from(b).unwrap_or(usize::MAX));
        }
        let fs = Arc::new(fs);
        self.plex
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(s.id, (key, Arc::clone(&fs)));
        Ok(fs)
    }

    /// The Plex tree of source `id`.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if the source is not a Plex server;
    /// [`Error::Source`] if it has no valid sign-in.
    pub fn plex(&self, id: i64) -> Result<Arc<PlexFs>> {
        let stored = self.stored(id)?;
        if stored.source.kind != SourceKind::Plex {
            return Err(Error::Invalid(format!("source {id} is not a Plex server")));
        }
        let token = self.password_of(&stored)?;
        self.plex_tree(&stored, token)
            .map_err(|e| source_error(id, &e))
    }
}

/// How a file of a Plex source plays (NET-013, ADR-0134). `Debug` hides
/// the URL (it holds the token).
#[derive(Clone, PartialEq, Eq)]
pub struct PlexPlayback {
    /// Through the server's transcoder rather than the file as is.
    pub transcode: bool,
    /// For a transcode: the HLS URL for the player, with the token. `None`
    /// for direct play: play the file through the stream proxy.
    pub url: Option<String>,
    /// The transcode session, for the reports and to stop it.
    pub session: Option<String>,
    /// Why it is transcoded: `requested` (the user asked), `bitrate` (the
    /// file is above the bit rate asked for).
    pub reasons: Vec<String>,
}

impl std::fmt::Debug for PlexPlayback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlexPlayback")
            .field("transcode", &self.transcode)
            .field("url", &self.url.as_ref().map(|_| "***"))
            .field("session", &self.session)
            .field("reasons", &self.reasons)
            .finish()
    }
}

/// Quality the user asked for when transcoding on request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlexQuality {
    /// Upper bound of the video bit rate, kbit/s.
    pub max_bitrate_kbps: Option<u32>,
    /// Largest picture, width × height.
    pub resolution: Option<(u32, u32)>,
}

impl SourceManager {
    /// The media file at `rel_path` of Plex source `id`; `None` if the
    /// source is of another kind or the path is not a media file.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn plex_media(&self, id: i64, rel_path: &str) -> Result<Option<(Arc<PlexFs>, MediaRef)>> {
        if self.get(id)?.kind != SourceKind::Plex {
            return Ok(None);
        }
        let fs = self.plex(id)?;
        let path = pa_vfs::VfsPath::parse(rel_path).map_err(|e| source_error(id, &e))?;
        let media = match fs.media_at(&path) {
            Ok(m) => m,
            // The server changed since the scan.
            Err(e) if e.kind() == ErrorKind::NotFound => {
                fs.refresh();
                fs.media_at(&path).map_err(|e| source_error(id, &e))?
            }
            Err(e) => return Err(source_error(id, &e)),
        };
        Ok(media.map(|m| (fs, m)))
    }

    /// How file `rel_path` of Plex source `id` plays: directly (through
    /// the stream proxy) unless `force_transcode` or the file is above
    /// `quality`'s bit rate; then the transcoder's HLS URL. `None` if it
    /// is not a Plex media file.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn plex_playback(
        &self,
        id: i64,
        rel_path: &str,
        force_transcode: bool,
        quality: PlexQuality,
    ) -> Result<Option<PlexPlayback>> {
        let Some((fs, m)) = self.plex_media(id, rel_path)? else {
            return Ok(None);
        };
        if m.audio
            || !pa_plex::needs_transcode(m.bitrate_kbps, force_transcode, quality.max_bitrate_kbps)
        {
            return Ok(Some(PlexPlayback {
                transcode: false,
                url: None,
                session: None,
                reasons: Vec::new(),
            }));
        }
        let session = pa_plex::new_session_id();
        let url = fs.server().transcode_url(&pa_plex::TranscodeRequest {
            rating_key: m.rating_key.clone(),
            media_index: m.media_index,
            part_index: m.part_index,
            max_bitrate_kbps: quality.max_bitrate_kbps,
            resolution: quality.resolution,
            // Asked for a lower quality: re-encode; else a remux may do.
            direct_stream: quality.max_bitrate_kbps.is_none(),
            session: session.clone(),
        });
        Ok(Some(PlexPlayback {
            transcode: true,
            url: Some(url),
            session: Some(session),
            reasons: vec![
                if force_transcode {
                    "requested"
                } else {
                    "bitrate"
                }
                .to_owned(),
            ],
        }))
    }

    /// Reports playback of file `rel_path` of Plex source `id` as a
    /// timeline (`session`: the transcode session, if it plays one); the
    /// stop also ends the transcode. Does nothing for other sources.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn plex_report(
        &self,
        id: i64,
        rel_path: &str,
        event: crate::ReportEvent,
        position_ms: i64,
        paused: bool,
        session: Option<&str>,
    ) -> Result<()> {
        let Some((fs, m)) = self.plex_media(id, rel_path)? else {
            return Ok(());
        };
        let state = match event {
            crate::ReportEvent::Stop => pa_plex::PlayState::Stopped,
            _ if paused => pa_plex::PlayState::Paused,
            _ => pa_plex::PlayState::Playing,
        };
        let server = fs.server();
        server
            .timeline(&pa_plex::Timeline {
                rating_key: m.rating_key.clone(),
                state,
                position_ms: position_ms.max(0),
                duration_ms: m.duration_ms,
                session: session.map(str::to_owned),
            })
            .map_err(|e| source_error(id, &pa_plex::vfs_error(&e)))?;
        if state == pa_plex::PlayState::Stopped
            && let Some(s) = session
        {
            // The server ends an idle transcode by itself too.
            let _ = server.stop_transcode(s);
        }
        Ok(())
    }

    /// Marks file `rel_path` of Plex source `id` watched or not at the
    /// server. Does nothing for other sources.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn plex_set_played(&self, id: i64, rel_path: &str, played: bool) -> Result<()> {
        let Some((fs, m)) = self.plex_media(id, rel_path)? else {
            return Ok(());
        };
        let server = fs.server();
        if played {
            server.scrobble(&m.rating_key)
        } else {
            server.unscrobble(&m.rating_key)
        }
        .map_err(|e| source_error(id, &pa_plex::vfs_error(&e)))
    }

    /// Takes file `rel_path` out of the user's Continue Watching at the
    /// server. Does nothing for other sources.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn plex_remove_from_resume(&self, id: i64, rel_path: &str) -> Result<()> {
        let Some((fs, m)) = self.plex_media(id, rel_path)? else {
            return Ok(());
        };
        fs.server()
            .remove_from_continue_watching(&m.rating_key)
            .map_err(|e| source_error(id, &pa_plex::vfs_error(&e)))
    }

    /// Every media file of Plex source `id` with the user's state as the
    /// server has it now (position, watched, last view), paths relative
    /// to the source root without a leading `/`, like the catalog's
    /// `rel_path`.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn plex_user_data(&self, id: i64) -> Result<Vec<(String, MediaRef)>> {
        let fs = self.plex(id)?;
        fs.refresh();
        Ok(fs
            .media_files()
            .map_err(|e| source_error(id, &e))?
            .into_iter()
            .map(|(p, m)| (p.as_str().trim_start_matches('/').to_owned(), m))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_of_urls() {
        assert_eq!(host_of("http://192.168.1.41:32400"), Some("192.168.1.41"));
        assert_eq!(host_of("https://[fd00::1]:32400/x"), Some("fd00::1"));
        assert_eq!(host_of("nas"), Some("nas"));
        assert_eq!(host_of("http://"), None);
    }

    #[test]
    fn plex_is_a_network_kind_with_a_secret() {
        assert_eq!("plex".parse::<SourceKind>().unwrap(), SourceKind::Plex);
        assert!(SourceKind::Plex.is_network());
        assert!(!SourceKind::Plex.is_open());
        assert!(!SourceKind::Plex.is_media_server());
    }
}
