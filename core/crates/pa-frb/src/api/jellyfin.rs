// SPDX-License-Identifier: MPL-2.0
//! Jellyfin and Emby servers (NET-014, NET-015) for Dart — and, for what
//! happens while an item plays, Plex servers too (NET-013, ADR-0134).
//!
//! A Jellyfin or Emby source is a source like the others (`CoreSources`):
//! its libraries are scanned into the library and its files play through
//! the stream proxy. [`CoreJellyfin`] adds what only a media server has:
//!
//! - signing in with a password (both) or Quick Connect (Jellyfin); the
//!   access token goes to the secret store, the password is used once and
//!   dropped; `server` arguments are `jellyfin` or `emby`;
//! - how an item plays on this device (DirectPlay through the proxy, or a
//!   DirectStream/Transcode HLS URL);
//! - playback reports, favorites, "watched", removal from the server's
//!   Continue Watching, all by library item id;
//! - importing the server's resume positions, watched marks and favorites
//!   into the library.
//!
//! The item calls (playback, reports, "watched", Continue Watching,
//! importing the user's state) serve items of Plex sources as well: Plex
//! plays directly unless a transcode is asked for, reports are its
//! timeline, "watched" is its scrobble; Plex has no favorites. Signing in
//! to Plex is `CorePlex`.
//!
//! Calls for items of other sources do nothing (or return `None`), so the
//! app can call them for every item. Every method blocks on the network.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use flutter_rust_bridge::frb;
use pa_catalog as c;
use pa_secrets::SecretString;
use pa_sources as s;
use s::pa_jellyfin as jf;

use super::library::CoreLibrary;
use super::sources::{CoreManagedSource, SourcesError};

type JfResult<T> = std::result::Result<T, SourcesError>;

fn other(message: impl Into<String>) -> SourcesError {
    SourcesError {
        code: "other".into(),
        message: message.into(),
    }
}

fn catalog_error(e: c::Error) -> SourcesError {
    other(e.to_string())
}

/// A Jellyfin or Emby server, before signing in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreJellyfinServer {
    /// `jellyfin` or `emby`.
    pub kind: String,
    pub id: String,
    pub name: String,
    pub version: String,
    /// The LAN address the server reports (a candidate second address).
    pub local_address: Option<String>,
    pub quick_connect: bool,
}

/// A Quick Connect sign-in in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreQuickConnect {
    pub handle: u64,
    /// Six digits to enter in a signed-in Jellyfin app (Settings → Quick
    /// Connect).
    pub code: String,
    pub server_name: String,
}

/// How an item plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreJellyfinPlayback {
    /// `direct_play`, `direct_stream` or `transcode`.
    pub method: String,
    /// HLS URL for `direct_stream` and `transcode` (contains the token:
    /// do not log or show it); `None` for `direct_play`: play the item's
    /// stream URI.
    pub url: Option<String>,
    /// Why the server transcodes (`ContainerNotSupported`, …).
    pub reasons: Vec<String>,
}

/// A library of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreJellyfinLibrary {
    pub id: String,
    pub name: String,
    /// `movies`, `tvshows`, `music`, …
    pub collection_type: Option<String>,
}

/// What [`CoreJellyfin::import_user_data`] changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreJellyfinImport {
    pub progress: u32,
    pub watched: u32,
    pub favorites: u32,
}

/// The server of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaServer {
    /// Jellyfin or Emby.
    Jellyfin,
    /// Plex (NET-013).
    Plex,
}

/// An item playing, for the reports.
#[derive(Clone)]
struct Play {
    source_id: i64,
    rel_path: String,
    server: MediaServer,
    /// Jellyfin's plan.
    plan: Option<jf::PlaybackPlan>,
    /// Plex's transcode session.
    session: Option<String>,
    started: bool,
}

/// Jellyfin actions over the sources of one library.
#[frb(opaque)]
pub struct CoreJellyfin {
    catalog: Arc<c::Catalog>,
    sources: Arc<s::SourceManager>,
    pending: Mutex<HashMap<u64, s::QuickConnect>>,
    next: AtomicU64,
    plays: Mutex<HashMap<String, Play>>,
}

fn addresses(list: Vec<String>) -> Vec<String> {
    list.into_iter()
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
        .collect()
}

fn server_kind(server: &str) -> JfResult<jf::ServerKind> {
    server.parse().map_err(|e: jf::Error| SourcesError {
        code: "invalid".into(),
        message: e.to_string(),
    })
}

fn method_name(m: jf::PlayMethod) -> &'static str {
    match m {
        jf::PlayMethod::DirectPlay => "direct_play",
        jf::PlayMethod::DirectStream => "direct_stream",
        jf::PlayMethod::Transcode => "transcode",
    }
}

impl CoreJellyfin {
    /// Opens the Jellyfin actions for `library`, with the secret store
    /// whose fallback file is in `data_dir` (the same as for
    /// `CoreSources.open`).
    pub fn open(library: &CoreLibrary, data_dir: String) -> JfResult<CoreJellyfin> {
        let catalog = library.catalog();
        let opened =
            crate::shared::secret_store(std::path::Path::new(&data_dir)).map_err(s::Error::from)?;
        let sources = s::SourceManager::open(catalog.path(), Arc::clone(&opened.store))?;
        Ok(Self {
            catalog,
            sources: Arc::new(sources),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            plays: Mutex::new(HashMap::new()),
        })
    }

    /// Checks that `address` is a server of kind `server` (`jellyfin` or
    /// `emby`) and whether it offers Quick Connect. No sign-in.
    pub fn probe(&self, address: String, server: String) -> JfResult<CoreJellyfinServer> {
        let kind = server_kind(&server)?;
        let client = jf::Client::new(&[address], jf::DeviceInfo::kinolune("probe", "probe"))
            .map_err(|e| s::Error::Invalid(e.to_string()))?
            .with_kind(kind);
        let info = client.public_info().map_err(|e| {
            let v = jf::vfs_error(&e);
            s::Error::Server {
                kind: v.kind(),
                message: v.message().to_owned(),
            }
        })?;
        // Jellyfin names itself in `ProductName`, Emby sends none.
        let wrong = match kind {
            jf::ServerKind::Jellyfin => !info.is_jellyfin(),
            jf::ServerKind::Emby => !info.product_name.trim().is_empty(),
        };
        if wrong {
            let actual = if info.is_jellyfin() {
                "Jellyfin".to_owned()
            } else if info.product_name.trim().is_empty() {
                "Emby".to_owned()
            } else {
                info.product_name.clone()
            };
            return Err(SourcesError {
                code: "invalid".into(),
                message: format!("not a {} server: {actual}", kind.name()),
            });
        }
        Ok(CoreJellyfinServer {
            kind: kind.as_str().into(),
            quick_connect: client.quick_connect_enabled().unwrap_or(false),
            id: info.id,
            name: info.server_name,
            version: info.version,
            local_address: info.local_address,
        })
    }

    /// Signs in to a server of kind `server` (`jellyfin` or `emby`) with a
    /// user name and password and adds it as a source (`addresses`: the
    /// main one first, then others to try).
    #[allow(clippy::too_many_arguments)]
    pub fn sign_in(
        &self,
        server: String,
        addresses: Vec<String>,
        username: String,
        password: String,
        device_name: String,
        name: Option<String>,
        libraries: Vec<String>,
    ) -> JfResult<CoreManagedSource> {
        let account = s::JellyfinAccount::sign_in_to(
            server_kind(&server)?,
            &self::addresses(addresses),
            &device_name,
            username.trim(),
            &SecretString::new(password),
        )?;
        Ok(self
            .sources
            .create_jellyfin(account, name.as_deref(), libraries)?
            .into())
    }

    /// Starts a Quick Connect sign-in: show the code, then poll.
    pub fn quick_connect_start(
        &self,
        addresses: Vec<String>,
        device_name: String,
    ) -> JfResult<CoreQuickConnect> {
        let qc = s::QuickConnect::start(&self::addresses(addresses), &device_name)?;
        let handle = self.next.fetch_add(1, Ordering::Relaxed);
        let out = CoreQuickConnect {
            handle,
            code: qc.code().to_owned(),
            server_name: qc.server().server_name.clone(),
        };
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(handle, qc);
        Ok(out)
    }

    /// Whether the code of `handle` has been approved.
    pub fn quick_connect_poll(&self, handle: u64) -> JfResult<bool> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let qc = pending
            .get_mut(&handle)
            .ok_or_else(|| other("no such Quick Connect request"))?;
        Ok(qc.poll()?)
    }

    /// Finishes an approved Quick Connect sign-in and adds the source.
    pub fn quick_connect_finish(
        &self,
        handle: u64,
        name: Option<String>,
        libraries: Vec<String>,
    ) -> JfResult<CoreManagedSource> {
        let qc = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&handle)
            .ok_or_else(|| other("no such Quick Connect request"))?;
        let account = qc.finish()?;
        Ok(self
            .sources
            .create_jellyfin(account, name.as_deref(), libraries)?
            .into())
    }

    /// Forgets a Quick Connect request.
    #[frb(sync)]
    pub fn quick_connect_cancel(&self, handle: u64) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&handle);
    }

    /// Libraries of the signed-in user of source `source_id`.
    pub fn libraries(&self, source_id: i64) -> JfResult<Vec<CoreJellyfinLibrary>> {
        let fs = self.sources.jellyfin(source_id)?;
        let libs = fs
            .client()
            .libraries(fs.user_id())
            .map_err(|e| other(e.to_string()))?;
        Ok(libs
            .into_iter()
            .map(|l| CoreJellyfinLibrary {
                id: l.id,
                name: l.name,
                collection_type: l.collection_type,
            })
            .collect())
    }

    /// The stream URL of the server's own trailer of movie `item_id`
    /// (UI-036: second after the user's own trailer file); `None` if the
    /// item is not on a Jellyfin or Emby server or the server has none
    /// (Plex: none here). The URL carries the access token: do not log or
    /// show it.
    pub fn server_trailer(&self, item_id: String) -> JfResult<Option<String>> {
        let Some((source_id, rel_path, MediaServer::Jellyfin)) = self.file_of(&item_id)? else {
            return Ok(None);
        };
        let Some((fs, media)) = self.sources.jellyfin_media(source_id, &rel_path)? else {
            return Ok(None);
        };
        let client = fs.client();
        let trailers = client
            .local_trailers(fs.user_id(), &media.item_id)
            .map_err(|e| other(e.to_string()))?;
        Ok(trailers
            .into_iter()
            .find(|t| !t.id.is_empty())
            .map(|t| client.video_stream_url(&t.id)))
    }

    /// Source, path and server kind of the file of library item
    /// `item_id`, if it is on a Jellyfin, Emby or Plex source.
    fn file_of(&self, item_id: &str) -> JfResult<Option<(i64, String, MediaServer)>> {
        let Ok(id) = item_id.parse::<c::ItemRef>() else {
            return Ok(None);
        };
        if matches!(id, c::ItemRef::Show(_)) {
            return Ok(None);
        }
        let Some(item) = self.catalog.item(id).map_err(catalog_error)? else {
            return Ok(None);
        };
        let server = match self.sources.get(item.source_id).map(|s| s.kind) {
            Ok(s::SourceKind::Plex) => MediaServer::Plex,
            Ok(kind) if kind.is_media_server() => MediaServer::Jellyfin,
            _ => return Ok(None),
        };
        Ok(Some((item.source_id, item.rel_path, server)))
    }

    /// Whether library item `item_id` is on a media server (Jellyfin,
    /// Emby, Plex): the app offers "transcode on the server" for it
    /// (NET-013). Reads the library only.
    pub fn is_server_item(&self, item_id: String) -> JfResult<bool> {
        Ok(self.file_of(&item_id)?.is_some())
    }

    /// How library item `item_id` plays on this device from `start_ms`;
    /// `None` if it is not on a media server. `max_bitrate` (bit/s)
    /// makes the server transcode bigger files; `force_transcode` asks for
    /// a transcode in any case. Plex (NET-013) plays directly unless one
    /// of the two asks otherwise. Remembers the plan for the reports.
    pub fn playback(
        &self,
        item_id: String,
        start_ms: i64,
        max_bitrate: Option<u64>,
        force_transcode: bool,
    ) -> JfResult<Option<CoreJellyfinPlayback>> {
        let Some((source_id, rel_path, server)) = self.file_of(&item_id)? else {
            return Ok(None);
        };
        let max_bitrate = max_bitrate.filter(|b| *b > 0);
        let (out, plan, session) = match server {
            MediaServer::Jellyfin => {
                let mut caps = jf::DeviceCapabilities::libmpv();
                caps.max_bitrate = max_bitrate;
                let Some(p) = self.sources.jellyfin_playback(
                    source_id,
                    &rel_path,
                    caps,
                    start_ms,
                    force_transcode,
                )?
                else {
                    return Ok(None);
                };
                let out = CoreJellyfinPlayback {
                    method: method_name(p.plan.method).into(),
                    url: p.url,
                    reasons: p.plan.transcode_reasons.clone(),
                };
                (out, Some(p.plan), None)
            }
            MediaServer::Plex => {
                let kbps = max_bitrate.map(|b| u32::try_from(b / 1000).unwrap_or(u32::MAX));
                let quality = s::PlexQuality {
                    max_bitrate_kbps: kbps,
                    resolution: kbps.and_then(resolution_for),
                };
                let Some(p) =
                    self.sources
                        .plex_playback(source_id, &rel_path, force_transcode, quality)?
                else {
                    return Ok(None);
                };
                let out = CoreJellyfinPlayback {
                    method: if p.transcode {
                        "transcode"
                    } else {
                        "direct_play"
                    }
                    .into(),
                    url: p.url.clone(),
                    reasons: p.reasons.clone(),
                };
                (out, None, p.session)
            }
        };
        self.plays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                item_id,
                Play {
                    source_id,
                    rel_path,
                    server,
                    plan,
                    session,
                    started: false,
                },
            );
        Ok(Some(out))
    }

    /// Sends one report of the playing item to its server.
    fn send_report(
        &self,
        play: &Play,
        event: s::ReportEvent,
        position_ms: i64,
        paused: bool,
    ) -> JfResult<()> {
        match play.server {
            MediaServer::Jellyfin => self.sources.jellyfin_report(
                play.source_id,
                &play.rel_path,
                event,
                position_ms,
                paused,
                play.plan.as_ref(),
            )?,
            MediaServer::Plex => self.sources.plex_report(
                play.source_id,
                &play.rel_path,
                event,
                position_ms,
                paused,
                play.session.as_deref(),
            )?,
        }
        Ok(())
    }

    /// The play of `item_id` remembered by [`playback`](Self::playback),
    /// or a new one (direct play) for an item of a media server; `None`
    /// for items of other sources.
    fn play_of(&self, item_id: &str, take: bool) -> JfResult<Option<Play>> {
        let known = {
            let mut plays = self.plays.lock().unwrap_or_else(PoisonError::into_inner);
            if take {
                plays.remove(item_id)
            } else {
                plays.get(item_id).cloned()
            }
        };
        if known.is_some() {
            return Ok(known);
        }
        Ok(self
            .file_of(item_id)?
            .map(|(source_id, rel_path, server)| Play {
                source_id,
                rel_path,
                server,
                plan: None,
                session: None,
                started: false,
            }))
    }

    /// A position report of the player for library item `item_id`: the
    /// first one starts the playback at the server, the next ones update
    /// it (Plex: timeline `playing`/`paused`). Does nothing for items of
    /// other sources.
    pub fn report(&self, item_id: String, position_ms: i64, paused: bool) -> JfResult<()> {
        let Some(mut play) = self.play_of(&item_id, false)? else {
            return Ok(());
        };
        let event = if play.started {
            s::ReportEvent::Progress
        } else {
            s::ReportEvent::Start
        };
        self.send_report(&play, event, position_ms, paused)?;
        play.started = true;
        self.plays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(item_id, play);
        Ok(())
    }

    /// The player closed at `position_ms`: the server stores the resume
    /// position (or "watched") and stops a transcode.
    pub fn stopped(&self, item_id: String, position_ms: i64) -> JfResult<()> {
        match self.play_of(&item_id, true)? {
            Some(play) => self.send_report(&play, s::ReportEvent::Stop, position_ms, false),
            None => Ok(()),
        }
    }

    /// Sets the favorite flag of item `item_id` at the server too (Plex
    /// has no favorites: nothing to do).
    pub fn set_favorite(&self, item_id: String, favorite: bool) -> JfResult<()> {
        match self.file_of(&item_id)? {
            Some((id, rel, MediaServer::Jellyfin)) => {
                Ok(self.sources.jellyfin_set_favorite(id, &rel, favorite)?)
            }
            _ => Ok(()),
        }
    }

    /// Marks item `item_id` watched or not at the server too.
    pub fn set_watched(&self, item_id: String, watched: bool) -> JfResult<()> {
        match self.file_of(&item_id)? {
            Some((id, rel, MediaServer::Jellyfin)) => {
                Ok(self.sources.jellyfin_set_played(id, &rel, watched)?)
            }
            Some((id, rel, MediaServer::Plex)) => {
                Ok(self.sources.plex_set_played(id, &rel, watched)?)
            }
            None => Ok(()),
        }
    }

    /// Removes item `item_id` from the server's Continue Watching.
    pub fn remove_from_resume(&self, item_id: String) -> JfResult<()> {
        match self.file_of(&item_id)? {
            Some((id, rel, MediaServer::Jellyfin)) => {
                Ok(self.sources.jellyfin_remove_from_resume(id, &rel)?)
            }
            Some((id, rel, MediaServer::Plex)) => {
                Ok(self.sources.plex_remove_from_resume(id, &rel)?)
            }
            None => Ok(()),
        }
    }

    /// Brings the server's state of source `source_id` (Jellyfin, Emby or
    /// Plex) into the library (after a scan): resume positions newer than
    /// the library's, watched marks and favorites. Files the scan has not
    /// matched yet are skipped.
    pub fn import_user_data(&self, source_id: i64) -> JfResult<CoreJellyfinImport> {
        let kind = self.sources.get(source_id)?.kind;
        let states: Vec<c::ServerItemState> = if kind == s::SourceKind::Plex {
            self.sources
                .plex_user_data(source_id)?
                .into_iter()
                .map(|(rel_path, m)| c::ServerItemState {
                    rel_path,
                    position_ms: m.view_offset_ms.unwrap_or(0),
                    duration_ms: m.duration_ms.unwrap_or(0),
                    played: m.watched,
                    favorite: false,
                    at_ms: m.last_viewed_ms.unwrap_or(0),
                })
                .collect()
        } else if kind.is_media_server() {
            self.sources
                .jellyfin_user_data(source_id)?
                .into_iter()
                .filter_map(|(rel_path, media)| {
                    let data = media.user_data?;
                    Some(c::ServerItemState {
                        rel_path,
                        position_ms: data.position_ms(),
                        duration_ms: media.duration_ms.unwrap_or(0),
                        played: data.played,
                        favorite: data.is_favorite,
                        at_ms: data
                            .last_played_date
                            .as_deref()
                            .and_then(jf::parse_date_ms)
                            .unwrap_or(0),
                    })
                })
                .collect()
        } else {
            return Ok(CoreJellyfinImport::default());
        };
        let r = self
            .catalog
            .import_server_state(source_id, &states)
            .map_err(catalog_error)?;
        Ok(CoreJellyfinImport {
            progress: r.progress,
            watched: r.watched,
            favorites: r.favorites,
        })
    }
}

/// The largest picture for a transcode bound of `kbps` (NET-013): the
/// quality steps the app offers.
fn resolution_for(kbps: u32) -> Option<(u32, u32)> {
    Some(match kbps {
        0 => return None,
        1..=2_000 => (854, 480),
        2_001..=5_000 => (1280, 720),
        5_001..=12_000 => (1920, 1080),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_steps() {
        assert_eq!(resolution_for(1_500), Some((854, 480)));
        assert_eq!(resolution_for(4_000), Some((1280, 720)));
        assert_eq!(resolution_for(8_000), Some((1920, 1080)));
        assert_eq!(resolution_for(40_000), None);
        assert_eq!(resolution_for(0), None);
    }
}
