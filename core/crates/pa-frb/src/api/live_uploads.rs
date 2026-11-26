// SPDX-License-Identifier: MPL-2.0
//! Library uploads visible on every device (LIB-028, ADR-NNNN) for Dart:
//! the durable steps of an upload (the grey card), the guess before
//! copying and the provider search behind «Не то». Live progress is on
//! `CoreSync` (`upload_report`, `upload_watch`, `uploads`).

use pa_catalog as c;
use pa_catalog::pa_sync::UploadPhase;

use crate::api::library::{CoreLibrary, CoreMetadataSettings, LibraryError};
use crate::api::review::CoreMatchCandidate;

type Result<T> = std::result::Result<T, LibraryError>;

/// A durable step of a library upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreUploadPhase {
    /// Dropped and queued: the grey card appears.
    Started,
    /// Recognized (before copying, or corrected with «Не то»).
    Recognized,
    /// In the library: the card turns into the item.
    Done,
    /// Cancelled: the card goes away.
    Cancelled,
}

impl From<CoreUploadPhase> for UploadPhase {
    fn from(p: CoreUploadPhase) -> Self {
        match p {
            CoreUploadPhase::Started => Self::Started,
            CoreUploadPhase::Recognized => Self::Recognized,
            CoreUploadPhase::Done => Self::Done,
            CoreUploadPhase::Cancelled => Self::Cancelled,
        }
    }
}

/// An upload a card is shown for (this device's or another's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLiveUpload {
    pub upload_id: String,
    pub phase: CoreUploadPhase,
    /// The app's JSON of the upload (name, size, kind, guess, source,
    /// folder, device name).
    pub info: String,
    /// The device that uploads (`local` while sync is off).
    pub device_id: String,
    pub updated_ms: i64,
}

/// What the library will make of a file before it is copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreUploadGuess {
    /// `movie`, `show`, `anime`, or empty when the name says nothing.
    pub kind: String,
    pub title: String,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    /// `0..=100`.
    pub confidence: u8,
}

fn int(v: Option<i64>) -> Option<i32> {
    v.and_then(|v| i32::try_from(v).ok())
}

/// Recognition before copying (LIB-028): what the scan will make of a
/// file at `dest` (its path relative to the source root, where it is
/// going; a folder ends with `/`). Instant, no network.
#[flutter_rust_bridge::frb(sync)]
pub fn guess_upload(dest: String) -> CoreUploadGuess {
    let g = c::guess_upload(&dest);
    CoreUploadGuess {
        kind: g.kind,
        title: g.title,
        year: int(g.year),
        season: int(g.season),
        episode: int(g.episode),
        confidence: g.confidence,
    }
}

impl CoreLibrary {
    /// Records a step of an upload of this device and sends it to the
    /// family at once while sync is on.
    pub fn record_upload_event(
        &self,
        upload_id: String,
        phase: CoreUploadPhase,
        info: String,
    ) -> Result<()> {
        self.catalog()
            .record_upload_event(&upload_id, phase.into(), &info)?;
        self.flush_sync();
        Ok(())
    }

    /// The uploads to show cards for: those on their way and those that
    /// ended in the last day.
    pub fn live_uploads(&self) -> Result<Vec<CoreLiveUpload>> {
        Ok(self
            .catalog()
            .live_uploads()?
            .into_iter()
            .filter_map(|u| {
                let phase = match u.phase {
                    UploadPhase::Started => CoreUploadPhase::Started,
                    UploadPhase::Recognized => CoreUploadPhase::Recognized,
                    UploadPhase::Done => CoreUploadPhase::Done,
                    UploadPhase::Cancelled => CoreUploadPhase::Cancelled,
                    UploadPhase::Other => return None,
                };
                Some(CoreLiveUpload {
                    upload_id: u.upload_id,
                    phase,
                    info: u.info,
                    device_id: u.device_id,
                    updated_ms: u.updated_ms,
                })
            })
            .collect())
    }

    /// Provider results for a file not in the library yet (the poster of
    /// the guess, «Не то»): `kind` is `movie`, `show` or `anime`. Writes
    /// nothing.
    pub fn upload_search(
        &self,
        settings: CoreMetadataSettings,
        kind: String,
        title: String,
        year: Option<i32>,
    ) -> Result<Vec<CoreMatchCandidate>> {
        let providers = crate::api::review::review_providers(&settings)?;
        Ok(self
            .catalog()
            .guess_search(&providers, &kind, &title, year.map(i64::from))?
            .into_iter()
            .map(Into::into)
            .collect())
    }
}
