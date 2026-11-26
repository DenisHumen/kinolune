// SPDX-License-Identifier: MPL-2.0
//! LIB-028 on three processes in a LAN (loopback): a desktop drops a file
//! into the library (the source), a server is the hub, a phone watches.
//! Each process has its own catalog, keys and LAN node; the children are
//! this test binary run again with `LIVE_UPLOAD_CHILD` set, and talk over
//! stdin/stdout (one command per line).
//!
//! What it checks (the "done when" of LIB-028):
//! - the grey card (the `started` step) is on the phone < 1 s after the
//!   drop;
//! - progress comes at 8–10 Hz while the phone watches, ≤ 1 Hz otherwise;
//! - `done` reaches the phone < 1 s after the copy ends;
//! - a cancel from the phone reaches the source, and the card goes away on
//!   the phone and the server; a source that drops off the network reads as
//!   offline on both, and moves on when it is back;
//! - load: 4 uploads at once, watched — CPU of the phone's process and the
//!   bytes of progress it receives (printed for the PR).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_catalog::Catalog;
use pa_catalog::pa_sync::hlc::system_ms;
use pa_catalog::pa_sync::lan::service::{ServiceConfig, ServiceEvent, SyncLibrary, SyncService};
use pa_catalog::pa_sync::upload::{STALE_MS, UploadState};
use pa_catalog::pa_sync::{Op, OpId, UploadPhase};
use pa_discovery::DeviceClass;
use pa_secrets::MemoryStore;

const CHILD_ENV: &str = "LIVE_UPLOAD_CHILD";
const SIZE: u64 = 2_900_000_000;
const BPS: u64 = 85_000_000;

struct Lib(Arc<Catalog>);

impl SyncLibrary for Lib {
    fn enable(&self, device_id: &str) -> Result<(), String> {
        self.0.enable_sync(device_id).map_err(|e| e.to_string())
    }
    fn disable(&self) -> Result<(), String> {
        self.0.disable_sync().map_err(|e| e.to_string())
    }
    fn pending(&self) -> Result<Vec<Op>, String> {
        self.0.pending_ops(512).map_err(|e| e.to_string())
    }
    fn ack(&self, op_id: OpId) -> Result<(), String> {
        self.0.ack_op(op_id).map(drop).map_err(|e| e.to_string())
    }
    fn apply(&self, op: &Op) -> Result<bool, String> {
        self.0.apply_remote(op).map_err(|e| e.to_string())
    }
}

struct Device {
    catalog: Arc<Catalog>,
    service: SyncService,
    _dir: tempfile::TempDir,
}

fn device(name: &str, class: DeviceClass) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Arc::new(Catalog::open(dir.path().join("db/library.db")).unwrap());
    let config = ServiceConfig {
        bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        mdns: None,
        flush_interval: Duration::from_millis(200),
        ..ServiceConfig::new(dir.path().join("sync"), name, class)
    };
    let service = SyncService::open(
        config,
        Arc::new(MemoryStore::new()),
        Arc::new(Lib(Arc::clone(&catalog))),
    )
    .unwrap();
    Device {
        catalog,
        service,
        _dir: dir,
    }
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn class_of(name: &str) -> DeviceClass {
    match name {
        "server" => DeviceClass::Server,
        "phone" => DeviceClass::Mobile,
        _ => DeviceClass::Desktop,
    }
}

/// CPU time of this process (user + system), ms. Linux only.
fn cpu_ms() -> u64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return 0;
    };
    // Fields after the `(comm)`: state is 3rd, utime 14th, stime 15th.
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    // `CLK_TCK` is 100 on every Linux this runs on.
    (ticks(11) + ticks(12)) * 10
}

/// A child: joins the family, prints what it sees, answers commands.
fn child(role: &str, qr: &str) {
    let d = device(role, class_of(role));
    d.service.join(qr).unwrap();
    wait("child running", || d.service.is_running());
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let say = {
        let out = Arc::clone(&out);
        move |line: String| {
            let mut o = out.lock().unwrap();
            writeln!(o, "{line}").unwrap();
            o.flush().unwrap();
        }
    };
    say(format!(
        "READY {} {}",
        d.service.device_id(),
        d.service.sync_addr().unwrap()
    ));
    // What the cards show: the durable steps and the live ticks.
    let events = d.service.subscribe();
    let (catalog, service) = (Arc::clone(&d.catalog), Arc::new(()));
    let svc: &'static SyncService = Box::leak(Box::new(d.service));
    {
        let say = say.clone();
        std::thread::spawn(move || {
            let _keep = service;
            let mut phases: HashMap<String, UploadPhase> = HashMap::new();
            let mut ticks: HashMap<String, (UploadState, u64)> = HashMap::new();
            loop {
                let event = events.recv_timeout(Duration::from_millis(250));
                if matches!(event, Err(std::sync::mpsc::RecvTimeoutError::Disconnected)) {
                    return;
                }
                if matches!(event, Ok(ServiceEvent::LibraryChanged)) {
                    for u in catalog.live_uploads().unwrap() {
                        if phases.insert(u.upload_id.clone(), u.phase) != Some(u.phase) {
                            say(format!("PHASE {} {:?} {}", u.upload_id, u.phase, system_ms()));
                        }
                    }
                }
                if matches!(event, Ok(ServiceEvent::Uploads)) {
                    for u in svc.uploads() {
                        let now = (u.state, u.done);
                        let before = ticks.insert(u.id.clone(), now);
                        if before.map(|b| b.0) != Some(u.state) {
                            say(format!("STATE {} {:?} {}", u.id, u.state, system_ms()));
                        }
                        if before.map(|b| b.1) != Some(u.done) {
                            say(format!("TICK {} {} {}", u.id, u.done, system_ms()));
                        }
                    }
                }
            }
        });
    }
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["PEER", id, class, addr] => {
                svc.peer_up(id, class_of(class), addr.parse().unwrap());
                say("OK".into());
            }
            ["WATCH", on] => {
                svc.upload_watch(*on == "on");
                say("OK".into());
            }
            ["CANCEL", to, id] => {
                svc.upload_cancel(to, vec![(*id).to_owned()]).unwrap();
                say("OK".into());
            }
            ["HUB"] => say(format!("HUB {}", svc.hub().unwrap_or_default())),
            ["LOAD"] => {
                let t = svc.upload_traffic();
                say(format!(
                    "LOAD {} {} {} {}",
                    cpu_ms(),
                    t.recv_bytes,
                    t.recv_frames,
                    system_ms()
                ));
            }
            ["QUIT"] => return,
            _ => panic!("unknown command {line}"),
        }
    }
}

/// Entry of the child processes; a no-op in a normal test run.
#[test]
fn live_upload_child_process() {
    if let Ok(arg) = std::env::var(CHILD_ENV) {
        let (role, qr) = arg.split_once(' ').unwrap();
        child(role, qr);
    }
}

/// A child process and its lines.
struct Child {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: Receiver<String>,
    /// Lines read while looking for another tag.
    seen: Vec<String>,
    id: String,
    addr: String,
}

impl Child {
    fn spawn(role: &str, qr: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live_upload_child_process",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, format!("{role} {qr}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let mut c = Self {
            child,
            stdin,
            lines,
            seen: Vec::new(),
            id: String::new(),
            addr: String::new(),
        };
        let ready = c.expect("READY", "");
        c.id.clone_from(&ready[1]);
        c.addr.clone_from(&ready[2]);
        c
    }

    /// Moves every line read so far into `seen`.
    fn drain(&mut self) {
        while let Ok(line) = self.lines.try_recv() {
            self.seen.push(line);
        }
    }

    /// The words of the first line (read or seen) that starts with `tag`
    /// and contains `with`.
    fn expect(&mut self, tag: &str, with: &str) -> Vec<String> {
        let matches = |line: &str| {
            line.find(tag)
                .map(|at| &line[at..])
                .filter(|l| l.contains(with))
                .map(|l| l.split_whitespace().map(str::to_owned).collect())
        };
        if let Some(i) = self.seen.iter().position(|l| matches(l).is_some()) {
            let line = self.seen.remove(i);
            return matches(&line).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("child did not say {tag} {with}"));
            if let Some(words) = matches(&line) {
                return words;
            }
            self.seen.push(line);
        }
    }

    fn send(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn cmd(&mut self, line: &str) {
        self.send(line);
        self.expect("OK", "");
    }

    /// Ticks of `id` seen so far: `(done, local ms)`.
    fn ticks(&mut self, id: &str) -> Vec<(u64, u64)> {
        self.drain();
        let tag = format!("TICK {id} ");
        self.seen
            .iter()
            .filter_map(|l| l.find(&tag).map(|at| &l[at..]))
            .map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                (w[2].parse().unwrap(), w[3].parse().unwrap())
            })
            .collect()
    }

    fn load(&mut self) -> (u64, u64, u64, u64) {
        self.send("LOAD");
        let w = self.expect("LOAD", "");
        let n = |i: usize| w[i].parse::<u64>().unwrap();
        (n(1), n(2), n(3), n(4))
    }
}

/// Frames per second over the ticks between `from` and `to` (local ms).
fn rate(ticks: &[(u64, u64)], from: u64, to: u64) -> f64 {
    let t: Vec<u64> = ticks
        .iter()
        .map(|t| t.1)
        .filter(|ms| (from..to).contains(ms))
        .collect();
    if t.len() < 2 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let r = (t.len() - 1) as f64 * 1000.0 / (t[t.len() - 1] - t[0]) as f64;
    r
}

/// The source copies `ids` for `ms` at [`BPS`] each, reporting every 10 ms
/// like the app's upload callback.
fn copy(src: &SyncService, ids: &[&str], offset: &mut u64, ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        *offset = (*offset + BPS / 100).min(SIZE - 1);
        for id in ids {
            src.upload_report(id, UploadState::Copying, *offset, SIZE);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn info(name: &str) -> String {
    format!(
        r#"{{"name":"{name}","size":{SIZE},"kind":"movie","dir":"Movies","device_name":"Desktop","guess":{{"title":"Dune Part Two","year":2024}}}}"#
    )
}

#[test]
fn a_dropped_file_shows_on_every_device_with_live_progress() {
    let src = device("Desktop", DeviceClass::Desktop);
    // The phone joins first: the server, joining last, gets the whole
    // roster, and the phone hears of the server from the source (a hub
    // learns new members only from devices that dial it).
    let invite = src.service.invite().unwrap();
    let mut phone = Child::spawn("phone", &invite.qr);
    wait("source running", || src.service.is_running());
    let invite = src.service.invite().unwrap();
    let mut server = Child::spawn("server", &invite.qr);
    // No mDNS on loopback: everybody is told where the others are.
    let src_addr = src.service.sync_addr().unwrap().to_string();
    src.service
        .peer_up(&server.id, DeviceClass::Server, server.addr.parse().unwrap());
    src.service
        .peer_up(&phone.id, DeviceClass::Mobile, phone.addr.parse().unwrap());
    let (sid, saddr, pid, paddr) = (
        server.id.clone(),
        server.addr.clone(),
        phone.id.clone(),
        phone.addr.clone(),
    );
    let me = src.service.device_id().to_owned();
    server.cmd(&format!("PEER {me} desktop {src_addr}"));
    server.cmd(&format!("PEER {pid} phone {paddr}"));
    phone.cmd(&format!("PEER {me} desktop {src_addr}"));
    phone.cmd(&format!("PEER {sid} server {saddr}"));
    // The server outranks the others: it is the hub.
    wait("source links to the server", || {
        src.service.hub().as_deref() == Some(sid.as_str())
    });
    wait("phone links to the server", || {
        phone.send("HUB");
        let hub = phone.expect("HUB", "");
        std::thread::sleep(Duration::from_millis(100));
        hub.get(1) == Some(&sid)
    });
    let events = src.service.subscribe();

    // 1. The drop: the grey card is on the phone < 1 s later.
    let t0 = system_ms();
    src.catalog
        .record_upload_event("u1", UploadPhase::Started, &info("Dune.Part.Two.2024.mkv"))
        .unwrap();
    src.service.flush();
    src.service.upload_report("u1", UploadState::Waiting, 0, SIZE);
    let card = phone.expect("PHASE u1 Started", "")[3].parse::<u64>().unwrap();
    let card_ms = card - t0;
    eprintln!("LIB-028 card on the phone after {card_ms} ms");
    assert!(card_ms < 1_000, "card after {card_ms} ms");
    server.expect("PHASE u1 Started", "");

    // 2. Watched: 8–10 Hz.
    phone.cmd("WATCH on");
    let mut offset = 0;
    std::thread::sleep(Duration::from_millis(300));
    let from = system_ms();
    copy(&src.service, &["u1"], &mut offset, 3_000);
    let to = system_ms();
    let watched = rate(&phone.ticks("u1"), from + 200, to);
    eprintln!("LIB-028 watched rate on the phone: {watched:.2} Hz");
    assert!((8.0..=10.5).contains(&watched), "watched {watched:.2} Hz");

    // 3. Nobody watches (the subscription runs out): ≤ 1 Hz.
    phone.cmd("WATCH off");
    copy(&src.service, &["u1"], &mut offset, 3_500);
    let from = system_ms();
    copy(&src.service, &["u1"], &mut offset, 5_000);
    let to = system_ms();
    let idle = rate(&phone.ticks("u1"), from, to);
    eprintln!("LIB-028 idle rate on the phone: {idle:.2} Hz");
    assert!(idle > 0.5 && idle <= 1.05, "idle {idle:.2} Hz");

    // 4. Done: the card is available < 1 s after the copy ends.
    let t_done = system_ms();
    src.service.upload_report("u1", UploadState::Done, SIZE, SIZE);
    src.catalog
        .record_upload_event("u1", UploadPhase::Done, &info("Dune.Part.Two.2024.mkv"))
        .unwrap();
    src.service.flush();
    let live = phone.expect("STATE u1 Done", "")[3].parse::<u64>().unwrap();
    let stored = phone.expect("PHASE u1 Done", "")[3].parse::<u64>().unwrap();
    eprintln!(
        "LIB-028 done on the phone after {} ms (live), {} ms (stored)",
        live - t_done,
        stored - t_done
    );
    assert!(live - t_done < 1_000 && stored - t_done < 1_000);

    // 5. A cancel from the phone: the source cancels, the card goes away
    // on the phone and the server.
    src.catalog
        .record_upload_event("u2", UploadPhase::Started, &info("b.mkv"))
        .unwrap();
    src.service.flush();
    phone.expect("PHASE u2 Started", "");
    copy(&src.service, &["u2"], &mut offset, 300);
    phone.cmd(&format!("CANCEL {me} u2"));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left).expect("cancel reaches the source") {
            ServiceEvent::UploadCancel { ids, device_id } => {
                assert_eq!((ids.as_slice(), device_id.as_str()), (&["u2".to_owned()][..], pid.as_str()));
                break;
            }
            _ => continue,
        }
    }
    src.service.upload_report("u2", UploadState::Cancelled, offset, SIZE);
    src.catalog
        .record_upload_event("u2", UploadPhase::Cancelled, &info("b.mkv"))
        .unwrap();
    src.service.flush();
    for c in [&mut phone, &mut server] {
        c.expect("STATE u2 Cancelled", "");
        c.expect("PHASE u2 Cancelled", "");
    }

    // 6. The source drops off the network: offline on both, then it is
    // back and the bar moves on.
    src.catalog
        .record_upload_event("u3", UploadPhase::Started, &info("c.mkv"))
        .unwrap();
    src.service.flush();
    copy(&src.service, &["u3"], &mut offset, 500);
    phone.expect("STATE u3 Copying", "");
    let t_off = system_ms();
    src.service.stop();
    for c in [&mut phone, &mut server] {
        let at = c.expect("STATE u3 Offline", "")[3].parse::<u64>().unwrap();
        eprintln!("LIB-028 offline shown after {} ms", at - t_off);
        assert!(at - t_off <= STALE_MS + 1_000);
    }
    src.service.start().unwrap();
    wait("source back", || src.service.is_running());
    // A new node: without mDNS here, it is told about the others again.
    src.service
        .peer_up(&sid, DeviceClass::Server, saddr.parse().unwrap());
    src.service
        .peer_up(&pid, DeviceClass::Mobile, paddr.parse().unwrap());
    let mut back = 0;
    let deadline = Instant::now() + Duration::from_secs(20);
    while back == 0 {
        assert!(Instant::now() < deadline, "the bar moves on after the source is back");
        copy(&src.service, &["u3"], &mut offset, 500);
        phone.drain();
        back = phone
            .seen
            .iter()
            .filter(|l| l.contains("STATE u3 Copying"))
            .count();
    }

    // 7. Load: 4 uploads at once, the phone watching (the worst case).
    phone.cmd("WATCH on");
    let ids = ["l1", "l2", "l3", "l4"];
    copy(&src.service, &ids, &mut offset, 1_000);
    let (cpu0, bytes0, frames0, t0) = phone.load();
    copy(&src.service, &ids, &mut offset, 10_000);
    let (cpu1, bytes1, frames1, t1) = phone.load();
    #[allow(clippy::cast_precision_loss)]
    let (cpu, bps, fps) = {
        let secs = (t1 - t0) as f64 / 1000.0;
        (
            (cpu1 - cpu0) as f64 / (t1 - t0) as f64 * 100.0,
            (bytes1 - bytes0) as f64 / secs,
            (frames1 - frames0) as f64 / secs,
        )
    };
    eprintln!(
        "LIB-028 load, 4 uploads watched: phone CPU {cpu:.2} % of a core, progress {bps:.0} B/s in {fps:.1} frames/s"
    );
    assert!(cpu < 2.0, "CPU {cpu:.2} %");
    assert!(bps < 5_000.0, "{bps:.0} B/s");

    for mut c in [phone, server] {
        c.send("QUIT");
        let status = c.child.wait().unwrap();
        assert!(status.success(), "child failed: {status}");
    }
}

/// Recognition before copying is as good as the recognition after it:
/// for every name, the guess on the destination path is exactly what the
/// scan makes of the file once it is there.
#[test]
fn the_guess_before_copying_matches_the_scan_after_it() {
    use pa_scanner::CancelToken;
    let names = [
        "Movies/Dune.Part.Two.2024.2160p.WEB-DL.DDP5.1.Atmos.mkv",
        "Movies/The.Matrix.1999.1080p.BluRay.x264.mkv",
        "Movies/Inception (2010).mkv",
        "Movies/Брат 2 (2000) BDRip.avi",
        "Movies/Тіні забутих предків 1965.mkv",
        "Movies/Spirited Away 2001 [1080p].mp4",
        "Movies/Oppenheimer.2023.IMAX.2160p.mkv",
        "Movies/Amelie.2001.FRENCH.1080p.mkv",
        "Movies/Blade.Runner.2049.2017.mkv",
        "Movies/2001 A Space Odyssey 1968.mkv",
        "Movies/Мастер и Маргарита 2024 WEB-DL 1080p.mkv",
        "Movies/Parasite.2019.KOREAN.720p.mkv",
        "Movies/Alien (1979) Director's Cut.mkv",
        "Movies/Up.2009.1080p.mkv",
        "Movies/Avatar The Way of Water (2022).mkv",
        "TV Shows/Severance/Severance.S02E03.1080p.ATVP.WEB-DL.mkv",
        "TV Shows/Breaking Bad/Breaking.Bad.S05E14.Ozymandias.720p.mkv",
        "TV Shows/The Office/The Office US - 3x07 - Branch Wars.mkv",
        "TV Shows/Слуга народу/Слуга народу S01E05.mkv",
        "TV Shows/Fargo/Fargo.S01E01.1080p.mkv",
        "TV Shows/Shogun/Shogun.2024.S01E02.2160p.mkv",
        "TV Shows/Dark/Dark.S03E08.GERMAN.1080p.mkv",
        "TV Shows/Chernobyl/Chernobyl.S01E05.mkv",
        "TV Shows/One Piece/[SubsPlease] One Piece - 1085 (1080p) [ABCD1234].mkv",
        "TV Shows/Frieren/[Erai-raws] Sousou no Frieren - 05 [1080p].mkv",
    ];
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    for n in names {
        let p = root.join(n);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"0").unwrap();
    }
    let catalog = Catalog::open(dir.path().join("library.db")).unwrap();
    catalog
        .add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    catalog
        .scan(None, &CancelToken::new(), &mut |_| {})
        .unwrap();
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    // What the scan made of each file: (title, year, season, episode).
    let mut scanned = HashMap::new();
    for m in catalog.movies().unwrap() {
        scanned.insert(m.rel_path.clone(), (m.title, m.year, None, None));
    }
    for show in catalog.shows().unwrap() {
        for e in catalog.episodes(show.id).unwrap() {
            scanned.insert(
                e.rel_path.clone(),
                (show.title.clone(), show.year, e.season, e.episode),
            );
        }
    }
    let mut same = 0;
    for n in names {
        let guess = pa_catalog::guess_upload(n);
        let scan = scanned
            .get(n)
            .unwrap_or_else(|| panic!("the scan recognized {n}: {scanned:?}"));
        let ok = norm(&guess.title) == norm(&scan.0)
            && (guess.year == scan.1 || scan.1.is_none())
            && (guess.season == scan.2 || scan.2.is_none())
            && guess.episode == scan.3;
        let scanned = scan;
        if ok {
            same += 1;
        } else {
            eprintln!("{n}: guess {guess:?}, scan {scanned:?}");
        }
    }
    eprintln!("LIB-028 guess before copy = scan after copy: {same}/{}", names.len());
    assert_eq!(same, names.len());
}
