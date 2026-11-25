// SPDX-License-Identifier: MPL-2.0
//! "Someone is watching": what the server is doing right now that a reboot
//! must not interrupt (SRV-019).
//!
//! The OS update agent of Kinolune OS (`alpha-updated`, DIST-009) asks
//! `GET /api/v1/system/activity` before it applies an update and right
//! before it reboots, and postpones while the server is busy. Sources:
//!
//! - **playback**: devices whose last `pb.update` through the sync hub is
//!   `playing` and still within its TTL (`ttl_ms`, 15 s by default: three
//!   missed heartbeats and the device counts as gone), and streams the
//!   server serves itself (direct play, HLS, transcode) — each holds an
//!   [`ActivityGuard`] of [`ActivityKind::Playback`] while it runs;
//! - **recording**: Twitch live recordings and VOD downloads on the server;
//! - **scan**: library scans.
//!
//! Work that must not be interrupted holds a guard from [`Activity::begin`]
//! for as long as it runs; dropping the guard ends it, so a panic or an
//! early return cannot leave the server "busy" forever.
//!
//! **Access.** The update agent is a system service on the same machine and
//! has no user token, so the endpoint needs none — and answers only
//! requests from the machine itself: a peer address other than loopback,
//! or a request relayed by a local reverse proxy (`Forwarded`,
//! `X-Forwarded-For`, `X-Real-IP`), gets 403. With user accounts (SRV-003)
//! the check stays in the handler: the auth middleware lets requests
//! without a token through to the handler, which decides.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};

use crate::api::{ActivityReport, ApiError};
use crate::http::PeerAddr;
use crate::sync::Hub;

/// Path of the endpoint.
pub const PATH: &str = "/api/v1/system/activity";

/// Kind of work that keeps the server busy, in order of precedence for
/// `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivityKind {
    /// A stream served by the server (direct play, HLS, transcode).
    Playback,
    /// A Twitch live recording or a VOD download.
    Recording,
    /// A library scan.
    Scan,
}

impl ActivityKind {
    /// The `reason` string of the API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Playback => "playback",
            Self::Recording => "recording",
            Self::Scan => "scan",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Playback => 0,
            Self::Recording => 1,
            Self::Scan => 2,
        }
    }
}

/// Running work of the server by kind; cheap to clone.
#[derive(Clone, Default)]
pub struct Activity {
    counts: Arc<Mutex<[usize; 3]>>,
}

impl Activity {
    /// Marks the start of work that must not be interrupted; it ends when
    /// the guard is dropped.
    #[must_use = "the activity ends when the guard is dropped"]
    pub fn begin(&self, kind: ActivityKind) -> ActivityGuard {
        self.counts()[kind.index()] += 1;
        ActivityGuard {
            activity: self.clone(),
            kind,
        }
    }

    /// Number of running activities of `kind`.
    pub fn count(&self, kind: ActivityKind) -> usize {
        self.counts()[kind.index()]
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, [usize; 3]> {
        self.counts.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One running activity; ends on drop.
pub struct ActivityGuard {
    activity: Activity,
    kind: ActivityKind,
}

impl ActivityGuard {
    /// What this guard keeps running.
    pub fn kind(&self) -> ActivityKind {
        self.kind
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut counts = self.activity.counts();
        let n = &mut counts[self.kind.index()];
        *n = n.saturating_sub(1);
    }
}

impl std::fmt::Debug for ActivityGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityGuard")
            .field("kind", &self.kind)
            .finish()
    }
}

/// What the server reports: `busy`, the number of playback sessions and
/// the main reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) busy: bool,
    pub(crate) sessions: u32,
    pub(crate) reason: Option<ActivityKind>,
}

impl Snapshot {
    /// Combines served streams, devices playing through the sync hub
    /// (`playing`), recordings and scans.
    ///
    /// `sessions` counts playback only: the larger of served streams and
    /// playing devices, because a device playing a stream from this server
    /// shows up in both.
    pub(crate) fn new(activity: &Activity, playing: usize) -> Self {
        let streams = activity.count(ActivityKind::Playback);
        let sessions = streams.max(playing);
        let reason = if sessions > 0 {
            Some(ActivityKind::Playback)
        } else if activity.count(ActivityKind::Recording) > 0 {
            Some(ActivityKind::Recording)
        } else if activity.count(ActivityKind::Scan) > 0 {
            Some(ActivityKind::Scan)
        } else {
            None
        };
        Self {
            busy: reason.is_some(),
            sessions: u32::try_from(sessions).unwrap_or(u32::MAX),
            reason,
        }
    }
}

/// Headers that a reverse proxy adds: a request with any of them came from
/// elsewhere, even when its TCP peer is this machine.
const PROXY_HEADERS: [&str; 3] = ["forwarded", "x-forwarded-for", "x-real-ip"];

/// The request comes from the server machine itself, not through a proxy.
fn from_this_machine(peer: IpAddr, headers: &HeaderMap) -> bool {
    peer.to_canonical().is_loopback() && !PROXY_HEADERS.iter().any(|h| headers.contains_key(*h))
}

/// Reports whether the server is busy (for OS updates).
///
/// Playback (devices playing through the sync hub, streams served by the
/// server), recordings and scans make the server busy; the OS update agent
/// postpones updates and reboots until it is not. Allowed from the server
/// machine itself only (loopback, not through a proxy); needs no token.
#[utoipa::path(
    get,
    path = "/api/v1/system/activity",
    tag = "system",
    operation_id = "systemActivity",
    responses(
        (status = 200, description = "What the server is doing", body = ActivityReport),
        (status = 403, description = "Not from the server machine itself", body = ApiError),
    )
)]
pub(crate) async fn system_activity(
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(activity): State<Activity>,
    State(hub): State<Hub>,
    headers: HeaderMap,
) -> Response {
    if !from_this_machine(peer.ip(), &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiError::new("allowed from the server itself only")),
        )
            .into_response();
    }
    let now = Snapshot::new(&activity, hub.playing_now());
    let report = ActivityReport {
        busy: now.busy,
        sessions: now.sessions,
        reason: now
            .reason
            .map(ActivityKind::as_str)
            .unwrap_or("")
            .to_owned(),
    };
    ([(header::CACHE_CONTROL, "no-store")], Json(report)).into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn only_this_machine_without_a_proxy() {
        let none = HeaderMap::new();
        for ip in ["127.0.0.1", "127.0.0.2", "::1", "::ffff:127.0.0.1"] {
            assert!(from_this_machine(ip.parse().unwrap(), &none), "{ip}");
        }
        for ip in [
            "192.168.1.10",
            "10.0.0.2",
            "172.16.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "8.8.8.8",
            "0.0.0.0",
            "fe80::1",
            "fd00::1",
            "2001:db8::1",
            "::ffff:192.168.1.10",
        ] {
            assert!(!from_this_machine(ip.parse().unwrap(), &none), "{ip}");
        }
        for h in PROXY_HEADERS {
            let mut headers = HeaderMap::new();
            headers.insert(h, HeaderValue::from_static("192.168.1.10"));
            assert!(
                !from_this_machine("127.0.0.1".parse().unwrap(), &headers),
                "{h}"
            );
        }
    }

    #[test]
    fn idle_is_not_busy() {
        let activity = Activity::default();
        let s = Snapshot::new(&activity, 0);
        assert_eq!(
            s,
            Snapshot {
                busy: false,
                sessions: 0,
                reason: None
            }
        );
    }

    #[test]
    fn guards_count_until_dropped() {
        let activity = Activity::default();
        let scan = activity.begin(ActivityKind::Scan);
        assert_eq!(Snapshot::new(&activity, 0).reason, Some(ActivityKind::Scan));
        let rec = activity.begin(ActivityKind::Recording);
        // Recording outranks a scan.
        assert_eq!(
            Snapshot::new(&activity, 0).reason,
            Some(ActivityKind::Recording)
        );
        // Playback outranks everything and is the only thing counted.
        let s = Snapshot::new(&activity, 2);
        assert_eq!(s.reason, Some(ActivityKind::Playback));
        assert_eq!(s.sessions, 2);
        drop(rec);
        drop(scan);
        assert!(!Snapshot::new(&activity, 0).busy);
    }

    #[test]
    fn a_stream_and_its_heartbeat_are_one_session() {
        let activity = Activity::default();
        let _stream = activity.begin(ActivityKind::Playback);
        assert_eq!(Snapshot::new(&activity, 1).sessions, 1);
        assert_eq!(Snapshot::new(&activity, 0).sessions, 1);
        assert_eq!(Snapshot::new(&activity, 3).sessions, 3);
    }
}
