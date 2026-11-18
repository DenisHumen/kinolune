// SPDX-License-Identifier: MPL-2.0
//! Files of sources as byte sources for native players (PLAY-AND-003, ADR-0127).
//!
//! The app gives every player the same URL of the loopback proxy
//! (`pa-proxy`, ADR-0038). A player in the same process (Media3 on Android)
//! first calls [`stream_open`] with that URL: when the proxy runs in this
//! library, the file comes back as a [`CoreByteSource`] with positional
//! reads, without a socket and HTTP in between. [`StreamError::NotLocal`]
//! means "not ours": the player reads the URL over HTTP as before.

use std::sync::Arc;

use pa_vfs::{ByteSource, ErrorKind, VfsError};

/// Largest read [`CoreByteSource::read_at`] returns at once (4 MiB).
pub const STREAM_MAX_READ: u32 = 4 * 1024 * 1024;

/// Why a stream could not be opened or read. Never contains the token of
/// the URL.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum StreamError {
    /// The URL is not served by a proxy of this library: read it over HTTP.
    #[error("not a stream of this core")]
    NotLocal,
    /// No such file (or a folder).
    #[error("not found: {details}")]
    NotFound {
        /// Details for logs.
        details: String,
    },
    /// The source refused access or the credentials are wrong.
    #[error("access denied: {details}")]
    Denied {
        /// Details for logs.
        details: String,
    },
    /// The source is unreachable or failed.
    #[error("source unavailable: {details}")]
    Unavailable {
        /// Details for logs.
        details: String,
    },
    /// The source did not answer in time.
    #[error("source timed out: {details}")]
    TimedOut {
        /// Details for logs.
        details: String,
    },
    /// Anything else (a broken URL path, an unexpected end of file).
    #[error("stream failed: {details}")]
    Failed {
        /// Details for logs.
        details: String,
    },
}

impl From<VfsError> for StreamError {
    fn from(e: VfsError) -> Self {
        let details = e.message().to_owned();
        match e.kind() {
            ErrorKind::NotFound | ErrorKind::IsADirectory | ErrorKind::NotADirectory => {
                Self::NotFound { details }
            }
            ErrorKind::PermissionDenied | ErrorKind::AuthenticationFailed => {
                Self::Denied { details }
            }
            ErrorKind::Unavailable => Self::Unavailable { details },
            ErrorKind::TimedOut => Self::TimedOut { details },
            _ => Self::Failed { details },
        }
    }
}

/// An opened file: fixed size, positional reads from any thread.
#[derive(Debug, uniffi::Object)]
pub struct CoreByteSource {
    inner: Arc<dyn ByteSource>,
}

#[uniffi::export]
impl CoreByteSource {
    /// Size in bytes, fixed at open time.
    pub fn size(&self) -> u64 {
        self.inner.size()
    }

    /// Up to `max_len` bytes from `offset` (at most [`STREAM_MAX_READ`]).
    /// Fewer bytes is a short read, not an error; an empty result means
    /// `offset` is at or past the end. Blocks the calling thread.
    pub fn read_at(&self, offset: u64, max_len: u32) -> Result<Vec<u8>, StreamError> {
        let size = self.inner.size();
        if offset >= size || max_len == 0 {
            return Ok(Vec::new());
        }
        let want = u64::from(max_len.min(STREAM_MAX_READ)).min(size - offset);
        // `want` is at most STREAM_MAX_READ, so it fits in usize.
        let mut buf = vec![0; usize::try_from(want).unwrap_or(0)];
        let n = self.inner.read_at(offset, &mut buf)?;
        buf.truncate(n.min(buf.len()));
        if buf.is_empty() {
            return Err(StreamError::Failed {
                details: format!("unexpected end of file at offset {offset} of {size}"),
            });
        }
        Ok(buf)
    }
}

/// Opens the file behind a URL of the loopback proxy running in this
/// library (the `stream_uri` the app gives its players).
///
/// [`StreamError::NotLocal`] for every other URL, including a proxy of
/// another copy of the core or a stopped one: read the URL over HTTP then.
/// Other errors are what the proxy would have answered with `4xx`/`5xx`.
#[uniffi::export]
pub fn stream_open(url: String) -> Result<Arc<CoreByteSource>, StreamError> {
    match pa_proxy::open_local(&url) {
        None => Err(StreamError::NotLocal),
        Some(Err(e)) => Err(e.into()),
        Some(Ok(inner)) => Ok(Arc::new(CoreByteSource { inner })),
    }
}

#[cfg(feature = "testing")]
pub use testing::*;

/// Test servers for instrumented tests of native players (feature
/// `testing`, never in app builds): a proxy over a local folder or an SMB
/// share inside this library, so the player can go direct or, with another
/// port in front, through HTTP.
#[cfg(feature = "testing")]
mod testing {
    use std::sync::{Arc, Mutex};

    use pa_proxy::{Opener, Proxy, ProxyOptions};
    use pa_vfs::{ByteSource, LocalFs, Vfs, VfsPath};

    use super::StreamError;

    /// Proxies of the test process, alive until [`stream_test_stop_all`].
    static SERVERS: Mutex<Vec<Proxy>> = Mutex::new(Vec::new());

    /// Opens resources as paths in a [`Vfs`] and keeps the last few files
    /// open, like the library's opener (`pa-catalog`).
    struct VfsOpener<V> {
        vfs: V,
        recent: Mutex<Vec<(String, Arc<dyn ByteSource>)>>,
    }

    impl<V: Vfs> Opener for VfsOpener<V> {
        fn open(&self, resource: &str) -> pa_vfs::Result<Arc<dyn ByteSource>> {
            let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, s)) = recent.iter().find(|(r, _)| r == resource) {
                return Ok(Arc::clone(s));
            }
            let source: Arc<dyn ByteSource> = Arc::from(self.vfs.open(&VfsPath::parse(resource)?)?);
            recent.insert(0, (resource.to_owned(), Arc::clone(&source)));
            recent.truncate(4);
            Ok(source)
        }
    }

    fn serve<V: Vfs + 'static>(vfs: V) -> Result<String, StreamError> {
        let opener = VfsOpener {
            vfs,
            recent: Mutex::new(Vec::new()),
        };
        let proxy = Proxy::start(Arc::new(opener), ProxyOptions::default())
            .map_err(|e| StreamError::Failed { details: e.to_string() })?;
        let base = proxy.base_url();
        SERVERS.lock().unwrap_or_else(|e| e.into_inner()).push(proxy);
        Ok(base)
    }

    /// Serves a local folder; returns the base URL (append `/` and the
    /// percent-encoded path inside the folder).
    #[uniffi::export]
    pub fn stream_test_serve_dir(path: String) -> Result<String, StreamError> {
        serve(LocalFs::new(path)?)
    }

    /// Serves an SMB share (`server` is `host` or `host:port`); returns
    /// the base URL. libsmb2 must be loadable (`libsmb2.so` in the APK).
    #[uniffi::export]
    pub fn stream_test_serve_smb(
        server: String,
        share: String,
        user: String,
        password: String,
    ) -> Result<String, StreamError> {
        let config = pa_vfs_smb::SmbConfig::new(server, share)
            .with_credentials(pa_vfs_smb::Credentials::new(user, password));
        let fs = pa_vfs_smb::SmbFs::new(config)?;
        fs.check()?;
        serve(fs)
    }

    /// Stops every test proxy: their URLs are then neither local nor served.
    #[uniffi::export]
    pub fn stream_test_stop_all() {
        SERVERS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pa_proxy::{Opener, Proxy, ProxyOptions};

    #[derive(Debug)]
    struct Bytes(Vec<u8>);

    impl ByteSource for Bytes {
        fn size(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
            let start = usize::try_from(offset).unwrap().min(self.0.len());
            // Short reads of at most 1000 bytes, like a network backend.
            let n = buf.len().min(self.0.len() - start).min(1000);
            buf[..n].copy_from_slice(&self.0[start..start + n]);
            Ok(n)
        }
    }

    struct One;

    impl Opener for One {
        fn open(&self, resource: &str) -> pa_vfs::Result<Arc<dyn ByteSource>> {
            match resource {
                "movie.mkv" => Ok(Arc::new(Bytes((0..5000u32).map(|i| i as u8).collect()))),
                "denied.mkv" => Err(VfsError::new(ErrorKind::AuthenticationFailed, "bad password")),
                "down.mkv" => Err(VfsError::new(ErrorKind::Unavailable, "no route")),
                _ => Err(VfsError::new(ErrorKind::NotFound, resource)),
            }
        }
    }

    #[test]
    fn opens_urls_of_a_proxy_in_this_library() {
        let proxy = Proxy::start(Arc::new(One), ProxyOptions::default()).unwrap();
        let s = stream_open(proxy.url("movie.mkv")).unwrap();
        assert_eq!(s.size(), 5000);
        let part = s.read_at(10, 20).unwrap();
        assert_eq!(part, (10..30u32).map(|i| i as u8).collect::<Vec<_>>());
        // Short reads come back as they are.
        assert_eq!(s.read_at(0, 4000).unwrap().len(), 1000);
        // Clamped to the end, empty at and past it.
        assert_eq!(s.read_at(4990, 100).unwrap().len(), 10);
        assert!(s.read_at(5000, 100).unwrap().is_empty());
        assert!(s.read_at(9000, 100).unwrap().is_empty());
        assert!(s.read_at(0, 0).unwrap().is_empty());

        assert!(matches!(
            stream_open(proxy.url("none.mkv")),
            Err(StreamError::NotFound { .. })
        ));
        assert!(matches!(
            stream_open(proxy.url("denied.mkv")),
            Err(StreamError::Denied { .. })
        ));
        assert!(matches!(
            stream_open(proxy.url("down.mkv")),
            Err(StreamError::Unavailable { .. })
        ));
        assert!(matches!(
            stream_open(format!("{}/", proxy.base_url())),
            Err(StreamError::Failed { .. })
        ));

        let url = proxy.url("movie.mkv");
        drop(proxy);
        assert!(matches!(stream_open(url), Err(StreamError::NotLocal)));
        assert!(matches!(
            stream_open("file:///movie.mkv".into()),
            Err(StreamError::NotLocal)
        ));
    }

    #[cfg(feature = "testing")]
    #[test]
    fn test_servers_serve_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a b.mkv"), b"0123456789").unwrap();
        let base = stream_test_serve_dir(dir.path().to_string_lossy().into_owned()).unwrap();
        let s = stream_open(format!("{base}/a%20b.mkv")).unwrap();
        assert_eq!(s.read_at(2, 3).unwrap(), b"234");
        stream_test_stop_all();
        assert!(matches!(
            stream_open(format!("{base}/a%20b.mkv")),
            Err(StreamError::NotLocal)
        ));
    }
}
