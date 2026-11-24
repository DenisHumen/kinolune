// SPDX-License-Identifier: MPL-2.0
//! Offline copies (NET-019) for Dart: "Download for offline" of a movie,
//! an episode, a season, an album or a playlist, the downloads screen and
//! its settings.
//!
//! [`CoreOffline`] runs `pa-offline` on the shared job queue of the
//! library database (`pa-jobs` + `pa-storage`, the same queue as Twitch
//! downloads). Copies go to the `offline` storage target in the folder the
//! app sets; the library plays them through its stream proxy instead of
//! the network file (`pa_catalog::SourceOpener`), so nothing changes in
//! the item the app shows.

use std::path::PathBuf;
use std::sync::Arc;

use flutter_rust_bridge::frb;
use pa_catalog::ItemRef;
use pa_jobs::{Conditions, JobEvent, Network};
use pa_offline::{Download, DownloadOptions, Offline, Selection};

use super::library::CoreLibrary;
use crate::frb_generated::StreamSink;

/// Error of an offline call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreOfflineError {
    /// `no_folder`, `busy` (the folder is in use), `local` (already on the
    /// device), `unplayable`, `no_space` (can never fit), `not_found`,
    /// `invalid`, `jobs`, `storage`, `io`.
    pub code: String,
    pub message: String,
}

impl CoreOfflineError {
    fn new(code: &str, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
        }
    }
}

impl From<pa_offline::Error> for CoreOfflineError {
    fn from(e: pa_offline::Error) -> Self {
        use pa_offline::Error as E;
        let code = match &e {
            E::Busy(_) => "busy",
            E::Local(_) => "local",
            E::Unplayable(_) => "unplayable",
            E::NotFound(_) => "not_found",
            E::Invalid(m) if m.contains("folder") => "no_folder",
            E::Invalid(_) => "invalid",
            E::Jobs(_) => "jobs",
            E::Storage(pa_storage::Error::WontFit { .. }) => "no_space",
            E::Storage(_) => "storage",
            _ => "io",
        };
        Self::new(code, e)
    }
}

/// What to download: `item` (a movie, an episode or a track; a show means
/// all its episodes), `season` (`id` is the show, with `season`), `album`,
/// `playlist` (video) or `music_playlist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreOfflineTarget {
    pub kind: String,
    /// Item id (`m12`, `s3`, `e40`, `t7`) or a list id (`album:5`,
    /// `playlist:2`, or the bare number).
    pub id: String,
    pub season: Option<i64>,
    /// The version (`files.id`) of a single movie or episode.
    pub version_file_id: Option<i64>,
}

/// A download or a copy, for the downloads screen and the item pages.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreOfflineDownload {
    pub job_id: Option<i64>,
    pub copy_id: Option<i64>,
    /// Item id of the library.
    pub item_id: String,
    pub title: String,
    /// Version label when the item has several (`4K · HDR10`).
    pub version: Option<String>,
    /// `queued`, `waiting`, `running`, `paused`, `retrying`, `failed`,
    /// `cancelled`, `done`.
    pub state: String,
    pub bytes_done: i64,
    pub bytes_total: Option<i64>,
    /// Why it waits (`disk_reserve`, `quota`, …) or is paused (`user`,
    /// `network`, `resources`).
    pub waiting_for: Option<String>,
    pub error: Option<String>,
    pub watched: bool,
    pub created_at_ms: i64,
}

fn signed(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl From<Download> for CoreOfflineDownload {
    fn from(d: Download) -> Self {
        Self {
            job_id: d.job_id,
            copy_id: d.copy_id,
            item_id: d.item,
            title: d.title,
            version: d.version,
            state: d.state.as_str().to_owned(),
            bytes_done: signed(d.bytes_done),
            bytes_total: d.bytes_total.map(signed),
            waiting_for: d.waiting_for,
            error: d.error,
            watched: d.watched,
            created_at_ms: d.created_at_ms,
        }
    }
}

/// Settings and space of the offline folder.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreOfflineSettings {
    pub folder: Option<String>,
    pub wifi_only: bool,
    /// Delete watched copies this many days after watching (0: at the next
    /// cleanup); `None`: keep them.
    pub delete_watched_after_days: Option<u32>,
    /// Bytes of the copies.
    pub used_bytes: i64,
    /// Free space of the volume, if it is available.
    pub available_bytes: Option<i64>,
    pub total_bytes: Option<i64>,
}

/// A change of the downloads: refresh the list (`id` is the job), or the
/// progress of one job.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreOfflineEvent {
    pub job_id: i64,
    /// `changed` or `progress`.
    pub kind: String,
    pub bytes_done: i64,
    pub bytes_total: Option<i64>,
}

/// Offline copies of one library.
#[frb(opaque)]
pub struct CoreOffline {
    offline: Offline,
    queue: pa_jobs::JobQueue,
}

fn parse_id(id: &str, prefix: &str) -> Result<i64, CoreOfflineError> {
    id.strip_prefix(prefix)
        .unwrap_or(id)
        .parse()
        .map_err(|_| CoreOfflineError::new("invalid", format!("bad id {id:?}")))
}

fn item_ref(id: &str) -> Result<ItemRef, CoreOfflineError> {
    id.parse()
        .map_err(|_| CoreOfflineError::new("invalid", format!("bad item id {id:?}")))
}

impl CoreOffline {
    /// Opens offline copies on `library`'s database with copies in
    /// `folder` (created if needed; kept when copies exist in another
    /// one). Downloads left unfinished resume.
    pub fn open(library: &CoreLibrary, folder: String) -> Result<CoreOffline, CoreOfflineError> {
        let catalog = library.catalog();
        let jobs =
            crate::shared::jobs(catalog.path()).map_err(|e| CoreOfflineError::new("jobs", e))?;
        let offline = Offline::new(
            Arc::clone(&catalog),
            jobs.queue.clone(),
            jobs.storage.clone(),
        )?;
        match offline.set_folder(&PathBuf::from(&folder)) {
            Ok(()) | Err(pa_offline::Error::Busy(_)) => {}
            Err(e) => return Err(e.into()),
        }
        offline.register();
        let _ = jobs.storage.clean_tmp(&jobs.queue);
        Ok(Self {
            offline,
            queue: jobs.queue,
        })
    }

    /// Queues the copies of `target`; returns the new job ids (empty if
    /// everything is on the device or downloading already).
    pub fn download(&self, target: CoreOfflineTarget) -> Result<Vec<i64>, CoreOfflineError> {
        let selection = match target.kind.as_str() {
            "item" => Selection::Item(item_ref(&target.id)?),
            "season" => Selection::Season {
                show: item_ref(&target.id)?,
                season: target
                    .season
                    .ok_or_else(|| CoreOfflineError::new("invalid", "no season"))?,
            },
            "album" => Selection::Album(parse_id(&target.id, "album:")?),
            "playlist" => Selection::Playlist(parse_id(&target.id, "playlist:")?),
            "music_playlist" => Selection::MusicPlaylist(parse_id(&target.id, "playlist:")?),
            k => return Err(CoreOfflineError::new("invalid", format!("kind {k:?}"))),
        };
        Ok(self.offline.download(
            &selection,
            &DownloadOptions {
                version_file_id: target.version_file_id,
            },
        )?)
    }

    /// Downloads in the queue and copies on the device, newest first.
    pub fn downloads(&self) -> Result<Vec<CoreOfflineDownload>, CoreOfflineError> {
        Ok(self
            .offline
            .downloads()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The copy or download of one item, if any.
    pub fn item_state(
        &self,
        item_id: String,
    ) -> Result<Option<CoreOfflineDownload>, CoreOfflineError> {
        Ok(self.offline.state(&item_id)?.map(Into::into))
    }

    /// `pause`, `resume`, `cancel`, `retry` or `remove`.
    pub fn control(&self, job_id: i64, action: String) -> Result<(), CoreOfflineError> {
        Ok(self.offline.control(job_id, &action)?)
    }

    /// Deletes a copy now; returns the bytes freed.
    pub fn delete_copy(&self, copy_id: i64) -> Result<i64, CoreOfflineError> {
        Ok(signed(self.offline.delete_copy(copy_id)?))
    }

    /// Deletes the copies of an item and cancels its downloads.
    pub fn delete_item(&self, item_id: String) -> Result<i64, CoreOfflineError> {
        Ok(signed(self.offline.delete_item(&item_id)?))
    }

    pub fn settings(&self) -> Result<CoreOfflineSettings, CoreOfflineError> {
        let s = self.offline.settings()?;
        let usage = self.offline.usage()?;
        Ok(CoreOfflineSettings {
            folder: s.folder.map(|p| p.to_string_lossy().into_owned()),
            wifi_only: s.wifi_only,
            delete_watched_after_days: s.delete_watched_after_days,
            used_bytes: usage.as_ref().map_or(0, |u| signed(u.used)),
            available_bytes: usage.as_ref().and_then(|u| u.available).map(signed),
            total_bytes: usage.as_ref().and_then(|u| u.total).map(signed),
        })
    }

    /// Moves the offline folder (fails with `busy` while copies or
    /// downloads use the current one).
    pub fn set_folder(&self, folder: String) -> Result<(), CoreOfflineError> {
        Ok(self.offline.set_folder(&PathBuf::from(folder))?)
    }

    /// Wi-Fi only for new and unfinished downloads.
    pub fn set_wifi_only(&self, on: bool) -> Result<(), CoreOfflineError> {
        self.offline.set_wifi_only(on)?;
        Ok(())
    }

    /// "Delete watched copies" after `days` (0: at the next cleanup), or
    /// never.
    pub fn set_delete_watched(&self, days: Option<u32>) -> Result<(), CoreOfflineError> {
        Ok(self.offline.set_delete_watched(days)?)
    }

    /// Marks watched copies and applies the retention rules; returns how
    /// many copies went to the trash.
    pub fn cleanup(&self) -> Result<u32, CoreOfflineError> {
        let report = self.offline.cleanup()?;
        Ok(u32::try_from(report.trashed.len()).unwrap_or(u32::MAX))
    }

    /// The network of the device, for every job of the queue: `unmetered`
    /// (Wi-Fi, Ethernet), `metered` (mobile data) or `offline`.
    pub fn set_network(&self, network: String) -> Result<(), CoreOfflineError> {
        let network = match network.as_str() {
            "unmetered" => Network::Unmetered,
            "metered" => Network::Metered,
            "offline" => Network::Offline,
            n => return Err(CoreOfflineError::new("invalid", format!("network {n:?}"))),
        };
        let current = self.queue.conditions();
        self.queue.set_conditions(Conditions { network, ..current });
        Ok(())
    }

    /// Changes of offline downloads: a job was added, changed state or was
    /// removed (`changed`), or made progress (`progress`), until
    /// `core_stream_close(id)` (BUG-008). `id` comes from
    /// `core_stream_next_id`; in Dart use `CoreOffline.watch()` of pa_core.
    /// Sync: the subscription exists when this returns.
    #[frb(sync)]
    pub fn watch_listen(&self, id: i64, sink: StreamSink<CoreOfflineEvent>) {
        let rx = self.queue.subscribe();
        let kinds = self.queue.clone();
        let sink = crate::streams::Closable::open(id, sink);
        std::thread::spawn(move || {
            let ours = |id: i64| {
                kinds
                    .get(id)
                    .map(|j| j.kind == pa_offline::KIND)
                    // Removed rows: let the list refresh.
                    .unwrap_or(true)
            };
            while let Some(event) = sink.recv(&rx) {
                let out = match event {
                    JobEvent::Added { id, kind } | JobEvent::StateChanged { id, kind, .. } => {
                        (kind == pa_offline::KIND).then(|| CoreOfflineEvent {
                            job_id: id,
                            kind: "changed".into(),
                            bytes_done: 0,
                            bytes_total: None,
                        })
                    }
                    JobEvent::Removed { id } => Some(CoreOfflineEvent {
                        job_id: id,
                        kind: "changed".into(),
                        bytes_done: 0,
                        bytes_total: None,
                    }),
                    JobEvent::Progress {
                        id,
                        bytes_done,
                        bytes_total,
                        ..
                    } => ours(id).then(|| CoreOfflineEvent {
                        job_id: id,
                        kind: "progress".into(),
                        bytes_done: signed(bytes_done),
                        bytes_total: bytes_total.map(signed),
                    }),
                };
                if let Some(e) = out
                    && !sink.send(|s| s.add(e).is_ok())
                {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(parse_id("album:5", "album:").unwrap(), 5);
        assert_eq!(parse_id("7", "playlist:").unwrap(), 7);
        assert!(parse_id("x", "album:").is_err());
        assert_eq!(item_ref("m3").unwrap(), ItemRef::Movie(3));
    }

    #[test]
    fn errors_have_codes() {
        let e: CoreOfflineError = pa_offline::Error::Busy("copies").into();
        assert_eq!(e.code, "busy");
        let e: CoreOfflineError = pa_offline::Error::Invalid("no offline folder".into()).into();
        assert_eq!(e.code, "no_folder");
    }
}
