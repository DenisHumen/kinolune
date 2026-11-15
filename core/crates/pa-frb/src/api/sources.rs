// SPDX-License-Identifier: MPL-2.0
//! The source manager (`pa-sources`, NET-011) for Dart.
//!
//! [`CoreSources`] keeps local, SMB and WebDAV sources in the library
//! database and their passwords in the OS secret store (`pa-secrets`).
//! Passwords go from Dart to Rust only; no call returns one.
//!
//! On Android and iOS the app opens the platform secret store first
//! ([`secrets_open_platform`](super::secrets::secrets_open_platform),
//! CORE-015); [`CoreSources::open`] then uses it. Without an opened store
//! desktop uses the OS keychain or the encrypted fallback file in
//! `data_dir`, and mobile falls back to that file too. The store is shared
//! with [`CoreLibrary`](super::library::CoreLibrary), which reads the
//! passwords to scan and play network sources (LIB-018).
//!
//! Every method blocks (SQLite, keychain, network), so none is
//! `#[frb(sync)]`.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use flutter_rust_bridge::frb;
use pa_secrets::SecretString;
use pa_sources as s;

/// Error of a source manager call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcesError {
    /// `not_found`, `invalid`, `secrets`, `unreachable` or `other`.
    pub code: String,
    /// Human-readable message, secrets masked.
    pub message: String,
}

impl From<s::Error> for SourcesError {
    fn from(e: s::Error) -> Self {
        let code = match &e {
            s::Error::NotFound(_) => "not_found",
            s::Error::Invalid(_) => "invalid",
            s::Error::Secrets(_) => "secrets",
            s::Error::Source { .. } | s::Error::Server { .. } | s::Error::Probe { .. } => {
                "unreachable"
            }
            _ => "other",
        };
        Self {
            code: code.into(),
            message: e.to_string(),
        }
    }
}

type SourcesResult<T> = std::result::Result<T, SourcesError>;

fn invalid(message: String) -> SourcesError {
    SourcesError {
        code: "invalid".into(),
        message,
    }
}

/// Backend options; see `pa_sources::SourceOptions`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreSourceOptions {
    pub smb_domain: Option<String>,
    /// `any`, `smb2` or `smb3`.
    pub smb_version: String,
    pub smb_signing_required: bool,
    /// `auto`, `required` or `off`.
    pub smb_encryption: String,
    /// WebDAV and FTPS certificate pin, `sha256:<64 hex>`.
    pub tls_pin: Option<String>,
    /// SFTP host key, `ssh-ed25519 SHA256:<base64>` (NET-006): stored by the
    /// core on the first connect; the form shows it and sends it back, and
    /// an empty value lets the next connect learn the key again.
    pub ssh_host_key: Option<String>,
    pub timeout_secs: Option<u32>,
    pub read_ahead_bytes: Option<u64>,
    /// Jellyfin (NET-014): other addresses of the server, tried when the
    /// main one cannot be reached. The signed-in user stays as it is on
    /// update.
    pub jellyfin_alt_addresses: Vec<String>,
}

impl From<s::SourceOptions> for CoreSourceOptions {
    fn from(o: s::SourceOptions) -> Self {
        Self {
            smb_domain: o.smb_domain,
            smb_version: match o.smb_version {
                s::SmbVersion::Any => "any",
                s::SmbVersion::Smb2 => "smb2",
                s::SmbVersion::Smb3 => "smb3",
            }
            .into(),
            smb_signing_required: o.smb_signing_required,
            smb_encryption: match o.smb_encryption {
                s::SmbEncryption::Auto => "auto",
                s::SmbEncryption::Required => "required",
                s::SmbEncryption::Off => "off",
            }
            .into(),
            tls_pin: o.tls_pin,
            ssh_host_key: o.ssh_host_key,
            timeout_secs: o.timeout_secs,
            read_ahead_bytes: o.read_ahead_bytes,
            jellyfin_alt_addresses: o.jellyfin.map(|j| j.alt_addresses).unwrap_or_default(),
        }
    }
}

impl TryFrom<CoreSourceOptions> for s::SourceOptions {
    type Error = SourcesError;

    fn try_from(o: CoreSourceOptions) -> SourcesResult<Self> {
        Ok(Self {
            smb_domain: o.smb_domain.filter(|d| !d.is_empty()),
            smb_version: match o.smb_version.as_str() {
                "" | "any" => s::SmbVersion::Any,
                "smb2" => s::SmbVersion::Smb2,
                "smb3" => s::SmbVersion::Smb3,
                other => return Err(invalid(format!("SMB version {other:?}"))),
            },
            smb_signing_required: o.smb_signing_required,
            smb_encryption: match o.smb_encryption.as_str() {
                "" | "auto" => s::SmbEncryption::Auto,
                "required" => s::SmbEncryption::Required,
                "off" => s::SmbEncryption::Off,
                other => return Err(invalid(format!("SMB encryption {other:?}"))),
            },
            tls_pin: o.tls_pin.filter(|p| !p.is_empty()),
            ssh_host_key: o
                .ssh_host_key
                .map(|k| k.trim().to_owned())
                .filter(|k| !k.is_empty()),
            timeout_secs: o.timeout_secs,
            read_ahead_bytes: o.read_ahead_bytes,
            // Set by `CoreJellyfin` sign-ins and kept on update.
            jellyfin: None,
        })
    }
}

/// Device-local status of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSourceStatus {
    pub online: Option<bool>,
    pub checked_at_ms: Option<i64>,
    pub last_scan_started_at_ms: Option<i64>,
    pub last_scan_finished_at_ms: Option<i64>,
    pub scanning: bool,
    pub last_error: Option<String>,
    pub last_error_at_ms: Option<i64>,
    pub speed_bytes_per_sec: Option<u64>,
    pub speed_latency_ms: Option<u64>,
    pub speed_measured_at_ms: Option<i64>,
}

impl From<s::SourceStatus> for CoreSourceStatus {
    fn from(st: s::SourceStatus) -> Self {
        Self {
            scanning: st.is_scanning(),
            online: st.online,
            checked_at_ms: st.checked_at_ms,
            last_scan_started_at_ms: st.last_scan_started_at_ms,
            last_scan_finished_at_ms: st.last_scan_finished_at_ms,
            last_error: st.last_error,
            last_error_at_ms: st.last_error_at_ms,
            speed_bytes_per_sec: st.speed.and_then(|x| x.bytes_per_sec),
            speed_latency_ms: st.speed.map(|x| x.latency_ms),
            speed_measured_at_ms: st.speed.map(|x| x.measured_at_ms),
        }
    }
}

/// A stored source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreManagedSource {
    pub id: i64,
    /// `local`, `smb`, `webdav`, `sftp`, `ftp`, `jellyfin` or `emby`.
    pub kind: String,
    pub name: String,
    /// Root URI without credentials.
    pub uri: String,
    pub username: Option<String>,
    pub has_password: bool,
    pub options: CoreSourceOptions,
    pub scan_into_library: bool,
    pub auto_scan: bool,
    pub enabled: bool,
    /// Sync to the paired devices (SYNC-010): `off`, `settings` (without
    /// the password) or `password` (with it, sealed end-to-end).
    pub sync: String,
    pub status: CoreSourceStatus,
}

impl From<s::Source> for CoreManagedSource {
    fn from(x: s::Source) -> Self {
        Self {
            id: x.id,
            kind: x.kind.as_str().into(),
            name: x.name,
            uri: x.uri,
            username: x.username,
            has_password: x.has_password,
            options: x.options.into(),
            scan_into_library: x.scan_into_library,
            auto_scan: x.auto_scan,
            enabled: x.enabled,
            sync: x.sync.as_str().into(),
            status: x.status.into(),
        }
    }
}

/// Settings of a new or edited source. `Debug` hides the password.
#[derive(Clone, PartialEq, Eq)]
pub struct CoreSourceInput {
    /// `local`, `smb`, `webdav`, `sftp` or `ftp`; ignored on update.
    /// An SFTP key login sends the key as the password (see
    /// `pa_sources::ssh_key_secret`).
    pub kind: String,
    pub name: String,
    /// Folder path or URI without credentials.
    pub address: String,
    pub username: Option<String>,
    /// On create: the password. On update: `None` keeps the stored one,
    /// an empty string removes it. With [`ssh_private_key`](Self::ssh_private_key):
    /// the passphrase of the key.
    pub password: Option<String>,
    /// SFTP (NET-006): a private key (OpenSSH, PEM or PuTTY) to log in
    /// with instead of a password. Stored with its passphrase in the secret
    /// store as the source's secret; never returned.
    pub ssh_private_key: Option<String>,
    pub options: CoreSourceOptions,
    pub scan_into_library: bool,
    pub auto_scan: bool,
    pub enabled: bool,
    /// `off`, `settings` or `password` (see [`CoreManagedSource::sync`]);
    /// network sources only.
    pub sync: String,
}

impl fmt::Debug for CoreSourceInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreSourceInput")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("address", &self.address)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field(
                "ssh_private_key",
                &self.ssh_private_key.as_ref().map(|_| "***"),
            )
            .field("sync", &self.sync)
            .finish_non_exhaustive()
    }
}

/// Result of a speed test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSpeedReport {
    pub latency_ms: u64,
    /// Path of the file that was read, inside the source.
    pub file: Option<String>,
    pub bytes_read: u64,
    pub elapsed_ms: u64,
    pub bytes_per_sec: Option<u64>,
}

/// An entry of a live folder listing ([`CoreSources::list_folder`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSourceDirEntry {
    pub name: String,
    /// Relative to the source root, without a leading slash.
    pub rel_path: String,
    pub is_folder: bool,
    /// Files: size in bytes; folders: 0.
    pub size_bytes: u64,
    /// Last modification, Unix milliseconds, if the backend reports it.
    pub modified_ms: Option<i64>,
}

/// Server and logon for picking a folder on an SMB server (UI-028).
/// `Debug` hides the password.
#[derive(Clone, PartialEq, Eq)]
pub struct CoreSmbLogin {
    /// A stored SMB source whose password (and user name, if
    /// [`username`](Self::username) is `None`) is used: fixing a broken
    /// source without typing the password again.
    pub source_id: Option<i64>,
    /// `host`, `host:port` or `[ipv6]:port`.
    pub server: String,
    /// `None` with a source: its user. Empty: guest (anonymous).
    pub username: Option<String>,
    /// `None` with a source: its stored password.
    pub password: Option<String>,
    pub options: CoreSourceOptions,
}

impl fmt::Debug for CoreSmbLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreSmbLogin")
            .field("source_id", &self.source_id)
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish_non_exhaustive()
    }
}

/// A shared folder of an SMB server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSmbShare {
    pub name: String,
    /// The server's comment, may be empty.
    pub comment: String,
}

/// A folder inside a share ([`CoreSources::smb_find`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSmbPlace {
    pub share: String,
    /// `/`-separated, empty for the share itself.
    pub folder: String,
}

/// Whether SFTP sources accept RSA host and user keys (NET-006; the app's
/// "RSA keys" setting, on by default). The `rsa` crate has an unfixed
/// timing side channel (RUSTSEC-2023-0071). Applies to every source
/// manager of the process, including the library's scans and playback.
#[frb(sync)]
pub fn sources_set_ssh_rsa_allowed(allowed: bool) {
    s::set_ssh_rsa_allowed(allowed);
}

/// The SFTP private key of an input, if one was given.
fn key_of(input: &CoreSourceInput) -> Option<String> {
    input
        .ssh_private_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
}

/// The source manager of one library database.
#[frb(opaque)]
pub struct CoreSources {
    inner: s::SourceManager,
    /// The library database, to tell its sync service about changes.
    db_path: std::path::PathBuf,
    backend: String,
    fallback_reason: Option<String>,
}

impl CoreSources {
    /// Opens the library database at `db_path` with the secret store opened
    /// by `secrets_open_platform`/`secrets_open_default`, or, if none is
    /// open, the default store (keychain, or the encrypted fallback file in
    /// `data_dir`).
    pub fn open(db_path: String, data_dir: String) -> SourcesResult<CoreSources> {
        let opened =
            crate::shared::secret_store(std::path::Path::new(&data_dir)).map_err(s::Error::from)?;
        Ok(Self {
            backend: opened.backend().as_str().into(),
            fallback_reason: opened.fallback_reason,
            inner: s::SourceManager::open(&db_path, Arc::clone(&opened.store))?,
            db_path: db_path.into(),
        })
    }

    /// Where passwords are kept: `keychain`, `secret-service`,
    /// `credential-manager`, `encrypted-file`…
    #[frb(sync)]
    pub fn secret_backend(&self) -> String {
        self.backend.clone()
    }

    /// Why the encrypted fallback file is used (the app warns the user).
    #[frb(sync)]
    pub fn secret_fallback_reason(&self) -> Option<String> {
        self.fallback_reason.clone()
    }

    /// All sources in display order.
    pub fn list(&self) -> SourcesResult<Vec<CoreManagedSource>> {
        Ok(self.inner.list()?.into_iter().map(Into::into).collect())
    }

    /// One source.
    pub fn get(&self, id: i64) -> SourcesResult<CoreManagedSource> {
        Ok(self.inner.get(id)?.into())
    }

    /// Adds a source.
    pub fn create(&self, input: CoreSourceInput) -> SourcesResult<CoreManagedSource> {
        let kind: s::SourceKind = input.kind.parse()?;
        let password = match key_of(&input) {
            Some(key) => Some(s::ssh_key_secret(&key, input.password.as_deref())),
            None => input.password.map(SecretString::new),
        };
        let new = s::NewSource {
            kind,
            name: input.name,
            address: input.address,
            username: input.username,
            password,
            options: input.options.try_into()?,
            scan_into_library: input.scan_into_library,
            auto_scan: input.auto_scan,
            enabled: input.enabled,
            sync: input.sync.parse()?,
        };
        let created = self.inner.create(new)?;
        crate::shared::sources_changed(&self.db_path);
        Ok(created.into())
    }

    /// Replaces the settings of source `id` (the kind cannot change).
    pub fn update(&self, id: i64, input: CoreSourceInput) -> SourcesResult<CoreManagedSource> {
        let password = match (key_of(&input), input.password) {
            (Some(key), passphrase) => {
                s::PasswordChange::Set(s::ssh_key_secret(&key, passphrase.as_deref()))
            }
            (None, None) => s::PasswordChange::Keep,
            (None, Some(p)) if p.is_empty() => s::PasswordChange::Clear,
            (None, Some(p)) => s::PasswordChange::Set(SecretString::new(p)),
        };
        let alt = input.options.jellyfin_alt_addresses.clone();
        let mut options: s::SourceOptions = input.options.try_into()?;
        // The Jellyfin sign-in (user, device) is not in the form: keep it.
        options.jellyfin = self.inner.get(id)?.options.jellyfin.map(|mut j| {
            j.alt_addresses = alt
                .into_iter()
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty())
                .collect();
            j
        });
        let update = s::SourceUpdate {
            name: Some(input.name),
            address: Some(input.address),
            username: Some(input.username),
            password,
            options: Some(options),
            scan_into_library: Some(input.scan_into_library),
            auto_scan: Some(input.auto_scan),
            enabled: Some(input.enabled),
            sync: Some(input.sync.parse()?),
        };
        let updated = self.inner.update(id, update)?;
        crate::shared::sources_changed(&self.db_path);
        Ok(updated.into())
    }

    /// Deletes a source and its password (and, if it syncs, on the paired
    /// devices too).
    pub fn delete(&self, id: i64) -> SourcesResult<()> {
        self.inner.delete(id)?;
        crate::shared::sources_changed(&self.db_path);
        Ok(())
    }

    /// Checks that the source is reachable; the result is in the status.
    pub fn check(&self, id: i64) -> SourcesResult<CoreSourceStatus> {
        Ok(self.inner.check(id)?.into())
    }

    /// Folders and files of folder `dir` of source `id` (`""` = its root),
    /// listed live through the source's backend (UI-006 file browser), in no
    /// particular order. Devices, sockets and broken links are left out.
    pub fn list_folder(&self, id: i64, dir: String) -> SourcesResult<Vec<CoreSourceDirEntry>> {
        Ok(self
            .inner
            .list_folder(id, &dir)?
            .into_iter()
            .filter(|e| e.metadata.is_dir() || e.metadata.is_file())
            .map(|e| CoreSourceDirEntry {
                rel_path: e.path.as_str().trim_start_matches('/').to_owned(),
                is_folder: e.metadata.is_dir(),
                size_bytes: e.metadata.size,
                modified_ms: e.metadata.modified_ms,
                name: e.name,
            })
            .collect())
    }

    fn smb_login(&self, login: CoreSmbLogin) -> SourcesResult<s::SmbLogin> {
        let mut out = match login.source_id {
            Some(id) => self.inner.smb_login_of(id)?,
            None => s::SmbLogin::default(),
        };
        if !login.server.trim().is_empty() {
            out.server = login.server.trim().to_owned();
        }
        if let Some(user) = login.username {
            out.username = Some(user.trim().to_owned()).filter(|u| !u.is_empty());
        }
        if let Some(p) = login.password {
            out.password = Some(SecretString::new(p)).filter(|p| !p.is_empty());
        }
        out.options = login.options.try_into()?;
        if out.server.is_empty() {
            return Err(invalid("empty server".into()));
        }
        Ok(out)
    }

    /// Shared folders of an SMB server, without printers and administrative
    /// shares (UI-028). Also the logon check of the add flow: a wrong user
    /// or password fails with `authentication failed` in the message.
    pub fn smb_shares(&self, login: CoreSmbLogin) -> SourcesResult<Vec<CoreSmbShare>> {
        let login = self.smb_login(login)?;
        Ok(s::smb_shares(&login)?
            .into_iter()
            .map(|x| CoreSmbShare {
                name: x.name,
                comment: x.comment,
            })
            .collect())
    }

    /// Folders and files of `folder` (`""` = the share itself) in `share`.
    pub fn smb_list_folder(
        &self,
        login: CoreSmbLogin,
        share: String,
        folder: String,
    ) -> SourcesResult<Vec<CoreSourceDirEntry>> {
        let login = self.smb_login(login)?;
        let base = folder.trim_matches('/').to_owned();
        Ok(s::smb_list_folder(&login, &share, &base)?
            .into_iter()
            .filter(|e| e.metadata.is_dir() || e.metadata.is_file())
            .map(|e| CoreSourceDirEntry {
                rel_path: if base.is_empty() {
                    e.name.clone()
                } else {
                    format!("{base}/{}", e.name)
                },
                is_folder: e.metadata.is_dir(),
                size_bytes: e.metadata.size,
                modified_ms: e.metadata.modified_ms,
                name: e.name,
            })
            .collect())
    }

    /// Where a path the user typed (`share` + `folder`, possibly a path as
    /// on the server's disk) really is: verified share and folder pairs,
    /// most specific first; empty if nothing fits (see `pa_sources::smb_find`).
    pub fn smb_find(
        &self,
        login: CoreSmbLogin,
        share: String,
        folder: String,
    ) -> SourcesResult<Vec<CoreSmbPlace>> {
        let login = self.smb_login(login)?;
        Ok(s::smb_find(&login, &share, &folder)?
            .into_iter()
            .map(|p| CoreSmbPlace {
                share: p.share,
                folder: p.folder,
            })
            .collect())
    }

    /// Measures latency and read speed for at most `max_seconds`.
    pub fn speed_test(&self, id: i64, max_seconds: u32) -> SourcesResult<CoreSpeedReport> {
        let limits = s::SpeedTestOptions {
            max_duration: Duration::from_secs(max_seconds.clamp(1, 60).into()),
            ..Default::default()
        };
        let r = self.inner.speed_test(id, limits)?;
        Ok(CoreSpeedReport {
            latency_ms: r.latency_ms,
            file: r.file.map(|p| p.as_str().to_owned()),
            bytes_read: r.bytes_read,
            elapsed_ms: u64::try_from(r.elapsed.as_millis()).unwrap_or(u64::MAX),
            bytes_per_sec: r.bytes_per_sec,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: &str, address: &str, password: Option<&str>) -> CoreSourceInput {
        CoreSourceInput {
            kind: kind.into(),
            name: "Src".into(),
            address: address.into(),
            username: password.map(|_| "anna".into()),
            password: password.map(Into::into),
            ssh_private_key: None,
            options: CoreSourceOptions::default(),
            scan_into_library: true,
            auto_scan: true,
            enabled: true,
            sync: "off".into(),
        }
    }

    #[test]
    fn crud_through_the_facade() {
        let dir = tempfile::tempdir().unwrap();
        let sources = CoreSources {
            inner: s::SourceManager::open(
                dir.path().join("library.db"),
                Arc::new(pa_secrets::MemoryStore::new()),
            )
            .unwrap(),
            backend: "memory".into(),
            fallback_reason: None,
            db_path: dir.path().join("library.db"),
        };
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        let local = sources
            .create(input("local", media.to_str().unwrap(), None))
            .unwrap();
        assert_eq!(local.kind, "local");
        assert_eq!(sources.check(local.id).unwrap().online, Some(true));
        std::fs::create_dir(media.join("Movies")).unwrap();
        std::fs::write(media.join("Movies/a.mkv"), b"abc").unwrap();
        let listed = sources.list_folder(local.id, "Movies".into()).unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|e| (e.rel_path.as_str(), e.is_folder, e.size_bytes))
                .collect::<Vec<_>>(),
            [("Movies/a.mkv", false, 3)]
        );
        assert_eq!(
            sources.list_folder(local.id, String::new()).unwrap()[0].name,
            "Movies"
        );

        let mut smb = input("smb", "smb://nas/media", Some("hunter2-frb"));
        smb.options.smb_version = "smb3".into();
        let created = sources.create(smb.clone()).unwrap();
        assert!(created.has_password);
        assert_eq!(created.options.smb_version, "smb3");
        assert!(!format!("{smb:?}").contains("hunter2-frb"));

        smb.password = Some(String::new());
        assert!(!sources.update(created.id, smb).unwrap().has_password);
        assert_eq!(sources.list().unwrap().len(), 2);
        sources.delete(created.id).unwrap();
        let err = sources.get(created.id).unwrap_err();
        assert_eq!(err.code, "not_found");
        let err = sources.create(input("nfs", "nfs://x", None)).unwrap_err();
        assert_eq!(err.code, "invalid");
        let mut synced = input("smb", "smb://nas/media", Some("pw-frb"));
        synced.sync = "password".into();
        assert_eq!(sources.create(synced).unwrap().sync, "password");
        let mut local = input("local", media.to_str().unwrap(), None);
        local.sync = "settings".into();
        assert_eq!(sources.create(local).unwrap_err().code, "invalid");
    }

    #[test]
    fn sftp_key_and_host_key_go_through() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(pa_secrets::MemoryStore::new());
        let sources = CoreSources {
            inner: s::SourceManager::open(dir.path().join("library.db"), store.clone()).unwrap(),
            backend: "memory".into(),
            fallback_reason: None,
            db_path: dir.path().join("library.db"),
        };
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----";
        let mut sftp = input("sftp", "sftp://nas/srv", Some("phrase-frb"));
        sftp.ssh_private_key = Some(key.into());
        sftp.options.ssh_host_key = Some("ssh-ed25519 SHA256:abcDEF123".into());
        assert!(!format!("{sftp:?}").contains("PRIVATE"));
        let created = sources.create(sftp.clone()).unwrap();
        assert!(created.has_password);
        assert_eq!(
            created.options.ssh_host_key.as_deref(),
            Some("ssh-ed25519 SHA256:abcDEF123")
        );
        let stored = store.keys();
        assert_eq!(stored.len(), 1);
        let secret = pa_secrets::SecretStore::get(&*store, &stored[0])
            .unwrap()
            .unwrap();
        assert!(
            secret
                .expose()
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert!(secret.expose().ends_with("Passphrase: phrase-frb"));
        // An empty host key in the form lets the next connect learn it.
        sftp.options.ssh_host_key = Some(String::new());
        sftp.ssh_private_key = None;
        sftp.password = None;
        let updated = sources.update(created.id, sftp).unwrap();
        assert_eq!(updated.options.ssh_host_key, None);
        assert!(updated.has_password);
    }
}
