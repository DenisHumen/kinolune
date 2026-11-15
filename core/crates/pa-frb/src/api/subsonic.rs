// SPDX-License-Identifier: MPL-2.0
//! Subsonic and OpenSubsonic music servers (NET-016) for Dart.
//!
//! A Subsonic source is a source like the others (`CoreSources`, kind
//! `subsonic`, with a user name and password, or no user name and an
//! OpenSubsonic API key): its songs are scanned into the music library
//! and play through the stream proxy. [`CoreSubsonic`] adds what only a
//! Subsonic server has:
//!
//! - checking the address and the credentials before the source is added;
//! - scrobbles of the songs the music player plays ("now playing" and
//!   "played"), by library item id.
//!
//! Calls for items of other sources do nothing, so the app can call them
//! for every track. Every method blocks on the network.

use std::sync::Arc;

use pa_catalog as c;
use pa_secrets::SecretString;
use pa_sources as s;

use super::library::CoreLibrary;
use super::sources::SourcesError;

type SubResult<T> = std::result::Result<T, SourcesError>;

/// What a Subsonic server says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSubsonicServer {
    /// Subsonic API version (`1.16.1`).
    pub api_version: String,
    /// Server software (`navidrome`, …); empty for a plain Subsonic
    /// server.
    pub server_type: String,
    /// Version of the server software.
    pub server_version: String,
    /// Whether it speaks OpenSubsonic.
    pub open_subsonic: bool,
}

/// Subsonic actions over the sources of one library.
#[flutter_rust_bridge::frb(opaque)]
pub struct CoreSubsonic {
    catalog: Arc<c::Catalog>,
    sources: Arc<s::SourceManager>,
}

impl CoreSubsonic {
    /// Opens the Subsonic actions for `library`, with the secret store
    /// whose fallback file is in `data_dir` (the same as for
    /// `CoreSources.open`).
    pub fn open(library: &CoreLibrary, data_dir: String) -> SubResult<CoreSubsonic> {
        let catalog = library.catalog();
        let opened =
            crate::shared::secret_store(std::path::Path::new(&data_dir)).map_err(s::Error::from)?;
        let sources = s::SourceManager::open(catalog.path(), Arc::clone(&opened.store))?;
        Ok(Self {
            catalog,
            sources: Arc::new(sources),
        })
    }

    /// Checks that `address` is a Subsonic server and that the
    /// credentials work: a user name and password, or, with no user name,
    /// an OpenSubsonic API key in `secret`. Adds nothing.
    pub fn probe(
        &self,
        address: String,
        username: Option<String>,
        secret: String,
    ) -> SubResult<CoreSubsonicServer> {
        let info = s::subsonic_probe(&address, username.as_deref(), &SecretString::new(secret))?;
        Ok(CoreSubsonicServer {
            api_version: info.version,
            server_type: info.server_type,
            server_version: info.server_version,
            open_subsonic: info.open_subsonic,
        })
    }

    /// Scrobbles library track `item_id` if it is on a Subsonic source:
    /// `submission = false` says it is playing now, `true` that it was
    /// played (at `time_ms`, Unix ms; default now). Returns whether a
    /// scrobble was sent.
    pub fn scrobble(
        &self,
        item_id: String,
        submission: bool,
        time_ms: Option<i64>,
    ) -> SubResult<bool> {
        let Ok(id) = item_id.parse::<c::ItemRef>() else {
            return Ok(false);
        };
        if matches!(id, c::ItemRef::Show(_)) {
            return Ok(false);
        }
        let item = self.catalog.item(id).map_err(|e| SourcesError {
            code: "other".into(),
            message: e.to_string(),
        })?;
        let Some(item) = item else {
            return Ok(false);
        };
        Ok(self
            .sources
            .subsonic_scrobble(item.source_id, &item.rel_path, submission, time_ms)?)
    }
}
