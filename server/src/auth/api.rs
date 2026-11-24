// SPDX-License-Identifier: MPL-2.0
//! HTTP endpoints of accounts and sign-in (`/api/v1/auth/*`,
//! `/api/v1/users*`). Bodies are JSON; errors are [`ApiError`] with
//! `WWW-Authenticate: Bearer` on 401 and `Retry-After` on 429.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{Auth, AuthError, Challenge, Method, NewUser, Principal, Scope, Tokens, UserInfo, UserPatch};
use crate::api::ApiError;
use crate::http::PeerAddr;

/// Paths whose body carries the credential: a stale access token in the
/// header must not stop them.
pub(crate) const REFRESH_PATH: &str = "/api/v1/auth/refresh";
/// See [`REFRESH_PATH`].
pub(crate) const LOGOUT_PATH: &str = "/api/v1/auth/logout";

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Locked(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(e) => {
                tracing::error!(error = %e, "auth: internal error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let mut response = (status, Json(ApiError::new(self.to_string()))).into_response();
        let headers = response.headers_mut();
        match self {
            Self::Unauthorized(_) => {
                headers.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            }
            Self::Locked(wait) => {
                let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
                headers.insert(header::RETRY_AFTER, HeaderValue::from(secs.max(1)));
            }
            _ => {}
        }
        response
    }
}

/// The caller, when the request had a valid access token.
type Who = Option<Extension<Principal>>;

fn who(who: Who) -> Option<Principal> {
    who.map(|Extension(p)| p)
}

fn signed_in(who: Who) -> Result<Principal, AuthError> {
    self::who(who).ok_or(AuthError::Unauthorized("sign-in required"))
}

fn parse<T: DeserializeOwned>(body: &Bytes) -> Result<T, AuthError> {
    serde_json::from_slice(body).map_err(|e| AuthError::Invalid(format!("request body: {e}")))
}

/// First administrator.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    /// Name of the administrator.
    pub name: String,
    /// Password, 8–256 characters.
    pub password: String,
    /// The code from `setup-code.txt` in the data directory; not needed on
    /// the server itself.
    #[serde(default)]
    pub setup_code: Option<String>,
}

/// The first administrator, signed in.
#[derive(Debug, Serialize, ToSchema)]
pub struct SetupResponse {
    /// The new account.
    pub user: UserInfo,
    /// Its session.
    pub tokens: Tokens,
}

/// Name and password.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// User name (any case).
    pub name: String,
    /// Password.
    pub password: String,
}

/// Profile PIN.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinRequest {
    /// The profile (user id) picked on the device.
    pub user_id: String,
    /// Its PIN.
    pub pin: String,
}

/// Proof of a paired device key.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceLoginRequest {
    /// Device id in the family.
    pub device_id: String,
    /// The device's own fresh nonce (64 hex characters).
    pub nonce: String,
    /// The challenge from `/api/v1/auth/challenge`.
    pub challenge: String,
    /// `pa_pairing::auth::prove(identity, server_id, nonce, challenge)`:
    /// Ed25519 signature, hex.
    pub sig: String,
}

/// A refresh token.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {
    /// The refresh token of the last pair.
    pub refresh_token: String,
}

/// The caller's session.
#[derive(Debug, Serialize, ToSchema)]
pub struct Me {
    /// Session id.
    pub session_id: String,
    /// What the session may do.
    pub scope: Scope,
    /// How it was opened.
    pub method: Method,
    /// The paired device it is bound to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// The signed-in user; absent for a device session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfo>,
}

/// Accounts of the server.
#[derive(Debug, Serialize, ToSchema)]
pub struct Users {
    /// By name.
    pub users: Vec<UserInfo>,
}

/// Creates the first administrator.
///
/// Works once, while no administrator exists: from the server itself, or
/// from elsewhere with the code of `setup-code.txt` in the data directory.
/// Signs the administrator in.
#[utoipa::path(
    post,
    path = "/api/v1/auth/setup",
    tag = "auth",
    operation_id = "setup",
    request_body = SetupRequest,
    responses(
        (status = 201, description = "Administrator created and signed in", body = SetupResponse),
        (status = 400, description = "Invalid name or password", body = ApiError),
        (status = 401, description = "Invalid access token", body = ApiError),
        (status = 403, description = "Setup code missing or wrong", body = ApiError),
        (status = 409, description = "An administrator already exists", body = ApiError),
        (status = 429, description = "Too many wrong codes", body = ApiError),
    )
)]
pub(crate) async fn setup(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    body: Bytes,
) -> Result<Response, AuthError> {
    let req: SetupRequest = parse(&body)?;
    let (user, tokens) = auth
        .setup(req.name, req.password, req.setup_code, peer.ip())
        .await?;
    Ok((StatusCode::CREATED, Json(SetupResponse { user, tokens })).into_response())
}

/// Signs in with name and password.
///
/// With the access token of a device session in `Authorization`, the new
/// session is bound to that device and ends when the device is revoked.
/// Five wrong passwords lock the account for 30 s, doubling up to 15 min;
/// 20 failures lock the client address the same way.
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    operation_id = "login",
    request_body = LoginRequest,
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "Signed in", body = Tokens),
        (status = 400, description = "Invalid body", body = ApiError),
        (status = 401, description = "Wrong name or password, or invalid access token", body = ApiError),
        (status = 429, description = "Locked after failed attempts; see Retry-After", body = ApiError),
    )
)]
pub(crate) async fn login(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    caller: Who,
    body: Bytes,
) -> Result<Json<Tokens>, AuthError> {
    let req: LoginRequest = parse(&body)?;
    auth.login(req.name, req.password, peer.ip(), who(caller))
        .await
        .map(Json)
}

/// Signs a household member in with a profile PIN.
///
/// Only on a paired device: the request needs the access token of a session
/// bound to one (a device session). Locks like a password.
#[utoipa::path(
    post,
    path = "/api/v1/auth/pin",
    tag = "auth",
    operation_id = "loginPin",
    request_body = PinRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Signed in", body = Tokens),
        (status = 400, description = "Invalid body", body = ApiError),
        (status = 401, description = "Wrong profile or PIN, or no valid access token", body = ApiError),
        (status = 403, description = "Not on a paired device", body = ApiError),
        (status = 429, description = "Locked after failed attempts; see Retry-After", body = ApiError),
    )
)]
pub(crate) async fn login_pin(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    caller: Who,
    body: Bytes,
) -> Result<Json<Tokens>, AuthError> {
    let device = signed_in(caller)?;
    let req: PinRequest = parse(&body)?;
    auth.login_pin(req.user_id, req.pin, peer.ip(), device)
        .await
        .map(Json)
}

/// A challenge for a paired device to sign.
///
/// Valid for 60 s and once.
#[utoipa::path(
    post,
    path = "/api/v1/auth/challenge",
    tag = "auth",
    operation_id = "createChallenge",
    responses(
        (status = 200, description = "A fresh challenge", body = Challenge),
        (status = 401, description = "Invalid access token", body = ApiError),
        (status = 429, description = "Too many open challenges", body = ApiError),
    )
)]
pub(crate) async fn challenge(State(auth): State<Auth>) -> Result<Json<Challenge>, AuthError> {
    auth.challenge().map(Json)
}

/// Signs a paired device in with its device key.
///
/// The device signs the challenge (SYNC-005 `auth` transcript with the
/// server as verifier) and gets a device session: it can list profiles and
/// sign members in with a PIN. Revoking the device ends the session.
#[utoipa::path(
    post,
    path = "/api/v1/auth/device",
    tag = "auth",
    operation_id = "loginDevice",
    request_body = DeviceLoginRequest,
    responses(
        (status = 200, description = "Device signed in", body = Tokens),
        (status = 400, description = "Invalid body", body = ApiError),
        (status = 401, description = "Unknown or revoked device, bad signature or challenge", body = ApiError),
        (status = 429, description = "Locked after failed attempts; see Retry-After", body = ApiError),
    )
)]
pub(crate) async fn login_device(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    body: Bytes,
) -> Result<Json<Tokens>, AuthError> {
    let req: DeviceLoginRequest = parse(&body)?;
    auth.login_device(req.device_id, req.nonce, req.challenge, req.sig, peer.ip())
        .await
        .map(Json)
}

/// Exchanges a refresh token for a new pair.
///
/// Each refresh token works once. Presenting a used one again means it was
/// copied: the whole session is revoked.
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    tag = "auth",
    operation_id = "refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "A new pair", body = Tokens),
        (status = 400, description = "Invalid body", body = ApiError),
        (status = 401, description = "Unknown, expired or reused token, or the session ended", body = ApiError),
    )
)]
pub(crate) async fn refresh(
    State(auth): State<Auth>,
    body: Bytes,
) -> Result<Json<Tokens>, AuthError> {
    let req: RefreshRequest = parse(&body)?;
    auth.refresh(req.refresh_token).await.map(Json)
}

/// Ends the session of a refresh token.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    operation_id = "logout",
    request_body = RefreshRequest,
    responses(
        (status = 204, description = "Signed out (also for unknown tokens)"),
        (status = 400, description = "Invalid body", body = ApiError),
    )
)]
pub(crate) async fn logout(State(auth): State<Auth>, body: Bytes) -> Result<StatusCode, AuthError> {
    let req: RefreshRequest = parse(&body)?;
    auth.logout(req.refresh_token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's session.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth",
    operation_id = "me",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The session", body = Me),
        (status = 401, description = "No valid access token", body = ApiError),
    )
)]
pub(crate) async fn me(State(auth): State<Auth>, caller: Who) -> Result<Json<Me>, AuthError> {
    let p = signed_in(caller)?;
    let user = match p.user_id.clone() {
        Some(id) => auth.user(id).await?,
        None => None,
    };
    Ok(Json(Me {
        session_id: p.session_id,
        scope: p.scope,
        method: p.method,
        device_id: p.device_id,
        user,
    }))
}

/// Profiles to pick on a device.
///
/// Every account with its name and whether it has a PIN; for any signed-in
/// session, typically a device session before a member picks a profile.
#[utoipa::path(
    get,
    path = "/api/v1/auth/profiles",
    tag = "auth",
    operation_id = "listProfiles",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The accounts", body = Users),
        (status = 401, description = "No valid access token", body = ApiError),
    )
)]
pub(crate) async fn profiles(State(auth): State<Auth>, caller: Who) -> Result<Json<Users>, AuthError> {
    signed_in(caller)?;
    Ok(Json(Users {
        users: auth.users().await?,
    }))
}

/// Lists the accounts.
///
/// For an administrator (or the server itself before the first
/// administrator exists).
#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "auth",
    operation_id = "listUsers",
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The accounts", body = Users),
        (status = 401, description = "Sign-in required", body = ApiError),
        (status = 403, description = "Not an administrator", body = ApiError),
    )
)]
pub(crate) async fn list_users(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    caller: Who,
) -> Result<Json<Users>, AuthError> {
    auth.require_admin(who(caller).as_ref(), peer.ip())?;
    Ok(Json(Users {
        users: auth.users().await?,
    }))
}

/// Adds an account: a household member (with a PIN and/or a password) or
/// another administrator (with a password).
#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "auth",
    operation_id = "createUser",
    request_body = NewUser,
    security((), ("bearer" = [])),
    responses(
        (status = 201, description = "Created", body = UserInfo),
        (status = 400, description = "Invalid name, password or PIN", body = ApiError),
        (status = 401, description = "Sign-in required", body = ApiError),
        (status = 403, description = "Not an administrator", body = ApiError),
        (status = 409, description = "The name is taken", body = ApiError),
    )
)]
pub(crate) async fn create_user(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    caller: Who,
    body: Bytes,
) -> Result<Response, AuthError> {
    auth.require_admin(who(caller).as_ref(), peer.ip())?;
    let new: NewUser = parse(&body)?;
    let user = auth.create_user(new).await?;
    Ok((StatusCode::CREATED, Json(user)).into_response())
}

/// Changes the password and/or PIN of an account.
///
/// For an administrator or the user themselves. Ends the user's other
/// sessions.
#[utoipa::path(
    patch,
    path = "/api/v1/users/{id}",
    tag = "auth",
    operation_id = "updateUser",
    params(("id" = String, Path, description = "User id")),
    request_body = UserPatch,
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "Changed", body = UserInfo),
        (status = 400, description = "Invalid password or PIN", body = ApiError),
        (status = 401, description = "Sign-in required", body = ApiError),
        (status = 403, description = "Neither an administrator nor the user", body = ApiError),
        (status = 404, description = "No such user", body = ApiError),
    )
)]
pub(crate) async fn update_user(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    caller: Who,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<UserInfo>, AuthError> {
    let caller = who(caller);
    let own = caller
        .as_ref()
        .filter(|p| p.user_id.as_deref() == Some(id.as_str()) && p.method != Method::Device);
    if own.is_none() {
        auth.require_admin(caller.as_ref(), peer.ip())?;
    }
    let patch: UserPatch = parse(&body)?;
    let keep = own.map(|p| p.session_id.clone());
    auth.update_user(id, patch, keep).await.map(Json)
}

/// Deletes an account and ends its sessions.
///
/// The last administrator cannot be deleted.
#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}",
    tag = "auth",
    operation_id = "deleteUser",
    params(("id" = String, Path, description = "User id")),
    security((), ("bearer" = [])),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Sign-in required", body = ApiError),
        (status = 403, description = "Not an administrator", body = ApiError),
        (status = 404, description = "No such user", body = ApiError),
        (status = 409, description = "The last administrator", body = ApiError),
    )
)]
pub(crate) async fn delete_user(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(auth): State<Auth>,
    caller: Who,
    Path(id): Path<String>,
) -> Result<StatusCode, AuthError> {
    auth.require_admin(who(caller).as_ref(), peer.ip())?;
    auth.delete_user(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
