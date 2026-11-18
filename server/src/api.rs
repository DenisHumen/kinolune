// SPDX-License-Identifier: MPL-2.0
//! Description of the HTTP API: OpenAPI 3.1 generated from the handlers
//! (SRV-002, ADR-NNNN).
//!
//! The routes of the server and the spec come from the same
//! `#[utoipa::path]` annotations (`utoipa_axum::routes!`), so a route cannot
//! exist without its description. The generated document is committed as
//! `docs/api/openapi.json`; the contract tests (`server/tests/contract/`)
//! fail when the file and the code differ, when a live response does not
//! match its schema, and when a change breaks a frozen contract of a
//! supported older version (`docs/api/compat/`).
//!
//! Versioning: [`API_MAJOR`] is the `v` of the mDNS TXT record and of
//! `/api/v1`; [`API_MINOR`] grows with additive changes. A client and a
//! server work together when their majors differ by at most one
//! ([`pa_discovery::api_compatible`]).

use serde::Serialize;
use utoipa::openapi::OpenApi as Spec;
use utoipa::{OpenApi, ToSchema};

/// Major version of the HTTP API (`/api/v1`, TXT `v`).
pub const API_MAJOR: u32 = 1;

/// Minor version: grows with additive changes within [`API_MAJOR`].
pub const API_MINOR: u32 = 0;

/// `info.version` of the spec: `"<major>.<minor>"`.
pub fn api_version() -> String {
    format!("{API_MAJOR}.{API_MINOR}")
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Kinolune server API",
        description = "HTTP API of the Kinolune media server (alpha-media). \
            Generated from the server code; do not edit by hand. \
            Clients and servers whose major versions differ by at most one \
            work together (ADR-NNNN).",
        license(name = "MPL-2.0", identifier = "MPL-2.0"),
    ),
    tags(
        (name = "server", description = "Liveness and versions"),
        (name = "pairing", description = "Device family: pairing and revocation (SYNC-005)"),
        (name = "sync", description = "Sync protocol over WebSocket (docs/api/sync-protocol.md)"),
    )
)]
pub(crate) struct ApiDoc;

/// The OpenAPI document of this server.
pub fn openapi() -> Spec {
    let mut spec = crate::http::api_router().into_openapi();
    spec.info.version = api_version();
    spec
}

/// [`openapi`] as pretty JSON with a trailing newline, exactly as
/// `docs/api/openapi.json` stores it.
pub fn openapi_json() -> String {
    let mut json = openapi()
        .to_pretty_json()
        .expect("the OpenAPI document serializes");
    json.push('\n');
    json
}

/// Error body of the API endpoints (status codes 4xx and 5xx).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiError {
    /// Human-readable reason, in English.
    pub error: String,
}

impl ApiError {
    pub(crate) fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
        }
    }
}

/// Versions of the API this server speaks.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiVersion {
    /// Major version: the same as TXT `v` and `/api/v<major>`.
    pub major: u32,
    /// Minor version: grows with additive changes.
    pub minor: u32,
}

impl ApiVersion {
    pub(crate) fn current() -> Self {
        Self {
            major: API_MAJOR,
            minor: API_MINOR,
        }
    }
}

/// `ok` or `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Works.
    Ok,
    /// Does not work.
    Error,
}

/// State of the library database.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DbHealth {
    /// Whether the database answers.
    pub status: Status,
    /// Newest applied schema version; absent when the database fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<i64>,
}

/// What `/health` returns.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Health {
    /// Service name, `alpha-media`.
    pub service: String,
    /// `ok` when the server can serve requests.
    pub status: Status,
    /// Version of the server program.
    pub version: String,
    /// Version of the HTTP API.
    pub api: ApiVersion,
    /// The library database.
    pub db: DbHealth,
}

/// A new pairing invitation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteResponse {
    /// The code to type on the new device.
    pub code: String,
    /// QR payload (`projectalpha://pair?…`) with the same code.
    pub qr: String,
    /// When the invitation expires, Unix ms.
    pub expires_ms: u64,
    /// Fingerprint of the server's device key in the family.
    pub fingerprint: String,
}

/// One device of the family.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeviceInfo {
    /// Device id.
    pub device_id: String,
    /// Name shown to people.
    pub name: String,
    /// `phone`, `tablet`, `tv`, `desktop`, `server` or `other`; clients
    /// treat unknown values as `other`.
    pub kind: String,
    /// Fingerprint of the device key.
    pub fingerprint: String,
    /// When the device joined, Unix ms.
    pub added_ms: u64,
    /// When the device was revoked, Unix ms; absent for active devices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_ms: Option<u64>,
}

impl From<pa_pairing::Device> for DeviceInfo {
    fn from(d: pa_pairing::Device) -> Self {
        Self {
            device_id: d.device_id,
            name: d.name,
            kind: d.kind.as_str().to_owned(),
            fingerprint: d.fingerprint,
            added_ms: d.added_ms,
            revoked_ms: d.revoked_ms,
        }
    }
}

/// The device family of the server.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Devices {
    /// Family id.
    pub family: String,
    /// Devices of the family, revoked ones too.
    pub devices: Vec<DeviceInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_major_dot_minor() {
        assert_eq!(openapi().info.version, format!("{API_MAJOR}.{API_MINOR}"));
        assert_eq!(crate::announce::API_VERSION, API_MAJOR);
    }

    #[test]
    fn spec_is_openapi_3_1_with_every_route() {
        let spec: serde_json::Value = serde_json::from_str(&openapi_json()).unwrap();
        assert!(spec["openapi"].as_str().unwrap().starts_with("3.1"));
        for path in [crate::sync::PATH, crate::pairing::PAIR_PATH] {
            assert!(spec["paths"][path].is_object(), "{path} is not described");
        }
    }
}
