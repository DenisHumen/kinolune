// SPDX-License-Identifier: MPL-2.0
//! NET-018 done-when: an M3U playlist of HTTP links is a library source
//! whose files play (stream through the loopback proxy byte for byte);
//! a `.strm` file in a folder plays the link it holds, and a `.strm` link
//! to this device is refused.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};

use pa_catalog::{Catalog, SourceOpener, SourcesConnector};
use pa_proxy::{Proxy, ProxyOptions};
use pa_scanner::CancelToken;
use pa_secrets::MemoryStore;
use pa_sources::{NewSource, SourceKind, SourceManager};
use pa_vfs::VfsPath;

type Files = Arc<Mutex<HashMap<String, (&'static str, Vec<u8>)>>>;

/// A small HTTP server with byte ranges on `ip`.
fn serve(ip: IpAddr, files: Files) -> String {
    let l = TcpListener::bind((ip, 0)).unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let files = Arc::clone(&files);
            std::thread::spawn(move || {
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut w = conn;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut it = line.split_whitespace();
                    let method = it.next().unwrap_or("").to_owned();
                    let path = it.next().unwrap_or("").to_owned();
                    let mut range = None;
                    loop {
                        let mut h = String::new();
                        r.read_line(&mut h).unwrap();
                        let h = h.trim_end();
                        if h.is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range:") {
                            range = Some(v.trim().to_owned());
                        }
                    }
                    let Some((ty, body)) = files.lock().unwrap().get(&path).cloned() else {
                        write!(w, "HTTP/1.1 404 No\r\nContent-Length: 0\r\n\r\n").unwrap();
                        continue;
                    };
                    let (status, a, b) = match range {
                        Some(r) => {
                            let (a, b) = r.strip_prefix("bytes=").unwrap().split_once('-').unwrap();
                            let a: usize = a.parse().unwrap();
                            let b = b.parse::<usize>().map_or(body.len() - 1, |b| b.min(body.len() - 1));
                            (206, a, b + 1)
                        }
                        None => (200, 0, body.len()),
                    };
                    let mut head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: {ty}\r\nContent-Length: {}\r\n",
                        b - a
                    );
                    if status == 206 {
                        head.push_str(&format!("Content-Range: bytes {a}-{}/{}\r\n", b - 1, body.len()));
                    }
                    head.push_str("\r\n");
                    w.write_all(head.as_bytes()).unwrap();
                    if method != "HEAD" && w.write_all(&body[a..b]).is_err() {
                        return;
                    }
                }
            });
        }
    });
    base
}

fn data(n: usize, seed: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 31 + seed) % 251) as u8).collect()
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

struct Setup {
    _dir: tempfile::TempDir,
    db: std::path::PathBuf,
    cat: Arc<Catalog>,
    manager: Arc<SourceManager>,
    proxy: Proxy,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    cat.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));
    let proxy = Proxy::start(
        Arc::new(SourceOpener::new(Arc::clone(&cat))),
        ProxyOptions::default(),
    )
    .unwrap();
    Setup {
        _dir: dir,
        db,
        cat,
        manager,
        proxy,
    }
}

fn scanned(db: &std::path::Path, source: i64) -> Vec<(String, i64)> {
    let conn = pa_library::rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare("SELECT path, size_bytes FROM files WHERE source_id = ?1 ORDER BY path")
        .unwrap();
    stmt.query_map([source], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn an_m3u_playlist_of_http_links_plays() {
    let files: Files = Arc::default();
    let base = serve("127.0.0.1".parse().unwrap(), Arc::clone(&files));
    let movie = data(700_001, 1);
    let episode = data(300_000, 2);
    {
        let mut f = files.lock().unwrap();
        f.insert("/v/movie.mkv".into(), ("video/x-matroska", movie.clone()));
        f.insert("/v/e1".into(), ("video/mp4", episode.clone()));
        f.insert(
            "/list.m3u8".into(),
            (
                "audio/x-mpegurl",
                format!(
                    "#EXTM3U\n#EXTINF:6000,The Test Movie (2019)\nv/movie.mkv\n\
                     #EXTINF:1500 group-title=\"Test Show\",Test Show S01E01\n{base}/v/e1\n\
                     #EXTINF:-1,Live\n{base}/live/index.m3u8\n"
                )
                .into_bytes(),
            ),
        );
    }
    let s = setup();
    let source = s
        .manager
        .create(NewSource::new(
            SourceKind::Playlist,
            "Links",
            format!("{base}/list.m3u8"),
        ))
        .unwrap();
    assert_eq!(source.kind, SourceKind::Playlist);
    assert_eq!(s.manager.check(source.id).unwrap().online, Some(true));
    let mut errors = Vec::new();
    s.cat
        .scan(Some(source.id), &CancelToken::new(), &mut |u| {
            errors.extend(u.error)
        })
        .unwrap();
    assert_eq!(errors, Vec::<String>::new());
    let rows = scanned(&s.db, source.id);
    let paths: Vec<_> = rows.iter().map(|(p, _)| p.trim_start_matches('/')).collect();
    assert_eq!(
        paths,
        ["Test Show/Test Show S01E01.mp4", "The Test Movie (2019).mkv"],
        "{rows:?}"
    );

    for (rel, want) in [
        ("The Test Movie (2019).mkv", &movie),
        ("Test Show/Test Show S01E01.mp4", &episode),
    ] {
        let url = s.proxy.url(&SourceOpener::resource(
            source.id,
            &VfsPath::parse(rel).unwrap(),
        ));
        let (status, all) = get(&s.proxy, &url, "bytes=0-");
        assert!(status == 200 || status == 206, "{rel}: {status}");
        assert_eq!(&all, want, "{rel}");
        let (status, part) = get(&s.proxy, &url, "bytes=1000-20999");
        assert_eq!(status, 206);
        assert_eq!(part, want[1000..21_000]);
    }
}

/// A private address of this machine (a LAN zone), if it has one.
fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    pa_vfs_http::Zone::of(ip)
        .filter(|z| *z == pa_vfs_http::Zone::Lan)
        .map(|_| ip)
}

#[test]
fn strm_files_play_their_link_but_never_this_device() {
    let s = setup();
    let folder = tempfile::tempdir().unwrap();
    let local_files: Files = Arc::default();
    let local = serve("127.0.0.1".parse().unwrap(), Arc::clone(&local_files));
    local_files
        .lock()
        .unwrap()
        .insert("/m.mkv".into(), ("video/x-matroska", data(1000, 3)));
    std::fs::write(
        folder.path().join("Local Link (2001).strm"),
        format!("{local}/m.mkv\n"),
    )
    .unwrap();
    let lan = lan_ip();
    let remote = data(400_000, 4);
    if let Some(ip) = lan {
        let lan_files: Files = Arc::default();
        let base = serve(ip, Arc::clone(&lan_files));
        lan_files
            .lock()
            .unwrap()
            .insert("/f/film.mkv".into(), ("video/x-matroska", remote.clone()));
        std::fs::write(
            folder.path().join("Film (2020).strm"),
            format!("# Kodi\n{base}/f/film.mkv\n"),
        )
        .unwrap();
    }
    let source = s
        .manager
        .create(NewSource::new(
            SourceKind::Local,
            "Folder",
            folder.path().to_string_lossy(),
        ))
        .unwrap();
    s.cat
        .scan(Some(source.id), &CancelToken::new(), &mut |_| {})
        .unwrap();
    let paths: Vec<_> = scanned(&s.db, source.id)
        .into_iter()
        .map(|(p, _)| p.trim_start_matches('/').to_owned())
        .collect();
    assert!(paths.contains(&"Local Link (2001).strm".to_owned()), "{paths:?}");

    let url = |rel: &str| {
        s.proxy.url(&SourceOpener::resource(
            source.id,
            &VfsPath::parse(rel).unwrap(),
        ))
    };
    let (status, _) = get(&s.proxy, &url("Local Link (2001).strm"), "bytes=0-");
    assert!(
        status >= 400,
        "a .strm link to this device is refused, got {status}"
    );
    match lan {
        Some(_) => {
            let (status, body) = get(&s.proxy, &url("Film (2020).strm"), "bytes=0-");
            assert!(status == 200 || status == 206, "{status}");
            assert_eq!(body, remote);
            let (_, part) = get(&s.proxy, &url("Film (2020).strm"), "bytes=5-9");
            assert_eq!(part, remote[5..10]);
        }
        None => eprintln!("no LAN address here: the .strm play check is skipped"),
    }
}
