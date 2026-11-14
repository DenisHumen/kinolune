// SPDX-License-Identifier: MPL-2.0
//! Choosing the backend: platform callback → OS keychain → encrypted file.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::timeout::{DEFAULT_CALL_TIMEOUT, DEFAULT_OPEN_TIMEOUT, TimeoutStore, run_with_timeout};
use crate::{
    Backend, EncryptedFileStore, PlatformKeystore, PlatformStore, Result, SecretError, SecretKey,
    SecretStore, SecretString, forget_known, register_known,
};

/// Keychain service name of the app: the items show up under this name in
/// Keychain Access, Credential Manager and Seahorse.
pub const DEFAULT_SERVICE: &str = "Kinolune";

/// Whether this build has a desktop OS keychain backend
/// ([`NativeStore`](crate::NativeStore)).
pub const NATIVE_AVAILABLE: bool = cfg!(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "macos",
    windows
));

/// Connects to an OS keychain for a service name. Blocking; [`open`] calls
/// it with the deadline [`OpenOptions::open_timeout`].
pub type NativeConnect = Arc<dyn Fn(&str) -> Result<Arc<dyn SecretStore>> + Send + Sync>;

/// What [`open`] may use.
#[derive(Clone)]
pub struct OpenOptions {
    /// Keychain service name, [`DEFAULT_SERVICE`] by default. Tests use a
    /// unique name.
    pub service: String,
    /// Secure storage of the app (mobile, tvOS). Takes precedence over the
    /// OS keychain.
    pub platform: Option<Arc<dyn PlatformKeystore>>,
    /// Try the desktop OS keychain. Default: [`NATIVE_AVAILABLE`].
    pub native: bool,
    /// Replaces the connection to the OS keychain
    /// ([`NativeStore::open`](crate::NativeStore::open) by default). Tests
    /// inject slow, locked or missing keychains here.
    pub native_connect: Option<NativeConnect>,
    /// Deadline for connecting to the OS keychain and checking that it
    /// answers. Past it the keychain is skipped
    /// ([`FallbackKind::TimedOut`]). Default [`DEFAULT_OPEN_TIMEOUT`].
    pub open_timeout: Duration,
    /// Deadline of each OS keychain call once open; a late call fails with
    /// [`SecretError::Timeout`]. Default [`DEFAULT_CALL_TIMEOUT`].
    pub call_timeout: Duration,
    /// Directory of the encrypted fallback file (the data directory of the
    /// core). Also checked for a previous fallback file to migrate.
    pub fallback_dir: Option<PathBuf>,
    /// Use the encrypted file when no secure backend works. Default: true on
    /// desktop, false on mobile (there the platform callback is required).
    pub allow_fallback: bool,
}

impl OpenOptions {
    /// Defaults for this platform with the given fallback directory.
    pub fn new(fallback_dir: Option<PathBuf>) -> Self {
        Self {
            service: DEFAULT_SERVICE.to_owned(),
            platform: None,
            native: NATIVE_AVAILABLE,
            native_connect: None,
            open_timeout: DEFAULT_OPEN_TIMEOUT,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            fallback_dir,
            allow_fallback: NATIVE_AVAILABLE,
        }
    }

    /// Sets the platform keystore.
    #[must_use]
    pub fn with_platform(mut self, platform: Arc<dyn PlatformKeystore>) -> Self {
        self.platform = Some(platform);
        self
    }
}

impl fmt::Debug for OpenOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenOptions")
            .field("service", &self.service)
            .field("platform", &self.platform.is_some())
            .field("native", &self.native)
            .field("native_connect", &self.native_connect.is_some())
            .field("open_timeout", &self.open_timeout)
            .field("call_timeout", &self.call_timeout)
            .field("fallback_dir", &self.fallback_dir)
            .field("allow_fallback", &self.allow_fallback)
            .finish()
    }
}

/// Why [`open`] did not use the OS keychain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FallbackKind {
    /// No keychain: no D-Bus session, no Secret Service provider, or none
    /// in this build.
    Unavailable,
    /// The keychain is locked or access was denied; unlocking it would need
    /// a prompt.
    Locked,
    /// The keychain did not answer in time.
    TimedOut,
    /// The keychain answered with an error.
    Failed,
}

impl FallbackKind {
    /// The kind of a keychain error.
    pub fn of(error: &SecretError) -> Self {
        match error {
            SecretError::Unavailable(_) => Self::Unavailable,
            SecretError::Locked(_) => Self::Locked,
            SecretError::Timeout(_) => Self::TimedOut,
            _ => Self::Failed,
        }
    }

    /// Stable lowercase name: `unavailable`, `locked`, `timeout`, `failed`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Locked => "locked",
            Self::TimedOut => "timeout",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for FallbackKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Result of [`open`].
#[derive(Debug, Clone)]
pub struct OpenedStore {
    /// The store. Values passing through it are registered for
    /// [`redact`](crate::redact).
    pub store: Arc<dyn SecretStore>,
    /// Why the secure backends were not used, when
    /// [`Backend::EncryptedFile`] was chosen. The app shows a warning.
    pub fallback_reason: Option<String>,
    /// The kind of [`fallback_reason`](Self::fallback_reason).
    pub fallback_kind: Option<FallbackKind>,
    /// The encrypted file, when [`Backend::EncryptedFile`] was chosen (the
    /// app tells the user where secrets are).
    pub fallback_file: Option<PathBuf>,
    /// Secrets moved from an old fallback file into the secure backend.
    pub migrated: usize,
    /// The old fallback file could not be read and was moved aside
    /// (`secrets.enc.corrupt`): secrets in it must be entered again.
    pub discarded_corrupted: bool,
}

impl OpenedStore {
    /// Backend of [`store`](Self::store).
    pub fn backend(&self) -> Backend {
        self.store.backend()
    }
}

/// Opens the best available secret store.
///
/// Order: [`OpenOptions::platform`] → OS keychain → encrypted file in
/// [`OpenOptions::fallback_dir`]. The OS keychain is skipped when it is
/// missing, locked, does not answer within [`OpenOptions::open_timeout`] or
/// fails; the reason is in [`OpenedStore::fallback_reason`] and
/// [`OpenedStore::fallback_kind`]. Once open, every OS keychain call has
/// the deadline [`OpenOptions::call_timeout`]. So this never blocks on a
/// keychain longer than the open deadline plus, when an old fallback file
/// is migrated, one call deadline.
///
/// When a secure backend is chosen and the fallback directory still has a
/// secrets file from an earlier run, its secrets are moved to the secure
/// backend (existing values there win) and the file is deleted. If the OS
/// keychain fails during the move, the file stays and is used instead.
pub fn open(options: OpenOptions) -> Result<OpenedStore> {
    let mut reasons = Vec::new();
    let mut kind = None;
    let mut secure: Option<Arc<dyn SecretStore>> = None;

    if let Some(platform) = &options.platform {
        secure = Some(Arc::new(PlatformStore::new(platform.clone())));
    } else if options.native {
        match open_native(&options) {
            Ok(store) => secure = Some(store),
            Err(e) => {
                kind = Some(FallbackKind::of(&e));
                reasons.push(format!("OS keychain: {e}"));
            }
        }
    } else {
        kind = Some(FallbackKind::Unavailable);
        reasons.push("no OS keychain on this platform".to_owned());
    }

    let mut discarded_corrupted = false;
    if let Some(store) = secure {
        let mut migrated = Some(0);
        if let Some(dir) = &options.fallback_dir
            && EncryptedFileStore::exists(dir)
        {
            match EncryptedFileStore::open(dir) {
                Ok(file) => match migrate(&file, store.as_ref()) {
                    Ok(n) => migrated = Some(n),
                    // The OS keychain stopped working mid-way: keep using
                    // the file, which is still complete.
                    Err(e) if options.platform.is_none() && is_keychain_failure(&e) => {
                        kind = Some(FallbackKind::of(&e));
                        reasons.push(format!("OS keychain: {e}"));
                        migrated = None;
                    }
                    Err(e) => return Err(e),
                },
                Err(SecretError::Corrupted) => {
                    set_aside_corrupted(dir)?;
                    discarded_corrupted = true;
                }
                Err(e) => return Err(e),
            }
        }
        if let Some(migrated) = migrated {
            return Ok(OpenedStore {
                store: Arc::new(Tracked(store)),
                fallback_reason: None,
                fallback_kind: None,
                fallback_file: None,
                migrated,
                discarded_corrupted,
            });
        }
    }

    let reason = reasons.join("; ");
    let Some(dir) = options.fallback_dir.filter(|_| options.allow_fallback) else {
        return Err(match kind {
            Some(FallbackKind::Locked) => SecretError::Locked(reason),
            Some(FallbackKind::TimedOut) => SecretError::Timeout(reason),
            _ => SecretError::Unavailable(reason),
        });
    };
    let file = match EncryptedFileStore::open(&dir) {
        Err(SecretError::Corrupted) => {
            set_aside_corrupted(&dir)?;
            discarded_corrupted = true;
            EncryptedFileStore::open(&dir)?
        }
        other => other?,
    };
    Ok(OpenedStore {
        store: Arc::new(Tracked(Arc::new(file))),
        fallback_reason: Some(reason),
        fallback_kind: Some(kind.unwrap_or(FallbackKind::Failed)),
        fallback_file: Some(dir.join(crate::SECRETS_FILE)),
        migrated: 0,
        discarded_corrupted,
    })
}

/// Errors that mean "the keychain does not work now", as opposed to errors
/// of the file or of the data.
fn is_keychain_failure(e: &SecretError) -> bool {
    matches!(
        e,
        SecretError::Unavailable(_)
            | SecretError::Locked(_)
            | SecretError::Timeout(_)
            | SecretError::Backend(_)
    )
}

/// Connects to the OS keychain within the open deadline and wraps it with
/// the call deadline.
fn open_native(options: &OpenOptions) -> Result<Arc<dyn SecretStore>> {
    let connect: NativeConnect = options
        .native_connect
        .clone()
        .unwrap_or_else(|| Arc::new(connect_native));
    let service = options.service.clone();
    let store = run_with_timeout(
        "connecting to the OS keychain",
        options.open_timeout,
        None,
        move || connect(&service),
    )?;
    Ok(Arc::new(TimeoutStore::new(store, options.call_timeout)))
}

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "macos",
    windows
))]
fn connect_native(service: &str) -> Result<Arc<dyn SecretStore>> {
    Ok(Arc::new(crate::NativeStore::open(service)?))
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "macos",
    windows
)))]
fn connect_native(_service: &str) -> Result<Arc<dyn SecretStore>> {
    Err(SecretError::Unavailable(
        "no OS keychain backend in this build".into(),
    ))
}

/// Copies every secret of `file` that `target` does not have yet, then
/// deletes the file. Returns the number of copied secrets.
fn migrate(file: &EncryptedFileStore, target: &dyn SecretStore) -> Result<usize> {
    let mut copied = 0;
    for key in file.keys() {
        if target.get(&key)?.is_none()
            && let Some(value) = file.get(&key)?
        {
            target.set(&key, &value)?;
            copied += 1;
        }
    }
    EncryptedFileStore::remove_files(file.dir())?;
    Ok(copied)
}

fn set_aside_corrupted(dir: &std::path::Path) -> Result<()> {
    let path = dir.join(crate::SECRETS_FILE);
    std::fs::rename(&path, path.with_extension("enc.corrupt"))?;
    // The key file may be the reason; a fresh one is made on next open.
    match std::fs::remove_file(dir.join(crate::KEY_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Registers every value it sees for log redaction.
#[derive(Debug)]
struct Tracked(Arc<dyn SecretStore>);

impl SecretStore for Tracked {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>> {
        let value = self.0.get(key)?;
        if let Some(value) = &value {
            register_known(value);
        }
        Ok(value)
    }

    fn set(&self, key: &SecretKey, value: &SecretString) -> Result<()> {
        register_known(value);
        self.0.set(key, value)
    }

    fn delete(&self, key: &SecretKey) -> Result<bool> {
        if let Ok(Some(old)) = self.0.get(key) {
            forget_known(&old);
        }
        self.0.delete(key)
    }

    fn backend(&self) -> Backend {
        self.0.backend()
    }
}
