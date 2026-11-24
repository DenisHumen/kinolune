// SPDX-License-Identifier: MPL-2.0
//! OAuth 2.0 for native apps (RFC 8252) with PKCE (RFC 7636): Kinolune is
//! a *public client*. There is no client secret in the code; the project's
//! client ids come from the build (`PA_*_CLIENT_ID` or, through the Flutter
//! build hook, `CARGO_PA_*_CLIENT_ID`; empty in the
//! repository, see `docs/dev/secrets.md`) and a user may enter their own
//! (advanced settings). Scopes are read-only.
//!
//! The sign-in: [`AuthRequest::new`] → open [`AuthRequest::url`] in the
//! browser → the provider redirects to the [`Loopback`] listener on
//! `127.0.0.1` (or, on Android, to the app's `kinolune://oauth` link, see
//! [`AuthRequest::code_from_redirect`]) → [`AuthRequest::exchange`] gives
//! [`Tokens`]. Only the refresh token is kept (in the secret store);
//! [`TokenSource`] gets fresh access tokens from it and hands a rotated
//! refresh token (Microsoft rotates them) to its [`TokenSink`].

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pa_vfs::{ErrorKind, VfsError};
use sha2::Digest;

use crate::http::{Http, Request, encode, status_error};
use crate::time::now_secs;

/// The loopback port tried first. Providers that match redirect URIs
/// exactly (Dropbox) have `http://127.0.0.1:47290/` registered; Google and
/// Microsoft accept any port of a loopback redirect, so a busy port falls
/// back to one the system picks.
pub const LOOPBACK_PORT: u16 = 47_290;

/// The redirect of the Android app (`kinolune://oauth`), for providers and
/// own client ids registered with it.
pub const APP_REDIRECT: &str = "kinolune://oauth";

/// An OAuth provider: endpoints, scopes and the client.
#[derive(Clone, PartialEq, Eq)]
pub struct Provider {
    /// `google`, `microsoft`, `dropbox`.
    pub id: &'static str,
    /// Authorization endpoint.
    pub auth_url: String,
    /// Token endpoint.
    pub token_url: String,
    /// Scopes, space-separated.
    pub scope: String,
    /// Extra parameters of the authorization request.
    pub extra: Vec<(&'static str, &'static str)>,
    /// The public client id; empty: not configured in this build.
    pub client_id: String,
    /// A client secret the provider insists on even for installed apps
    /// (Google "Desktop" clients). It is not confidential in a native app
    /// (RFC 8252 §8.5); it comes only from the build or the user, never
    /// from the repository.
    pub client_secret: Option<String>,
}

impl fmt::Debug for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .finish_non_exhaustive()
    }
}

fn build_env(value: Option<&'static str>) -> String {
    let v = value.unwrap_or("").trim();
    if v.eq_ignore_ascii_case("changeme") {
        String::new()
    } else {
        v.to_owned()
    }
}

impl Provider {
    /// Google (Drive, read-only). `drive.readonly` is the narrowest scope
    /// that sees files the app did not create; Google lists it as
    /// restricted (app verification; until then, up to 100 test users or
    /// the user's own client id).
    #[must_use]
    pub fn google() -> Self {
        let secret = build_env(
            option_env!("PA_GDRIVE_CLIENT_SECRET").or(option_env!("CARGO_PA_GDRIVE_CLIENT_SECRET")),
        );
        Self {
            id: "google",
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            scope: "https://www.googleapis.com/auth/drive.readonly".into(),
            extra: vec![("access_type", "offline"), ("prompt", "consent")],
            client_id: build_env(
                option_env!("PA_GDRIVE_CLIENT_ID").or(option_env!("CARGO_PA_GDRIVE_CLIENT_ID")),
            ),
            client_secret: (!secret.is_empty()).then_some(secret),
        }
    }

    /// Microsoft identity platform (OneDrive personal, work or school),
    /// public client, `Files.Read` and `offline_access`.
    #[must_use]
    pub fn microsoft() -> Self {
        Self {
            id: "microsoft",
            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
            scope: "Files.Read offline_access".into(),
            extra: vec![("prompt", "select_account")],
            client_id: build_env(
                option_env!("PA_ONEDRIVE_CLIENT_ID").or(option_env!("CARGO_PA_ONEDRIVE_CLIENT_ID")),
            ),
            client_secret: None,
        }
    }

    /// Dropbox, PKCE without a secret, `files.metadata.read`,
    /// `files.content.read` and `account_info.read`; `offline` gives a
    /// refresh token.
    #[must_use]
    pub fn dropbox() -> Self {
        Self {
            id: "dropbox",
            auth_url: "https://www.dropbox.com/oauth2/authorize".into(),
            token_url: "https://api.dropboxapi.com/oauth2/token".into(),
            scope: "files.metadata.read files.content.read account_info.read".into(),
            extra: vec![("token_access_type", "offline")],
            client_id: build_env(
                option_env!("PA_DROPBOX_CLIENT_ID").or(option_env!("CARGO_PA_DROPBOX_CLIENT_ID")),
            ),
            client_secret: None,
        }
    }

    /// The same provider with the user's own client (advanced settings).
    #[must_use]
    pub fn with_client(mut self, client_id: &str, client_secret: Option<&str>) -> Self {
        let id = client_id.trim();
        if !id.is_empty() {
            self.client_id = id.to_owned();
            self.client_secret = client_secret
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
        }
        self
    }

    /// The same provider with both endpoints at `base` (tests).
    #[must_use]
    pub fn at(mut self, base: &str) -> Self {
        self.auth_url = format!("{base}/authorize");
        self.token_url = format!("{base}/token");
        self
    }

    /// Whether this build (or the user) has a client id for it.
    #[must_use]
    pub fn configured(&self) -> bool {
        !self.client_id.is_empty()
    }
}

/// Access and refresh tokens. `Debug` hides them.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    /// Bearer token of API requests.
    pub access: String,
    /// What gets new access tokens; kept in the secret store.
    pub refresh: Option<String>,
    /// When `access` stops working, Unix seconds.
    pub expires_at: u64,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("refresh", &self.refresh.is_some())
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(serde::Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(serde::Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
}

fn random_b64(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    // Without randomness there is no safe sign-in; the OS always has it.
    getrandom::fill(&mut buf).expect("system randomness");
    URL_SAFE_NO_PAD.encode(buf)
}

/// The PKCE challenge (S256) of `verifier`.
#[must_use]
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// POSTs a token request.
fn token_request(
    http: &Http,
    p: &Provider,
    fields: &[(&str, &str)],
) -> pa_vfs::Result<TokenAnswer> {
    let mut all: Vec<(&str, &str)> = fields.to_vec();
    all.push(("client_id", p.client_id.as_str()));
    if let Some(s) = &p.client_secret {
        all.push(("client_secret", s.as_str()));
    }
    let r = http.send_retrying(3, &|| {
        Request::post(p.token_url.clone())
            .header("accept", "application/json")
            .form(&all)
    })?;
    if r.ok() {
        return r.json("token");
    }
    // `invalid_grant`: the refresh token was revoked or expired.
    let reason = serde_json::from_slice::<TokenError>(&r.body)
        .map(|e| e.error)
        .unwrap_or_default();
    if matches!(r.status, 400 | 401) && !reason.is_empty() {
        return Err(VfsError::new(
            ErrorKind::AuthenticationFailed,
            format!("sign-in refused ({reason}); sign in again"),
        ));
    }
    Err(status_error("token", r.status))
}

fn tokens_of(a: TokenAnswer, old_refresh: Option<&str>) -> Tokens {
    Tokens {
        access: a.access_token,
        refresh: a.refresh_token.or_else(|| old_refresh.map(str::to_owned)),
        expires_at: now_secs() + a.expires_in.unwrap_or(3600),
    }
}

/// A sign-in in progress: the PKCE verifier, the state and the redirect.
/// `Debug` hides the verifier.
pub struct AuthRequest {
    provider: Provider,
    verifier: String,
    state: String,
    redirect_uri: String,
}

impl fmt::Debug for AuthRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthRequest")
            .field("provider", &self.provider.id)
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

impl AuthRequest {
    /// A new sign-in with `provider` returning to `redirect_uri`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unsupported`] if the provider has no client id in this
    /// build (and the user did not enter one).
    pub fn new(provider: Provider, redirect_uri: impl Into<String>) -> pa_vfs::Result<Self> {
        if !provider.configured() {
            return Err(VfsError::new(
                ErrorKind::Unsupported,
                format!("no client id for {} in this build", provider.id),
            ));
        }
        Ok(Self {
            provider,
            verifier: random_b64(48),
            state: random_b64(16),
            redirect_uri: redirect_uri.into(),
        })
    }

    /// The provider.
    #[must_use]
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// The `state` the redirect must carry back.
    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    /// The redirect URI.
    #[must_use]
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// The page to open in the browser.
    #[must_use]
    pub fn url(&self) -> String {
        let p = &self.provider;
        let mut q = vec![
            ("response_type", "code"),
            ("client_id", p.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", p.scope.as_str()),
            ("state", self.state.as_str()),
            ("code_challenge_method", "S256"),
        ];
        let challenge = pkce_challenge(&self.verifier);
        q.push(("code_challenge", challenge.as_str()));
        q.extend(p.extra.iter().copied());
        let query: Vec<String> = q
            .iter()
            .map(|(k, v)| format!("{k}={}", encode(v)))
            .collect();
        format!("{}?{}", p.auth_url, query.join("&"))
    }

    /// The code of a redirect URL (`…?code=…&state=…`): checks the state
    /// and turns a provider error (`access_denied`) into an error.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::PermissionDenied`] when the user declined,
    /// [`ErrorKind::AuthenticationFailed`] on a wrong state or no code.
    pub fn code_from_redirect(&self, redirect: &str) -> pa_vfs::Result<String> {
        let url = url::Url::parse(redirect)
            .or_else(|_| url::Url::parse(&format!("http://127.0.0.1{redirect}")))
            .map_err(|_| VfsError::new(ErrorKind::InvalidPath, "malformed redirect"))?;
        let get = |k: &str| {
            url.query_pairs()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.into_owned())
        };
        if let Some(e) = get("error") {
            let kind = if e == "access_denied" {
                ErrorKind::PermissionDenied
            } else {
                ErrorKind::AuthenticationFailed
            };
            return Err(VfsError::new(kind, format!("sign-in refused ({e})")));
        }
        if get("state").as_deref() != Some(self.state.as_str()) {
            return Err(VfsError::new(
                ErrorKind::AuthenticationFailed,
                "the sign-in answer does not belong to this sign-in",
            ));
        }
        get("code")
            .filter(|c| !c.is_empty())
            .ok_or_else(|| VfsError::new(ErrorKind::AuthenticationFailed, "no code in the answer"))
    }

    /// Exchanges `code` for tokens.
    ///
    /// # Errors
    ///
    /// The provider refuses the code, network errors.
    pub fn exchange(&self, code: &str) -> pa_vfs::Result<Tokens> {
        let http = Http::default();
        let a = token_request(
            &http,
            &self.provider,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", self.verifier.as_str()),
                ("redirect_uri", self.redirect_uri.as_str()),
            ],
        )?;
        let t = tokens_of(a, None);
        if t.refresh.is_none() {
            return Err(VfsError::new(
                ErrorKind::AuthenticationFailed,
                "the provider gave no refresh token",
            ));
        }
        Ok(t)
    }
}

/// A one-shot listener on `127.0.0.1` for the redirect of a sign-in.
#[derive(Debug)]
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

/// Page shown in the browser after the redirect.
const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Kinolune</title>\
<body style=\"font-family:sans-serif;text-align:center;margin-top:20vh\">\
<h2>Kinolune</h2><p>Вход выполнен — вернитесь в приложение.</p>\
<p>Signed in — return to the app.</p>";

impl Loopback {
    /// Listens on [`LOOPBACK_PORT`], or a port the system picks when it is
    /// busy.
    ///
    /// # Errors
    ///
    /// No loopback socket can be opened.
    pub fn bind() -> pa_vfs::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", LOOPBACK_PORT))
            .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
            .map_err(io)?;
        let port = listener.local_addr().map_err(io)?.port();
        Ok(Self { listener, port })
    }

    /// `http://127.0.0.1:<port>/`.
    #[must_use]
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Waits up to `timeout` for the browser to come back and returns the
    /// path and query it asked for (`/?code=…&state=…`). Other requests
    /// (`/favicon.ico`) get a 404 and are skipped.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::TimedOut`] when nobody came back in time,
    /// [`ErrorKind::Cancelled`] when `cancelled` turned true.
    pub fn wait(&self, timeout: Duration, cancelled: &dyn Fn() -> bool) -> pa_vfs::Result<String> {
        self.listener.set_nonblocking(true).map_err(io)?;
        let deadline = Instant::now() + timeout;
        loop {
            if cancelled() {
                return Err(VfsError::new(ErrorKind::Cancelled, "sign-in cancelled"));
            }
            if Instant::now() >= deadline {
                return Err(VfsError::new(
                    ErrorKind::TimedOut,
                    "the browser did not come back",
                ));
            }
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if let Some(target) = answer(stream, peer)? {
                        return Ok(target);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(io(e)),
            }
        }
    }
}

fn io(e: std::io::Error) -> VfsError {
    VfsError::from_io(e, "sign-in listener")
}

/// Reads one request; returns its target if it carries a sign-in answer.
fn answer(stream: TcpStream, peer: SocketAddr) -> pa_vfs::Result<Option<String>> {
    if !peer.ip().is_loopback() {
        return Ok(None);
    }
    stream.set_nonblocking(false).map_err(io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(io)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return Ok(None);
    }
    // Drain the headers.
    let mut h = String::new();
    while reader.read_line(&mut h).is_ok_and(|n| n > 2) {
        h.clear();
    }
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let is_answer = target.contains("code=") || target.contains("error=");
    let mut stream = reader.into_inner();
    let (status, body) = if is_answer {
        ("200 OK", DONE_PAGE)
    } else {
        ("404 Not Found", "")
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
    Ok(is_answer.then_some(target))
}

/// Receives a rotated refresh token to store in place of the old one.
pub type TokenSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Fresh access tokens from a refresh token, shared by the requests of
/// one source.
pub struct TokenSource {
    provider: Provider,
    http: Http,
    state: Mutex<Tokens>,
    sink: Option<TokenSink>,
}

impl fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSource")
            .field("provider", &self.provider.id)
            .finish_non_exhaustive()
    }
}

impl TokenSource {
    /// Tokens of `provider` from a stored `refresh_token`; `sink` gets a
    /// new refresh token when the provider rotates it.
    #[must_use]
    pub fn new(provider: Provider, refresh_token: &str, sink: Option<TokenSink>) -> Self {
        Self {
            provider,
            http: Http::default(),
            state: Mutex::new(Tokens {
                access: String::new(),
                refresh: Some(refresh_token.to_owned()),
                expires_at: 0,
            }),
            sink,
        }
    }

    /// Tokens from a fresh sign-in (no refresh needed yet).
    #[must_use]
    pub fn from_tokens(provider: Provider, tokens: Tokens, sink: Option<TokenSink>) -> Self {
        Self {
            provider,
            http: Http::default(),
            state: Mutex::new(tokens),
            sink,
        }
    }

    /// A valid access token, refreshed when it expires within a minute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::AuthenticationFailed`] when the refresh token no longer
    /// works (revoked, expired): the user signs in again.
    pub fn access(&self) -> pa_vfs::Result<String> {
        let mut t = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !t.access.is_empty() && t.expires_at > now_secs() + 60 {
            return Ok(t.access.clone());
        }
        let refresh = t.refresh.clone().ok_or_else(|| {
            VfsError::new(
                ErrorKind::AuthenticationFailed,
                "no refresh token; sign in again",
            )
        })?;
        let a = token_request(
            &self.http,
            &self.provider,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
            ],
        )?;
        let fresh = tokens_of(a, Some(&refresh));
        if let (Some(sink), Some(new)) = (&self.sink, &fresh.refresh)
            && *new != refresh
        {
            sink(new);
        }
        *t = fresh;
        Ok(t.access.clone())
    }

    /// Forgets the access token after the API refused it (401).
    pub fn invalidate(&self) {
        let mut t = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        t.access.clear();
        t.expires_at = 0;
    }

    /// The current refresh token (to store after a sign-in).
    #[must_use]
    pub fn refresh_token(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .refresh
            .clone()
    }
}

/// Sends a request with a bearer token; on a 401 the token is refreshed
/// and the request sent once more.
pub(crate) fn send_authed(
    http: &Http,
    tokens: &TokenSource,
    make: &dyn Fn(&str) -> Request,
) -> pa_vfs::Result<crate::http::Response> {
    let token = tokens.access()?;
    let r = http.send_retrying(3, &|| make(&token))?;
    if r.status != 401 {
        return Ok(r);
    }
    tokens.invalidate();
    let token = tokens.access()?;
    http.send_retrying(3, &|| make(&token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_rfc7636_vector() {
        // RFC 7636 appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEXJQjDZ2IoOGuSvNd8BRa3plbZy8o"
        );
    }

    #[test]
    fn auth_url_and_redirect_checks() {
        let p = Provider::dropbox().with_client("my-app", None);
        let r = AuthRequest::new(p, "http://127.0.0.1:47290/").unwrap();
        let url = r.url();
        assert!(url.starts_with("https://www.dropbox.com/oauth2/authorize?"));
        assert!(url.contains("client_id=my-app"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("token_access_type=offline"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A47290%2F"));
        assert!(!url.contains(&r.verifier));

        let ok = format!("/?code=abc&state={}", r.state());
        assert_eq!(r.code_from_redirect(&ok).unwrap(), "abc");
        let app = format!("kinolune://oauth?state={}&code=xyz", r.state());
        assert_eq!(r.code_from_redirect(&app).unwrap(), "xyz");
        assert_eq!(
            r.code_from_redirect("/?code=abc&state=other")
                .unwrap_err()
                .kind(),
            ErrorKind::AuthenticationFailed
        );
        assert_eq!(
            r.code_from_redirect("/?error=access_denied")
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn unconfigured_provider_is_refused() {
        let mut p = Provider::microsoft();
        p.client_id.clear();
        assert_eq!(
            AuthRequest::new(p, "http://127.0.0.1:1/")
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
    }

    #[test]
    fn loopback_receives_the_redirect() {
        let lb = Loopback::bind().unwrap();
        let uri = lb.redirect_uri();
        let t = std::thread::spawn(move || {
            let addr = uri.trim_start_matches("http://").trim_end_matches('/');
            // A stray request first, then the answer.
            for target in ["/favicon.ico", "/?code=c1&state=s1"] {
                let mut s = TcpStream::connect(addr).unwrap();
                write!(s, "GET {target} HTTP/1.1\r\nhost: x\r\n\r\n").unwrap();
                let mut out = String::new();
                let _ = std::io::Read::read_to_string(&mut s, &mut out);
            }
        });
        let got = lb.wait(Duration::from_secs(10), &|| false).unwrap();
        assert_eq!(got, "/?code=c1&state=s1");
        t.join().unwrap();
    }
}
