// SPDX-License-Identifier: MPL-2.0
//! Cloud storage sources (NET-017, ADR-NNNN): MEGA, Google Drive,
//! OneDrive, Dropbox through `pa-cloud`.
//!
//! - **MEGA account:** [`SourceManager::create_mega`] signs in with e-mail,
//!   password and (when asked) the 2FA code; the secret store keeps the
//!   session, the password is dropped at once.
//! - **MEGA link:** [`SourceManager::create_mega_link`]; the key after `#`
//!   is the secret, the address is the link without it.
//! - **OAuth clouds:** [`CloudSignIn`] — open [`CloudSignIn::url`] in the
//!   browser, then [`wait`](CloudSignIn::wait) for the loopback redirect
//!   (or [`complete`](CloudSignIn::complete) with the `kinolune://oauth`
//!   link the app was opened with), then
//!   [`SourceManager::create_cloud`]. The secret store keeps the refresh
//!   token ([`CloudSecret`]); a rotated one replaces it.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use pa_cloud::mega::{MegaDrive, PublicLink, Session};
use pa_cloud::oauth::{AuthRequest, Loopback};
use pa_cloud::{Cloud, CloudFs, Provider, TokenSink, TokenSource, Tokens};
use pa_secrets::{SecretKey, SecretStore, SecretString};
use pa_vfs::{ErrorKind, Vfs, VfsError, VfsPath};

use crate::manager::Stored;
use crate::model::CloudOptions;
use crate::{Error, NewSource, Result, Source, SourceKind, SourceManager, SourceOptions};

/// Cached trees: source id → (what they were built from, tree).
pub(crate) type Trees = HashMap<i64, (u64, Arc<dyn Vfs>)>;

/// Where the cloud APIs are (tests point them at mocks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudEndpoints {
    /// MEGA's API host.
    pub mega: String,
    /// Google Drive API v3.
    pub gdrive: String,
    /// Microsoft Graph.
    pub onedrive: String,
    /// Dropbox RPC host.
    pub dropbox_api: String,
    /// Dropbox content host.
    pub dropbox_content: String,
    /// All OAuth endpoints at `<base>/authorize` and `<base>/token`;
    /// `None`: each provider's own.
    pub oauth: Option<String>,
}

impl Default for CloudEndpoints {
    fn default() -> Self {
        Self {
            mega: pa_cloud::mega::API.into(),
            gdrive: pa_cloud::gdrive::API.into(),
            onedrive: pa_cloud::onedrive::API.into(),
            dropbox_api: pa_cloud::dropbox::API.into(),
            dropbox_content: pa_cloud::dropbox::CONTENT.into(),
            oauth: None,
        }
    }
}

/// The stored secret of an OAuth cloud: the refresh token and, for a user's
/// own client that has one, its client secret. `Debug` hides both.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CloudSecret {
    /// The refresh token.
    pub refresh_token: String,
    /// The secret of the user's own client, if the provider wants one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for CloudSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CloudSecret(***)")
    }
}

impl CloudSecret {
    /// As stored in the secret store (JSON).
    pub fn to_secret(&self) -> SecretString {
        SecretString::new(serde_json::to_string(self).unwrap_or_default())
    }

    /// From the secret store; a bare string is a refresh token.
    pub fn parse(secret: &str) -> Self {
        serde_json::from_str(secret).unwrap_or_else(|_| Self {
            refresh_token: secret.trim().to_owned(),
            client_secret: None,
        })
    }
}

fn server_error(e: &VfsError) -> Error {
    Error::Server {
        kind: e.kind(),
        message: crate::scrub(e.message(), None),
    }
}

/// The OAuth provider of `cloud` with the user's own client, if any.
fn provider(
    cloud: Cloud,
    endpoints: &CloudEndpoints,
    client_id: &str,
    client_secret: Option<&str>,
) -> pa_vfs::Result<Provider> {
    let mut p = cloud
        .oauth()
        .ok_or_else(|| VfsError::new(ErrorKind::Unsupported, "not an OAuth cloud"))?
        .with_client(client_id, client_secret);
    if let Some(base) = &endpoints.oauth {
        p = p.at(base);
    }
    Ok(p)
}

/// The tree of an OAuth cloud for `tokens`.
fn oauth_drive(cloud: Cloud, endpoints: &CloudEndpoints, tokens: Arc<TokenSource>) -> OAuthDrive {
    match cloud {
        Cloud::GoogleDrive => OAuthDrive::Google(pa_cloud::gdrive::GoogleDrive::with_api(
            tokens,
            &endpoints.gdrive,
        )),
        Cloud::OneDrive => OAuthDrive::OneDrive(pa_cloud::onedrive::OneDrive::with_api(
            tokens,
            &endpoints.onedrive,
        )),
        Cloud::Dropbox | Cloud::Mega => OAuthDrive::Dropbox(pa_cloud::dropbox::Dropbox::with_api(
            tokens,
            &endpoints.dropbox_api,
            &endpoints.dropbox_content,
        )),
    }
}

enum OAuthDrive {
    Google(pa_cloud::gdrive::GoogleDrive),
    OneDrive(pa_cloud::onedrive::OneDrive),
    Dropbox(pa_cloud::dropbox::Dropbox),
}

impl OAuthDrive {
    fn account(&self) -> pa_vfs::Result<String> {
        match self {
            Self::Google(d) => d.account(),
            Self::OneDrive(d) => d.account(),
            Self::Dropbox(d) => d.account(),
        }
    }

    fn into_vfs(self, root: VfsPath) -> Arc<dyn Vfs> {
        match self {
            Self::Google(d) => Arc::new(CloudFs::with_root(d, root)),
            Self::OneDrive(d) => Arc::new(CloudFs::with_root(d, root)),
            Self::Dropbox(d) => Arc::new(CloudFs::with_root(d, root)),
        }
    }
}

/// An OAuth sign-in in progress (Google Drive, OneDrive, Dropbox).
/// `Debug` hides the tokens.
pub struct CloudSignIn {
    cloud: Cloud,
    endpoints: CloudEndpoints,
    request: AuthRequest,
    loopback: Option<Loopback>,
    own_client: CloudOptions,
    own_secret: Option<String>,
    tokens: Option<Tokens>,
    account: String,
}

impl std::fmt::Debug for CloudSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudSignIn")
            .field("cloud", &self.cloud)
            .field("signed_in", &self.tokens.is_some())
            .finish_non_exhaustive()
    }
}

impl CloudSignIn {
    /// Starts a sign-in to `cloud` (not MEGA) with the client of this
    /// build, or the user's own `client_id` (and secret). `app_redirect`:
    /// return to `kinolune://oauth` (Android, a client registered with it)
    /// instead of a loopback listener.
    ///
    /// # Errors
    ///
    /// [`Error::Server`] with [`ErrorKind::Unsupported`] when this build
    /// has no client id for the cloud and none was given.
    pub fn start(
        cloud: Cloud,
        client_id: Option<&str>,
        client_secret: Option<&str>,
        app_redirect: bool,
        endpoints: CloudEndpoints,
    ) -> Result<Self> {
        let own_id = client_id.map(str::trim).unwrap_or("").to_owned();
        let p =
            provider(cloud, &endpoints, &own_id, client_secret).map_err(|e| server_error(&e))?;
        let (loopback, redirect) = if app_redirect {
            (None, pa_cloud::oauth::APP_REDIRECT.to_owned())
        } else {
            let l = Loopback::bind().map_err(|e| server_error(&e))?;
            let r = l.redirect_uri();
            (Some(l), r)
        };
        let request = AuthRequest::new(p, redirect).map_err(|e| server_error(&e))?;
        Ok(Self {
            cloud,
            endpoints,
            request,
            loopback,
            own_secret: client_secret
                .map(str::trim)
                .filter(|s| !s.is_empty() && !own_id.is_empty())
                .map(str::to_owned),
            own_client: CloudOptions { client_id: own_id },
            tokens: None,
            account: String::new(),
        })
    }

    /// The cloud.
    pub fn cloud(&self) -> Cloud {
        self.cloud
    }

    /// The page to open in the browser.
    pub fn url(&self) -> String {
        self.request.url()
    }

    /// The signed-in account, once signed in.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Whether the sign-in is done.
    pub fn signed_in(&self) -> bool {
        self.tokens.is_some()
    }

    /// Waits up to `timeout` for the browser to come back to the loopback
    /// listener, then signs in; returns the account.
    ///
    /// # Errors
    ///
    /// [`Error::Server`]: timed out, cancelled, declined, refused.
    pub fn wait(&mut self, timeout: Duration, cancelled: &dyn Fn() -> bool) -> Result<String> {
        let target = self
            .loopback
            .as_ref()
            .ok_or_else(|| Error::Invalid("this sign-in returns to the app link".into()))?
            .wait(timeout, cancelled)
            .map_err(|e| server_error(&e))?;
        self.complete(&target)
    }

    /// Signs in with the redirect the browser came back with (the
    /// `kinolune://oauth?…` link, or a loopback URL); returns the account.
    ///
    /// # Errors
    ///
    /// [`Error::Server`]: declined, a foreign answer, refused code.
    pub fn complete(&mut self, redirect: &str) -> Result<String> {
        let code = self
            .request
            .code_from_redirect(redirect)
            .map_err(|e| server_error(&e))?;
        let tokens = self.request.exchange(&code).map_err(|e| server_error(&e))?;
        let source = Arc::new(TokenSource::from_tokens(
            self.request.provider().clone(),
            tokens.clone(),
            None,
        ));
        let account = oauth_drive(self.cloud, &self.endpoints, Arc::clone(&source))
            .account()
            .map_err(|e| server_error(&e))?;
        // The account call may have refreshed (and rotated) the token.
        let mut tokens = tokens;
        tokens.refresh = source.refresh_token().or(tokens.refresh);
        self.tokens = Some(tokens);
        self.account = account.clone();
        Ok(account)
    }
}

fn not_signed_in() -> Error {
    Error::Server {
        kind: ErrorKind::AuthenticationFailed,
        message: "the browser sign-in has not finished yet".into(),
    }
}

fn hash_of(parts: &[&str]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

impl SourceManager {
    /// Where the cloud APIs are; tests point them at mocks.
    pub fn set_cloud_endpoints(&self, endpoints: CloudEndpoints) {
        *self
            .cloud_endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = endpoints;
        self.cloud
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The cloud API endpoints in use.
    pub fn cloud_endpoints(&self) -> CloudEndpoints {
        self.cloud_endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Adds the cloud of a finished `sign_in` as a source named `name`
    /// (default: the cloud and the account), rooted at `folder` of the
    /// cloud (default: all of it).
    ///
    /// # Errors
    ///
    /// [`Error::Server`] when the sign-in has not finished; database and
    /// secret store errors.
    pub fn create_cloud(
        &self,
        sign_in: CloudSignIn,
        name: Option<&str>,
        folder: Option<&str>,
    ) -> Result<Source> {
        let tokens = sign_in.tokens.clone().ok_or_else(not_signed_in)?;
        let refresh = tokens.refresh.clone().ok_or_else(not_signed_in)?;
        let kind = SourceKind::of_cloud(sign_in.cloud);
        let options = SourceOptions {
            cloud: (!sign_in.own_client.client_id.is_empty()).then(|| sign_in.own_client.clone()),
            ..SourceOptions::default()
        };
        let mut new = NewSource::new(
            kind,
            default_name(sign_in.cloud, name, &sign_in.account),
            format!("{}:/{}", kind.as_str(), folder.unwrap_or("")),
        )
        .with_options(options);
        new.username = Some(sign_in.account.clone()).filter(|a| !a.trim().is_empty());
        new.password = Some(
            CloudSecret {
                refresh_token: refresh,
                client_secret: sign_in.own_secret.clone(),
            }
            .to_secret(),
        );
        self.create(new)
    }

    /// Signs in to MEGA and adds the account's Cloud Drive (or its
    /// `folder`) as a source. The password is used for the sign-in only;
    /// the session is stored.
    ///
    /// # Errors
    ///
    /// [`Error::Server`] with [`ErrorKind::AuthenticationFailed`]: wrong
    /// e-mail or password, or a 2FA code is needed (the message holds
    /// [`pa_cloud::mega::MFA_REQUIRED_TEXT`]) or wrong.
    pub fn create_mega(
        &self,
        email: &str,
        password: &SecretString,
        mfa: Option<&str>,
        name: Option<&str>,
        folder: Option<&str>,
    ) -> Result<Source> {
        let base = self.cloud_endpoints().mega;
        let session = pa_cloud::mega::login(&base, email, password.expose(), mfa).map_err(|e| {
            Error::Server {
                kind: e.kind(),
                message: crate::scrub(e.message(), Some(password)),
            }
        })?;
        let email = email.trim().to_lowercase();
        let mut new = NewSource::new(
            SourceKind::Mega,
            default_name(Cloud::Mega, name, &email),
            format!("mega:/{}", folder.unwrap_or("")),
        );
        new.username = Some(email);
        new.password = Some(SecretString::new(session.to_secret()));
        self.create(new)
    }

    /// Adds a public MEGA folder or file link (with its key after `#`).
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for a link without a valid key; [`Error::Server`]
    /// when MEGA does not know the link or the key does not open it.
    pub fn create_mega_link(&self, link: &str, name: Option<&str>) -> Result<Source> {
        let parsed =
            PublicLink::parse(link, None).map_err(|e| Error::Invalid(e.message().to_owned()))?;
        // Check it opens before saving it.
        let drive = MegaDrive::public(&self.cloud_endpoints().mega, &parsed)
            .map_err(|e| server_error(&e))?;
        CloudFs::new(drive)
            .list(&VfsPath::root())
            .map_err(|e| server_error(&e))?;
        let mut new = NewSource::new(
            SourceKind::Mega,
            name.map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or("MEGA"),
            parsed.address(),
        );
        new.password = Some(SecretString::new(parsed.key.clone()));
        self.create(new)
    }

    /// The tree of a cloud source with its secret; reused while the
    /// settings and the secret stay the same.
    pub(crate) fn cloud_tree(
        &self,
        stored: &Stored,
        secret: Option<SecretString>,
    ) -> pa_vfs::Result<Arc<dyn Vfs>> {
        let s = &stored.source;
        let cloud = s
            .kind
            .cloud()
            .ok_or_else(|| VfsError::new(ErrorKind::Unsupported, "not a cloud source"))?;
        let secret = secret.ok_or_else(|| {
            VfsError::new(
                ErrorKind::AuthenticationFailed,
                format!("not signed in to {}; sign in again", cloud.name()),
            )
        })?;
        let endpoints = self.cloud_endpoints();
        let options = serde_json::to_string(&s.options).unwrap_or_default();
        let fingerprint = hash_of(&[&s.uri, &options, secret.expose(), &format!("{endpoints:?}")]);
        if let Some((k, fs)) = self
            .cloud
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&s.id)
            && *k == fingerprint
        {
            return Ok(Arc::clone(fs));
        }
        let root = crate::address::cloud_root(&s.uri).unwrap_or_else(VfsPath::root);
        let fs: Arc<dyn Vfs> = match cloud {
            Cloud::Mega if PublicLink::is_link(&s.uri) => {
                let link = PublicLink::parse(&s.uri, Some(secret.expose()))?;
                Arc::new(CloudFs::new(MegaDrive::public(&endpoints.mega, &link)?))
            }
            Cloud::Mega => {
                let session = Session::from_secret(secret.expose())?;
                Arc::new(CloudFs::with_root(
                    MegaDrive::account(&endpoints.mega, &session)?,
                    root,
                ))
            }
            _ => {
                let stored_secret = CloudSecret::parse(secret.expose());
                let own = s.options.cloud.clone().unwrap_or_default();
                let p = provider(
                    cloud,
                    &endpoints,
                    &own.client_id,
                    stored_secret.client_secret.as_deref(),
                )?;
                let sink = stored
                    .credential_ref
                    .as_ref()
                    .and_then(|k| SecretKey::new(k.clone()).ok())
                    .map(|key| {
                        rotation_sink(
                            self.secret_store(),
                            key,
                            stored_secret.client_secret.clone(),
                        )
                    });
                let tokens = Arc::new(TokenSource::new(p, &stored_secret.refresh_token, sink));
                oauth_drive(cloud, &endpoints, tokens).into_vfs(root)
            }
        };
        self.cloud
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(s.id, (fingerprint, Arc::clone(&fs)));
        Ok(fs)
    }
}

/// Stores a rotated refresh token in place of the old one. A failed write
/// only means the old token is used next time (and the provider may ask
/// to sign in again).
fn rotation_sink(
    store: Arc<dyn SecretStore>,
    key: SecretKey,
    client_secret: Option<String>,
) -> TokenSink {
    Arc::new(move |refresh: &str| {
        let secret = CloudSecret {
            refresh_token: refresh.to_owned(),
            client_secret: client_secret.clone(),
        }
        .to_secret();
        let _ = store.set(&key, &secret);
    })
}

/// `name`, or "<cloud> · <account>".
fn default_name(cloud: Cloud, name: Option<&str>, account: &str) -> String {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_owned(),
        None if account.trim().is_empty() => cloud.name().to_owned(),
        None => format!("{} · {}", cloud.name(), account.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_secret_forms() {
        let s = CloudSecret {
            refresh_token: "r".into(),
            client_secret: None,
        };
        assert_eq!(CloudSecret::parse(s.to_secret().expose()), s);
        assert_eq!(CloudSecret::parse(" bare ").refresh_token, "bare");
        assert_eq!(format!("{s:?}"), "CloudSecret(***)");
    }

    #[test]
    fn names() {
        assert_eq!(
            default_name(Cloud::Dropbox, None, "a@b.c"),
            "Dropbox · a@b.c"
        );
        assert_eq!(default_name(Cloud::Mega, Some(" Films "), "x"), "Films");
        assert_eq!(default_name(Cloud::OneDrive, None, ""), "OneDrive");
    }
}
