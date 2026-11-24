// SPDX-License-Identifier: MPL-2.0
//! The NET-024 availability contract against real servers in Docker
//! ("done when"): Samba, WebDAV (rclone), SFTP (OpenSSH), FTP (vsftpd)
//! and, when `PA_AV_JELLYFIN` is set, Jellyfin.
//!
//! - container stopped → "unavailable" within 35 s;
//! - container started → "available" within 10 s of a network event;
//! - wrong password → "no access", not "unavailable";
//! - a source that arrived by sync and the sync folder are probed like
//!   sources added by hand.
//!
//! Run with `tests/availability_servers/run.sh`: it starts the servers of
//! the NET-023 write contract (`tests/write_servers`) under the compose
//! project `pa-test-net024` and sets the variables read here.
#![cfg(feature = "server-tests")]

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_runtime::availability::{
    Monitor, Policy, ProbeOutcome, Prober, Status, SystemClock, Target, TargetKind, TargetSource,
};
use pa_secrets::{MemoryStore, SecretString};
use pa_sources::availability::{self as av, SourceProber};
use pa_sources::{NewSource, SourceKind, SourceManager, SourceSync};

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set: use tests/availability_servers/run.sh")
    })
}

fn docker(args: &[&str]) {
    let ok = Command::new("docker")
        .args(args)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "docker {args:?} failed");
}

/// Container of a compose service of the run.
fn container(service: &str) -> String {
    format!("{}-{service}-1", var("PA_W_PROJECT"))
}

struct Sources {
    manager: Arc<SourceManager>,
    folder: i64,
    extra: Vec<Target>,
}

impl TargetSource for Sources {
    fn targets(&self) -> Vec<Target> {
        let mut out: Vec<Target> = self
            .manager
            .list()
            .unwrap()
            .iter()
            .filter_map(|s| av::target_of(s, Some(self.folder), true))
            .collect();
        out.extend(self.extra.iter().cloned());
        out
    }
}

/// Sources through [`SourceProber`], other targets by a TCP connect.
struct ContractProber {
    sources: SourceProber,
    tcp: Vec<(String, String, u16)>,
}

impl Prober for ContractProber {
    fn probe(&self, target: &Target) -> ProbeOutcome {
        if let Some((_, host, port)) = self.tcp.iter().find(|(id, _, _)| *id == target.id) {
            return match av::tcp_probe(host, *port, av::CONNECT_TIMEOUT) {
                Ok(rtt) => ProbeOutcome::Ok { rtt_ms: rtt },
                Err(e) => ProbeOutcome::Unreachable(e),
            };
        }
        self.sources.probe(target)
    }
}

fn login(new: NewSource) -> NewSource {
    new.with_login(var("PA_W_USER"), SecretString::new(var("PA_W_PASS")))
}

fn status_of(monitor: &Monitor, id: &str) -> Status {
    monitor
        .snapshot()
        .iter()
        .find(|s| s.target.id == id)
        .map_or(Status::Unknown, |s| s.status)
}

/// Waits until every `ids` has `status`; returns how long it took.
fn wait_all(monitor: &Monitor, ids: &[String], status: Status, limit: Duration) -> Duration {
    let start = Instant::now();
    loop {
        let states: Vec<(String, Status)> = ids
            .iter()
            .map(|id| (id.clone(), status_of(monitor, id)))
            .collect();
        if states.iter().all(|(_, s)| *s == status) {
            return start.elapsed();
        }
        assert!(
            start.elapsed() <= limit,
            "not {status:?} within {limit:?}: {states:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn servers_go_down_and_come_back() {
    let manager = Arc::new(SourceManager::new(
        pa_library::open_in_memory().unwrap(),
        Arc::new(MemoryStore::new()),
    ));
    let smb = manager
        .create(login(NewSource::new(
            SourceKind::Smb,
            "Samba",
            format!("smb://{}/rw", var("PA_W_SMB")),
        )))
        .unwrap();
    // Arrived by sync from another device: checked the same way.
    let dav = manager
        .create(
            login(NewSource::new(
                SourceKind::WebDav,
                "WebDAV",
                var("PA_W_RCLONE"),
            ))
            .with_sync(SourceSync::WithPassword),
        )
        .unwrap();
    // The sync folder (picked automatically).
    let sftp = manager
        .create(login(NewSource::new(
            SourceKind::Sftp,
            "SFTP",
            format!("sftp://{}/data", var("PA_W_SFTP")),
        )))
        .unwrap();
    let ftp = manager
        .create(login(NewSource::new(
            SourceKind::Ftp,
            "FTP",
            format!("ftp://{}/", var("PA_W_FTP")),
        )))
        .unwrap();
    // The same Samba with a wrong password.
    let wrong = manager
        .create(
            NewSource::new(
                SourceKind::Smb,
                "Samba, wrong password",
                format!("smb://{}/rw", var("PA_W_SMB")),
            )
            .with_login(var("PA_W_USER"), SecretString::new("wrong".into())),
        )
        .unwrap();

    let mut extra = Vec::new();
    let mut tcp = Vec::new();
    let mut containers = vec![
        container("samba"),
        container("rclone"),
        container("openssh"),
        container("vsftpd"),
    ];
    if let Ok(addr) = std::env::var("PA_AV_JELLYFIN") {
        let (host, port) = addr.rsplit_once(':').unwrap();
        let t = Target::new("endpoint:jellyfin", TargetKind::Jellyfin, "Jellyfin");
        tcp.push((t.id.clone(), host.to_owned(), port.parse().unwrap()));
        extra.push(t);
        containers.push(var("PA_AV_JELLYFIN_CONTAINER"));
    }
    let up: Vec<String> = [smb.id, dav.id, sftp.id, ftp.id]
        .into_iter()
        .map(av::target_id)
        .chain(extra.iter().map(|t| t.id.clone()))
        .collect();

    let source = Arc::new(Sources {
        manager: Arc::clone(&manager),
        folder: sftp.id,
        extra,
    });
    let monitor = Monitor::new(
        Policy::default(),
        Arc::new(ContractProber {
            sources: SourceProber::new(Arc::clone(&manager)),
            tcp,
        }),
        Arc::clone(&source) as Arc<dyn TargetSource>,
        Arc::new(SystemClock),
    );
    monitor.start().unwrap();

    wait_all(&monitor, &up, Status::Available, Duration::from_secs(60));
    assert_eq!(
        status_of(&monitor, &av::target_id(wrong.id)),
        Status::NoAccess,
        "a wrong password is \"no access\""
    );
    let snapshot = monitor.snapshot();
    let synced = snapshot
        .iter()
        .find(|s| s.target.id == av::target_id(dav.id))
        .unwrap();
    assert_eq!(synced.target.origin, pa_runtime::availability::Origin::Synced);
    let folder = snapshot
        .iter()
        .find(|s| s.target.id == av::target_id(sftp.id))
        .unwrap();
    assert!(folder.target.sync_channel);

    // Stop every container: each is "unavailable" within 35 s (probe
    // interval 30 s plus the 5 s connect timeout).
    for c in &containers {
        docker(&["stop", "-t", "1", c]);
    }
    let took = wait_all(&monitor, &up, Status::Unavailable, Duration::from_secs(35));
    eprintln!("unavailable after {took:?}");

    // Start them, wait until they answer, then a network event: each is
    // "available" within 10 s.
    for c in &containers {
        docker(&["start", c]);
    }
    let ready = Instant::now();
    for (host, port) in [
        var("PA_W_SMB"),
        var("PA_W_SFTP"),
        var("PA_W_FTP"),
        var("PA_W_RCLONE")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_owned(),
    ]
    .iter()
    .map(|a| a.rsplit_once(':').unwrap())
    {
        while av::tcp_probe(host, port.parse().unwrap(), Duration::from_secs(1)).is_err() {
            assert!(ready.elapsed() < Duration::from_secs(180), "{host}:{port}");
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    // The daemons inside need a moment after the port opens.
    std::thread::sleep(Duration::from_secs(3));
    monitor.network_changed(true);
    let took = wait_all(&monitor, &up, Status::Available, Duration::from_secs(10));
    eprintln!("available after {took:?}");
    monitor.stop();
}

/// Without a network event the backoff still brings it back within 5 min
/// (slow: only with `PA_AV_SLOW=1`).
#[test]
fn backoff_brings_a_server_back_within_5_min() {
    if std::env::var("PA_AV_SLOW").is_err() {
        eprintln!("skipped: set PA_AV_SLOW=1");
        return;
    }
    let manager = Arc::new(SourceManager::new(
        pa_library::open_in_memory().unwrap(),
        Arc::new(MemoryStore::new()),
    ));
    let ftp = manager
        .create(login(NewSource::new(
            SourceKind::Ftp,
            "FTP",
            format!("ftp://{}/", var("PA_W_FTP")),
        )))
        .unwrap();
    let id = av::target_id(ftp.id);
    let targets = Arc::new(Mutex::new(vec![
        av::target_of(&manager.get(ftp.id).unwrap(), None, false).unwrap(),
    ]));
    let monitor = Monitor::new(
        Policy::default(),
        Arc::new(SourceProber::new(Arc::clone(&manager))),
        targets as Arc<dyn TargetSource>,
        Arc::new(SystemClock),
    );
    monitor.start().unwrap();
    let ids = [id];
    wait_all(&monitor, &ids, Status::Available, Duration::from_secs(60));
    let c = container("vsftpd");
    docker(&["stop", "-t", "1", &c]);
    wait_all(&monitor, &ids, Status::Unavailable, Duration::from_secs(35));
    // Down for a few backoff steps, then back.
    std::thread::sleep(Duration::from_secs(200));
    docker(&["start", &c]);
    wait_all(&monitor, &ids, Status::Available, Duration::from_secs(300 + 15));
    monitor.stop();
}
