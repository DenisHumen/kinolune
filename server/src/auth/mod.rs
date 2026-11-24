// SPDX-License-Identifier: MPL-2.0
//! Accounts, sign-in and tokens of the server (SRV-003, ADR-NNNN). Fully
//! offline: no cloud account, everything lives in the data directory.
//!
//! - **Accounts** ([`store`]): local users, an administrator and household
//!   members. Passwords and PINs are stored as Argon2id hashes
//!   (`pa_library::profiles`, the same format as profile PINs).
//! - **First start:** while no administrator exists, one can be created from
//!   the server itself (loopback) or with the one-time setup code from
//!   `<data_dir>/setup-code.txt` ([`SETUP_CODE_FILE`]).
//! - **Devices:** a client pairs with the server first (SYNC-005: QR or a
//!   6-digit code, [`crate::pairing`]), then proves its device key to get a
//!   device session (`POST /api/v1/auth/challenge` + `/device`). With it a
//!   household member picks a profile and enters a PIN. A PIN works only on
//!   a paired device; a password works anywhere.
//! - **Tokens:** access tokens are PASETO `v4.public` ([`paseto`]), valid
//!   for [`ACCESS_TTL`], checked against the session on every request, so a
//!   revoked session or device loses access at once. Refresh tokens are
//!   opaque, single-use and rotate on every exchange; a used one presented
//!   again revokes the whole session (reuse detection).
//! - **Brute force** ([`limit`]): failures per account and per address,
//!   exponential lockout.
//! - **Revocation:** revoking a device (`DELETE /api/v1/devices/{id}` or a
//!   family update from another member) revokes its sessions.

pub(crate) mod api;
pub mod limit;
pub mod paseto;
pub mod store;

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{SigningKey, VerifyingKey};
use pa_library::profiles::{hash_pin, hash_secret, pin_matches};
use pa_secrets::{SecretKey, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::Error;
use crate::pairing::Pairing;
use limit::{ACCOUNT, ADDRESS, Limiter};
use store::{Refresh, Session, Store, User};

pub use crate::config::AuthConfig;

/// Lifetime of an access token.
pub const ACCESS_TTL: Duration = Duration::from_secs(15 * 60);
/// Lifetime of a refresh token; every exchange gives a new one.
pub const REFRESH_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// One-time code for creating the first administrator from another machine.
pub const SETUP_CODE_FILE: &str = "setup-code.txt";
/// How long a device challenge is valid.
pub const CHALLENGE_TTL: Duration = Duration::from_secs(60);

/// The signing key in the server's encrypted secrets file.
const SIGNING_KEY_SECRET: &str = "server.auth.signing-key";
/// Open device challenges at once (memory guard).
const MAX_CHALLENGES: usize = 1024;
/// Password length in characters (NIST SP 800-63B: at least 8).
const PASSWORD_CHARS: std::ops::RangeInclusive<usize> = 8..=256;
/// User name length in characters.
const NAME_CHARS: std::ops::RangeInclusive<usize> = 1..=64;

/// Role of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Manages the server: users, devices, pairing.
    Admin,
    /// A household member.
    Member,
}

/// What a session may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// An administrator.
    Admin,
    /// A household member.
    Member,
    /// A paired device before anybody signed in on it.
    Device,
}

/// How a session was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// Name and password.
    Password,
    /// Profile PIN on a paired device.
    Pin,
    /// The paired device key.
    Device,
}

/// Who makes a request: a checked access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Session id.
    pub session_id: String,
    /// The signed-in user.
    pub user_id: Option<String>,
    /// The paired device the session is bound to.
    pub device_id: Option<String>,
    /// What the session may do.
    pub scope: Scope,
    /// How it was opened.
    pub method: Method,
}

impl Principal {
    /// Administration needs an administrator who signed in with a password
    /// (a PIN is for picking a profile on the sofa, not for managing).
    pub fn is_admin(&self) -> bool {
        self.scope == Scope::Admin && self.method == Method::Password
    }
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// Malformed or invalid input (400).
    #[error("{0}")]
    Invalid(String),
    /// No valid credentials (401).
    #[error("{0}")]
    Unauthorized(&'static str),
    /// Valid credentials without the right (403).
    #[error("{0}")]
    Forbidden(&'static str),
    /// No such object (404).
    #[error("{0}")]
    NotFound(&'static str),
    /// Conflicts with the current state (409).
    #[error("{0}")]
    Conflict(&'static str),
    /// Too many failed attempts: try again after this (429).
    #[error("too many attempts, try again later")]
    Locked(Duration),
    /// Storage failed (500).
    #[error("internal error")]
    Internal(String),
}

impl From<pa_library::rusqlite::Error> for AuthError {
    fn from(e: pa_library::rusqlite::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

/// Access and refresh tokens of a session.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Tokens {
    /// PASETO `v4.public` access token for `Authorization: Bearer`.
    pub access_token: String,
    /// Always `Bearer`.
    pub token_type: String,
    /// Seconds the access token is valid.
    pub expires_in: u64,
    /// Single-use refresh token: exchange it at `/api/v1/auth/refresh`
    /// for a new pair. Presenting a used one again ends the session.
    pub refresh_token: String,
    /// Seconds the refresh token is valid.
    pub refresh_expires_in: u64,
    /// What the session may do.
    pub scope: Scope,
}

/// A user, without secrets.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserInfo {
    /// User id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Role.
    pub role: Role,
    /// Signs in with a password.
    pub has_password: bool,
    /// Has a profile PIN.
    pub has_pin: bool,
    /// Created, Unix ms.
    pub created_ms: i64,
}

impl From<&User> for UserInfo {
    fn from(u: &User) -> Self {
        Self {
            id: u.id.clone(),
            name: u.name.clone(),
            role: u.role,
            has_password: u.password_hash.is_some(),
            has_pin: u.pin_hash.is_some(),
            created_ms: u.created_ms,
        }
    }
}

/// A device challenge.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Challenge {
    /// Nonce to sign (64 hex characters).
    pub challenge: String,
    /// Id of the server: the verifier in the signed transcript.
    pub server_id: String,
    /// Expiry, Unix ms.
    pub expires_ms: i64,
}

/// Changes of an account; absent fields stay.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UserPatch {
    /// New password; an empty string removes it (not for administrators).
    #[serde(default)]
    pub password: Option<String>,
    /// New PIN of 4–8 digits; an empty string removes it.
    #[serde(default)]
    pub pin: Option<String>,
}

/// A new account.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewUser {
    /// Name, unique without regard to case.
    pub name: String,
    /// `member` (default) or `admin`.
    #[serde(default)]
    pub role: Option<Role>,
    /// Password (required for an administrator).
    #[serde(default)]
    pub password: Option<String>,
    /// PIN of 4–8 digits.
    #[serde(default)]
    pub pin: Option<String>,
}

/// Claims of an access token.
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    iat: String,
    exp: String,
    jti: String,
    sid: String,
    scope: Scope,
    amr: Method,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dev: Option<String>,
}

struct Inner {
    store: Mutex<Store>,
    key: SigningKey,
    public: VerifyingKey,
    footer: Vec<u8>,
    issuer: String,
    pairing: Pairing,
    limiter: Limiter,
    challenges: Mutex<HashMap<String, Instant>>,
    trust_loopback: bool,
    has_admin: AtomicBool,
    setup_code: Mutex<Option<String>>,
    setup_file: PathBuf,
    /// Checked against for unknown names, so they take as long as known ones.
    dummy_hash: String,
}

/// Accounts and tokens; cheap to clone.
#[derive(Clone)]
pub struct Auth {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    crate::sync::now_ms()
}

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("system random source");
    out
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn sha256_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// Equality in time independent of where the inputs differ.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Days since 1970-01-01 to a civil date (H. Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix ms as the RFC 3339 time of PASETO claims (`…T…+00:00`).
fn rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

/// Parses what [`rfc3339`] writes (also with `Z`); Unix ms.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.strip_suffix("+00:00").or_else(|| s.strip_suffix('Z'))?;
    let (date, time) = s.split_once('T')?;
    let num = |p: &str| -> Option<i64> {
        p.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| p.parse().ok())
            .flatten()
    };
    let mut date = date.split('-');
    let (y, m, d) = (num(date.next()?)?, num(date.next()?)?, num(date.next()?)?);
    let mut time = time.split(':');
    let (hh, mm, ss) = (num(time.next()?)?, num(time.next()?)?, num(time.next()?)?);
    if date.next().is_some()
        || time.next().is_some()
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
        || hh > 23
        || mm > 59
        || ss > 59
    {
        return None;
    }
    Some((days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss) * 1000)
}

/// A one-time setup code: 16 characters of an unambiguous alphabet in groups
/// of four (80 bits).
fn new_setup_code() -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes = random::<16>();
    let mut code = String::with_capacity(19);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            code.push('-');
        }
        code.push(char::from(ALPHABET[usize::from(b % 32)]));
    }
    code
}

fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

fn check_name(name: &str) -> Result<String, AuthError> {
    let name = name.trim();
    if !NAME_CHARS.contains(&name.chars().count()) || name.chars().any(char::is_control) {
        return Err(AuthError::Invalid(
            "name: 1-64 characters, no control characters".into(),
        ));
    }
    Ok(name.to_owned())
}

fn hash_password(password: &str) -> Result<String, AuthError> {
    if !PASSWORD_CHARS.contains(&password.chars().count()) {
        return Err(AuthError::Invalid("password: 8-256 characters".into()));
    }
    hash_secret(password).map_err(|e| AuthError::Internal(e.to_string()))
}

fn hash_user_pin(pin: &str) -> Result<String, AuthError> {
    hash_pin(pin).map_err(|_| AuthError::Invalid("PIN: 4-8 digits".into()))
}

/// Runs a blocking `f` on the blocking pool.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
}

impl Auth {
    /// Opens `auth.sqlite3` in `data_dir`, loads or creates the signing key
    /// in the family's secrets file and, without an administrator, writes a
    /// setup code. Blocking.
    pub fn open(data_dir: &Path, pairing: Pairing, config: &AuthConfig) -> Result<Self, Error> {
        let path = data_dir.join(store::FILE);
        let err = |e: String| Error::Auth(format!("{}: {e}", path.display()));
        let store = Store::open(&path).map_err(|e| err(e.to_string()))?;
        let key = Self::signing_key(&pairing).map_err(err)?;
        let public = key.verifying_key();
        let kid = hex(&Sha256::digest(public.as_bytes())[..8]);
        let footer = serde_json::to_vec(&serde_json::json!({ "kid": kid }))
            .expect("the footer serializes");
        let has_admin = store.has_admin().map_err(|e| err(e.to_string()))?;
        let dummy_hash = hash_secret("not anybody's password").map_err(|e| err(e.to_string()))?;
        let setup_file = data_dir.join(SETUP_CODE_FILE);
        let auth = Self {
            inner: Arc::new(Inner {
                store: Mutex::new(store),
                key,
                public,
                footer,
                issuer: pairing.device_id(),
                pairing,
                limiter: Limiter::new(),
                challenges: Mutex::new(HashMap::new()),
                trust_loopback: config.trust_loopback,
                has_admin: AtomicBool::new(has_admin),
                setup_code: Mutex::new(None),
                setup_file,
            dummy_hash,
            }),
        };
        if has_admin {
            let _ = std::fs::remove_file(&auth.inner.setup_file);
        } else {
            let code = new_setup_code();
            match write_private(&auth.inner.setup_file, &format!("{code}\n")) {
                Ok(()) => tracing::info!(
                    file = %auth.inner.setup_file.display(),
                    "no administrator yet: create one from this machine, or elsewhere with \
                     the setup code from the file"
                ),
                Err(e) => tracing::warn!(error = %e, "setup code not written; set up from this machine"),
            }
            *lock(&auth.inner.setup_code) = Some(code);
        }
        Ok(auth)
    }

    fn signing_key(pairing: &Pairing) -> Result<SigningKey, String> {
        let store = pairing.secrets();
        let name = SecretKey::new(SIGNING_KEY_SECRET).map_err(|e| e.to_string())?;
        if let Some(seed) = store.get(&name).map_err(|e| e.to_string())? {
            let seed = unhex32(seed.expose()).ok_or("malformed signing key")?;
            return Ok(SigningKey::from_bytes(&seed));
        }
        let seed = random::<32>();
        store
            .set(&name, &SecretString::new(hex(&seed)))
            .map_err(|e| e.to_string())?;
        tracing::info!("created the token signing key");
        Ok(SigningKey::from_bytes(&seed))
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        lock(&self.inner.store)
    }

    /// Whether an administrator account exists.
    pub fn has_admin(&self) -> bool {
        self.inner.has_admin.load(Ordering::Acquire)
    }

    /// The public key of access tokens (for tests and other members).
    pub fn public_key(&self) -> VerifyingKey {
        self.inner.public
    }

    /// Allows administration: an administrator's password session, or the
    /// server itself (loopback) while no administrator exists or with
    /// `[auth] trust_loopback`.
    pub fn require_admin(&self, who: Option<&Principal>, peer: IpAddr) -> Result<(), AuthError> {
        match who {
            Some(p) if p.is_admin() => Ok(()),
            Some(_) => Err(AuthError::Forbidden(
                "an administrator signed in with a password is required",
            )),
            None if peer.is_loopback() && (self.inner.trust_loopback || !self.has_admin()) => {
                Ok(())
            }
            None => Err(AuthError::Unauthorized("sign-in required")),
        }
    }

    // --- tokens ---------------------------------------------------------------

    fn access_token(&self, session: &Session, now: i64) -> String {
        let claims = Claims {
            iss: self.inner.issuer.clone(),
            sub: session
                .user_id
                .clone()
                .or_else(|| session.device_id.clone())
                .unwrap_or_default(),
            iat: rfc3339(now),
            exp: rfc3339(now + ms(ACCESS_TTL)),
            jti: hex(&random::<16>()),
            sid: session.id.clone(),
            scope: session.scope,
            amr: session.method,
            dev: session.device_id.clone(),
        };
        let message = serde_json::to_vec(&claims).expect("claims serialize");
        paseto::sign(&self.inner.key, &message, &self.inner.footer, b"")
    }

    fn issue(&self, store: &Store, session: &Session, now: i64) -> Result<Tokens, AuthError> {
        let refresh = URL_SAFE_NO_PAD.encode(random::<32>());
        store.insert_refresh(&sha256_hex(&refresh), &session.id, now + ms(REFRESH_TTL))?;
        Ok(Tokens {
            access_token: self.access_token(session, now),
            token_type: "Bearer".to_owned(),
            expires_in: ACCESS_TTL.as_secs(),
            refresh_token: refresh,
            refresh_expires_in: REFRESH_TTL.as_secs(),
            scope: session.scope,
        })
    }

    fn open_session(
        &self,
        user: Option<&User>,
        device_id: Option<String>,
        method: Method,
    ) -> Result<Tokens, AuthError> {
        let now = now_ms();
        let session = Session {
            id: hex(&random::<16>()),
            user_id: user.map(|u| u.id.clone()),
            device_id,
            scope: match user.map(|u| u.role) {
                Some(Role::Admin) => Scope::Admin,
                Some(Role::Member) => Scope::Member,
                None => Scope::Device,
            },
            method,
            created_ms: now,
            revoked_ms: None,
        };
        let store = self.store();
        store.prune(now - ms(REFRESH_TTL))?;
        store.insert_session(&session)?;
        self.issue(&store, &session, now)
    }

    /// Checks an access token: signature, issuer, expiry, a live session and,
    /// for a device-bound session, a device that is still in the family.
    pub async fn authenticate(&self, token: String) -> Result<Principal, AuthError> {
        let this = self.clone();
        blocking(move || this.authenticate_blocking(&token)).await
    }

    fn authenticate_blocking(&self, token: &str) -> Result<Principal, AuthError> {
        const BAD: AuthError = AuthError::Unauthorized("invalid access token");
        let footer = paseto::footer(token).map_err(|_| BAD)?;
        if !ct_eq(&footer, &self.inner.footer) {
            return Err(AuthError::Unauthorized("access token of another key"));
        }
        let (message, _) = paseto::verify(&self.inner.public, token, b"").map_err(|_| BAD)?;
        let claims: Claims = serde_json::from_slice(&message).map_err(|_| BAD)?;
        if claims.iss != self.inner.issuer {
            return Err(BAD);
        }
        let exp = parse_rfc3339(&claims.exp).ok_or(BAD)?;
        let now = now_ms();
        if exp <= now {
            return Err(AuthError::Unauthorized("access token expired"));
        }
        let store = self.store();
        let session = store.session(&claims.sid)?.ok_or(BAD)?;
        if session.revoked_ms.is_some() {
            return Err(AuthError::Unauthorized("session revoked"));
        }
        self.check_device(&store, &session, now)?;
        Ok(Principal {
            session_id: session.id,
            user_id: session.user_id,
            device_id: session.device_id,
            scope: session.scope,
            method: session.method,
        })
    }

    /// A session bound to a device that left the family is revoked.
    fn check_device(&self, store: &Store, session: &Session, now: i64) -> Result<(), AuthError> {
        if let Some(device) = &session.device_id
            && !self.inner.pairing.is_active(device)
        {
            store.revoke_device_sessions(device, "device revoked", now)?;
            return Err(AuthError::Unauthorized("device revoked"));
        }
        Ok(())
    }

    // --- sign-in --------------------------------------------------------------

    fn throttle(&self, keys: &[&str]) -> Result<(), AuthError> {
        self.inner
            .limiter
            .check(keys, Instant::now())
            .map_err(AuthError::Locked)
    }

    fn failed(&self, account: &str, address: &str) {
        let now = Instant::now();
        let lock = self.inner.limiter.fail(account, &ACCOUNT, now);
        let ip_lock = self.inner.limiter.fail(address, &ADDRESS, now);
        let lock = lock.max(ip_lock);
        if !lock.is_zero() {
            tracing::warn!(account, address, lock_secs = lock.as_secs(), "auth: locked after failed attempts");
        }
    }

    /// Name and password. With `device` (a device session in the request) the
    /// new session is bound to that device and ends with it.
    pub async fn login(
        &self,
        name: String,
        password: String,
        peer: IpAddr,
        device: Option<Principal>,
    ) -> Result<Tokens, AuthError> {
        let this = self.clone();
        blocking(move || {
            let address = format!("ip:{peer}");
            let user = this.store().user_by_name(name.trim())?;
            let account = match &user {
                Some(u) => format!("user:{}", u.id),
                None => format!("name:{}", name.trim().to_lowercase()),
            };
            this.throttle(&[&account, &address])?;
            let hash = user.as_ref().and_then(|u| u.password_hash.as_deref());
            let ok = pin_matches(hash.unwrap_or(&this.inner.dummy_hash), &password) && hash.is_some();
            let Some(user) = user.filter(|_| ok) else {
                this.failed(&account, &address);
                return Err(AuthError::Unauthorized("wrong name or password"));
            };
            this.inner.limiter.succeed(&account);
            let device = device.and_then(|p| p.device_id);
            tracing::info!(user = %user.id, device = ?device, "auth: signed in with a password");
            this.open_session(Some(&user), device, Method::Password)
        })
        .await
    }

    /// Profile PIN of `user_id` on the paired device of `device`.
    pub async fn login_pin(
        &self,
        user_id: String,
        pin: String,
        peer: IpAddr,
        device: Principal,
    ) -> Result<Tokens, AuthError> {
        let Some(device_id) = device.device_id else {
            return Err(AuthError::Forbidden("a PIN works only on a paired device"));
        };
        let this = self.clone();
        blocking(move || {
            let address = format!("ip:{peer}");
            let account = format!("user:{user_id}");
            this.throttle(&[&account, &address])?;
            let user = this.store().user(&user_id)?;
            let hash = user.as_ref().and_then(|u| u.pin_hash.as_deref());
            let ok = pin_matches(hash.unwrap_or(&this.inner.dummy_hash), &pin) && hash.is_some();
            let Some(user) = user.filter(|_| ok) else {
                this.failed(&account, &address);
                return Err(AuthError::Unauthorized("wrong profile or PIN"));
            };
            this.inner.limiter.succeed(&account);
            tracing::info!(user = %user.id, device = %device_id, "auth: signed in with a PIN");
            this.open_session(Some(&user), Some(device_id), Method::Pin)
        })
        .await
    }

    /// A fresh nonce for a device to sign.
    pub fn challenge(&self) -> Result<Challenge, AuthError> {
        let now = Instant::now();
        let mut challenges = lock(&self.inner.challenges);
        challenges.retain(|_, expires| *expires > now);
        if challenges.len() >= MAX_CHALLENGES {
            return Err(AuthError::Locked(CHALLENGE_TTL));
        }
        let nonce = pa_pairing::auth::nonce().map_err(|e| AuthError::Internal(e.to_string()))?;
        challenges.insert(nonce.clone(), now + CHALLENGE_TTL);
        Ok(Challenge {
            challenge: nonce,
            server_id: self.inner.issuer.clone(),
            expires_ms: now_ms() + ms(CHALLENGE_TTL),
        })
    }

    /// A device session for a paired device that signed `challenge`
    /// (`pa_pairing::auth::prove` with the server as verifier, `nonce` as
    /// its own nonce).
    pub async fn login_device(
        &self,
        device_id: String,
        nonce: String,
        challenge: String,
        sig: String,
        peer: IpAddr,
    ) -> Result<Tokens, AuthError> {
        let this = self.clone();
        blocking(move || {
            let address = format!("ip:{peer}");
            let account = format!("device:{device_id}");
            this.throttle(&[&account, &address])?;
            let issued = lock(&this.inner.challenges).remove(&challenge);
            if !issued.is_some_and(|expires| expires > Instant::now()) {
                return Err(AuthError::Unauthorized("unknown or expired challenge"));
            }
            if let Err(e) = this.inner.pairing.verify(&device_id, &nonce, &challenge, &sig) {
                tracing::info!(device = %device_id, reason = %e, "auth: device not admitted");
                this.failed(&account, &address);
                return Err(AuthError::Unauthorized("device is not paired"));
            }
            this.inner.limiter.succeed(&account);
            tracing::info!(device = %device_id, "auth: device signed in");
            this.open_session(None, Some(device_id), Method::Device)
        })
        .await
    }

    /// Exchanges a refresh token for a new pair. A token that was already
    /// used revokes its whole session.
    pub async fn refresh(&self, token: String) -> Result<Tokens, AuthError> {
        let this = self.clone();
        blocking(move || this.refresh_blocking(&token)).await
    }

    fn refresh_blocking(&self, token: &str) -> Result<Tokens, AuthError> {
        let hash = sha256_hex(token);
        let now = now_ms();
        let store = self.store();
        let (session_id, expires_ms) = match store.refresh(&hash)? {
            Refresh::Unknown => return Err(AuthError::Unauthorized("unknown refresh token")),
            Refresh::Used { session_id } => {
                store.revoke_session(&session_id, "refresh token reuse", now)?;
                tracing::warn!(session = %session_id, "auth: refresh token reused, session revoked");
                return Err(AuthError::Unauthorized("refresh token reused; session revoked"));
            }
            Refresh::Fresh {
                session_id,
                expires_ms,
            } => (session_id, expires_ms),
        };
        if expires_ms <= now {
            return Err(AuthError::Unauthorized("refresh token expired"));
        }
        let session = store
            .session(&session_id)?
            .ok_or(AuthError::Unauthorized("session ended"))?;
        if session.revoked_ms.is_some() {
            return Err(AuthError::Unauthorized("session revoked"));
        }
        self.check_device(&store, &session, now)?;
        if let Some(user) = &session.user_id
            && store.user(user)?.is_none()
        {
            store.revoke_session(&session.id, "user deleted", now)?;
            return Err(AuthError::Unauthorized("session ended"));
        }
        if !store.use_refresh(&hash, now)? {
            store.revoke_session(&session.id, "refresh token reuse", now)?;
            return Err(AuthError::Unauthorized("refresh token reused; session revoked"));
        }
        self.issue(&store, &session, now)
    }

    /// Ends the session of a refresh token (unknown tokens are ignored).
    pub async fn logout(&self, token: String) -> Result<(), AuthError> {
        let this = self.clone();
        blocking(move || {
            let store = this.store();
            match store.refresh(&sha256_hex(&token))? {
                Refresh::Unknown => {}
                Refresh::Used { session_id } | Refresh::Fresh { session_id, .. } => {
                    store.revoke_session(&session_id, "logout", now_ms())?;
                }
            }
            Ok(())
        })
        .await
    }

    /// Revokes every session bound to `device_id` (the device was revoked).
    pub async fn revoke_device(&self, device_id: String) {
        let this = self.clone();
        blocking(move || {
            match this
                .store()
                .revoke_device_sessions(&device_id, "device revoked", now_ms())
            {
                Ok(0) => {}
                Ok(n) => tracing::info!(device = %device_id, sessions = n, "auth: device sessions revoked"),
                Err(e) => tracing::error!(error = %e, "auth: device sessions not revoked"),
            }
        })
        .await;
    }

    // --- accounts -------------------------------------------------------------

    /// Creates the first administrator: from the server itself, or with the
    /// setup code from [`SETUP_CODE_FILE`]. Signs them in.
    pub async fn setup(
        &self,
        name: String,
        password: String,
        code: Option<String>,
        peer: IpAddr,
    ) -> Result<(UserInfo, Tokens), AuthError> {
        if self.has_admin() {
            return Err(AuthError::Conflict("an administrator already exists"));
        }
        let address = format!("ip:{peer}");
        self.throttle(&[&address])?;
        if !peer.is_loopback() {
            let expected = lock(&self.inner.setup_code).clone().unwrap_or_default();
            let given = normalize_code(code.as_deref().unwrap_or_default());
            if expected.is_empty() || !ct_eq(given.as_bytes(), normalize_code(&expected).as_bytes())
            {
                self.inner
                    .limiter
                    .fail(&address, &ADDRESS, Instant::now());
                return Err(AuthError::Forbidden(
                    "the setup code is required outside the server itself",
                ));
            }
        }
        let name = check_name(&name)?;
        let this = self.clone();
        blocking(move || {
            let user = User {
                id: hex(&random::<16>()),
                name,
                role: Role::Admin,
                password_hash: Some(hash_password(&password)?),
                pin_hash: None,
                created_ms: now_ms(),
            };
            {
                let store = this.store();
                if store.has_admin()? {
                    return Err(AuthError::Conflict("an administrator already exists"));
                }
                if !store.insert_user(&user)? {
                    return Err(AuthError::Conflict("the name is taken"));
                }
            }
            this.inner.has_admin.store(true, Ordering::Release);
            *lock(&this.inner.setup_code) = None;
            let _ = std::fs::remove_file(&this.inner.setup_file);
            tracing::info!(user = %user.id, "auth: first administrator created");
            let tokens = this.open_session(Some(&user), None, Method::Password)?;
            Ok((UserInfo::from(&user), tokens))
        })
        .await
    }

    /// All accounts.
    pub async fn users(&self) -> Result<Vec<UserInfo>, AuthError> {
        let this = self.clone();
        blocking(move || Ok(this.store().users()?.iter().map(UserInfo::from).collect())).await
    }

    /// The account `id`.
    pub async fn user(&self, id: String) -> Result<Option<UserInfo>, AuthError> {
        let this = self.clone();
        blocking(move || Ok(this.store().user(&id)?.as_ref().map(UserInfo::from))).await
    }

    /// Adds an account.
    pub async fn create_user(&self, new: NewUser) -> Result<UserInfo, AuthError> {
        let name = check_name(&new.name)?;
        let role = new.role.unwrap_or(Role::Member);
        if role == Role::Admin && new.password.is_none() {
            return Err(AuthError::Invalid("an administrator needs a password".into()));
        }
        if new.password.is_none() && new.pin.is_none() {
            return Err(AuthError::Invalid("a password or a PIN is required".into()));
        }
        let this = self.clone();
        blocking(move || {
            let user = User {
                id: hex(&random::<16>()),
                name,
                role,
                password_hash: new.password.as_deref().map(hash_password).transpose()?,
                pin_hash: new.pin.as_deref().map(hash_user_pin).transpose()?,
                created_ms: now_ms(),
            };
            if !this.store().insert_user(&user)? {
                return Err(AuthError::Conflict("the name is taken"));
            }
            if role == Role::Admin {
                this.inner.has_admin.store(true, Ordering::Release);
            }
            tracing::info!(user = %user.id, ?role, "auth: user created");
            Ok(UserInfo::from(&user))
        })
        .await
    }

    /// Changes the password and/or PIN of `id` and ends the user's other
    /// sessions (`keep`: the session making the change).
    pub async fn update_user(
        &self,
        id: String,
        patch: UserPatch,
        keep: Option<String>,
    ) -> Result<UserInfo, AuthError> {
        let this = self.clone();
        blocking(move || {
            let user = this
                .store()
                .user(&id)?
                .ok_or(AuthError::NotFound("no such user"))?;
            let password = match patch.password.as_deref() {
                None => None,
                Some("") if user.role == Role::Admin => {
                    return Err(AuthError::Invalid("an administrator needs a password".into()));
                }
                Some("") => Some(None),
                Some(p) => Some(Some(hash_password(p)?)),
            };
            let pin = match patch.pin.as_deref() {
                None => None,
                Some("") => Some(None),
                Some(p) => Some(Some(hash_user_pin(p)?)),
            };
            let has_password = password.as_ref().map_or(user.password_hash.is_some(), Option::is_some);
            let has_pin = pin.as_ref().map_or(user.pin_hash.is_some(), Option::is_some);
            if !has_password && !has_pin {
                return Err(AuthError::Invalid("a password or a PIN is required".into()));
            }
            let store = this.store();
            store.set_credentials(
                &id,
                password.as_ref().map(Option::as_deref),
                pin.as_ref().map(Option::as_deref),
            )?;
            if password.is_some() || pin.is_some() {
                let n = store.revoke_user_sessions(&id, keep.as_deref(), "credentials changed", now_ms())?;
                tracing::info!(user = %id, sessions_ended = n, "auth: credentials changed");
            }
            let user = store.user(&id)?.ok_or(AuthError::NotFound("no such user"))?;
            Ok(UserInfo::from(&user))
        })
        .await
    }

    /// Deletes the account `id` and ends its sessions. The last
    /// administrator stays.
    pub async fn delete_user(&self, id: String) -> Result<(), AuthError> {
        let this = self.clone();
        blocking(move || {
            let store = this.store();
            let user = store.user(&id)?.ok_or(AuthError::NotFound("no such user"))?;
            if user.role == Role::Admin && store.admin_count()? <= 1 {
                return Err(AuthError::Conflict("the last administrator cannot be deleted"));
            }
            store.delete_user(&id)?;
            store.revoke_user_sessions(&id, None, "user deleted", now_ms())?;
            this.inner
                .has_admin
                .store(store.has_admin()?, Ordering::Release);
            tracing::info!(user = %id, "auth: user deleted");
            Ok(())
        })
        .await
    }

    /// Moves the WAL into the main file (shutdown).
    pub async fn checkpoint(&self) {
        let this = self.clone();
        blocking(move || {
            if let Err(e) = this.store().checkpoint() {
                tracing::warn!(error = %e, "auth checkpoint failed");
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trip() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(rfc3339(1_640_995_200_000), "2022-01-01T00:00:00+00:00");
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00+00:00");
        for ms in [0, 1_640_995_200_000, 951_825_599_000, 4_102_444_800_000] {
            assert_eq!(parse_rfc3339(&rfc3339(ms)), Some(ms));
        }
        assert_eq!(parse_rfc3339("2022-01-01T00:00:00Z"), Some(1_640_995_200_000));
        for bad in ["", "2022-01-01", "2022-13-01T00:00:00Z", "2022-01-01T24:00:00Z", "x-01-01T00:00:00Z", "2022-01-01T00:00:00+02:00"] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn setup_codes() {
        let code = new_setup_code();
        assert_eq!(code.len(), 19);
        assert_eq!(code.matches('-').count(), 3);
        assert_ne!(code, new_setup_code());
        assert_eq!(normalize_code(&code.to_lowercase().replace('-', " ")), normalize_code(&code));
    }

    #[test]
    fn names_and_passwords() {
        assert_eq!(check_name("  Mom ").unwrap(), "Mom");
        assert!(check_name("").is_err());
        assert!(check_name("a\u{7}b").is_err());
        assert!(check_name(&"x".repeat(65)).is_err());
        assert!(hash_password("short").is_err());
        assert!(hash_user_pin("12a4").is_err());
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
