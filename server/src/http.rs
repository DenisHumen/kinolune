// SPDX-License-Identifier: MPL-2.0
//! HTTP routes and request logging.
//!
//! Every route is registered from the `#[utoipa::path]` of its handler
//! ([`api_router`]), so the OpenAPI document ([`crate::api`]) cannot miss a
//! route or describe one that does not exist (SRV-002).

use std::time::Instant;

use std::net::SocketAddr;

use axum::Router;
use axum::extract::connect_info::Connected;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, FromRef, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::serve::IncomingStream;
use serde::Deserialize;
use tokio::net::TcpListener;
use utoipa::{IntoParams, OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::tls::TlsListener;

use crate::activity::Activity;
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
    activity: Activity,
}

impl FromRef<AppState> for Activity {
    fn from_ref(state: &AppState) -> Self {
        state.activity.clone()
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
        .routes(routes!(crate::activity::system_activity))
}

pub(crate) fn router(library: Library, hub: Hub, activity: Activity) -> Router {
    let (router, _spec) = api_router().split_for_parts();
    router
        .with_state(AppState {
            library,
            hub,
            activity,
        })
        .layer(middleware::from_fn(log_request))
}

/// A JSON error response ([`ApiError`]).
fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(ApiError::new(message))).into_response()
}

/// Administration of the family is allowed from this machine only, until
/// user accounts exist (SRV-003). Returns the refusal for other peers.
fn local_only(peer: SocketAddr) -> Option<Response> {
    if peer.ip().is_loopback() {
        None
    } else {
        Some(error(
            StatusCode::FORBIDDEN,
            "allowed from the server itself only",
        ))
    }
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
/// A new code and its QR payload, replacing an open invitation. Allowed
/// from the server itself only (loopback) until user accounts exist.
#[utoipa::path(
    post,
    path = "/api/v1/pair/invite",
    tag = "pairing",
    operation_id = "createInvite",
    params(InviteQuery),
    responses(
        (status = 200, description = "A new invitation", body = InviteResponse),
        (status = 400, description = "Invalid query"),
        (status = 403, description = "Not from the server itself", body = ApiError),
        (status = 500, description = "The invitation could not be made", body = ApiError),
    )
)]
async fn invite(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(pairing): State<Pairing>,
    Query(query): Query<InviteQuery>,
) -> Response {
    if let Some(denied) = local_only(peer) {
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
/// Allowed from the server itself only (loopback) until user accounts exist.
#[utoipa::path(
    get,
    path = "/api/v1/devices",
    tag = "pairing",
    operation_id = "listDevices",
    responses(
        (status = 200, description = "The family", body = Devices),
        (status = 403, description = "Not from the server itself", body = ApiError),
    )
)]
async fn devices(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(pairing): State<Pairing>,
) -> Response {
    if let Some(denied) = local_only(peer) {
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
/// Allowed from the server itself only (loopback) until user accounts exist.
#[utoipa::path(
    delete,
    path = "/api/v1/devices/{id}",
    tag = "pairing",
    operation_id = "revokeDevice",
    params(("id" = String, Path, description = "Device id")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 400, description = "The device cannot be revoked", body = ApiError),
        (status = 403, description = "Not from the server itself", body = ApiError),
        (status = 404, description = "No such device", body = ApiError),
        (status = 410, description = "Already revoked", body = ApiError),
    )
)]
async fn revoke(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(pairing): State<Pairing>,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = local_only(peer) {
        return denied;
    }
    let result = tokio::task::spawn_blocking(move || pairing.revoke(&id))
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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
