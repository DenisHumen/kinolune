// SPDX-License-Identifier: MPL-2.0
//! Cloud storages (NET-017, ADR-0144) for Dart.
//!
//! A cloud source is a source like the others (`CoreSources`): scanned
//! into the library, its files played through the stream proxy.
//! [`CoreCloud`] adds the sign-ins:
//!
//! - **MEGA:** [`CoreCloud::add_mega`] with e-mail and password; when the
//!   account has two-factor authentication the call fails with the code
//!   `mfa_required` and is repeated with the code. The password is not
//!   stored. [`CoreCloud::add_mega_link`] adds a public link.
//! - **Google Drive, OneDrive, Dropbox:** [`CoreCloud::start`] gives the
//!   page to open in the browser; [`CoreCloud::wait`] blocks until the
//!   browser comes back to the loopback listener (or
//!   [`CoreCloud::complete`] with the `kinolune://oauth` link on
//!   Android); [`CoreCloud::finish`] adds the source.
//!
//! Every method but [`CoreCloud::providers`] and [`CoreCloud::cancel`]
//! blocks on the network.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use flutter_rust_bridge::frb;
use pa_secrets::SecretString;
use pa_sources as s;
use s::pa_cloud::Cloud;

use super::library::CoreLibrary;
use super::sources::{CoreManagedSource, SourcesError};

type CloudResult<T> = std::result::Result<T, SourcesError>;

fn error(code: &str, message: &str) -> SourcesError {
    SourcesError {
        code: code.into(),
        message: message.into(),
    }
}

/// A sources error; a MEGA sign-in that needs the 2FA code gets the code
/// `mfa_required`, a declined browser sign-in `cancelled`.
fn mapped(e: s::Error) -> SourcesError {
    let text = e.to_string();
    if text.contains(s::pa_cloud::mega::MFA_REQUIRED_TEXT) {
        return error("mfa_required", &text);
    }
    if matches!(
        e.source_kind(),
        Some(pa_vfs::ErrorKind::Cancelled | pa_vfs::ErrorKind::PermissionDenied)
    ) && matches!(e, s::Error::Server { .. })
    {
        return error("cancelled", &text);
    }
    e.into()
}

fn cloud_of(id: &str) -> CloudResult<Cloud> {
    id.parse()
        .map_err(|_| error("invalid", &format!("unknown cloud {id:?}")))
}

/// A cloud this build can add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCloudProvider {
    /// `mega`, `gdrive`, `onedrive`, `dropbox` (also the source kind).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Signs in in the browser (OAuth); else with e-mail and password.
    pub oauth: bool,
    /// This build has a client id for it; without one the user enters
    /// their own (advanced).
    pub configured: bool,
}

/// A browser sign-in waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCloudAuth {
    pub handle: u64,
    /// The page to open in the browser.
    pub url: String,
}

/// Cloud sign-ins over the sources of one library.
#[frb(opaque)]
pub struct CoreCloud {
    sources: Arc<s::SourceManager>,
    /// Sign-ins by handle; `None` while one is waiting for the browser.
    pending: Mutex<HashMap<u64, Option<s::CloudSignIn>>>,
    cancelled: Arc<Mutex<HashSet<u64>>>,
    next: AtomicU64,
}

impl CoreCloud {
    /// Opens the cloud sign-ins for `library`, with the secret store whose
    /// fallback file is in `data_dir` (the same as for `CoreSources.open`).
    pub fn open(library: &CoreLibrary, data_dir: String) -> CloudResult<CoreCloud> {
        let catalog = library.catalog();
        let opened =
            crate::shared::secret_store(std::path::Path::new(&data_dir)).map_err(s::Error::from)?;
        let sources = s::SourceManager::open(catalog.path(), Arc::clone(&opened.store))?;
        Ok(Self {
            sources: Arc::new(sources),
            pending: Mutex::new(HashMap::new()),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            next: AtomicU64::new(1),
        })
    }

    /// The clouds, in the order the app lists them (MEGA and Google Drive
    /// first).
    #[frb(sync)]
    pub fn providers(&self) -> Vec<CoreCloudProvider> {
        [
            Cloud::Mega,
            Cloud::GoogleDrive,
            Cloud::OneDrive,
            Cloud::Dropbox,
        ]
        .into_iter()
        .map(|c| CoreCloudProvider {
            id: c.as_str().into(),
            name: c.name().into(),
            oauth: c.oauth().is_some(),
            configured: c.oauth().is_none_or(|p| p.configured()),
        })
        .collect()
    }

    /// Starts a browser sign-in to `cloud` (`gdrive`, `onedrive`,
    /// `dropbox`); `client_id` (and `client_secret`): the user's own
    /// client; `app_redirect`: return to `kinolune://oauth` instead of the
    /// loopback listener.
    pub fn start(
        &self,
        cloud: String,
        client_id: Option<String>,
        client_secret: Option<String>,
        app_redirect: bool,
    ) -> CloudResult<CoreCloudAuth> {
        let cloud = cloud_of(&cloud)?;
        let sign_in = s::CloudSignIn::start(
            cloud,
            client_id.as_deref(),
            client_secret.as_deref(),
            app_redirect,
            self.sources.cloud_endpoints(),
        )
        .map_err(mapped)?;
        let handle = self.next.fetch_add(1, Ordering::Relaxed);
        let url = sign_in.url();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(handle, Some(sign_in));
        Ok(CoreCloudAuth { handle, url })
    }

    fn take(&self, handle: u64) -> CloudResult<s::CloudSignIn> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&handle)
            .and_then(Option::take)
            .ok_or_else(|| error("other", "no such sign-in, or it is busy"))
    }

    fn put_back(&self, handle: u64, sign_in: s::CloudSignIn) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = pending.get_mut(&handle) {
            *slot = Some(sign_in);
        }
    }

    /// Waits up to `timeout_secs` for the browser to come back, then signs
    /// in; returns the account (e-mail or name).
    pub fn wait(&self, handle: u64, timeout_secs: u32) -> CloudResult<String> {
        let mut sign_in = self.take(handle)?;
        let cancelled = Arc::clone(&self.cancelled);
        let result = sign_in.wait(Duration::from_secs(timeout_secs.into()), &|| {
            cancelled
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&handle)
        });
        self.put_back(handle, sign_in);
        result.map_err(mapped)
    }

    /// Signs in with the redirect link the app was opened with
    /// (`kinolune://oauth?…`); returns the account.
    pub fn complete(&self, handle: u64, redirect: String) -> CloudResult<String> {
        let mut sign_in = self.take(handle)?;
        let result = sign_in.complete(&redirect);
        self.put_back(handle, sign_in);
        result.map_err(mapped)
    }

    /// Adds the signed-in cloud as a source named `name` (default: cloud ·
    /// account), rooted at `folder` (default: all of it).
    pub fn finish(
        &self,
        handle: u64,
        name: Option<String>,
        folder: Option<String>,
    ) -> CloudResult<CoreManagedSource> {
        let sign_in = self.take(handle)?;
        if !sign_in.signed_in() {
            self.put_back(handle, sign_in);
            return Err(error("other", "the browser sign-in has not finished"));
        }
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&handle);
        let source = self
            .sources
            .create_cloud(sign_in, name.as_deref(), folder.as_deref())
            .map_err(mapped)?;
        Ok(source.into())
    }

    /// Drops a sign-in; a [`wait`](Self::wait) on it returns `cancelled`.
    #[frb(sync)]
    pub fn cancel(&self, handle: u64) {
        self.cancelled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(handle);
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&handle);
    }

    /// Signs in to MEGA and adds the account (or its `folder`). Fails with
    /// `mfa_required` when the account needs its 2FA code: ask for it and
    /// call again with `mfa`.
    pub fn add_mega(
        &self,
        email: String,
        password: String,
        mfa: Option<String>,
        name: Option<String>,
        folder: Option<String>,
    ) -> CloudResult<CoreManagedSource> {
        let password = SecretString::new(password);
        let source = self
            .sources
            .create_mega(
                &email,
                &password,
                mfa.as_deref(),
                name.as_deref(),
                folder.as_deref(),
            )
            .map_err(mapped)?;
        Ok(source.into())
    }

    /// Adds a public MEGA folder or file link (with its key after `#`).
    pub fn add_mega_link(
        &self,
        link: String,
        name: Option<String>,
    ) -> CloudResult<CoreManagedSource> {
        let source = self
            .sources
            .create_mega_link(&link, name.as_deref())
            .map_err(mapped)?;
        Ok(source.into())
    }
}
