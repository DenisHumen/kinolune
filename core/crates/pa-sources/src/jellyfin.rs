// SPDX-License-Identifier: MPL-2.0
//! Jellyfin sources (NET-014): sign-in (password or Quick Connect) that
//! stores the access token, never the password; the cached
//! [`JellyfinFs`] tree of a source; and the server-side actions on a file
//! of the tree (playback plan, playback reports, favorite, "Continue
//! Watching", the user's state for import).

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

use pa_jellyfin::{
    Client, DeviceCapabilities, DeviceInfo, JellyfinFs, MediaRef, PlayMethod, PlaybackPlan,
    PlaybackReport, PlaybackRequest, PublicSystemInfo, QuickConnectState, Session, Token,
};
use pa_secrets::SecretString;
use pa_vfs::{ErrorKind, VfsError, VfsPath};

use crate::manager::Stored;
use crate::model::JellyfinOptions;
use crate::{
    Error, NewSource, Result, Source, SourceKind, SourceManager, SourceOptions, scrub,
};

/// Cached trees: source id → (settings and token they were built with,
/// tree).
pub(crate) type Trees = HashMap<i64, (String, Arc<JellyfinFs>)>;

fn server_error(e: &pa_jellyfin::Error) -> Error {
    let v = pa_jellyfin::vfs_error(e);
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

/// The main address first, then the others, without duplicates.
fn addresses(main: &str, alt: &[String]) -> Vec<String> {
    let mut all = vec![main.to_owned()];
    for a in alt {
        if let Ok(a) = pa_jellyfin::normalize_address(a)
            && !all.contains(&a)
        {
            all.push(a);
        }
    }
    all
}

/// A successful sign-in, ready for [`SourceManager::create_jellyfin`].
/// `Debug` hides the token.
#[derive(Debug, Clone)]
pub struct JellyfinAccount {
    /// Addresses, main first.
    pub addresses: Vec<String>,
    /// The server.
    pub server: PublicSystemInfo,
    /// User and token.
    pub session: Session,
    /// This device as the server knows it.
    pub device: DeviceInfo,
}

impl JellyfinAccount {
    /// Signs in to the server at `addresses` (main first) with a user name
    /// and password, as a new device named `device_name`. The password
    /// is used once and not kept.
    ///
    /// # Errors
    ///
    /// [`Error::Server`]: unreachable, wrong password
    /// ([`ErrorKind::AuthenticationFailed`]), …
    pub fn sign_in(
        addresses: &[String],
        device_name: &str,
        username: &str,
        password: &SecretString,
    ) -> Result<Self> {
        let (client, device) = new_client(addresses, device_name)?;
        let server = client.public_info().map_err(|e| server_error(&e))?;
        let session = client
            .sign_in(username, password.expose())
            .map_err(|e| server_error(&e))?;
        Ok(Self {
            addresses: client.addresses().to_vec(),
            server,
            session,
            device,
        })
    }
}

fn new_client(addresses: &[String], device_name: &str) -> Result<(Client, DeviceInfo)> {
    let device = DeviceInfo::kinolune(
        device_name.trim(),
        uuid::Uuid::now_v7().simple().to_string(),
    );
    let client = Client::new(addresses, device.clone()).map_err(|e| server_error(&e))?;
    Ok((client, device))
}

/// A Quick Connect sign-in in progress: show [`code`](Self::code), wait
/// for [`poll`](Self::poll) to say it was approved, then
/// [`finish`](Self::finish). `Debug` hides the request secret.
pub struct QuickConnect {
    client: Client,
    device: DeviceInfo,
    server: PublicSystemInfo,
    state: QuickConnectState,
}

impl std::fmt::Debug for QuickConnect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuickConnect")
            .field("client", &self.client)
            .field("code", &self.state.code)
            .field("authenticated", &self.state.authenticated)
            .finish_non_exhaustive()
    }
}

impl QuickConnect {
    /// Starts Quick Connect at the server at `addresses` as a new device
    /// named `device_name`.
    ///
    /// # Errors
    ///
    /// [`Error::Server`] if the server cannot be reached or has Quick
    /// Connect off.
    pub fn start(addresses: &[String], device_name: &str) -> Result<Self> {
        let (client, device) = new_client(addresses, device_name)?;
        let server = client.public_info().map_err(|e| server_error(&e))?;
        let state = client
            .quick_connect_start()
            .map_err(|e| server_error(&e))?;
        Ok(Self {
            client,
            device,
            server,
            state,
        })
    }

    /// The code to enter in a signed-in Jellyfin client.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.state.code
    }

    /// The server.
    #[must_use]
    pub fn server(&self) -> &PublicSystemInfo {
        &self.server
    }

    /// Whether the code has been approved.
    ///
    /// # Errors
    ///
    /// [`Error::Server`]; [`ErrorKind::NotFound`] once the code expired.
    pub fn poll(&mut self) -> Result<bool> {
        self.state = self
            .client
            .quick_connect_state(&self.state.secret)
            .map_err(|e| server_error(&e))?;
        Ok(self.state.authenticated)
    }

    /// Trades the approved code for a token.
    ///
    /// # Errors
    ///
    /// [`Error::Server`] with [`ErrorKind::AuthenticationFailed`] if not
    /// approved.
    pub fn finish(self) -> Result<JellyfinAccount> {
        let session = self
            .client
            .sign_in_quick_connect(&self.state.secret)
            .map_err(|e| server_error(&e))?;
        Ok(JellyfinAccount {
            addresses: self.client.addresses().to_vec(),
            server: self.server,
            session,
            device: self.device,
        })
    }
}

/// How a file of a Jellyfin source plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JellyfinPlayback {
    /// The server's plan.
    pub plan: PlaybackPlan,
    /// For DirectStream and Transcode: the HLS URL with the token as
    /// `api_key`, for the player. `None` for DirectPlay: play the file
    /// through the stream proxy. Contains a secret: do not log it.
    pub url: Option<String>,
}

/// A playback event for [`SourceManager::jellyfin_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportEvent {
    /// Playback started.
    Start,
    /// Position update.
    Progress,
    /// Playback ended.
    Stop,
}

impl SourceManager {
    /// Adds the signed-in Jellyfin server of `account` as a source named
    /// `name` (default: the server's name). The token goes to the secret
    /// store; `libraries` (ids) limits the source to those libraries.
    ///
    /// # Errors
    ///
    /// As [`create`](Self::create).
    pub fn create_jellyfin(
        &self,
        account: JellyfinAccount,
        name: Option<&str>,
        libraries: Vec<String>,
    ) -> Result<Source> {
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map_or_else(
                || {
                    let s = account.server.server_name.trim();
                    if s.is_empty() { "Jellyfin".to_owned() } else { s.to_owned() }
                },
                str::to_owned,
            );
        let (main, alt) = account
            .addresses
            .split_first()
            .ok_or_else(|| Error::Invalid("no server address".into()))?;
        let options = SourceOptions {
            jellyfin: Some(JellyfinOptions {
                user_id: account.session.user.id.clone(),
                server_id: account.session.server_id.clone(),
                device_id: account.device.device_id.clone(),
                device_name: account.device.device.clone(),
                alt_addresses: alt.to_vec(),
                libraries,
            }),
            ..SourceOptions::default()
        };
        self.create(
            NewSource::new(SourceKind::Jellyfin, name, main.as_str())
                .with_login(
                    account.session.user.name.clone(),
                    SecretString::new(account.session.token.expose()),
                )
                .with_options(options),
        )
    }

    /// The tree of a Jellyfin source, signed in with its stored token;
    /// reused while settings and token stay the same.
    pub(crate) fn jellyfin_tree(
        &self,
        stored: &Stored,
        token: Option<SecretString>,
    ) -> pa_vfs::Result<Arc<JellyfinFs>> {
        let s = &stored.source;
        let unauthorized =
            |m: &str| VfsError::new(ErrorKind::AuthenticationFailed, m.to_owned());
        let token = token.ok_or_else(|| unauthorized("not signed in to the Jellyfin server"))?;
        let o = s
            .options
            .jellyfin
            .clone()
            .filter(|o| !o.user_id.is_empty() && !o.device_id.is_empty())
            .ok_or_else(|| unauthorized("no Jellyfin user: sign in again"))?;
        let key = format!(
            "{}\n{}\n{}",
            s.uri,
            serde_json::to_string(&s.options).unwrap_or_default(),
            token.expose()
        );
        {
            let trees = self.jellyfin.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((k, fs)) = trees.get(&s.id)
                && *k == key
            {
                return Ok(Arc::clone(fs));
            }
        }
        let device = DeviceInfo::kinolune(o.device_name.clone(), o.device_id.clone());
        let client = Client::new(&addresses(&s.uri, &o.alt_addresses), device)
            .map_err(|e| pa_jellyfin::vfs_error(&e))?;
        client.set_token(Some(Token::new(token.expose())));
        let mut fs = JellyfinFs::new(Arc::new(client), o.user_id).with_libraries(o.libraries);
        if let Some(b) = s.options.read_ahead_bytes {
            fs = fs.with_read_ahead(usize::try_from(b).unwrap_or(usize::MAX));
        }
        let fs = Arc::new(fs);
        self.jellyfin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(s.id, (key, Arc::clone(&fs)));
        Ok(fs)
    }

    /// The Jellyfin tree of source `id`.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if the source is not a Jellyfin server;
    /// [`Error::Source`] if it has no valid sign-in.
    pub fn jellyfin(&self, id: i64) -> Result<Arc<JellyfinFs>> {
        let stored = self.stored(id)?;
        if stored.source.kind != SourceKind::Jellyfin {
            return Err(Error::Invalid(format!("source {id} is not a Jellyfin server")));
        }
        let token = self.password_of(&stored)?;
        self.jellyfin_tree(&stored, token)
            .map_err(|e| source_error(id, &e))
    }

    /// The media file at `rel_path` of Jellyfin source `id`; `None` if
    /// the source is of another kind or the path is not a media file.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn jellyfin_media(&self, id: i64, rel_path: &str) -> Result<Option<(Arc<JellyfinFs>, MediaRef)>> {
        if self.get(id)?.kind != SourceKind::Jellyfin {
            return Ok(None);
        }
        let fs = self.jellyfin(id)?;
        let path = VfsPath::parse(rel_path).map_err(|e| source_error(id, &e))?;
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

    /// How file `rel_path` of Jellyfin source `id` plays on a device with
    /// `capabilities`; `None` if it is not a Jellyfin media file.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached or refuses.
    pub fn jellyfin_playback(
        &self,
        id: i64,
        rel_path: &str,
        capabilities: DeviceCapabilities,
        start_ms: i64,
        force_transcode: bool,
    ) -> Result<Option<JellyfinPlayback>> {
        let Some((fs, m)) = self.jellyfin_media(id, rel_path)? else {
            return Ok(None);
        };
        let mut req = PlaybackRequest::new(&m.item_id, capabilities);
        req.media_source_id = Some(m.media_source_id.clone());
        req.start_ms = start_ms;
        req.force_transcode = force_transcode;
        let plan = fs
            .client()
            .playback_plan(fs.user_id(), &req)
            .map_err(|e| source_error(id, &pa_jellyfin::vfs_error(&e)))?;
        let url = (plan.method != PlayMethod::DirectPlay)
            .then(|| fs.client().url_with_key(&plan.stream_path));
        Ok(Some(JellyfinPlayback { plan, url }))
    }

    /// Reports playback of file `rel_path` of Jellyfin source `id`
    /// (`plan`: from [`jellyfin_playback`](Self::jellyfin_playback), if
    /// asked). Does nothing for other sources.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn jellyfin_report(
        &self,
        id: i64,
        rel_path: &str,
        event: ReportEvent,
        position_ms: i64,
        paused: bool,
        plan: Option<&PlaybackPlan>,
    ) -> Result<()> {
        let Some((fs, m)) = self.jellyfin_media(id, rel_path)? else {
            return Ok(());
        };
        let report = PlaybackReport {
            item_id: m.item_id.clone(),
            media_source_id: plan.map_or(m.media_source_id.clone(), |p| p.media_source_id.clone()),
            play_session_id: plan.and_then(|p| p.play_session_id.clone()),
            method: plan.map_or(PlayMethod::DirectPlay, |p| p.method),
            position_ms: position_ms.max(0),
            paused,
        };
        let c = fs.client();
        match event {
            ReportEvent::Start => c.report_start(&report),
            ReportEvent::Progress => c.report_progress(&report),
            ReportEvent::Stop => c.report_stopped(&report),
        }
        .map_err(|e| source_error(id, &pa_jellyfin::vfs_error(&e)))
    }

    /// Sets the user's favorite flag of file `rel_path` at the server.
    /// Does nothing for other sources.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn jellyfin_set_favorite(&self, id: i64, rel_path: &str, favorite: bool) -> Result<()> {
        let Some((fs, m)) = self.jellyfin_media(id, rel_path)? else {
            return Ok(());
        };
        fs.client()
            .set_favorite(fs.user_id(), &m.item_id, favorite)
            .map_err(|e| source_error(id, &pa_jellyfin::vfs_error(&e)))
    }

    /// Removes file `rel_path` from the server's Continue Watching. Does
    /// nothing for other sources.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn jellyfin_remove_from_resume(&self, id: i64, rel_path: &str) -> Result<()> {
        let Some((fs, m)) = self.jellyfin_media(id, rel_path)? else {
            return Ok(());
        };
        fs.client()
            .remove_from_resume(fs.user_id(), &m.item_id)
            .map_err(|e| source_error(id, &pa_jellyfin::vfs_error(&e)))
    }

    /// Marks file `rel_path` watched or not at the server. Does nothing for
    /// other sources.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn jellyfin_set_played(&self, id: i64, rel_path: &str, played: bool) -> Result<()> {
        let Some((fs, m)) = self.jellyfin_media(id, rel_path)? else {
            return Ok(());
        };
        fs.client()
            .set_played(fs.user_id(), &m.item_id, played)
            .map_err(|e| source_error(id, &pa_jellyfin::vfs_error(&e)))
    }

    /// Every media file of Jellyfin source `id` with the user's state as
    /// the server has it now (resume position, watched, favorite), paths
    /// relative to the source root without a leading `/`, like the
    /// catalog's `rel_path`.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the server cannot be reached.
    pub fn jellyfin_user_data(&self, id: i64) -> Result<Vec<(String, MediaRef)>> {
        let fs = self.jellyfin(id)?;
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
    fn addresses_main_first_without_duplicates() {
        assert_eq!(
            addresses(
                "http://a:8096",
                &["http://b:8096/".into(), "http://a:8096".into(), "bad".into()]
            ),
            ["http://a:8096", "http://b:8096"]
        );
    }
}
