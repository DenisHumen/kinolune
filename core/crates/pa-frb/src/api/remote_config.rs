// SPDX-License-Identifier: MPL-2.0
//! Remote config (`pa-remote-config`, CORE-014) for Dart: signed flags and
//! kill-switches from the public repo, with disk cache and built-in copy.
//!
//! Dart opens one [`CoreRemoteConfig`] at start (no network), reads flags
//! synchronously and calls [`CoreRemoteConfig::refresh_if_due`] from time to
//! time (app start, resume); errors there never change the flags in use.

use std::collections::HashMap;
use std::sync::Arc;

use flutter_rust_bridge::frb;
use pa_remote_config as rc;

/// `User-Agent` of config downloads.
const USER_AGENT: &str = concat!(
    "Kinolune/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/DenisHumen/kinolune)"
);

/// Error of a refresh; Dart receives it as an exception. The flags in use
/// stay as they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteConfigError {
    /// Stable code (CORE-009): `network` (offline), `disabled` (no trusted
    /// key in this build), `http`, `bad_signature` (also a bad key),
    /// `invalid` (also an unsupported schema), `rollback` (older than the
    /// document in use) or `io` (the cache).
    pub code: String,
    /// Detail for logs (no secrets; only public URLs are involved).
    pub message: String,
    /// No network: try again later.
    pub offline: bool,
    /// Remote updates are off in this build (no production key yet).
    pub disabled: bool,
}

impl From<rc::Error> for RemoteConfigError {
    fn from(e: rc::Error) -> Self {
        let code = match &e {
            rc::Error::Offline(_) => "network",
            rc::Error::NoTrustedKey => "disabled",
            rc::Error::Http { .. } => "http",
            rc::Error::BadKey(_) | rc::Error::BadSignature(_) => "bad_signature",
            rc::Error::Invalid(_) | rc::Error::UnsupportedSchema(_) => "invalid",
            rc::Error::Rollback { .. } => "rollback",
            rc::Error::Io(_) => "io",
        };
        Self {
            code: code.into(),
            offline: matches!(e, rc::Error::Offline(_)),
            disabled: matches!(e, rc::Error::NoTrustedKey),
            message: e.to_string(),
        }
    }
}

/// Where the flags in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteConfigSource {
    BuiltIn,
    Cache,
    Remote,
}

/// State of the remote config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteConfigStatus {
    pub source: RemoteConfigSource,
    /// `serial` of the document in use.
    pub serial: u64,
    /// False while the build has no production key: built-in copy only.
    pub remote_enabled: bool,
    /// Unix seconds of the last successful check.
    pub checked_at: Option<u64>,
    /// Unix seconds of the last failed check after it.
    pub failed_at: Option<u64>,
}

/// Result of a refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteConfigRefresh {
    /// A newer document is in use now.
    Updated,
    /// Nothing new.
    Unchanged,
    /// Not time yet (refresh policy).
    NotDue,
}

/// Twitch player to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwitchPlayer {
    Native,
    Embed,
}

/// Twitch flags resolved for this app (flags minus kill-switches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchFlags {
    pub gql_playback: bool,
    pub chapters_gql: bool,
    pub chat_replay: bool,
    pub archive: bool,
    pub player: TwitchPlayer,
    pub gql_client_id: Option<String>,
    pub persisted_queries: HashMap<String, String>,
}

/// The app's remote config client.
#[frb(opaque)]
pub struct CoreRemoteConfig {
    inner: Arc<rc::RemoteConfigClient>,
    context: rc::ClientContext,
}

impl CoreRemoteConfig {
    /// Opens the client with its cache in `cache_dir` (e.g.
    /// `<app support>/remote-config`). `app_version` is the app's semver,
    /// `platform` the lowercase OS name (`android`, `linux`, `ios`, …): both
    /// only select kill-switches locally and are never sent anywhere.
    /// Does not touch the network.
    ///
    /// Debug builds only: with `KINOLUNE_DEBUG_REMOTE_CONFIG=<file>` the
    /// app runs on that **unsigned** document instead of its built-in copy,
    /// without cache or network (TWI-030: checking a new version before the
    /// owner signs it). Release builds compile this out.
    #[frb(sync)]
    pub fn open(cache_dir: String, app_version: String, platform: String) -> CoreRemoteConfig {
        let options = rc::ClientOptions::new(cache_dir);
        let fetcher = Arc::new(rc::UreqFetcher::new(USER_AGENT));
        let client = match debug_unsigned_copy() {
            Some(config) => {
                rc::RemoteConfigClient::open_with_builtin(None, options, fetcher, config)
            }
            None => {
                rc::RemoteConfigClient::open(rc::TrustedKey::production().ok(), options, fetcher)
            }
        };
        Self {
            inner: Arc::new(client),
            context: rc::ClientContext {
                app_version,
                platform: platform.to_lowercase(),
            },
        }
    }

    /// Downloads a newer document if the refresh policy says it is time.
    pub fn refresh_if_due(&self) -> Result<RemoteConfigRefresh, RemoteConfigError> {
        Ok(self.inner.refresh_if_due()?.into())
    }

    /// Downloads a newer document now (e.g. a "check now" button).
    pub fn refresh(&self) -> Result<RemoteConfigRefresh, RemoteConfigError> {
        Ok(self.inner.refresh()?.into())
    }

    /// Source, serial and check times.
    #[frb(sync)]
    pub fn status(&self) -> RemoteConfigStatus {
        let s = self.inner.snapshot();
        RemoteConfigStatus {
            source: match s.source {
                rc::ConfigSource::BuiltIn => RemoteConfigSource::BuiltIn,
                rc::ConfigSource::Cache => RemoteConfigSource::Cache,
                rc::ConfigSource::Remote => RemoteConfigSource::Remote,
            },
            serial: s.config.serial,
            remote_enabled: self.inner.remote_enabled(),
            checked_at: s.checked_at,
            failed_at: s.failed_at,
        }
    }

    /// Flag `feature` is on and not killed for this app. Missing: off.
    #[frb(sync)]
    pub fn is_enabled(&self, feature: String) -> bool {
        self.inner.config().is_enabled(&feature, &self.context)
    }

    /// A kill-switch turns `feature` off for this app (for features that
    /// are on by default and only need an emergency stop).
    #[frb(sync)]
    pub fn is_killed(&self, feature: String) -> bool {
        self.inner.config().is_killed(&feature, &self.context)
    }

    /// The minisign key that signs app releases (`updates.release_public_key`
    /// of the signed config, PKG-018); None while the owner has not
    /// published one. `update_verify_artifact` takes it.
    #[frb(sync)]
    pub fn release_public_key(&self) -> Option<String> {
        self.inner.config().release_public_key().map(str::to_owned)
    }

    /// The config in use and this client, for other core objects
    /// (`CoreTwitchPlayback`, TWI-014).
    #[frb(ignore)]
    pub(crate) fn config_and_context(&self) -> (Arc<rc::RemoteConfig>, rc::ClientContext) {
        (self.inner.config(), self.context.clone())
    }

    /// Twitch module settings (TWI-004).
    #[frb(sync)]
    pub fn twitch(&self) -> CoreTwitchFlags {
        let t = self.inner.config().twitch_flags(&self.context);
        CoreTwitchFlags {
            gql_playback: t.gql_playback,
            chapters_gql: t.chapters_gql,
            chat_replay: t.chat_replay,
            archive: t.archive,
            player: match t.player {
                rc::PlayerMode::Native => TwitchPlayer::Native,
                rc::PlayerMode::Embed => TwitchPlayer::Embed,
            },
            gql_client_id: t.gql_client_id,
            persisted_queries: t.persisted_queries.into_iter().collect(),
        }
    }
}

/// Environment variable naming an unsigned config document for debug
/// builds (see [`CoreRemoteConfig::open`]).
const DEBUG_REMOTE_CONFIG_ENV: &str = "KINOLUNE_DEBUG_REMOTE_CONFIG";

/// The unsigned document of [`DEBUG_REMOTE_CONFIG_ENV`]; always None in
/// release builds. A file that is missing or invalid is ignored (logged).
fn debug_unsigned_copy() -> Option<rc::RemoteConfig> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let path = std::env::var_os(DEBUG_REMOTE_CONFIG_ENV)?;
    read_unsigned(std::path::Path::new(&path))
}

fn read_unsigned(path: &std::path::Path) -> Option<rc::RemoteConfig> {
    match std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|b| rc::RemoteConfig::parse(&b).map_err(|e| e.to_string()))
    {
        Ok(config) => {
            eprintln!(
                "remote config: DEBUG unsigned copy {} (serial {})",
                path.display(),
                config.serial
            );
            Some(config)
        }
        Err(e) => {
            eprintln!("remote config: debug copy {} ignored: {e}", path.display());
            None
        }
    }
}

impl From<rc::RefreshOutcome> for RemoteConfigRefresh {
    fn from(o: rc::RefreshOutcome) -> Self {
        match o {
            rc::RefreshOutcome::Updated { .. } => Self::Updated,
            rc::RefreshOutcome::Unchanged => Self::Unchanged,
            rc::RefreshOutcome::NotDue => Self::NotDue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_unsigned_next_copy_reads_for_debug_runs() {
        // TWI-030: what `KINOLUNE_DEBUG_REMOTE_CONFIG` points debug runs at.
        let next = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tools/remote-config/remote-config.next.json");
        let config = read_unsigned(&next).expect("valid unsigned next copy");
        assert!(config.serial > rc::builtin().serial);
        let dir = tempfile::tempdir().unwrap();
        assert!(read_unsigned(&dir.path().join("missing.json")).is_none());
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, b"{ \"schema\": 9, \"serial\": 1 }").unwrap();
        assert!(read_unsigned(&bad).is_none());
        // Without the variable a debug run stays on the built-in copy.
        if std::env::var_os(DEBUG_REMOTE_CONFIG_ENV).is_none() {
            assert!(debug_unsigned_copy().is_none());
        }
    }

    #[test]
    fn opens_on_the_builtin_copy_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let config = CoreRemoteConfig::open(
            dir.path().to_string_lossy().into(),
            "0.1.0".into(),
            "Linux".into(),
        );
        let status = config.status();
        assert_eq!(status.source, RemoteConfigSource::BuiltIn);
        if !rc::production_key_is_placeholder() {
            return;
        }
        // No production key yet: remote updates are off and refresh says so
        // without touching the network.
        assert!(!status.remote_enabled);
        assert_eq!(config.refresh_if_due(), Ok(RemoteConfigRefresh::NotDue));
        let err = config.refresh().unwrap_err();
        assert!(err.disabled && !err.offline);
        assert!(!config.is_enabled("twitch.gql_playback".into()));
        assert!(!config.is_killed("twitch.gql_playback".into()));
        let twitch = config.twitch();
        assert_eq!(twitch.player, TwitchPlayer::Embed);
        assert!(!twitch.gql_playback && twitch.gql_client_id.is_none());
    }
}
