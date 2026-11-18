// SPDX-License-Identifier: MPL-2.0
//! NET-018: HTTP files, playlists and `.strm` links against a small HTTP
//! server on loopback (byte ranges, servers without ranges, redirects,
//! the link policy).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pa_vfs::{ByteSource, EntryKind, ErrorKind, Vfs, VfsPath};
use pa_vfs_http::{
    HttpClient, LinkPolicy, PlaylistFs, PlaylistLocation, Zone, open_strm, open_strm_with,
};
use url::Url;

#[derive(Clone)]
struct Res {
    ty: &'static str,
    body: Vec<u8>,
    ranges: bool,
    redirect: Option<String>,
}

#[derive(Clone)]
struct Server {
    base: String,
    files: Arc<Mutex<HashMap<String, Res>>>,
    gets: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let s = Self {
            base,
            files: Arc::default(),
            gets: Arc::default(),
        };
        let me = s.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let me = me.clone();
                std::thread::spawn(move || me.serve(conn));
            }
        });
        s
    }

    fn add(&self, path: &str, ty: &'static str, body: Vec<u8>, ranges: bool) {
        self.files.lock().unwrap().insert(
            path.into(),
            Res {
                ty,
                body,
                ranges,
                redirect: None,
            },
        );
    }

    fn redirect(&self, path: &str, to: &str) {
        self.files.lock().unwrap().insert(
            path.into(),
            Res {
                ty: "text/plain",
                body: Vec::new(),
                ranges: false,
                redirect: Some(to.into()),
            },
        );
    }

    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("{}{path}", self.base)).unwrap()
    }

    fn serve(&self, conn: TcpStream) {
        let mut reader = BufReader::new(conn.try_clone().unwrap());
        let mut out = conn;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_owned();
            let target = parts.next().unwrap_or("").to_owned();
            let path = target.split('?').next().unwrap_or("").to_owned();
            let mut range = None;
            let mut length = 0usize;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                let (k, v) = h.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "range" => range = Some(v.trim().to_owned()),
                    "content-length" => length = v.trim().parse().unwrap(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            if method == "GET" {
                self.gets.fetch_add(1, Ordering::SeqCst);
            }
            let res = self.files.lock().unwrap().get(&path).cloned();
            let Some(res) = res else {
                write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").unwrap();
                continue;
            };
            if let Some(to) = res.redirect {
                write!(
                    out,
                    "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n"
                )
                .unwrap();
                continue;
            }
            let total = res.body.len();
            let (status, start, end) = match range.filter(|_| res.ranges) {
                Some(r) => {
                    let r = r.strip_prefix("bytes=").unwrap();
                    let (a, b) = r.split_once('-').unwrap();
                    let a: usize = a.parse().unwrap();
                    let b: usize = if b.is_empty() {
                        total - 1
                    } else {
                        b.parse::<usize>().unwrap().min(total - 1)
                    };
                    (206, a, b + 1)
                }
                None => (200, 0, total),
            };
            let mut head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: {}\r\nContent-Length: {}\r\n",
                res.ty,
                end - start
            );
            if status == 206 {
                head.push_str(&format!(
                    "Content-Range: bytes {start}-{}/{total}\r\n",
                    end - 1
                ));
            }
            head.push_str("\r\n");
            out.write_all(head.as_bytes()).unwrap();
            if method != "HEAD" && out.write_all(&res.body[start..end]).is_err() {
                return;
            }
        }
    }
}

fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 % 251) as u8).collect()
}

#[test]
fn reads_a_file_with_ranges_and_without() {
    let s = Server::start();
    let body = data(3 * 1024 * 1024 + 17);
    s.add("/a.mkv", "video/x-matroska", body.clone(), true);
    s.add("/plain.mp4", "video/mp4", body.clone(), false);
    let client = Arc::new(HttpClient::new(LinkPolicy::typed()));
    for path in ["/a.mkv", "/plain.mp4"] {
        let url = s.url(path);
        let p = client.probe(&url).unwrap();
        assert_eq!(p.size, Some(body.len() as u64), "{path}");
        let f = client.open(url, p.size.unwrap()).unwrap();
        let mut all = Vec::new();
        let mut buf = vec![0; 100_000];
        let mut off = 0u64;
        loop {
            let n = f.read_at(off, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
            off += n as u64;
        }
        assert_eq!(all, body, "{path}");
        // A seek back and a read at the end.
        let mut b = [0u8; 10];
        f.read_exact_at(1000, &mut b).unwrap();
        assert_eq!(&b[..], &body[1000..1010]);
        let tail = body.len() as u64 - 3;
        let mut t = [0u8; 10];
        assert_eq!(f.read_at(tail, &mut t).unwrap(), 3);
        assert_eq!(f.read_at(body.len() as u64, &mut t).unwrap(), 0);
        assert!(!format!("{f:?}").contains(path), "Debug hides the link");
    }
}

#[test]
fn sequential_reads_reuse_one_request() {
    let s = Server::start();
    s.add("/a.mkv", "video/x-matroska", data(1_000_000), true);
    let client = Arc::new(HttpClient::new(LinkPolicy::typed()));
    let f = client.open(s.url("/a.mkv"), 1_000_000).unwrap();
    let before = s.gets.load(Ordering::SeqCst);
    let mut buf = vec![0; 4096];
    let mut off = 0;
    while off < 1_000_000 {
        off += f.read_at(off, &mut buf).unwrap() as u64;
    }
    assert_eq!(s.gets.load(Ordering::SeqCst) - before, 1);
}

#[test]
fn the_policy_blocks_links_into_less_public_zones() {
    let s = Server::start();
    s.add("/a.mkv", "video/x-matroska", data(10), true);
    // A playlist from the Internet may not reach this device.
    let public = HttpClient::new(LinkPolicy::from(Zone::Public));
    let e = public.probe(&s.url("/a.mkv")).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::PermissionDenied, "{e}");
    // Nor a file on a share, nor through a redirect.
    s.redirect("/jump", &format!("{}/a.mkv", s.base));
    let lan = HttpClient::new(LinkPolicy::local_file());
    assert_eq!(
        lan.probe(&s.url("/jump")).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    // The cloud metadata address is never reached, even when typed.
    let typed = HttpClient::new(LinkPolicy::typed());
    let meta = Url::parse("http://169.254.169.254/latest/meta-data/").unwrap();
    assert_eq!(
        typed.probe(&meta).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    // Typed addresses on this device are fine; redirects are followed.
    assert_eq!(typed.probe(&s.url("/jump")).unwrap().size, Some(10));
}

#[test]
fn a_playlist_is_a_folder_of_files() {
    let s = Server::start();
    s.add("/m/One.mkv", "video/x-matroska", data(2000), true);
    s.add("/m/two", "video/mp4", data(3000), true);
    s.add("/m/song.mp3", "audio/mpeg", data(500), true);
    s.add("/m/page", "text/html", b"<html>".to_vec(), true);
    s.add(
        "/list.m3u8",
        "audio/x-mpegurl",
        format!(
            "#EXTM3U\n\
             #EXTINF:100,First Movie (2001)\nm/One.mkv\n\
             #EXTINF:100,First Movie (2001)\n{base}/m/two\n\
             #EXTINF:-1 group-title=\"TV\",Live\n{base}/live/1.m3u8\n\
             #EXTINF:200 group-title=\"Music\",A/B Song\nm/song.mp3\n\
             #EXTINF:10,Web page\nm/page\n\
             #EXTINF:10,Gone\nm/missing.mkv\n",
            base = s.base
        )
        .into_bytes(),
        false,
    );
    let fs = PlaylistFs::new(PlaylistLocation::Url(s.url("/list.m3u8"))).unwrap();
    let mut root: Vec<_> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.metadata.kind, e.metadata.size))
        .collect();
    root.sort();
    assert_eq!(
        root,
        vec![
            ("First Movie (2001).mkv".to_owned(), EntryKind::File, 2000),
            ("First Movie (2001).mp4".to_owned(), EntryKind::File, 3000),
            ("Music".to_owned(), EntryKind::Dir, 0),
        ]
    );
    let music = fs.list(&VfsPath::parse("/Music").unwrap()).unwrap();
    assert_eq!(music.len(), 1);
    assert_eq!(music[0].name, "A-B Song.mp3");
    assert_eq!(music[0].path.as_str(), "/Music/A-B Song.mp3");

    let f = fs
        .open(&VfsPath::parse("/First Movie (2001).mkv").unwrap())
        .unwrap();
    let mut b = vec![0; 2000];
    f.read_exact_at(0, &mut b).unwrap();
    assert_eq!(b, data(2000));
    assert_eq!(
        fs.stat(&VfsPath::parse("/Music").unwrap()).unwrap().kind,
        EntryKind::Dir
    );
    assert_eq!(
        fs.stat(&VfsPath::parse("/nope.mkv").unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.list(&VfsPath::parse("/First Movie (2001).mkv").unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fs.open(&VfsPath::parse("/Music").unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::IsADirectory
    );
}

#[test]
fn a_playlist_file_cannot_point_at_this_device() {
    let s = Server::start();
    s.add("/a.mkv", "video/x-matroska", data(100), true);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("list.m3u");
    std::fs::write(&path, format!("{}/a.mkv\n", s.base)).unwrap();
    let fs = PlaylistFs::new(PlaylistLocation::File(path)).unwrap();
    // The link is refused, so the playlist has no files.
    assert!(fs.list(&VfsPath::root()).unwrap().is_empty());
}

#[test]
fn hls_streams_are_not_playlists() {
    let s = Server::start();
    s.add(
        "/live.m3u8",
        "application/vnd.apple.mpegurl",
        b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\nseg.ts\n".to_vec(),
        false,
    );
    let fs = PlaylistFs::new(PlaylistLocation::Url(s.url("/live.m3u8"))).unwrap();
    assert_eq!(
        fs.list(&VfsPath::root()).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[derive(Debug)]
struct Text(Vec<u8>);

impl ByteSource for Text {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        let o = usize::try_from(offset).unwrap().min(self.0.len());
        let n = buf.len().min(self.0.len() - o);
        buf[..n].copy_from_slice(&self.0[o..o + n]);
        Ok(n)
    }
}

#[test]
fn strm_files_open_their_link() {
    let s = Server::start();
    s.add("/m.mkv", "video/x-matroska", data(5000), true);
    let strm = Text(format!("{}/m.mkv\n", s.base).into_bytes());
    // A .strm on a share may not reach this device…
    assert_eq!(
        open_strm(&strm).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    // …a server on it is reached under a wider policy.
    let f = open_strm_with(&strm, LinkPolicy::typed()).unwrap();
    assert_eq!(f.size(), 5000);
    let mut b = vec![0; 5000];
    f.read_exact_at(0, &mut b).unwrap();
    assert_eq!(b, data(5000));
    assert_eq!(
        open_strm(&Text(b"plugin://x".to_vec())).unwrap_err().kind(),
        ErrorKind::InvalidPath
    );
}
