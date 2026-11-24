// SPDX-License-Identifier: MPL-2.0
//! HTTP routes and request logging.
//!
//! Every route is registered from the `#[utoipa::path]` of its handler
//! ([`api_router`]), so the OpenAPI document ([`crate::api`]) cannot miss a
//! route or describe one that does not exist (SRV-002).
//!
//! Every `/api/` request with an `Authorization: Bearer` header has its
//! access token checked first ([`authenticate`], SRV-003): an invalid,
//! expired or revoked one gets 401; a valid one becomes the
//! [`Principal`] of the request.

use std::time::Instant;

use std::net::SocketAddr;

use axum::extract::connect_info::Connected;
use axum::{Extension, Router};
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, FromRef, Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::serve::IncomingStream;
use serde::Deserialize;
use tokio::net::TcpListener;
use utoipa::{IntoParams, OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::api as auth_api;
use crate::auth::{Auth, AuthError, Principal};
use crate::tls::TlsListener;

use crate::api::{
    ApiDoc, ApiError, ApiVersion, DbHealth, DeviceInfo, Devices, Health, InviteResponse, Status,
};
use crate::pairing::Pairing;
use crate::sync::Hub;
use crate::{Library, SERVICE, VERSION};

/// Remote address of a connection, for plain TCP and TLS alike (axum only
/// implements `Connected` for `SocketAddr` over `TcpListener`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PeerAddr(pub(crate) SocketAddr);

impl Connected<IncomingStream<'_, TcpListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

impl Connected<IncomingStream<'_, TlsListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

#[derive(Clone)]
pub(crate) struct AppState {
    library: Library,
    hub: Hub,
    auth: Auth,
}

impl FromRef<AppState> for Auth {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

impl FromRef<AppState> for Library {
    fn from_ref(state: &AppState) -> Self {
        state.library.clone()
    }
}

impl FromRef<AppState> for Hub {
    fn from_ref(state: &AppState) -> Self {
        state.hub.clone()
    }
}

impl FromRef<AppState> for Pairing {
    fn from_ref(state: &AppState) -> Self {
        state.hub.pairing().clone()
    }
}

/// Routes of the server together with their OpenAPI description.
pub(crate) fn api_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health))
        .routes(routes!(healthz))
        .routes(routes!(sync_socket))
        .routes(routes!(pair_socket))
        .routes(routes!(invite))
        .routes(routes!(devices))
        .routes(routes!(revoke))
        .routes(routes!(auth_api::setup))
        .routes(routes!(auth_api::login))
        .routes(routes!(auth_api::login_pin))
        .routes(routes!(auth_api::challenge))
        .routes(routes!(auth_api::login_device))
        .routes(routes!(auth_api::refresh))
        .routes(routes!(auth_api::logout))
        .routes(routes!(auth_api::me))
        .routes(routes!(auth_api::profiles))
        .routes(routes!(auth_api::list_users, auth_api::create_user))
        .routes(routes!(auth_api::update_user, auth_api::delete_user))
}

pub(crate) fn router(library: Library, hub: Hub, auth: Auth) -> Router {
    let (router, _spec) = api_router().split_for_parts();
    router
        .with_state(AppState {
            library,
            hub,
            auth: auth.clone(),
        })
        .layer(middleware::from_fn_with_state(auth, authenticate))
        .layer(middleware::from_fn(log_request))
}

/// Checks the access token of `/api/` requests that carry one. Requests
/// without a token pass: each handler decides what it needs.
async fn authenticate(State(auth): State<Auth>, mut request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let checked = path.starts_with("/api/")
        && path != auth_api::REFRESH_PATH
        && path != auth_api::LOGOUT_PATH;
    if checked && let Some(value) = request.headers().get(header::AUTHORIZATION) {
        let token = value
            .to_str()
            .ok()
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim().to_owned());
        let Some(token) = token else {
            return AuthError::Unauthorized("expected `Authorization: Bearer <token>`")
                .into_response();
        };
        match auth.authenticate(token).await {
            Ok(principal) => {
                request.extensions_mut().insert(principal);
            }
            Err(e) => return e.into_response(),
        }
    }
    next.run(request).await
}

/// A JSON error response ([`ApiError`]).
fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(ApiError::new(message))).into_response()
}

/// Administration of the family: an administrator's password session, or
/// the server itself (loopback) before the first administrator exists
/// (SRV-003). Returns the refusal.
fn admin_only(auth: &Auth, who: Option<&Extension<Principal>>, peer: SocketAddr) -> Option<Response> {
    auth.require_admin(who.map(|Extension(p)| p), peer.ip())
        .err()
        .map(IntoResponse::into_response)
}

/// Kind of pairing code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
enum InviteKind {
    /// 6 digits to type on the new device.
    #[default]
    Code,
    /// A long code for a QR code only.
    Qr,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct InviteQuery {
    /// `code` (default): 6 digits to type; `qr`: a long code for a QR code
    /// only.
    #[serde(default)]
    #[param(inline)]
    kind: Option<InviteKind>,
}

/// Opens a pairing invitation.
///
/// A new code and its QR payload, replacing an open invitation. For an
/// administrator (or the server itself before the first administrator
/// exists).
#[utoipa::path(
    post,
    path = "/api/v1/pair/invite",
    tag = "pairing",
    operation_id = "createInvite",
    params(InviteQuery),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "A new invitation", body = InviteResponse),
        (status = 400, description = "Invalid query"),
        (status = 401, description = "Sign-in required", body = ApiError),
        (status = 403, description = "Not an administrator", body = ApiError),
        (status = 500, description = "The invitation could not be made", body = ApiError),
    )
)]
async fn invite(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(pairing): State<Pairing>,
    State(auth): State<Auth>,
    who: Option<Extension<Principal>>,
    Query(query): Query<InviteQuery>,
) -> Response {
    if let Some(denied) = admin_only(&auth, who.as_ref(), peer) {
        return denied;
    }
    match pairing.invite(query.kind == Some(InviteKind::Qr)) {
        Ok(invite) => Json(InviteResponse {
            code: invite.code,
            qr: invite.qr,
            expires_ms: invite.expires_ms,
            fingerprint: pairing.fingerprint(),
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "pairing: no invitation");
            error(StatusCode::INTERNAL_SERVER_ERROR, "no invitation")
        }
    }
}

/// Lists the devices of the family, revoked ones too.
///
/// For an administrator (or the server itself before the first
/// administrator exists).
#[utoipa::path(
    get,
    path = "/api/v1/devices",
    tag = "pairing",
    operation_id = "listDevices",
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The family", body = Devices),
        (status = 401, description = "Sign-in required", body = ApiError),
        (status = 403, description = "Not an administrator", body = ApiError),
    )
)]
async fn devices(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(pairing): State<Pairing>,
    State(auth): State<Auth>,
    who: Option<Extension<Principal>>,
) -> Response {
    if let Some(denied) = admin_only(&auth, who.as_ref(), peer) {
        return denied;
    }
    Json(Devices {
        family: pairing.family_id(),
        devices: pairing
            .devices()
            .into_iter()
            .map(DeviceInfo::from)
            .collect(),
    })
    .into_response()
}

/// Revokes a device and rotates the family key.
///
/// Its connections close and every session bound to it ends (its tokens stop
/// working at once). For an administrator (or the server itself before the
/// first administrator exists).
#[utoipa::path(
    delete,
    path = "/api/v1/devices/{id}",
    tag = "pairing",
    operation_id = "revokeDevice",
    params(("id" = String, Path, description = "Device id")),
    security((), ("bearer" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 400, description = "The device cannot be revoked", body = ApiError),
        (status = 401, description = "Sign-in required", body = ApiError),
        (status = 403, description = "Not an administrator", body = ApiError),
        (status = 404, description = "No such device", body = ApiError),
        (status = 410, description = "Already revoked", body = ApiError),
    )
)]
async fn revoke(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(pairing): State<Pairing>,
    State(auth): State<Auth>,
    who: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = admin_only(&auth, who.as_ref(), peer) {
        return denied;
    }
    let device = id.clone();
    let result = tokio::task::spawn_blocking(move || pairing.revoke(&device))
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
    match result {
        Ok(()) => {
            auth.revoke_device(id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(pa_pairing::Error::Unknown(_)) => error(StatusCode::NOT_FOUND, "no such device"),
        Err(pa_pairing::Error::Revoked(_)) => error(StatusCode::GONE, "already revoked"),
        Err(e) => error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// Pairs a new device (WebSocket).
///
/// Upgrades to the pairing protocol (`pa_pairing::pair`, SPAKE2 with the
/// code of the open invitation); the server is the inviter.
#[utoipa::path(
    get,
    path = "/api/v1/pair",
    tag = "pairing",
    operation_id = "pairSocket",
    responses(
        (status = 101, description = "WebSocket: pairing messages follow"),
        (status = 400, description = "Not a WebSocket upgrade"),
        (status = 401, description = "Invalid access token", body = ApiError),
    )
)]
async fn pair_socket(State(pairing): State<Pairing>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(pa_pairing::pair::MAX_FRAME_LEN)
        .max_frame_size(pa_pairing::pair::MAX_FRAME_LEN)
        .on_upgrade(move |socket| pairing.serve(socket))
}

/// Syncs a paired device (WebSocket).
///
/// Upgrades to the sync protocol (`docs/api/sync-protocol.md`). Devices
/// authenticate with their paired keys (SYNC-005).
#[utoipa::path(
    get,
    path = "/api/v1/sync",
    tag = "sync",
    operation_id = "syncSocket",
    responses(
        (status = 101, description = "WebSocket: sync protocol frames follow"),
        (status = 400, description = "Not a WebSocket upgrade"),
        (status = 401, description = "Invalid access token", body = ApiError),
        (status = 503, description = "The hub has no free connection slot", body = ApiError),
    )
)]
async fn sync_socket(State(hub): State<Hub>, upgrade: WebSocketUpgrade) -> Response {
    let Some(slot) = hub.try_slot() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "sync hub is full");
    };
    upgrade
        .max_message_size(pa_sync::proto::MAX_FRAME_LEN)
        .max_frame_size(pa_sync::proto::MAX_FRAME_LEN)
        .on_upgrade(move |socket| hub.serve(socket, slot))
}

/// Liveness, readiness and versions.
///
/// 200 if the library database answers, 503 if not.
#[utoipa::path(
    get,
    path = "/health",
    tag = "server",
    operation_id = "health",
    responses(
        (status = 200, description = "The server works", body = Health),
        (status = 503, description = "The library database does not answer", body = Health),
    )
)]
async fn health(State(library): State<Library>) -> Response {
    health_response(&library).await
}

/// Same as `/health`.
///
/// The NAS OS greenboot checks probe both paths (`distro/`).
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "server",
    operation_id = "healthz",
    responses(
        (status = 200, description = "The server works", body = Health),
        (status = 503, description = "The library database does not answer", body = Health),
    )
)]
async fn healthz(State(library): State<Library>) -> Response {
    health_response(&library).await
}

async fn health_response(library: &Library) -> Response {
    let (status, db) = match library.schema_version().await {
        Ok(schema) => (
            StatusCode::OK,
            DbHealth {
                status: Status::Ok,
                schema_version: Some(schema),
            },
        ),
        Err(e) => {
            tracing::error!(error = %e, "health: library database unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                DbHealth {
                    status: Status::Error,
                    schema_version: None,
                },
            )
        }
    };
    let health = Health {
        service: SERVICE.to_owned(),
        status: db.status,
        version: VERSION.to_owned(),
        api: ApiVersion::current(),
        db,
    };
    (status, Json(health)).into_response()
}

/// One structured log line per request. Query strings are not logged: later
/// endpoints may carry tokens there.
async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    tracing::info!(
        %method,
        path,
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "request"
    );
    response
}
