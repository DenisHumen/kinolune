// SPDX-License-Identifier: MPL-2.0
//! BUG-004: a server found on the LAN is saved by IP with its stable name;
//! when DHCP gives it another IP, the source finds it again by that name
//! and saves the new address, without the user. Sources saved by IP before
//! BUG-004 learn their name while the IP still answers.
//!
//! The "stand" is a tiny WebDAV server that moves from 127.0.0.2 to
//! 127.0.0.3 (Linux routes all of 127/8 to loopback); a fake resolver
//! plays the network's mDNS / NetBIOS answers.
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_secrets::MemoryStore;
use pa_sources::{
    LanResolver, NewSource, Source, SourceKind, SourceManager, SourceOptions, SourceUpdate,
};
use pa_vfs::{ErrorKind, VfsPath};

/// A WebDAV share with one file, `film.mkv`, under `/dav/`.
struct Stand {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl Stand {
    fn start(ip: &str, port: u16) -> Self {
        let listener = TcpListener::bind((ip, port)).expect("bind the stand");
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(mut s) = conn {
                    serve(&mut s);
                }
            }
        });
        Self { addr, stop }
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }
}

impl Drop for Stand {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop; the listener closes with the thread.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn serve(s: &mut TcpStream) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let mut depth0 = false;
    let mut length = 0u64;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).is_err() {
            return;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "depth" {
                depth0 = v == "0";
            } else if k == "content-length" {
                length = v.parse().unwrap_or(0);
            }
        }
    }
    let _ = std::io::copy(&mut r.take(length), &mut std::io::sink());
    let entry = |href: &str, file: bool| {
        let kind = if file {
            "<D:resourcetype/><D:getcontentlength>4</D:getcontentlength>"
        } else {
            "<D:resourcetype><D:collection/></D:resourcetype>"
        };
        format!(
            "<D:response><D:href>{href}</D:href><D:propstat><D:prop>{kind}\
             <D:getlastmodified>Thu, 19 Nov 2026 10:00:00 GMT</D:getlastmodified>\
             </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
        )
    };
    let mut xml =
        String::from(r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">"#);
    xml.push_str(&entry("/dav/", false));
    if !depth0 {
        xml.push_str(&entry("/dav/film.mkv", true));
    }
    xml.push_str("</D:multistatus>");
    let head = format!(
        "HTTP/1.1 207 Multi-Status\r\nConnection: close\r\n\
         Content-Type: application/xml; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
        xml.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(xml.as_bytes());
    let _ = s.flush();
}

/// The network as the test sets it: names → current IPs, IPs → names.
#[derive(Default)]
struct FakeLan {
    hosts: Mutex<HashMap<String, IpAddr>>,
    names: Mutex<HashMap<IpAddr, String>>,
    lookups: AtomicUsize,
}

impl FakeLan {
    fn set_host(&self, name: &str, ip: &str) {
        self.hosts
            .lock()
            .unwrap()
            .insert(name.to_owned(), ip.parse().unwrap());
    }
}

impl LanResolver for FakeLan {
    fn host(&self, name: &str) -> Option<IpAddr> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.hosts.lock().unwrap().get(name).copied()
    }

    fn media_server(&self, _: SourceKind, _: &str) -> Option<String> {
        None
    }

    fn dlna(&self, _: &str) -> Option<String> {
        None
    }

    fn name_of(&self, _: &Source, ip: IpAddr) -> Option<String> {
        self.names.lock().unwrap().get(&ip).cloned()
    }
}

fn manager(lan: &Arc<FakeLan>) -> SourceManager {
    let m = SourceManager::new(
        pa_library::open_in_memory().unwrap(),
        Arc::new(MemoryStore::new()),
    );
    m.set_lan_resolver(Arc::clone(lan) as Arc<dyn LanResolver>);
    m
}

fn names(m: &SourceManager, id: i64) -> Vec<String> {
    m.connect(id)
        .expect("connect")
        .list(&VfsPath::root())
        .expect("list")
        .into_iter()
        .map(|e| e.name)
        .collect()
}

fn found_on_lan(uri: &str, lan_name: Option<&str>) -> NewSource {
    NewSource::new(SourceKind::WebDav, "NAS", uri).with_options(SourceOptions {
        lan_name: lan_name.map(str::to_owned),
        timeout_secs: Some(2),
        ..SourceOptions::default()
    })
}

#[test]
fn moved_server_is_found_by_its_name() {
    let lan = Arc::new(FakeLan::default());
    let m = manager(&lan);
    let stand = Stand::start("127.0.0.2", 0);
    let port = stand.port();
    lan.set_host("nas.local", "127.0.0.2");
    let s = m
        .create(found_on_lan(
            &format!("http://127.0.0.2:{port}/dav/"),
            Some("nas.local"),
        ))
        .unwrap();
    assert_eq!(s.options.lan_name.as_deref(), Some("nas.local"));
    assert_eq!(names(&m, s.id), ["film.mkv"]);
    // The IP answers: no search of the network.
    assert_eq!(lan.lookups.load(Ordering::SeqCst), 0);

    // DHCP gives the stand another address.
    drop(stand);
    let _stand = Stand::start("127.0.0.3", port);
    lan.set_host("nas.local", "127.0.0.3");

    assert_eq!(names(&m, s.id), ["film.mkv"]);
    let moved = m.get(s.id).unwrap();
    assert_eq!(moved.uri, format!("http://127.0.0.3:{port}/dav/"));
    assert_eq!(moved.options.lan_name.as_deref(), Some("nas.local"));
    assert_eq!(lan.lookups.load(Ordering::SeqCst), 1);
    // From now on the new IP is used directly.
    assert_eq!(m.check(s.id).unwrap().online, Some(true));
    assert_eq!(lan.lookups.load(Ordering::SeqCst), 1);
}

#[test]
fn server_not_found_keeps_the_address_and_the_error() {
    let lan = Arc::new(FakeLan::default());
    let m = manager(&lan);
    let stand = Stand::start("127.0.0.2", 0);
    let uri = format!("http://127.0.0.2:{}/dav/", stand.port());
    drop(stand);
    let s = m.create(found_on_lan(&uri, Some("nas.local"))).unwrap();
    let err = m.connect(s.id).map(|_| ()).unwrap_err();
    assert_eq!(err.source_kind(), Some(ErrorKind::Unavailable));
    assert_eq!(lan.lookups.load(Ordering::SeqCst), 1);
    assert_eq!(m.get(s.id).unwrap().uri, uri);
    let status = m.check(s.id).unwrap();
    assert_eq!(status.online, Some(false));
}

#[test]
fn source_saved_by_ip_learns_its_name_and_survives_a_move() {
    let lan = Arc::new(FakeLan::default());
    let m = manager(&lan);
    let stand = Stand::start("127.0.0.2", 0);
    let port = stand.port();
    // Saved before BUG-004: an IP and no name.
    let s = m
        .create(found_on_lan(&format!("http://127.0.0.2:{port}/dav/"), None))
        .unwrap();
    assert_eq!(s.options.lan_name, None);
    assert_eq!(names(&m, s.id), ["film.mkv"]);
    lan.names
        .lock()
        .unwrap()
        .insert("127.0.0.2".parse().unwrap(), "NAS".into());

    // A check that reaches it asks the network for the name, in the
    // background.
    assert_eq!(m.check(s.id).unwrap().online, Some(true));
    let deadline = Instant::now() + Duration::from_secs(5);
    while m.get(s.id).unwrap().options.lan_name.is_none() {
        assert!(Instant::now() < deadline, "the name was not learned");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(m.get(s.id).unwrap().options.lan_name.as_deref(), Some("NAS"));

    drop(stand);
    let _stand = Stand::start("127.0.0.3", port);
    lan.set_host("NAS", "127.0.0.3");
    assert_eq!(names(&m, s.id), ["film.mkv"]);
    assert_eq!(
        m.get(s.id).unwrap().uri,
        format!("http://127.0.0.3:{port}/dav/")
    );
}

#[test]
fn another_server_drops_the_name_another_folder_keeps_it() {
    let lan = Arc::new(FakeLan::default());
    let m = manager(&lan);
    let s = m
        .create(found_on_lan("http://192.0.2.10/dav/", Some("nas.local")))
        .unwrap();
    let folder = m
        .update(
            s.id,
            SourceUpdate {
                address: Some("http://192.0.2.10/dav/Films/".into()),
                ..SourceUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(folder.options.lan_name.as_deref(), Some("nas.local"));
    let other = m
        .update(
            s.id,
            SourceUpdate {
                address: Some("http://192.0.2.99/dav/".into()),
                ..SourceUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(other.options.lan_name, None);
}
