// SPDX-License-Identifier: MPL-2.0
//! NET-018: SSDP search, description, paged Browse and streaming against a
//! small UPnP media server on loopback.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use pa_dlna::{DlnaFs, SearchOptions, discover, search};
use pa_vfs::{ByteSource, EntryKind, ErrorKind, Vfs, VfsPath};
use url::Url;

fn media(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 253) as u8).collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The DIDL of `id`, all children (the server pages them).
fn children(base: &str, id: &str) -> Option<Vec<String>> {
    let video = |id: &str, title: &str, n: usize, size: bool| {
        format!(
            "<item id=\"{id}\" parentID=\"x\" restricted=\"1\"><dc:title>{}</dc:title><upnp:class>object.item.videoItem.movie</upnp:class><res {}protocolInfo=\"http-get:*:video/mp4:DLNA.ORG_CI=0\">{base}/media/{n}.mp4</res></item>",
            escape(title),
            if size { format!("size=\"{}\" ", 1000 + n) } else { String::new() }
        )
    };
    let folder = |id: &str, title: &str| {
        format!(
            "<container id=\"{id}\" parentID=\"x\" restricted=\"1\"><dc:title>{}</dc:title><upnp:class>object.container.storageFolder</upnp:class></container>",
            escape(title)
        )
    };
    Some(match id {
        "0" => vec![folder("64", "Browse Folders"), folder("2", "Video")],
        "64" => vec![folder("64$0", "Movies"), folder("64$1", "Many")],
        "64$0" => vec![
            video("m1", "Film (2020)", 1, true),
            video("m2", "No/size", 2, false),
            video("m3", "Film (2020)", 3, true),
        ],
        "64$1" => (0..450).map(|i| video(&format!("x{i}"), &format!("Clip {i:03}"), 100 + i, true)).collect(),
        // The same files in another view: never listed.
        "2" => vec![video("v1", "Film (2020)", 1, true)],
        _ => return None,
    })
}

fn serve(base: String, conn: TcpStream) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut out = conn;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_owned();
        let path = parts.next().unwrap_or("").to_owned();
        let (mut len, mut range) = (0usize, None);
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).unwrap();
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let (k, v) = h.split_once(':').unwrap();
            match k.to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse().unwrap(),
                "range" => range = Some(v.trim().to_owned()),
                _ => {}
            }
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        let body = String::from_utf8(body).unwrap();
        let (status, ty, payload, extra) = if path == "/desc.xml" {
            (
                200,
                "text/xml",
                format!("<?xml version=\"1.0\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><device><deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType><friendlyName>Mock NAS</friendlyName><UDN>uuid:mock</UDN><serviceList><service><serviceType>urn:schemas-upnp-org:service:ContentDirectory:1</serviceType><controlURL>/ctl</controlURL></service></serviceList></device></root>").into_bytes(),
                String::new(),
            )
        } else if path == "/ctl" && method == "POST" {
            let tag = |t: &str| {
                let a = body.find(&format!("<{t}>")).unwrap() + t.len() + 2;
                let b = body[a..].find('<').unwrap() + a;
                body[a..b].replace("&amp;", "&")
            };
            let id = tag("ObjectID");
            let start: usize = tag("StartingIndex").parse().unwrap();
            let count: usize = tag("RequestedCount").parse().unwrap();
            match children(&base, &id) {
                Some(all) => {
                    let page: Vec<_> = all.iter().skip(start).take(count).cloned().collect();
                    let didl = format!("<DIDL-Lite xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\" xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\">{}</DIDL-Lite>", page.concat());
                    (
                        200,
                        "text/xml",
                        // TotalMatches 0, like minidlna at the root.
                        format!("<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:BrowseResponse xmlns:u=\"urn:schemas-upnp-org:service:ContentDirectory:1\"><Result>{}</Result><NumberReturned>{}</NumberReturned><TotalMatches>0</TotalMatches></u:BrowseResponse></s:Body></s:Envelope>", escape(&didl), page.len()).into_bytes(),
                        String::new(),
                    )
                }
                None => (
                    500,
                    "text/xml",
                    b"<s:Envelope xmlns:s=\"x\"><s:Body><s:Fault><detail><UPnPError><errorCode>701</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>".to_vec(),
                    String::new(),
                ),
            }
        } else if let Some(n) = path.strip_prefix("/media/").and_then(|p| p.strip_suffix(".mp4")) {
            let data = media(1000 + n.parse::<usize>().unwrap());
            match range {
                Some(r) => {
                    let (a, b) = r.strip_prefix("bytes=").unwrap().split_once('-').unwrap();
                    let a: usize = a.parse().unwrap();
                    let b: usize = b.parse::<usize>().map_or(data.len() - 1, |b| b.min(data.len() - 1));
                    (
                        206,
                        "video/mp4",
                        data[a..=b].to_vec(),
                        format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()),
                    )
                }
                None => (200, "video/mp4", data, String::new()),
            }
        } else {
            (404, "text/plain", Vec::new(), String::new())
        };
        let head = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: {ty}\r\nContent-Length: {}\r\n{extra}\r\n",
            payload.len()
        );
        out.write_all(head.as_bytes()).unwrap();
        if method != "HEAD" && out.write_all(&payload).is_err() {
            return;
        }
    }
}

fn start() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let b = base.clone();
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let b = b.clone();
            std::thread::spawn(move || serve(b, c));
        }
    });
    base
}

#[test]
fn browses_the_folder_view_and_streams() {
    let base = start();
    let fs = DlnaFs::new(Url::parse(&format!("{base}/desc.xml")).unwrap()).unwrap();
    assert_eq!(fs.server().unwrap().name, "Mock NAS");
    let root = fs.list(&VfsPath::root()).unwrap();
    let names: Vec<_> = root.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Movies", "Many"], "the «Browse Folders» view is the root");

    let movies = fs.list(&VfsPath::parse("/Movies").unwrap()).unwrap();
    let files: Vec<_> = movies
        .iter()
        .map(|e| (e.name.as_str(), e.metadata.kind, e.metadata.size))
        .collect();
    assert_eq!(
        files,
        [
            ("Film (2020).mp4", EntryKind::File, 1001),
            // No size in the listing: a HEAD gave it.
            ("No-size.mp4", EntryKind::File, 1002),
            ("Film (2020) (2).mp4", EntryKind::File, 1003),
        ]
    );
    let many = fs.list(&VfsPath::parse("/Many").unwrap()).unwrap();
    assert_eq!(many.len(), 450, "three pages");
    assert_eq!(many[449].name, "Clip 449.mp4");

    let path = VfsPath::parse("/Movies/Film (2020) (2).mp4").unwrap();
    assert_eq!(fs.stat(&path).unwrap().size, 1003);
    let f = fs.open(&path).unwrap();
    let mut buf = vec![0; 1003];
    f.read_exact_at(0, &mut buf).unwrap();
    assert_eq!(buf, media(1003));
    let mut part = [0u8; 3];
    f.read_exact_at(500, &mut part).unwrap();
    assert_eq!(&part[..], &media(1003)[500..503]);

    assert_eq!(
        fs.stat(&VfsPath::parse("/Movies/nope.mp4").unwrap()).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.open(&VfsPath::parse("/Movies").unwrap()).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(
        fs.list(&path).unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
}

#[test]
fn an_unreachable_server_is_unavailable() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let fs = DlnaFs::new(Url::parse(&format!("http://{addr}/desc.xml")).unwrap()).unwrap();
    assert_eq!(
        fs.list(&VfsPath::root()).unwrap_err().kind(),
        ErrorKind::Unavailable
    );
    assert!(DlnaFs::new(Url::parse("http://u:p@h/desc.xml").unwrap()).is_err());
}

#[test]
fn ssdp_search_finds_the_server() {
    let base = start();
    let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
    // A device that answers M-SEARCH on loopback.
    let device = Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap());
    let target: SocketAddr = device.local_addr().unwrap();
    let d = Arc::clone(&device);
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = d.recv_from(&mut buf) {
            let req = String::from_utf8_lossy(&buf[..n]);
            if req.starts_with("M-SEARCH") && req.contains("MediaServer:1") {
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nST: urn:schemas-upnp-org:device:MediaServer:1\r\nUSN: uuid:mock::urn:schemas-upnp-org:device:MediaServer:1\r\nLOCATION: http://127.0.0.1:{port}/desc.xml\r\n\r\n"
                );
                d.send_to(answer.as_bytes(), from).unwrap();
            }
        }
    });
    let opts = SearchOptions {
        wait: Duration::from_millis(800),
        interfaces: vec!["127.0.0.1".parse().unwrap()],
        target,
    };
    let answers = search(&opts).unwrap();
    assert_eq!(answers.len(), 1, "two M-SEARCH, one answer per location");
    let servers = discover(&opts).unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "Mock NAS");
    assert_eq!(servers[0].udn, "uuid:mock");
    assert_eq!(servers[0].location.as_str(), format!("{base}/desc.xml"));
}
