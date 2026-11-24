// SPDX-License-Identifier: MPL-2.0
//! Sources as targets of the availability monitor (NET-024, ADR-0157 §8).
//!
//! Every stored source becomes a [`Target`] — added by hand, from the LAN
//! list, by QR, arrived by sync (SYNC-010) or picked as the sync folder
//! (SYNC-015); the monitor treats them all the same. [`SourceProber`]
//! checks one cheaply:
//!
//! 1. a TCP connect to the server's port (5 s timeout) — the round trip
//!    is the "slow" measure;
//! 2. on the first probe, after any failure and then every
//!    [`FULL_CHECK_EVERY`], the protocol hello with the stored login,
//!    through the source's own backend ([`SourceManager::connect`] and a
//!    listing of the root, read-only): SMB negotiate and session, WebDAV
//!    `PROPFIND`, SFTP and FTP login, the media server's API. Only this
//!    can tell a wrong password ("no access") from a server that is down.
//!
//! Clouds have no server of the user's: only step 2, and no more often
//! than every [`CLOUD_INTERVAL`] (API quotas). Local folders: whether the
//! folder is there (an external drive may be unplugged).
//!
//! Passive signals ([`SourceSignals`]): the status a scan or a check
//! records in the database ([`SourceStatus`]) updates the monitor without
//! another request.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pa_runtime::availability::{Origin, ProbeOutcome, Prober, Target, TargetKind};
use pa_vfs::{ErrorKind, VfsPath};

use crate::{Error, Source, SourceKind, SourceManager, SourceSync, file_uri_to_path, playlist_file};

/// Timeout of the TCP connect of a probe.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How often a source that answers gets the full, logged-in check.
pub const FULL_CHECK_EVERY: Duration = Duration::from_secs(300);
/// Shortest interval between probes of a cloud storage.
pub const CLOUD_INTERVAL: Duration = Duration::from_secs(300);

/// Prefix of the target id of a source: `source:<id>`.
pub const TARGET_PREFIX: &str = "source:";

/// Target id of source `id`.
#[must_use]
pub fn target_id(id: i64) -> String {
    format!("{TARGET_PREFIX}{id}")
}

/// Source id of a target id made by [`target_id`].
#[must_use]
pub fn source_id(target_id: &str) -> Option<i64> {
    target_id.strip_prefix(TARGET_PREFIX)?.parse().ok()
}

/// The monitor's kind of a source kind.
#[must_use]
pub fn target_kind(kind: SourceKind) -> TargetKind {
    match kind {
        SourceKind::Local => TargetKind::Local,
        SourceKind::Smb => TargetKind::Smb,
        SourceKind::WebDav => TargetKind::WebDav,
        SourceKind::Jellyfin => TargetKind::Jellyfin,
        SourceKind::Emby => TargetKind::Emby,
        SourceKind::Sftp => TargetKind::Sftp,
        SourceKind::Ftp => TargetKind::Ftp,
        SourceKind::Subsonic => TargetKind::Subsonic,
        SourceKind::Dlna => TargetKind::Dlna,
        SourceKind::Playlist => TargetKind::Playlist,
        SourceKind::Plex => TargetKind::Plex,
        SourceKind::Mega | SourceKind::GoogleDrive | SourceKind::OneDrive | SourceKind::Dropbox => {
            TargetKind::Cloud
        }
    }
}

/// The target of `source`; `None` for a disabled source (it is not used,
/// so its availability does not matter). `sync_folder` is the source
/// whose folder syncs the library (SYNC-007), if any, and `auto_folder`
/// whether the core picked it itself (SYNC-015).
#[must_use]
pub fn target_of(source: &Source, sync_folder: Option<i64>, auto_folder: bool) -> Option<Target> {
    if !source.enabled && sync_folder != Some(source.id) {
        return None;
    }
    let kind = target_kind(source.kind);
    let mut t = Target::new(target_id(source.id), kind, source.name.clone());
    t.origin = if sync_folder == Some(source.id) && auto_folder {
        Origin::Auto
    } else if source.sync != SourceSync::Off {
        // Synced sources look the same on every device of the family;
        // which one added it is not stored, so they are all "synced".
        Origin::Synced
    } else {
        Origin::Added
    };
    t.sync_channel = sync_folder == Some(source.id);
    if kind == TargetKind::Cloud {
        t.min_interval = Some(CLOUD_INTERVAL);
    }
    Some(t)
}

/// `host:port` a probe connects to, from the source's address; `None`
/// for local folders, local playlists and clouds.
#[must_use]
pub fn endpoint(source: &Source) -> Option<(String, u16)> {
    if source.kind.cloud().is_some() || source.kind == SourceKind::Local {
        return None;
    }
    if source.kind == SourceKind::Playlist && playlist_file(&source.uri).is_some() {
        return None;
    }
    let url = url::Url::parse(&source.uri).ok()?;
    let host = url.host_str()?.trim_matches(['[', ']']).to_owned();
    let port = url.port().or_else(|| match url.scheme() {
        "smb" => Some(445),
        "sftp" | "ssh" => Some(22),
        "ftp" | "ftpes" => Some(21),
        "ftps" => Some(990),
        "nfs" => Some(2049),
        other => url::Url::parse(&format!("{other}://x"))
            .ok()?
            .port_or_known_default(),
    })?;
    Some((host, port))
}

/// Connects to `host:port` within `timeout`; the round trip in ms.
///
/// # Errors
///
/// The reason (host not found, refused, timed out), without secrets.
pub fn tcp_probe(host: &str, port: u16, timeout: Duration) -> Result<u32, String> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("host not found: {e}"))?
        .collect();
    let mut last = String::from("host not found");
    for addr in addrs {
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(_) => {
                return Ok(u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX));
            }
            Err(e) => last = format!("{addr}: {e}"),
        }
    }
    Err(last)
}

/// Whether a backend error means "it answered but refused us".
#[must_use]
pub fn is_denied(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::AuthenticationFailed | ErrorKind::PermissionDenied
    )
}

/// The outcome of a source error.
#[must_use]
pub fn outcome_of(error: &Error) -> ProbeOutcome {
    let (kind, message) = match error {
        Error::Source { kind, message, .. }
        | Error::Server { kind, message }
        | Error::Probe { kind, message } => (*kind, message.clone()),
        // The password could not be read from the secret store: the user
        // has to type it again.
        Error::Secrets(e) => return ProbeOutcome::Denied(format!("secret store: {e}")),
        other => (ErrorKind::Other, other.to_string()),
    };
    let why = format!("{kind}: {message}");
    if is_denied(kind) {
        ProbeOutcome::Denied(why)
    } else {
        ProbeOutcome::Unreachable(why)
    }
}

/// The outcome of the last error a scan or check recorded
/// ([`SourceStatus::last_error`]), from its text: `"<kind>: <message>"`.
#[must_use]
pub fn outcome_of_recorded(online: bool, last_error: Option<&str>) -> ProbeOutcome {
    if online {
        return ProbeOutcome::Ok { rtt_ms: 0 };
    }
    let why = last_error.unwrap_or("unavailable").to_owned();
    let lower = why.to_ascii_lowercase();
    if [ErrorKind::AuthenticationFailed, ErrorKind::PermissionDenied]
        .iter()
        .any(|k| lower.contains(&k.to_string()))
    {
        ProbeOutcome::Denied(why)
    } else {
        ProbeOutcome::Unreachable(why)
    }
}

#[derive(Debug, Clone, Copy)]
struct FullCheck {
    at: Instant,
    ok: bool,
}

/// Probes sources (see the module docs). Shares the source manager with
/// the rest of the app; it only reads.
pub struct SourceProber {
    manager: Arc<SourceManager>,
    full: Mutex<HashMap<i64, FullCheck>>,
    timeout: Duration,
}

impl std::fmt::Debug for SourceProber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceProber").finish_non_exhaustive()
    }
}

impl SourceProber {
    /// A prober over `manager`.
    #[must_use]
    pub fn new(manager: Arc<SourceManager>) -> Self {
        Self {
            manager,
            full: Mutex::new(HashMap::new()),
            timeout: CONNECT_TIMEOUT,
        }
    }

    /// The full, logged-in check: the root of the source through its own
    /// backend.
    fn full_check(&self, id: i64, rtt_ms: u32) -> ProbeOutcome {
        let result = self
            .manager
            .connect(id)
            .and_then(|vfs| {
                vfs.list(&VfsPath::root())
                    .map(drop)
                    .map_err(|e| Error::Source {
                        id,
                        kind: e.kind(),
                        message: e.message().to_owned(),
                    })
            });
        let outcome = match result {
            Ok(()) => ProbeOutcome::Ok { rtt_ms },
            Err(e) => outcome_of(&e),
        };
        self.full
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id,
                FullCheck {
                    at: Instant::now(),
                    ok: matches!(outcome, ProbeOutcome::Ok { .. }),
                },
            );
        outcome
    }

    fn needs_full_check(&self, id: i64) -> bool {
        self.full
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .is_none_or(|c| !c.ok || c.at.elapsed() >= FULL_CHECK_EVERY)
    }

    /// Probes source `id`.
    pub fn probe_source(&self, id: i64) -> ProbeOutcome {
        let source = match self.manager.get(id) {
            Ok(s) => s,
            Err(e) => return ProbeOutcome::Unreachable(e.to_string()),
        };
        if source.kind == SourceKind::Local
            || (source.kind == SourceKind::Playlist && playlist_file(&source.uri).is_some())
        {
            let path = file_uri_to_path(&source.uri).or_else(|| playlist_file(&source.uri));
            let start = Instant::now();
            return match path.map(std::fs::metadata) {
                Some(Ok(_)) => ProbeOutcome::Ok {
                    rtt_ms: u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX),
                },
                Some(Err(e)) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    ProbeOutcome::Denied(e.to_string())
                }
                Some(Err(e)) => ProbeOutcome::Unreachable(e.to_string()),
                None => ProbeOutcome::Unreachable("not a local path".into()),
            };
        }
        let Some((host, port)) = endpoint(&source) else {
            // A cloud: the API call is the probe.
            return self.full_check(id, 0);
        };
        match tcp_probe(&host, port, self.timeout) {
            Err(why) => {
                // Down: the next answer gets the full check again.
                self.full
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
                ProbeOutcome::Unreachable(why)
            }
            Ok(rtt) if self.needs_full_check(id) => self.full_check(id, rtt),
            Ok(rtt) => ProbeOutcome::Ok { rtt_ms: rtt },
        }
    }
}

impl Prober for SourceProber {
    fn probe(&self, target: &Target) -> ProbeOutcome {
        match source_id(&target.id) {
            Some(id) => self.probe_source(id),
            None => ProbeOutcome::Unreachable(format!("not a source: {}", target.id)),
        }
    }
}

/// Passive signals from the source table: a status recorded by a scan or
/// a check since the last call.
#[derive(Debug, Default)]
pub struct SourceSignals {
    seen: Mutex<Option<HashMap<i64, i64>>>,
}

impl SourceSignals {
    /// What `sources` recorded since the last call. The first call only
    /// remembers the times (old results are not news).
    pub fn changes(&self, sources: &[Source]) -> Vec<(String, ProbeOutcome)> {
        let mut guard = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let first = guard.is_none();
        let seen = guard.get_or_insert_with(HashMap::new);
        let mut out = Vec::new();
        for s in sources {
            let Some(at) = s.status.checked_at_ms else {
                continue;
            };
            let before = seen.insert(s.id, at);
            if first || before.is_some_and(|b| b >= at) {
                continue;
            }
            if let Some(online) = s.status.online {
                out.push((
                    target_id(s.id),
                    outcome_of_recorded(online, s.status.last_error.as_deref()),
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceOptions, SourceStatus};
    use std::net::TcpListener;

    fn source(id: i64, kind: SourceKind, uri: &str) -> Source {
        Source {
            id,
            kind,
            name: format!("s{id}"),
            uri: uri.into(),
            username: None,
            has_password: false,
            options: SourceOptions::default(),
            scan_into_library: true,
            auto_scan: true,
            enabled: true,
            sync: SourceSync::Off,
            position: 0,
            created_at_ms: 0,
            updated_at_ms: None,
            status: SourceStatus::default(),
        }
    }

    #[test]
    fn target_ids_round_trip() {
        assert_eq!(source_id(&target_id(42)), Some(42));
        assert_eq!(source_id("peer:42"), None);
    }

    #[test]
    fn endpoints_by_scheme() {
        let cases = [
            (SourceKind::Smb, "smb://nas/media", ("nas", 445)),
            (SourceKind::Sftp, "sftp://nas:2222/x", ("nas", 2222)),
            (SourceKind::Ftp, "ftp://nas/x", ("nas", 21)),
            (SourceKind::Ftp, "ftps://nas/x", ("nas", 990)),
            (SourceKind::WebDav, "https://dav.example/x", ("dav.example", 443)),
            (SourceKind::Jellyfin, "http://10.0.0.2:8096", ("10.0.0.2", 8096)),
            (SourceKind::Plex, "http://[fd00::1]:32400", ("fd00::1", 32400)),
        ];
        for (kind, uri, (host, port)) in cases {
            assert_eq!(
                endpoint(&source(1, kind, uri)),
                Some((host.to_owned(), port)),
                "{uri}"
            );
        }
        assert_eq!(endpoint(&source(1, SourceKind::Local, "file:///m")), None);
        assert_eq!(endpoint(&source(1, SourceKind::GoogleDrive, "gdrive:/")), None);
    }

    #[test]
    fn every_origin_becomes_a_target() {
        let manual = source(1, SourceKind::Smb, "smb://nas/a");
        let mut synced = source(2, SourceKind::WebDav, "https://dav/x");
        synced.sync = SourceSync::WithPassword;
        let folder = source(3, SourceKind::Smb, "smb://nas/sync");
        let mut disabled = source(4, SourceKind::Smb, "smb://nas/b");
        disabled.enabled = false;
        let cloud = source(5, SourceKind::Dropbox, "dropbox:/");

        let t1 = target_of(&manual, Some(3), true).unwrap();
        assert_eq!((t1.origin, t1.sync_channel), (Origin::Added, false));
        let t2 = target_of(&synced, Some(3), true).unwrap();
        assert_eq!(t2.origin, Origin::Synced);
        assert!(t2.active);
        let t3 = target_of(&folder, Some(3), true).unwrap();
        assert_eq!((t3.origin, t3.sync_channel), (Origin::Auto, true));
        assert_eq!(target_of(&folder, Some(3), false).unwrap().origin, Origin::Added);
        assert!(target_of(&disabled, None, false).is_none());
        let t5 = target_of(&cloud, None, false).unwrap();
        assert_eq!(t5.kind, TargetKind::Cloud);
        assert_eq!(t5.min_interval, Some(CLOUD_INTERVAL));
    }

    #[test]
    fn errors_split_into_denied_and_unreachable() {
        let denied = Error::Source {
            id: 1,
            kind: ErrorKind::AuthenticationFailed,
            message: "logon failure".into(),
        };
        assert!(matches!(outcome_of(&denied), ProbeOutcome::Denied(_)));
        let down = Error::Source {
            id: 1,
            kind: ErrorKind::Unavailable,
            message: "refused".into(),
        };
        assert!(matches!(outcome_of(&down), ProbeOutcome::Unreachable(_)));
        assert!(matches!(
            outcome_of_recorded(false, Some("source 1: authentication failed: x")),
            ProbeOutcome::Denied(_)
        ));
        assert!(matches!(
            outcome_of_recorded(false, Some("timed out: x")),
            ProbeOutcome::Unreachable(_)
        ));
        assert_eq!(
            outcome_of_recorded(true, None),
            ProbeOutcome::Ok { rtt_ms: 0 }
        );
    }

    #[test]
    fn tcp_probe_sees_open_and_closed_ports() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(tcp_probe("127.0.0.1", port, Duration::from_secs(1)).is_ok());
        drop(listener);
        assert!(tcp_probe("127.0.0.1", port, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn signals_report_only_new_results() {
        let signals = SourceSignals::default();
        let mut s = source(1, SourceKind::Smb, "smb://nas/a");
        s.status.online = Some(true);
        s.status.checked_at_ms = Some(10);
        assert!(signals.changes(std::slice::from_ref(&s)).is_empty());
        assert!(signals.changes(std::slice::from_ref(&s)).is_empty());
        s.status.online = Some(false);
        s.status.checked_at_ms = Some(20);
        s.status.last_error = Some("source unavailable: refused".into());
        let c = signals.changes(std::slice::from_ref(&s));
        assert_eq!(c.len(), 1);
        assert!(matches!(c[0].1, ProbeOutcome::Unreachable(_)));
    }
}
