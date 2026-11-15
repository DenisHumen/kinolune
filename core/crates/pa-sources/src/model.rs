// SPDX-License-Identifier: MPL-2.0
//! The source model: kinds, settings, options and status.

use std::fmt;
use std::str::FromStr;

use pa_secrets::SecretString;
use serde::{Deserialize, Serialize};

use crate::Error;

/// Kind of a source (`sources.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceKind {
    /// A folder on this device (`file://`).
    Local,
    /// An SMB2/3 share (`smb://`), backend `pa-vfs-smb`.
    Smb,
    /// A WebDAV collection (`http(s)://`), backend `pa-vfs-webdav`.
    WebDav,
    /// A Jellyfin server (`http(s)://`, NET-014): its libraries as files
    /// through `pa_jellyfin::JellyfinFs`. The secret is the access token of
    /// the sign-in, never the password.
    Jellyfin,
    /// An Emby server (`http(s)://`, NET-015): like [`Jellyfin`](Self::Jellyfin),
    /// through the same client with `pa_jellyfin::ServerKind::Emby`.
    Emby,
}

impl SourceKind {
    /// Value of `sources.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Smb => "smb",
            Self::WebDav => "webdav",
            Self::Jellyfin => "jellyfin",
            Self::Emby => "emby",
        }
    }

    /// A media server of the MediaBrowser family (Jellyfin, Emby): added
    /// by signing in, read through `pa_jellyfin::JellyfinFs`.
    pub fn is_media_server(self) -> bool {
        matches!(self, Self::Jellyfin | Self::Emby)
    }

    /// The `pa-jellyfin` server kind of a media server source.
    pub fn server_kind(self) -> Option<pa_jellyfin::ServerKind> {
        match self {
            Self::Jellyfin => Some(pa_jellyfin::ServerKind::Jellyfin),
            Self::Emby => Some(pa_jellyfin::ServerKind::Emby),
            _ => None,
        }
    }

    /// Whether the source can have a login.
    pub fn is_network(self) -> bool {
        !matches!(self, Self::Local)
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SourceKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "local" => Ok(Self::Local),
            "smb" => Ok(Self::Smb),
            "webdav" => Ok(Self::WebDav),
            "jellyfin" => Ok(Self::Jellyfin),
            "emby" => Ok(Self::Emby),
            other => Err(Error::Invalid(format!("unsupported source kind {other:?}"))),
        }
    }
}

/// Whether a source syncs to the other devices of the family (SYNC-010,
/// `sources.sync_mode`). Only network sources sync: a local folder path
/// means nothing on another device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SourceSync {
    /// Stays on this device (default).
    #[default]
    Off,
    /// Settings sync; each device keeps its own password.
    Settings,
    /// Settings and the password, the password sealed end-to-end with the
    /// family key: only paired devices can open it.
    WithPassword,
}

impl SourceSync {
    /// Value of `sources.sync_mode`.
    pub fn as_i64(self) -> i64 {
        match self {
            Self::Off => 0,
            Self::Settings => 1,
            Self::WithPassword => 2,
        }
    }

    /// From `sources.sync_mode`; unknown values are `Off`.
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Settings,
            2 => Self::WithPassword,
            _ => Self::Off,
        }
    }

    /// Name in sync ops and the FFI: `off`, `settings`, `password`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Settings => "settings",
            Self::WithPassword => "password",
        }
    }
}

impl FromStr for SourceSync {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "" | "off" => Ok(Self::Off),
            "settings" => Ok(Self::Settings),
            "password" => Ok(Self::WithPassword),
            other => Err(Error::Invalid(format!("sync mode {other:?}"))),
        }
    }
}

/// SMB dialect to negotiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmbVersion {
    /// The highest the server offers (default).
    #[default]
    Any,
    /// Any SMB 2.x.
    Smb2,
    /// Any SMB 3.x (needed for encryption).
    Smb3,
}

/// SMB3 encryption policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmbEncryption {
    /// Encrypt when the server or share requires it (default).
    #[default]
    Auto,
    /// Always encrypt; fail if the server cannot.
    Required,
    /// Never encrypt.
    Off,
}

/// Backend options of a source (`sources.options_json`). Unknown fields are
/// ignored on read, so newer builds can add options. Never holds secrets.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceOptions {
    /// SMB: NTLM domain (workgroup) of the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smb_domain: Option<String>,
    /// SMB: dialect.
    pub smb_version: SmbVersion,
    /// SMB: require message signing.
    pub smb_signing_required: bool,
    /// SMB: encryption policy.
    pub smb_encryption: SmbEncryption,
    /// WebDAV: accept exactly this certificate (`sha256:<64 hex>`), for a
    /// NAS with a self-signed certificate. `None`: the system CAs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_pin: Option<String>,
    /// Request timeout in seconds; `None`: the backend default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u32>,
    /// Read-ahead buffer in bytes; `None`: the backend default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_ahead_bytes: Option<u64>,
    /// Jellyfin and Emby: the signed-in user and this device (NET-014,
    /// NET-015).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jellyfin: Option<JellyfinOptions>,
}

/// Settings of a Jellyfin or Emby source; no secrets (the token is in the secret
/// store).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JellyfinOptions {
    /// Signed-in user id.
    pub user_id: String,
    /// Server id (the same on every address of the server).
    pub server_id: String,
    /// Device id the token belongs to.
    pub device_id: String,
    /// Device name shown in the server dashboard.
    pub device_name: String,
    /// Other addresses of the server (LAN, public), tried when the main
    /// one cannot be reached.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alt_addresses: Vec<String>,
    /// Only these libraries (ids); empty: all of the user's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,
}

/// A new source for [`SourceManager::create`](crate::SourceManager::create).
///
/// `Debug` does not print the password.
#[derive(Debug, Clone)]
pub struct NewSource {
    /// Backend kind.
    pub kind: SourceKind,
    /// Display name.
    pub name: String,
    /// Path or URI without credentials (see the module `address`).
    pub address: String,
    /// User name of a network source; `None` for anonymous or guest access.
    pub username: Option<String>,
    /// Password; stored in the secret store, never in the database.
    pub password: Option<SecretString>,
    /// Backend options.
    pub options: SourceOptions,
    /// Whether files of the source go to the library (default `true`).
    pub scan_into_library: bool,
    /// Whether the source is rescanned automatically (default `true`).
    pub auto_scan: bool,
    /// Whether the source is used at all (default `true`).
    pub enabled: bool,
    /// Sync to the other devices (default off).
    pub sync: SourceSync,
}

impl NewSource {
    /// An anonymous source with default options, scanned into the library
    /// automatically.
    pub fn new(kind: SourceKind, name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            address: address.into(),
            username: None,
            password: None,
            options: SourceOptions::default(),
            scan_into_library: true,
            auto_scan: true,
            enabled: true,
            sync: SourceSync::Off,
        }
    }

    /// Sets the sync mode.
    #[must_use]
    pub fn with_sync(mut self, sync: SourceSync) -> Self {
        self.sync = sync;
        self
    }

    /// Sets the user name and password.
    #[must_use]
    pub fn with_login(mut self, username: impl Into<String>, password: SecretString) -> Self {
        self.username = Some(username.into());
        self.password = Some(password);
        self
    }

    /// Sets the backend options.
    #[must_use]
    pub fn with_options(mut self, options: SourceOptions) -> Self {
        self.options = options;
        self
    }
}

/// What to do with the stored password in an update.
#[derive(Debug, Clone, Default)]
pub enum PasswordChange {
    /// Leave it as is (default).
    #[default]
    Keep,
    /// Replace it.
    Set(SecretString),
    /// Delete it (anonymous access).
    Clear,
}

/// Changes for [`SourceManager::update`](crate::SourceManager::update);
/// `None` fields are left as they are. The kind cannot change.
#[derive(Debug, Clone, Default)]
pub struct SourceUpdate {
    /// New display name.
    pub name: Option<String>,
    /// New address (same kind).
    pub address: Option<String>,
    /// New user name; `Some(None)` removes it.
    pub username: Option<Option<String>>,
    /// Password change.
    pub password: PasswordChange,
    /// New backend options (replace the old ones).
    pub options: Option<SourceOptions>,
    /// New "scan into library" toggle.
    pub scan_into_library: Option<bool>,
    /// New auto-scan toggle.
    pub auto_scan: Option<bool>,
    /// New enabled toggle.
    pub enabled: Option<bool>,
    /// New sync mode.
    pub sync: Option<SourceSync>,
}

/// Result of the last speed test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedSample {
    /// Sequential read throughput; `None` if the source had no file to read.
    pub bytes_per_sec: Option<u64>,
    /// Round trip of one metadata request.
    pub latency_ms: u64,
    /// When it was measured, Unix ms.
    pub measured_at_ms: i64,
}

/// Device-local state of a source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceStatus {
    /// Result of the last check; `None` if never checked.
    pub online: Option<bool>,
    /// When it was last checked, Unix ms.
    pub checked_at_ms: Option<i64>,
    /// Start of the last scan.
    pub last_scan_started_at_ms: Option<i64>,
    /// End of the last scan; `None` while a scan runs or if none finished.
    pub last_scan_finished_at_ms: Option<i64>,
    /// Last connection or scan error, secrets masked.
    pub last_error: Option<String>,
    /// When the last error happened.
    pub last_error_at_ms: Option<i64>,
    /// Last speed test.
    pub speed: Option<SpeedSample>,
}

impl SourceStatus {
    /// Whether a scan was started and has not finished.
    pub fn is_scanning(&self) -> bool {
        match (self.last_scan_started_at_ms, self.last_scan_finished_at_ms) {
            (Some(start), Some(end)) => start > end,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

/// A stored source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// `sources.id`.
    pub id: i64,
    /// Backend kind.
    pub kind: SourceKind,
    /// Display name.
    pub name: String,
    /// Normalized root URI, without credentials.
    pub uri: String,
    /// User name of a network source.
    pub username: Option<String>,
    /// Whether a password is stored in the secret store.
    pub has_password: bool,
    /// Backend options.
    pub options: SourceOptions,
    /// Whether files of the source go to the library.
    pub scan_into_library: bool,
    /// Whether the source is rescanned automatically.
    pub auto_scan: bool,
    /// Whether the source is used at all.
    pub enabled: bool,
    /// Whether and how it syncs to the other devices.
    pub sync: SourceSync,
    /// Display order.
    pub position: i64,
    /// Creation time, Unix ms.
    pub created_at_ms: i64,
    /// Last settings change, Unix ms.
    pub updated_at_ms: Option<i64>,
    /// Device-local status.
    pub status: SourceStatus,
}

impl Source {
    /// Whether an automatic scan should include this source.
    pub fn wants_auto_scan(&self) -> bool {
        self.enabled && self.scan_into_library && self.auto_scan
    }
}
