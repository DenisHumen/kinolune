// SPDX-License-Identifier: MPL-2.0
//! Plex Media Servers (NET-012, ADR-0131) for Dart.
//!
//! A Plex source is a source like the others (`CoreSources`): its
//! libraries are scanned into the library and its files play through the
//! stream proxy. [`CorePlex`] adds the sign-in, which needs no password:
//!
//! 1. [`CorePlex::start`] — a code to show with `plex.tv/link`;
//! 2. [`CorePlex::poll`] every few seconds until the user entered it;
//! 3. if the account is in a Plex Home with several users,
//!    [`CorePlex::switch_user`] (with the user's PIN when protected);
//! 4. if there are several servers, the user picks one;
//! 5. [`CorePlex::finish`] picks the connection that answers (local, then
//!    remote, then relay) and adds the source. The server token goes to
//!    the secret store; the account token is dropped with the sign-in.
//!
//! Every method but [`CorePlex::cancel`] blocks on the network.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use flutter_rust_bridge::frb;
use pa_sources as s;

use super::library::CoreLibrary;
use super::sources::{CoreManagedSource, SourcesError};

type PlexResult<T> = std::result::Result<T, SourcesError>;

fn no_sign_in() -> SourcesError {
    SourcesError {
        code: "other".into(),
        message: "no such Plex sign-in".into(),
    }
}

/// A sign-in waiting for its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePlexPin {
    pub handle: u64,
    /// Four characters to enter at [`link_url`](Self::link_url).
    pub code: String,
    /// `https://plex.tv/link`.
    pub link_url: String,
}

/// A user of the account's Plex Home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePlexUser {
    pub uuid: String,
    pub name: String,
    /// The Home's owner (the signed-in account).
    pub admin: bool,
    /// Switching to this user needs their PIN.
    pub protected: bool,
    /// A managed user with restrictions.
    pub restricted: bool,
}

/// A server the user can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePlexServer {
    /// Machine identifier.
    pub id: String,
    pub name: String,
    pub version: String,
    /// The user owns it (else it is shared with them).
    pub owned: bool,
    /// Online, as plex.tv last saw it.
    pub online: bool,
    /// The one to preselect (matches the server found on the LAN, or
    /// the only one).
    pub suggested: bool,
}

/// Where the sign-in stands after [`CorePlex::poll`] or
/// [`CorePlex::switch_user`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePlexState {
    /// The code has been entered.
    pub linked: bool,
    /// The account's display name.
    pub account: String,
    /// The Home users (empty without a Home, or with only the account).
    pub users: Vec<CorePlexUser>,
    /// The uuid of the user the servers are listed for.
    pub current_user: Option<String>,
    pub servers: Vec<CorePlexServer>,
}

/// A Plex server added as a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePlexAdded {
    pub source: CoreManagedSource,
    /// How it is reached: `local`, `remote` or `relay`.
    pub connection: String,
}

struct Pending {
    sign_in: s::PlexSignIn,
    /// A server found on the LAN (its URL), to preselect.
    hint: Option<String>,
}

/// Plex sign-ins over the sources of one library.
#[frb(opaque)]
pub struct CorePlex {
    sources: Arc<s::SourceManager>,
    pending: Mutex<HashMap<u64, Pending>>,
    next: AtomicU64,
}

fn state(p: &Pending, linked: bool) -> CorePlexState {
    let si = &p.sign_in;
    let suggested = si
        .suggested_server(p.hint.as_deref())
        .map(|r| r.client_identifier.clone());
    // A Home with only the account has nothing to choose.
    let users = if si.users().len() > 1 {
        si.users()
            .iter()
            .map(|u| CorePlexUser {
                uuid: u.uuid.clone(),
                name: u.display_name().to_owned(),
                admin: u.admin,
                protected: u.protected,
                restricted: u.restricted,
            })
            .collect()
    } else {
        Vec::new()
    };
    CorePlexState {
        linked,
        account: si
            .account()
            .map(|a| a.display_name().to_owned())
            .unwrap_or_default(),
        users,
        current_user: si.current_user().map(str::to_owned),
        servers: si
            .servers()
            .iter()
            .map(|r| CorePlexServer {
                id: r.client_identifier.clone(),
                name: r.name.clone(),
                version: r.product_version.clone(),
                owned: r.owned,
                online: r.presence,
                suggested: suggested.as_deref() == Some(r.client_identifier.as_str()),
            })
            .collect(),
    }
}

impl CorePlex {
    /// Opens the Plex sign-in for `library`, with the secret store whose
    /// fallback file is in `data_dir` (the same as for `CoreSources.open`).
    pub fn open(library: &CoreLibrary, data_dir: String) -> PlexResult<CorePlex> {
        let catalog = library.catalog();
        let opened =
            crate::shared::secret_store(std::path::Path::new(&data_dir)).map_err(s::Error::from)?;
        let sources = s::SourceManager::open(catalog.path(), Arc::clone(&opened.store))?;
        Ok(Self {
            sources: Arc::new(sources),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        })
    }

    /// Starts a sign-in as a device named `device_name`: show the code
    /// and the link, then [`poll`](Self::poll). `hint`: the URL of a
    /// server found on the LAN, preselected later.
    pub fn start(&self, device_name: String, hint: Option<String>) -> PlexResult<CorePlexPin> {
        let sign_in = s::PlexSignIn::start(&device_name)?;
        let handle = self.next.fetch_add(1, Ordering::Relaxed);
        let out = CorePlexPin {
            handle,
            code: sign_in.code().to_owned(),
            link_url: s::PLEX_LINK_URL.to_owned(),
        };
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                handle,
                Pending {
                    sign_in,
                    hint: hint.filter(|h| !h.trim().is_empty()),
                },
            );
        Ok(out)
    }

    /// Whether the code of `handle` has been entered; once it has, the
    /// Home users and the servers.
    pub fn poll(&self, handle: u64) -> PlexResult<CorePlexState> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let p = pending.get_mut(&handle).ok_or_else(no_sign_in)?;
        let linked = p.sign_in.poll()?;
        Ok(state(p, linked))
    }

    /// Switches to Home user `uuid` (`pin`: their PIN when protected) and
    /// lists that user's servers.
    pub fn switch_user(
        &self,
        handle: u64,
        uuid: String,
        pin: Option<String>,
    ) -> PlexResult<CorePlexState> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let p = pending.get_mut(&handle).ok_or_else(no_sign_in)?;
        p.sign_in.switch_user(&uuid, pin.as_deref())?;
        Ok(state(p, true))
    }

    /// Picks the connection of server `server_id` that answers and adds
    /// the server as a source named `name` (default: the server's name).
    /// The sign-in ends.
    pub fn finish(
        &self,
        handle: u64,
        server_id: String,
        name: Option<String>,
    ) -> PlexResult<CorePlexAdded> {
        // Probing takes a few seconds: not under the lock.
        let p = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&handle)
            .ok_or_else(no_sign_in)?;
        let account = match p.sign_in.finish(&server_id) {
            Ok(a) => a,
            Err(e) => {
                // Let the user pick another server or try again.
                self.pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(handle, p);
                return Err(e.into());
            }
        };
        let connection = account.connected.1.as_str().to_owned();
        let source = self
            .sources
            .create_plex(account, name.as_deref(), Vec::new())?;
        Ok(CorePlexAdded {
            source: source.into(),
            connection,
        })
    }

    /// Forgets a sign-in.
    #[frb(sync)]
    pub fn cancel(&self, handle: u64) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&handle);
    }
}
