// SPDX-License-Identifier: MPL-2.0
//! Jellyfin and Emby servers (NET-014, NET-015) for Dart.
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

/// An item playing, for the reports.
struct Play {
    source_id: i64,
    rel_path: String,
    plan: Option<jf::PlaybackPlan>,
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

    /// Source and path of the file of library item `item_id`, if it is on
    /// a Jellyfin or Emby source.
    fn file_of(&self, item_id: &str) -> JfResult<Option<(i64, String)>> {
        let Ok(id) = item_id.parse::<c::ItemRef>() else {
            return Ok(None);
        };
        if matches!(id, c::ItemRef::Show(_)) {
            return Ok(None);
        }
        let Some(item) = self.catalog.item(id).map_err(catalog_error)? else {
            return Ok(None);
        };
        let kind = self.sources.get(item.source_id).map(|s| s.kind);
        Ok(kind
            .is_ok_and(s::SourceKind::is_media_server)
            .then_some((item.source_id, item.rel_path)))
    }

    /// How library item `item_id` plays on this device from `start_ms`;
    /// `None` if it is not on a Jellyfin or Emby server. `max_bitrate` (bit/s)
    /// makes the server transcode bigger files; `force_transcode` asks for
    /// a transcode in any case. Remembers the plan for the reports.
    pub fn playback(
        &self,
        item_id: String,
        start_ms: i64,
        max_bitrate: Option<u64>,
        force_transcode: bool,
    ) -> JfResult<Option<CoreJellyfinPlayback>> {
        let Some((source_id, rel_path)) = self.file_of(&item_id)? else {
            return Ok(None);
        };
        let mut caps = jf::DeviceCapabilities::libmpv();
        caps.max_bitrate = max_bitrate.filter(|b| *b > 0);
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
        self.plays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                item_id,
                Play {
                    source_id,
                    rel_path,
                    plan: Some(p.plan),
                    started: false,
                },
            );
        Ok(Some(out))
    }

    /// A position report of the player for library item `item_id`: the
    /// first one starts the playback at the server, the next ones update
    /// it. Does nothing for items of other sources.
    pub fn report(&self, item_id: String, position_ms: i64, paused: bool) -> JfResult<()> {
        let (source_id, rel_path, plan, first) = {
            let plays = self.plays.lock().unwrap_or_else(PoisonError::into_inner);
            match plays.get(&item_id) {
                Some(p) => (p.source_id, p.rel_path.clone(), p.plan.clone(), !p.started),
                None => {
                    drop(plays);
                    let Some((source_id, rel_path)) = self.file_of(&item_id)? else {
                        return Ok(());
                    };
                    (source_id, rel_path, None, true)
                }
            }
        };
        let event = if first {
            s::ReportEvent::Start
        } else {
            s::ReportEvent::Progress
        };
        self.sources.jellyfin_report(
            source_id,
            &rel_path,
            event,
            position_ms,
            paused,
            plan.as_ref(),
        )?;
        self.plays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                item_id,
                Play {
                    source_id,
                    rel_path,
                    plan,
                    started: true,
                },
            );
        Ok(())
    }

    /// The player closed at `position_ms`: the server stores the resume
    /// position (or "watched") and stops a transcode.
    pub fn stopped(&self, item_id: String, position_ms: i64) -> JfResult<()> {
        let play = self
            .plays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&item_id);
        let (source_id, rel_path, plan) = match play {
            Some(p) => (p.source_id, p.rel_path, p.plan),
            None => match self.file_of(&item_id)? {
                Some((s, r)) => (s, r, None),
                None => return Ok(()),
            },
        };
        Ok(self.sources.jellyfin_report(
            source_id,
            &rel_path,
            s::ReportEvent::Stop,
            position_ms,
            false,
            plan.as_ref(),
        )?)
    }

    /// Sets the favorite flag of item `item_id` at the server too.
    pub fn set_favorite(&self, item_id: String, favorite: bool) -> JfResult<()> {
        match self.file_of(&item_id)? {
            Some((id, rel)) => Ok(self.sources.jellyfin_set_favorite(id, &rel, favorite)?),
            None => Ok(()),
        }
    }

    /// Marks item `item_id` watched or not at the server too.
    pub fn set_watched(&self, item_id: String, watched: bool) -> JfResult<()> {
        match self.file_of(&item_id)? {
            Some((id, rel)) => Ok(self.sources.jellyfin_set_played(id, &rel, watched)?),
            None => Ok(()),
        }
    }

    /// Removes item `item_id` from the server's Continue Watching.
    pub fn remove_from_resume(&self, item_id: String) -> JfResult<()> {
        match self.file_of(&item_id)? {
            Some((id, rel)) => Ok(self.sources.jellyfin_remove_from_resume(id, &rel)?),
            None => Ok(()),
        }
    }

    /// Brings the server's state of source `source_id` into the library
    /// (after a scan): resume positions newer than the library's, watched
    /// marks and favorites. Files the scan has not matched yet are
    /// skipped.
    pub fn import_user_data(&self, source_id: i64) -> JfResult<CoreJellyfinImport> {
        let mut by_path: HashMap<String, c::ItemRef> = HashMap::new();
        let mut add = |items: Vec<c::Item>| {
            for i in items.into_iter().filter(|i| i.source_id == source_id) {
                by_path.entry(i.rel_path).or_insert(i.id);
            }
        };
        add(self.catalog.movies().map_err(catalog_error)?);
        add(self.catalog.tracks().map_err(catalog_error)?);
        for show in self.catalog.shows().map_err(catalog_error)? {
            add(self.catalog.episodes(show.id).map_err(catalog_error)?);
        }
        let mut report = CoreJellyfinImport::default();
        for (rel, media) in self.sources.jellyfin_user_data(source_id)? {
            let (Some(item), Some(data)) = (by_path.get(&rel).copied(), media.user_data) else {
                continue;
            };
            let at = data
                .last_played_date
                .as_deref()
                .and_then(jf::parse_date_ms)
                .unwrap_or(0);
            let local = self.catalog.progress(item).map_err(catalog_error)?;
            let newer = local.as_ref().is_none_or(|p| p.updated_at_ms < at);
            let duration = media.duration_ms.unwrap_or(0);
            if data.played && newer && !local.as_ref().is_some_and(|p| p.watched) {
                if self
                    .catalog
                    .save_progress(item, duration, duration, at.max(1), true)
                    .map_err(catalog_error)?
                {
                    report.watched += 1;
                }
            } else if data.playback_position_ticks > 0
                && newer
                && self
                    .catalog
                    .save_progress(item, data.position_ms(), duration, at.max(1), false)
                    .map_err(catalog_error)?
            {
                report.progress += 1;
            }
            if data.is_favorite
                && !matches!(item, c::ItemRef::Track(_))
                && !self.catalog.is_favorite(item).unwrap_or(true)
                && self.catalog.set_favorite(item, true).unwrap_or(false)
            {
                report.favorites += 1;
            }
        }
        Ok(report)
    }
}
