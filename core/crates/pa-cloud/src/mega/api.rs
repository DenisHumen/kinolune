// SPDX-License-Identifier: MPL-2.0
//! MEGA's JSON API: `POST /cs?id=<seq>[&sid=…][&n=…]` with an array of one
//! command; the answer is an array of one result or a negative error
//! number. The sign-in (`us0` → `us`) and the session.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pa_vfs::{ErrorKind, VfsError};
use serde_json::{Value, json};

use super::crypto::{
    b64_decode, b64_encode, decrypt_csid, ecb_decrypt, ecb_encrypt, v1_password_key, v1_user_hash,
    v2_keys,
};
use crate::http::{Http, Request, encode};

/// MEGA's API host.
pub const API: &str = "https://g.api.mega.co.nz";

/// The app key MEGA asks third-party clients to send (`ak`); from the build
/// (`KINOLUNE_MEGA_APP_KEY`), empty in the repository.
fn app_key() -> Option<&'static str> {
    option_env!("KINOLUNE_MEGA_APP_KEY")
        .map(str::trim)
        .filter(|k| !k.is_empty())
}

/// An error code of the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiError(pub i64);

impl ApiError {
    /// `EAGAIN`: try again.
    pub const AGAIN: i64 = -3;
    /// `ERATELIMIT`.
    pub const RATE_LIMIT: i64 = -4;
    /// `ENOENT`.
    pub const NOT_FOUND: i64 = -9;
    /// `EACCESS`.
    pub const ACCESS: i64 = -11;
    /// `ESID`: the session is no longer valid.
    pub const SESSION: i64 = -15;
    /// `EBLOCKED`.
    pub const BLOCKED: i64 = -16;
    /// `EOVERQUOTA`: the transfer quota is used up.
    pub const OVER_QUOTA: i64 = -17;
    /// `EMFAREQUIRED`: the account has two-factor authentication.
    pub const MFA_REQUIRED: i64 = -26;
    /// `EFAILED` with a 2FA code: the code is wrong.
    pub const FAILED: i64 = -27;

    /// The `pa-vfs` error of this code.
    #[must_use]
    pub fn to_vfs(self, what: &str) -> VfsError {
        let (kind, why) = match self.0 {
            Self::NOT_FOUND => (ErrorKind::NotFound, "not found"),
            Self::ACCESS => (ErrorKind::PermissionDenied, "access denied"),
            Self::SESSION => (
                ErrorKind::AuthenticationFailed,
                "the session has ended; sign in again",
            ),
            Self::BLOCKED => (ErrorKind::AuthenticationFailed, "the account is blocked"),
            Self::OVER_QUOTA => (
                ErrorKind::Unavailable,
                "the MEGA transfer quota is used up; try again later",
            ),
            Self::MFA_REQUIRED => (ErrorKind::AuthenticationFailed, MFA_REQUIRED_TEXT),
            Self::FAILED => (ErrorKind::AuthenticationFailed, "wrong two-factor code"),
            Self::AGAIN | Self::RATE_LIMIT => (ErrorKind::Unavailable, "MEGA is busy"),
            -2 => (ErrorKind::InvalidPath, "bad request"),
            _ => (ErrorKind::Other, "request failed"),
        };
        VfsError::new(kind, format!("MEGA {what}: {why} ({})", self.0))
    }
}

/// The message of a sign-in that needs a 2FA code: the app asks for it.
pub const MFA_REQUIRED_TEXT: &str = "two-factor code required";

/// A client of the API: a session id (an account) or a public folder
/// handle (`n`), never both.
pub struct Client {
    http: Http,
    base: String,
    seq: AtomicU64,
    sid: Option<String>,
    folder: Option<String>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("signed_in", &self.sid.is_some())
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// An anonymous client at `base` ([`API`] in the app).
    #[must_use]
    pub fn new(base: &str) -> Self {
        let mut seed = [0u8; 4];
        let _ = getrandom::fill(&mut seed);
        Self {
            http: Http::new(Duration::from_secs(10), Duration::from_secs(120)),
            base: base.trim_end_matches('/').to_owned(),
            seq: AtomicU64::new(u64::from(u32::from_le_bytes(seed))),
            sid: None,
            folder: None,
        }
    }

    /// The same client in session `sid`.
    #[must_use]
    pub fn with_session(mut self, sid: &str) -> Self {
        self.sid = Some(sid.to_owned());
        self
    }

    /// The same client reading public folder `handle`.
    #[must_use]
    pub fn with_folder(mut self, handle: &str) -> Self {
        self.folder = Some(handle.to_owned());
        self
    }

    pub(crate) fn http(&self) -> &Http {
        &self.http
    }

    /// Runs one command; retries while MEGA says "again" (up to ~10 s).
    ///
    /// # Errors
    ///
    /// [`ApiError`] codes as `pa-vfs` errors (see [`ApiError::to_vfs`]),
    /// network errors. The raw code is in the message.
    pub fn call(&self, what: &str, cmd: &Value) -> pa_vfs::Result<Value> {
        match self.call_raw(cmd)? {
            Ok(v) => Ok(v),
            Err(e) => Err(e.to_vfs(what)),
        }
    }

    /// Like [`call`](Self::call), with the API error code apart.
    ///
    /// # Errors
    ///
    /// Network errors and malformed answers.
    pub fn call_raw(&self, cmd: &Value) -> pa_vfs::Result<Result<Value, ApiError>> {
        let body = json!([cmd]);
        let mut wait = Duration::from_millis(250);
        for attempt in 0..6 {
            let id = self.seq.fetch_add(1, Ordering::Relaxed);
            let mut url = format!("{}/cs?id={id}", self.base);
            if let Some(sid) = &self.sid {
                url.push_str(&format!("&sid={}", encode(sid)));
            }
            if let Some(n) = &self.folder {
                url.push_str(&format!("&n={}", encode(n)));
            }
            if let Some(ak) = app_key() {
                url.push_str(&format!("&ak={}", encode(ak)));
            }
            let r = self
                .http
                .send_retrying(3, &|| Request::post(url.clone()).json(&body))?;
            if !r.ok() {
                return Err(crate::http::status_error("MEGA", r.status));
            }
            let v: Value = r.json("MEGA")?;
            let result = match v {
                Value::Number(n) => Err(ApiError(n.as_i64().unwrap_or(-1))),
                Value::Array(mut a) if !a.is_empty() => match a.swap_remove(0) {
                    Value::Number(n) if n.as_i64().is_some_and(|c| c < 0) => {
                        Err(ApiError(n.as_i64().unwrap_or(-1)))
                    }
                    other => Ok(other),
                },
                _ => {
                    return Err(VfsError::new(ErrorKind::Other, "MEGA: unexpected answer"));
                }
            };
            match result {
                Err(ApiError(c))
                    if (c == ApiError::AGAIN || c == ApiError::RATE_LIMIT) && attempt < 5 =>
                {
                    std::thread::sleep(wait);
                    wait *= 2;
                }
                other => return Ok(other),
            }
        }
        Ok(Err(ApiError(ApiError::AGAIN)))
    }
}

/// A signed-in MEGA session: what is kept in the secret store instead of
/// the password. `Debug` hides it.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Session {
    /// The session id.
    pub sid: String,
    /// The account's master key, base64.
    pub mk: String,
    /// The user handle (whose keys the nodes carry).
    pub user: String,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Session(***)")
    }
}

impl Session {
    /// The session as stored (JSON).
    #[must_use]
    pub fn to_secret(&self) -> String {
        json!({"v": 1, "sid": self.sid, "mk": self.mk, "user": self.user}).to_string()
    }

    /// A stored session.
    ///
    /// # Errors
    ///
    /// The secret is not a session (a password typed into an older form).
    pub fn from_secret(secret: &str) -> pa_vfs::Result<Self> {
        serde_json::from_str(secret).map_err(|_| {
            VfsError::new(
                ErrorKind::AuthenticationFailed,
                "no MEGA session stored; sign in again",
            )
        })
    }

    /// The master key.
    ///
    /// # Errors
    ///
    /// A damaged session.
    pub fn master_key(&self) -> pa_vfs::Result<[u8; 16]> {
        b64_decode(&self.mk)?
            .try_into()
            .map_err(|_| VfsError::new(ErrorKind::AuthenticationFailed, "damaged MEGA session"))
    }
}

/// Signs in with e-mail and password (and the 2FA code when the account
/// has one). The password is used here only and not kept.
///
/// # Errors
///
/// [`ErrorKind::AuthenticationFailed`]: wrong e-mail or password, a 2FA
/// code needed ([`MFA_REQUIRED_TEXT`]) or wrong; network errors.
pub fn login(
    base: &str,
    email: &str,
    password: &str,
    mfa: Option<&str>,
) -> pa_vfs::Result<Session> {
    let client = Client::new(base);
    let email = email.trim().to_lowercase();
    let pre = client.call("sign-in", &json!({"a": "us0", "user": email}))?;
    let version = pre.get("v").and_then(Value::as_i64).unwrap_or(1);
    let (pw_key, uh) = if version == 2 {
        let salt = b64_decode(pre.get("s").and_then(Value::as_str).unwrap_or_default())?;
        v2_keys(password, &salt)
    } else {
        let k = v1_password_key(password);
        let uh = v1_user_hash(&email, &k);
        (k, uh)
    };
    let mut cmd = json!({"a": "us", "user": email, "uh": uh});
    if let Some(code) = mfa.map(str::trim).filter(|c| !c.is_empty()) {
        cmd["mfa"] = json!(code);
    }
    let res = match client.call_raw(&cmd)? {
        Ok(v) => v,
        // ENOENT / EINCOMPLETE here: wrong e-mail or password.
        Err(ApiError(c)) if c == ApiError::NOT_FOUND || c == -13 => {
            return Err(VfsError::new(
                ErrorKind::AuthenticationFailed,
                "MEGA sign-in: wrong e-mail or password",
            ));
        }
        Err(e) => return Err(e.to_vfs("sign-in")),
    };
    let field = |k: &str| res.get(k).and_then(Value::as_str);
    let bad = || {
        VfsError::new(
            ErrorKind::AuthenticationFailed,
            "MEGA sign-in: unexpected answer",
        )
    };
    let mk_enc = b64_decode(field("k").ok_or_else(bad)?)?;
    let mk: [u8; 16] = ecb_decrypt(&pw_key, &mk_enc)?
        .get(..16)
        .and_then(|k| k.try_into().ok())
        .ok_or_else(bad)?;
    let sid = if let Some(tsid) = field("tsid") {
        // A temporary session: its first block encrypted with the master
        // key equals its last block.
        let raw = b64_decode(tsid)?;
        if raw.len() < 32 || ecb_encrypt(&mk, &raw[..16])? != raw[raw.len() - 16..] {
            return Err(VfsError::new(
                ErrorKind::AuthenticationFailed,
                "MEGA sign-in: wrong e-mail or password",
            ));
        }
        tsid.to_owned()
    } else {
        let privk = ecb_decrypt(&mk, &b64_decode(field("privk").ok_or_else(bad)?)?)?;
        decrypt_csid(&privk, field("csid").ok_or_else(bad)?)?
    };
    let user = field("u").map(str::to_owned).unwrap_or_default();
    let user = if user.is_empty() {
        let me = Client::new(base)
            .with_session(&sid)
            .call("account", &json!({"a": "ug"}))?;
        me.get("u")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(bad)?
    } else {
        user
    };
    Ok(Session {
        sid,
        mk: b64_encode(&mk),
        user,
    })
}
