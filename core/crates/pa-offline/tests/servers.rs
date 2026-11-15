// SPDX-License-Identifier: MPL-2.0
//! NET-019 "done when" on real servers: a 4 GiB movie is copied from Samba
//! while the network to the server drops for 20 s (the container is
//! paused: requests hang until the SMB timeout, as on a lost Wi-Fi) and
//! the copying process is killed with SIGKILL; it resumes from its offset
//! both times, the copy is byte for byte the source, and it plays through
//! the stream proxy with Samba stopped. An episode with its
//! subtitles comes from WebDAV and plays offline too. Progress reaches the
//! queue's event channel, which the app's Downloads screen listens to.
//! Run with `tests/servers/run.sh`, which starts the stand of LIB-018,
//! seeds the data and sets the variables read here.
#![cfg(feature = "server-tests")]

use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_catalog::{CancelToken, Catalog, SourceOpener, SourcesConnector};
use pa_jobs::{JobEvent, JobQueue, JobState, JobStore, QueueConfig};
use pa_offline::{CopyConfig, DownloadOptions, Offline, Selection};
use pa_proxy::{Proxy, ProxyOptions};
use pa_secrets::{MemoryStore, SecretString};
use pa_sources::{NewSource, PasswordChange, SourceKind, SourceManager, SourceUpdate};
use pa_storage::{StorageConfig, StorageManager};
use pa_vfs::{ByteSource, VfsPath};

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set: use tests/servers/run.sh"))
}

/// `docker compose … <args>` on the stand of this run.
fn compose(args: &str) {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{} {args}", var("PA_SERVERS_COMPOSE")))
        .status()
        .unwrap();
    assert!(status.success(), "compose {args}");
}

fn wait_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "port {port} did not come back");
        std::thread::sleep(Duration::from_millis(200));
    }
    // Samba accepts before it serves the share.
    std::thread::sleep(Duration::from_secs(2));
}

struct Env {
    catalog: Arc<Catalog>,
    queue: JobQueue,
    offline: Offline,
    source: i64,
}

/// Everything over the database in `dir` (reopened after a kill). The
/// source manager keeps the password in memory: the first open creates
/// the source, later opens update its password.
fn open(dir: &Path, kind: SourceKind, address: &str) -> Env {
    let db = dir.join("library.db");
    let catalog = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    let password = || SecretString::new(var("PA_SERVERS_PASS"));
    let source = match manager.list().unwrap().first() {
        // A restart: the in-memory secret store is new.
        Some(s) => {
            manager
                .update(
                    s.id,
                    SourceUpdate {
                        password: PasswordChange::Set(password()),
                        ..SourceUpdate::default()
                    },
                )
                .unwrap();
            s.id
        }
        None => {
            manager
                .create(
                    NewSource::new(kind, "NAS", address)
                        .with_login(var("PA_SERVERS_USER"), password()),
                )
                .unwrap()
                .id
        }
    };
    catalog.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));
    let storage = StorageManager::open(&db, StorageConfig::default()).unwrap();
    let queue = JobQueue::open(&db, QueueConfig::default()).unwrap();
    storage.attach(&queue);
    // The child copies at a set rate, so that the parent can stop the
    // server and kill it in the middle (loopback runs at ~1 GB/s).
    let config = CopyConfig {
        max_bytes_per_sec: std::env::var("PA_OFFLINE_RATE")
            .ok()
            .and_then(|r| r.parse().ok()),
        ..CopyConfig::default()
    };
    let offline =
        Offline::with_config(Arc::clone(&catalog), queue.clone(), storage.clone(), config).unwrap();
    offline.set_folder(&dir.join("offline")).unwrap();
    offline.register();
    Env {
        catalog,
        queue,
        offline,
        source,
    }
}

fn scan(cat: &Catalog, id: i64) {
    let mut errors = Vec::new();
    cat.scan(Some(id), &CancelToken::new(), &mut |u| {
        errors.extend(u.error)
    })
    .unwrap();
    assert_eq!(errors, Vec::<String>::new());
}

fn job_state(dir: &Path, job: i64) -> pa_jobs::Job {
    JobStore::open(dir.join("library.db"))
        .unwrap()
        .get(job)
        .unwrap()
}

fn part_len(dir: &Path, job: i64, rel: &str) -> u64 {
    let part = dir
        .join("offline/.tmp")
        .join(format!("job-{job}"))
        .join("out")
        .join(format!("{rel}.part"));
    std::fs::metadata(part).map(|m| m.len()).unwrap_or(0)
}

/// Compares two files in 8 MiB chunks.
fn same_bytes(a: &Path, b: &Path) -> bool {
    let (mut fa, mut fb) = (File::open(a).unwrap(), File::open(b).unwrap());
    if fa.metadata().unwrap().len() != fb.metadata().unwrap().len() {
        return false;
    }
    let (mut ba, mut bb) = (vec![0u8; 8 << 20], vec![0u8; 8 << 20]);
    loop {
        let n = fa.read(&mut ba).unwrap();
        if n == 0 {
            return true;
        }
        fb.read_exact(&mut bb[..n]).unwrap();
        if ba[..n] != bb[..n] {
            return false;
        }
    }
}

fn get(proxy: &Proxy, url: &str, range: &str) -> (u16, Vec<u8>) {
    let rest = url.strip_prefix("http://").unwrap();
    let path = &rest[rest.find('/').unwrap()..];
    let mut s = TcpStream::connect(proxy.local_addr()).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: x\r\nRange: {range}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = Vec::new();
    s.read_to_end(&mut reply).unwrap();
    let split = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&reply[..split]).into_owned();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        reply[split + 4..].to_vec(),
    )
}

fn local_range(path: &Path, offset: u64, len: usize) -> Vec<u8> {
    use std::io::{Seek, SeekFrom};
    let mut f = File::open(path).unwrap();
    f.seek(SeekFrom::Start(offset)).unwrap();
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).unwrap();
    buf
}

/// The child of the kill test: runs the queue until it is killed.
#[test]
#[ignore = "run by smb_4gb_copy_survives_a_drop_and_a_kill_and_plays_offline"]
fn copy_child_entry() {
    let Ok(dir) = std::env::var("PA_OFFLINE_CHILD_DIR") else {
        return;
    };
    let env = open(Path::new(&dir), SourceKind::Smb, &var("PA_SERVERS_SMB"));
    let _driver = env.queue.start();
    std::thread::sleep(Duration::from_secs(3600));
    panic!("the parent did not kill the child");
}

#[test]
fn smb_4gb_copy_survives_a_drop_and_a_kill_and_plays_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let big = var("PA_OFFLINE_BIG");
    let data = PathBuf::from(var("PA_SERVERS_DATA"));
    let size = std::fs::metadata(data.join(&big)).unwrap().len();
    let (job, item) = {
        let env = open(dir, SourceKind::Smb, &var("PA_SERVERS_SMB"));
        scan(&env.catalog, env.source);
        let movie = env
            .catalog
            .movies()
            .unwrap()
            .into_iter()
            .find(|m| m.rel_path == big)
            .expect("the big movie is in the library");
        let job = env
            .offline
            .download(&Selection::Item(movie.id), &DownloadOptions::default())
            .unwrap()[0];
        (job, movie.id)
    };

    // The copy runs in a child process, killed if this test fails.
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "copy_child_entry", "--ignored", "--nocapture"])
            .env("PA_OFFLINE_CHILD_DIR", dir)
            .env("PA_OFFLINE_RATE", (64u64 << 20).to_string())
            .spawn()
            .unwrap(),
    );
    let started = Instant::now();
    let wait_len = |at_least: u64, why: &str| {
        let deadline = Instant::now() + Duration::from_secs(1200);
        loop {
            let len = part_len(dir, job, &big);
            if len >= at_least {
                return len;
            }
            assert!(Instant::now() < deadline, "{why}: stuck at {len}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };

    // 1. The network to the server drops at 1 GiB for 20 s.
    let dropped_at = wait_len(1 << 30, "first GiB");
    compose("pause samba");
    let stopped_at = part_len(dir, job, &big);
    eprintln!(
        "samba paused at {stopped_at} bytes (asked at {dropped_at}) after {:?}",
        started.elapsed()
    );
    std::thread::sleep(Duration::from_secs(20));
    let during = part_len(dir, job, &big);
    // What the reader had prefetched before the server went away at most.
    assert!(
        during < stopped_at + (16 << 20),
        "kept copying without a server"
    );
    let state = job_state(dir, job);
    eprintln!(
        "while down: {} ({:?}), part {during}",
        state.state, state.error
    );
    compose("unpause samba");
    let resumed = Instant::now();
    // Resumed from the offset: the `.part` file never shrinks.
    let after = wait_len(during + (256 << 20), "resume after the drop");
    assert!(after > during);
    eprintln!(
        "resumed after the drop: {during} -> {after} in {:?}",
        resumed.elapsed()
    );

    // 2. Killed at 2.5 GiB.
    let killed_at = wait_len(size * 5 / 8, "kill point");
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let killed_at = killed_at.max(part_len(dir, job, &big));
    eprintln!("killed at {killed_at} bytes after {:?}", started.elapsed());
    assert_eq!(
        job_state(dir, job).state,
        JobState::Running,
        "killed while running"
    );

    // 3. The next start continues from the `.part` file.
    let env = open(dir, SourceKind::Smb, &var("PA_SERVERS_SMB"));
    let events = env.queue.subscribe();
    let restart = Instant::now();
    let _driver = env.queue.start();
    let mut first_progress = None;
    let mut reports = 0;
    let deadline = Instant::now() + Duration::from_secs(1800);
    loop {
        match events.recv_timeout(Duration::from_secs(1)) {
            Ok(JobEvent::Progress { id, bytes_done, .. }) if id == job => {
                first_progress.get_or_insert(bytes_done);
                reports += 1;
            }
            Ok(JobEvent::StateChanged { id, to, .. }) if id == job && to.is_terminal() => break,
            _ => {}
        }
        assert!(Instant::now() < deadline, "the copy did not finish");
    }
    let j = env.queue.get(job).unwrap();
    assert_eq!(j.state, JobState::Done, "{:?}", j.error);
    let first = first_progress.expect("progress events");
    eprintln!(
        "after the restart: first progress at {first}, {reports} reports, done in {:?}",
        restart.elapsed()
    );
    assert!(
        first + (16 << 20) >= killed_at,
        "restarted from {first}, not {killed_at}"
    );
    assert!(reports > 2, "progress reaches the event channel");

    let copies = env.offline.copies().unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].item, item.to_string());
    let copy = copies[0].folder.join(&big);
    assert!(same_bytes(&copy, &data.join(&big)), "the copy differs");
    // Linked, not a new item.
    assert_eq!(
        env.catalog
            .movies()
            .unwrap()
            .iter()
            .filter(|m| m.rel_path == big)
            .count(),
        1
    );

    // 4. Samba stopped (this stand cannot start it again; the run ends
    // with `compose down`): the proxy plays the copy.
    compose("stop -t 1 samba");
    let proxy = Proxy::start(
        Arc::new(SourceOpener::new(Arc::clone(&env.catalog))),
        ProxyOptions::default(),
    )
    .unwrap();
    let url = proxy.url(&SourceOpener::resource(
        env.source,
        &VfsPath::parse(&big).unwrap(),
    ));
    for offset in [0, size / 3, size - 1_000_000] {
        let (status, body) = get(
            &proxy,
            &url,
            &format!("bytes={offset}-{}", offset + 999_999),
        );
        assert_eq!(status, 206);
        assert_eq!(body, local_range(&data.join(&big), offset, 1_000_000));
    }
    eprintln!("played from the copy with Samba stopped");
    drop(proxy);
}

#[test]
fn webdav_episode_with_subtitles_plays_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let data = PathBuf::from(var("PA_SERVERS_DATA"));
    wait_port(18581);
    let env = open(dir, SourceKind::WebDav, &var("PA_SERVERS_WEBDAV"));
    scan(&env.catalog, env.source);
    let show = env.catalog.shows().unwrap()[0].id;
    let jobs = env
        .offline
        .download(
            &Selection::Season { show, season: 1 },
            &DownloadOptions::default(),
        )
        .unwrap();
    assert_eq!(jobs.len(), 2);
    let _driver = env.queue.start();
    let deadline = Instant::now() + Duration::from_secs(300);
    while env.offline.copies().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "{:?}", env.offline.downloads());
        std::thread::sleep(Duration::from_millis(100));
    }
    compose("stop -t 1 webdav");
    let opener = SourceOpener::new(Arc::clone(&env.catalog));
    for rel in [
        "Shows/Dark/Season 1/Dark.S01E01.Secrets.1080p.mkv",
        "Shows/Dark/Season 1/Dark.S01E01.Secrets.1080p.de.srt",
        "Shows/Dark/Season 1/Dark.S01E02.Lies.1080p.mkv",
    ] {
        let f = opener
            .open_file(env.source, &VfsPath::parse(rel).unwrap())
            .unwrap_or_else(|e| panic!("{rel}: {e}"));
        let mut buf = vec![0u8; usize::try_from(f.size()).unwrap()];
        f.read_exact_at(0, &mut buf).unwrap();
        assert_eq!(buf, std::fs::read(data.join(rel)).unwrap(), "{rel}");
    }
    eprintln!("WebDAV episodes and subtitles played with the server stopped");
}
