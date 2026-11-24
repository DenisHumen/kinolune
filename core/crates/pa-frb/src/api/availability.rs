// SPDX-License-Identifier: MPL-2.0
//! Availability of servers and sources for Dart (NET-024, ADR-0157 §8).
//!
//! [`CoreAvailability`] runs the one monitor of the core
//! (`pa_runtime::availability`) over everything the app talks to:
//!
//! - every source of the library database, however it was added (by hand,
//!   from the LAN list, by QR, arrived by sync, the automatic sync
//!   folder): `source:<id>`;
//! - the devices of the family once [`CoreAvailability::attach_sync`] is
//!   called (`peer:<device id>`, passive: their own LAN presence), and the
//!   sync folder marked as a sync channel;
//! - servers the app registers with [`CoreAvailability::set_endpoints`]:
//!   the own Kinolune server (`/health`), a sync hub, an NFS export
//!   (`endpoint:<id>`).
//!
//! The app reports when it goes to the background
//! ([`CoreAvailability::set_app_state`]) and network changes
//! ([`CoreAvailability::network_changed`]); a phone stops probing in the
//! background. The full status screen and the banners are UI-053; this
//! is their data.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use flutter_rust_bridge::frb;
use pa_catalog::Catalog;
use pa_catalog::pa_sync::lan::service::SyncService;
use pa_runtime::availability::{
    AppState, AvailabilityEvent, Monitor, Origin, Policy, ProbeOutcome, Prober, SystemClock,
    Target, TargetKind, TargetSource, TargetState, Transition,
};
use pa_sources as s;
use pa_sources::availability::{self as av, SourceProber, SourceSignals};

use crate::api::sync::CoreSync;
use crate::frb_generated::StreamSink;

/// Error of an availability call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreAvailabilityError {
    /// `secrets`, `library`, `invalid` or `other`.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

impl From<s::Error> for CoreAvailabilityError {
    fn from(e: s::Error) -> Self {
        let code = match &e {
            s::Error::Secrets(_) => "secrets",
            s::Error::Library(_) => "library",
            s::Error::Invalid(_) => "invalid",
            _ => "other",
        };
        Self {
            code: code.into(),
            message: e.to_string(),
        }
    }
}

/// One watched server or source and its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreAvailabilityTarget {
    /// `source:<id>`, `peer:<device id>` or `endpoint:<id>`.
    pub id: String,
    /// `smb`, `webdav`, `sftp`, `ftp`, `nfs`, `jellyfin`, `emby`, `plex`,
    /// `subsonic`, `dlna`, `playlist`, `cloud`, `local`, `kinolune`,
    /// `sync-hub`, `sync-peer`.
    pub kind: String,
    pub name: String,
    /// `added`, `synced` (arrived by sync) or `auto` (picked by the core).
    pub origin: String,
    /// Sync goes through it: "sync paused" while it is down.
    pub sync_channel: bool,
    /// `unknown`, `available`, `slow`, `no-access`, `unavailable`,
    /// `no-network`.
    pub status: String,
    /// Since when it has this status (Unix ms).
    pub since_ms: Option<i64>,
    /// Last time it answered (Unix ms).
    pub last_ok_ms: Option<i64>,
    /// Last check (Unix ms).
    pub checked_at_ms: Option<i64>,
    /// Round trip of the last probe.
    pub rtt_ms: Option<u32>,
    /// Why it is not available.
    pub reason: Option<String>,
    /// Next active probe (Unix ms); `None` while probes are paused.
    pub next_probe_ms: Option<i64>,
}

impl From<TargetState> for CoreAvailabilityTarget {
    fn from(s: TargetState) -> Self {
        Self {
            id: s.target.id,
            kind: s.target.kind.as_str().into(),
            name: s.target.name,
            origin: s.target.origin.as_str().into(),
            sync_channel: s.target.sync_channel,
            status: s.status.as_str().into(),
            since_ms: s.since_ms,
            last_ok_ms: s.last_ok_ms,
            checked_at_ms: s.checked_at_ms,
            rtt_ms: s.rtt_ms,
            reason: s.reason,
            next_probe_ms: s.next_probe_ms,
        }
    }
}

/// A status change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreAvailabilityTransition {
    pub target_id: String,
    pub name: String,
    pub from: String,
    pub to: String,
    pub at_ms: i64,
    pub reason: Option<String>,
    /// Caused by a real operation (scan, playback, sync), not a probe.
    pub passive: bool,
}

impl From<Transition> for CoreAvailabilityTransition {
    fn from(t: Transition) -> Self {
        Self {
            target_id: t.target_id,
            name: t.name,
            from: t.from.as_str().into(),
            to: t.to.as_str().into(),
            at_ms: t.at_ms,
            reason: t.reason,
            passive: t.passive,
        }
    }
}

/// An event of [`CoreAvailability::watch_listen`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreAvailabilityEvent {
    /// `changed` (see `transition`) or `targets` (added or removed).
    pub kind: String,
    pub transition: Option<CoreAvailabilityTransition>,
}

/// A server that is not a source: the own Kinolune server, a sync hub, an
/// NFS export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreEndpoint {
    /// Stable id chosen by the app; the target is `endpoint:<id>`.
    pub id: String,
    /// `kinolune`, `sync-hub`, `nfs`, … (see [`CoreAvailabilityTarget::kind`]).
    pub kind: String,
    pub name: String,
    /// `http(s)://host:port[/path]` or `nfs://host[:port]/export`. For
    /// `http://` the path is fetched (default `/health`); otherwise a TCP
    /// connect.
    pub url: String,
    /// Sync goes through it.
    pub sync_channel: bool,
}

struct Endpoint {
    target: Target,
    url: url::Url,
}

/// Where the targets come from.
struct AppTargets {
    manager: Arc<s::SourceManager>,
    signals: SourceSignals,
    sync: Mutex<Option<(Weak<Catalog>, Weak<SyncService>)>>,
    endpoints: Mutex<Vec<Endpoint>>,
    /// The last folder sync round seen (end time, error).
    folder_seen: Mutex<Option<(Option<i64>, Option<String>)>>,
}

impl AppTargets {
    fn sync(&self) -> Option<(Arc<Catalog>, Arc<SyncService>)> {
        let guard = self.sync.lock().unwrap_or_else(PoisonError::into_inner);
        let (c, svc) = guard.as_ref()?;
        Some((c.upgrade()?, svc.upgrade()?))
    }

    fn folder(&self) -> Option<pa_catalog::FolderSyncStatus> {
        self.sync()?.0.folder_sync_status().ok()
    }

    fn peers(&self) -> Vec<pa_catalog::pa_sync::lan::service::DeviceStatus> {
        self.sync()
            .map(|(_, svc)| svc.devices())
            .unwrap_or_default()
            .into_iter()
            .filter(|d| !d.this_device)
            .collect()
    }
}

impl TargetSource for AppTargets {
    fn targets(&self) -> Vec<Target> {
        let folder = self.folder();
        let folder_id = folder.as_ref().and_then(|f| f.source_id);
        let auto = folder.as_ref().is_some_and(|f| f.mode.as_str() == "auto");
        let mut out: Vec<Target> = self
            .manager
            .list()
            .unwrap_or_default()
            .iter()
            .filter_map(|src| av::target_of(src, folder_id, auto))
            .collect();
        for d in self.peers() {
            let mut t = Target::new(
                format!("peer:{}", d.device_id),
                TargetKind::SyncPeer,
                d.name,
            );
            t.active = false;
            t.sync_channel = true;
            out.push(t);
        }
        out.extend(
            self.endpoints
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|e| e.target.clone()),
        );
        out
    }

    fn passive(&self) -> Vec<(String, ProbeOutcome)> {
        let mut out = self
            .signals
            .changes(&self.manager.list().unwrap_or_default());
        // SYNC-007: a folder sync round that reached the folder, or failed.
        if let Some(f) = self.folder()
            && let Some(id) = f.source_id
        {
            let now = (f.last_sync_ms, f.last_error.clone());
            let mut seen = self
                .folder_seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if seen.as_ref().is_some_and(|s| *s != now) {
                let outcome = match f.last_error.as_deref() {
                    None => ProbeOutcome::Ok { rtt_ms: 0 },
                    Some("denied") => ProbeOutcome::Denied("sync folder: denied".into()),
                    Some("unavailable") => {
                        ProbeOutcome::Unreachable("sync folder: unavailable".into())
                    }
                    // Full or other: the folder answered.
                    Some(_) => ProbeOutcome::Ok { rtt_ms: 0 },
                };
                out.push((av::target_id(id), outcome));
            }
            *seen = Some(now);
        }
        // Peers announce themselves on the LAN; the sync service knows.
        for d in self.peers() {
            out.push((
                format!("peer:{}", d.device_id),
                if d.online {
                    ProbeOutcome::Ok { rtt_ms: 0 }
                } else {
                    ProbeOutcome::Unreachable("not seen on the network".into())
                },
            ));
        }
        out
    }
}

/// Fetches `url` (plain HTTP) and maps the status line: 2xx/3xx answered,
/// 401/403 refused, anything else down.
fn http_probe(url: &url::Url, timeout: Duration) -> ProbeOutcome {
    let Some(host) = url.host_str() else {
        return ProbeOutcome::Unreachable("no host".into());
    };
    let port = url.port_or_known_default().unwrap_or(80);
    let start = Instant::now();
    let addr = match (host.trim_matches(['[', ']']), port).to_socket_addrs() {
        Ok(mut a) => match a.next() {
            Some(a) => a,
            None => return ProbeOutcome::Unreachable("host not found".into()),
        },
        Err(e) => return ProbeOutcome::Unreachable(format!("host not found: {e}")),
    };
    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(e) => return ProbeOutcome::Unreachable(format!("{addr}: {e}")),
    };
    let rtt = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let path = match url.path() {
        "" | "/" => "/health",
        p => p,
    };
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if let Err(e) = stream.write_all(request.as_bytes()) {
        return ProbeOutcome::Unreachable(e.to_string());
    }
    let mut head = [0u8; 32];
    let n = match stream.read(&mut head) {
        Ok(n) => n,
        Err(e) => return ProbeOutcome::Unreachable(e.to_string()),
    };
    let line = String::from_utf8_lossy(&head[..n]);
    let code: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    match code {
        200..=399 => ProbeOutcome::Ok { rtt_ms: rtt },
        401 | 403 => ProbeOutcome::Denied(format!("HTTP {code}")),
        0 => ProbeOutcome::Unreachable("no HTTP answer".into()),
        c => ProbeOutcome::Unreachable(format!("HTTP {c}")),
    }
}

/// Probes sources and endpoints.
struct AppProber {
    sources: SourceProber,
    targets: Arc<AppTargets>,
}

impl Prober for AppProber {
    fn probe(&self, target: &Target) -> ProbeOutcome {
        if av::source_id(&target.id).is_some() {
            return self.sources.probe(target);
        }
        let url = self
            .targets
            .endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|e| e.target.id == target.id)
            .map(|e| e.url.clone());
        let Some(url) = url else {
            return ProbeOutcome::Unreachable("unknown target".into());
        };
        if url.scheme() == "http" {
            return http_probe(&url, av::CONNECT_TIMEOUT);
        }
        let Some(host) = url.host_str() else {
            return ProbeOutcome::Unreachable("no host".into());
        };
        let port = url
            .port_or_known_default()
            .unwrap_or(if url.scheme() == "nfs" { 2049 } else { 443 });
        match av::tcp_probe(host.trim_matches(['[', ']']), port, av::CONNECT_TIMEOUT) {
            Ok(rtt) => ProbeOutcome::Ok { rtt_ms: rtt },
            Err(why) => ProbeOutcome::Unreachable(why),
        }
    }
}

/// The availability monitor of this app.
#[frb(opaque)]
pub struct CoreAvailability {
    monitor: Monitor,
    targets: Arc<AppTargets>,
}

impl Drop for CoreAvailability {
    fn drop(&mut self) {
        self.monitor.stop();
    }
}

impl CoreAvailability {
    /// Opens the monitor over the sources of the library database at
    /// `db_path` (secret store as in `CoreSources::open`) and starts it.
    /// `mobile`: a phone or tablet, which never probes in the background.
    pub fn open(
        db_path: String,
        data_dir: String,
        mobile: bool,
    ) -> Result<CoreAvailability, CoreAvailabilityError> {
        let opened =
            crate::shared::secret_store(std::path::Path::new(&data_dir)).map_err(s::Error::from)?;
        let manager = Arc::new(s::SourceManager::open(&db_path, Arc::clone(&opened.store))?);
        let targets = Arc::new(AppTargets {
            manager: Arc::clone(&manager),
            signals: SourceSignals::default(),
            sync: Mutex::new(None),
            endpoints: Mutex::new(Vec::new()),
            folder_seen: Mutex::new(None),
        });
        let prober = AppProber {
            sources: SourceProber::new(manager),
            targets: Arc::clone(&targets),
        };
        let monitor = Monitor::new(
            Policy {
                mobile,
                ..Policy::default()
            },
            Arc::new(prober),
            Arc::clone(&targets) as Arc<dyn TargetSource>,
            Arc::new(SystemClock),
        );
        monitor.start().map_err(|e| CoreAvailabilityError {
            code: "other".into(),
            message: e.to_string(),
        })?;
        Ok(Self { monitor, targets })
    }

    /// Watches the devices of the family and the sync folder of `sync`.
    #[frb(sync)]
    pub fn attach_sync(&self, sync: &CoreSync) {
        let (catalog, service) = sync.parts();
        *self
            .targets
            .sync
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((catalog, service));
        self.monitor.probe_now(None);
    }

    /// Servers that are not sources (replaces the previous list).
    #[frb(sync)]
    pub fn set_endpoints(&self, endpoints: Vec<CoreEndpoint>) -> Result<(), CoreAvailabilityError> {
        let mut parsed = Vec::with_capacity(endpoints.len());
        for e in endpoints {
            let url = url::Url::parse(&e.url).map_err(|_| CoreAvailabilityError {
                code: "invalid".into(),
                message: format!("not a URL: {}", e.id),
            })?;
            let kind = TargetKind::parse(&e.kind).unwrap_or(TargetKind::KinoluneServer);
            let mut target = Target::new(format!("endpoint:{}", e.id), kind, e.name);
            target.sync_channel = e.sync_channel;
            target.origin = Origin::Added;
            parsed.push(Endpoint { target, url });
        }
        *self
            .targets
            .endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = parsed;
        self.monitor.probe_now(None);
        Ok(())
    }

    /// The app is on screen (`true`) or not. Coming back probes
    /// everything at once.
    #[frb(sync)]
    pub fn set_app_state(&self, foreground: bool) {
        self.monitor.set_app_state(if foreground {
            AppState::Foreground
        } else {
            AppState::Background
        });
    }

    /// The device lost its network (`false`), got one or switched
    /// networks (`true`): everything is probed at once.
    #[frb(sync)]
    pub fn network_changed(&self, online: bool) {
        self.monitor.network_changed(online);
    }

    /// Probe `target_id` (or everything) now: the "retry" button.
    #[frb(sync)]
    pub fn probe_now(&self, target_id: Option<String>) {
        self.monitor.probe_now(target_id.as_deref());
    }

    /// A real operation on `target_id` (playback, a download) worked.
    #[frb(sync)]
    pub fn report_ok(&self, target_id: String) {
        self.monitor
            .report(&target_id, &ProbeOutcome::Ok { rtt_ms: 0 });
    }

    /// A real operation on `target_id` failed: `denied` for a refused
    /// login or missing rights, otherwise "does not answer".
    #[frb(sync)]
    pub fn report_failure(&self, target_id: String, denied: bool, reason: String) {
        let outcome = if denied {
            ProbeOutcome::Denied(reason)
        } else {
            ProbeOutcome::Unreachable(reason)
        };
        self.monitor.report(&target_id, &outcome);
    }

    /// Every watched target and its status.
    #[frb(sync)]
    pub fn targets(&self) -> Vec<CoreAvailabilityTarget> {
        self.monitor
            .snapshot()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Recent status changes, oldest first.
    #[frb(sync)]
    pub fn history(&self) -> Vec<CoreAvailabilityTransition> {
        self.monitor.history().into_iter().map(Into::into).collect()
    }

    /// Status changes until the stream is closed. `id` comes from
    /// `core_stream_next_id`; in Dart use `CoreAvailability.watch()` of
    /// pa_core. Sync: the subscription exists when this returns.
    #[frb(sync)]
    pub fn watch_listen(&self, id: i64, sink: StreamSink<CoreAvailabilityEvent>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sub = self
            .monitor
            .events()
            .subscribe(move |e: &AvailabilityEvent| tx.send(e.clone()).is_ok());
        let sink = crate::streams::Closable::open(id, sink);
        let monitor = self.monitor.clone();
        std::thread::Builder::new()
            .name("pa-availability-events".into())
            .spawn(move || {
                while let Some(event) = sink.recv(&rx) {
                    let event = match event {
                        AvailabilityEvent::Changed(t) => CoreAvailabilityEvent {
                            kind: "changed".into(),
                            transition: Some(t.into()),
                        },
                        AvailabilityEvent::TargetsChanged => CoreAvailabilityEvent {
                            kind: "targets".into(),
                            transition: None,
                        },
                    };
                    if !sink.send(|s| s.add(event).is_ok()) {
                        break;
                    }
                }
                monitor.events().unsubscribe(sub);
            })
            .map(drop)
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(status: &'static str) -> url::Url {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 256];
                let _ = s.read(&mut buf);
                let _ = s.write_all(format!("HTTP/1.1 {status}\r\n\r\n").as_bytes());
            }
        });
        url::Url::parse(&format!("http://127.0.0.1:{port}")).unwrap()
    }

    #[test]
    fn http_probe_maps_status_codes() {
        let t = Duration::from_secs(2);
        assert!(matches!(
            http_probe(&serve_once("200 OK"), t),
            ProbeOutcome::Ok { .. }
        ));
        assert!(matches!(
            http_probe(&serve_once("401 Unauthorized"), t),
            ProbeOutcome::Denied(_)
        ));
        assert!(matches!(
            http_probe(&serve_once("503 Service Unavailable"), t),
            ProbeOutcome::Unreachable(_)
        ));
    }
}
