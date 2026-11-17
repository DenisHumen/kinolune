// SPDX-License-Identifier: MPL-2.0
//! Writes over WebDAV (NET-023): `MKCOL`, `PUT`, `MOVE`, `COPY`, `DELETE`.
//!
//! Uploads: a server that appends with a ranged `PUT` (`Content-Range`,
//! Apache `mod_dav`, probed once per source) gets the file in 8 MiB pieces,
//! so a broken upload loses at most one piece and resumes. Other servers
//! (nginx, `rclone serve`, Nextcloud) get one streamed `PUT`, and a broken
//! upload starts over: `write_file` with an offset answers `Unsupported`.

use std::fmt;
use std::io::{self, Read};
use std::sync::mpsc::{Receiver, sync_channel};

use pa_vfs::{ErrorKind, Result, Vfs, VfsError, VfsPath, VfsWrite};
use reqwest::blocking::Body;
use reqwest::{Method, StatusCode};

use crate::client::{Req, WebDavFs, status_error};

/// Size of one ranged `PUT`.
const CHUNK: usize = 8 * 1024 * 1024;
/// Pieces read ahead of a streamed `PUT`.
const STREAM_PIECE: usize = 256 * 1024;
/// File the partial-`PUT` probe writes and removes.
const PROBE: &str = ".kinolune-range-probe";

fn method(name: &str) -> Method {
    Method::from_bytes(name.as_bytes()).expect("valid method")
}

fn ok(status: StatusCode) -> bool {
    status.is_success()
}

impl WebDavFs {
    fn check_target(path: &VfsPath) -> Result<()> {
        if path.is_root() {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                "the source root cannot be written",
            ));
        }
        Ok(())
    }

    /// One request without a body; returns the status.
    fn simple(&self, req: &Req<'_>, what: &dyn fmt::Display) -> Result<StatusCode> {
        Ok(self.inner.send(req, what)?.status())
    }

    /// `PUT` of `bytes`, at `range` (`start`, total) when appending.
    fn put_bytes(&self, path: &VfsPath, bytes: &[u8], range: Option<(u64, u64)>) -> Result<()> {
        let mut req = Req::new(Method::PUT, self.inner.base.url_of(path, false));
        req.bytes = Some(bytes);
        if let Some((start, total)) = range {
            let end = start + bytes.len() as u64 - 1;
            req.headers
                .push(("Content-Range", format!("bytes {start}-{end}/{total}")));
        }
        let status = self.simple(&req, path)?;
        if ok(status) {
            Ok(())
        } else {
            Err(put_error(status, path))
        }
    }

    /// Whether the server appends with a ranged `PUT`: writes `ab`, then
    /// `c` at offset 2, and reads back `abc` (a server that ignores the
    /// range leaves `c`, one that refuses it answers an error).
    fn partial_put(&self, dir: &VfsPath) -> bool {
        *self.inner.partial_put.get_or_init(|| {
            let Ok(probe) = dir.join(&format!("{PROBE}-{}", std::process::id())) else {
                return false;
            };
            let works = self.put_bytes(&probe, b"ab", None).is_ok()
                && self.put_bytes(&probe, b"c", Some((2, 3))).is_ok()
                && self.open(&probe).is_ok_and(|f| {
                    let mut buf = [0u8; 3];
                    f.size() == 3 && f.read_exact_at(0, &mut buf).is_ok() && &buf == b"abc"
                });
            let _ = self.remove_file(&probe);
            works
        })
    }

    /// Ranged `PUT`s of `CHUNK` bytes each; the first one (at 0) creates
    /// or truncates the file.
    fn put_chunks(&self, path: &VfsPath, offset: u64, data: &mut dyn Read, len: u64) -> Result<u64> {
        let total = offset + len;
        let mut buf = vec![0u8; usize::try_from(len).unwrap_or(CHUNK).clamp(1, CHUNK)];
        let mut done = 0u64;
        loop {
            let want = usize::try_from(len - done).unwrap_or(usize::MAX).min(buf.len());
            let n = fill(data, &mut buf[..want])?;
            if n == 0 && !(done == 0 && offset == 0) {
                break;
            }
            let at = offset + done;
            let range = (at > 0).then_some((at, total));
            self.put_bytes(path, &buf[..n], range)?;
            done += n as u64;
            if n == 0 || done >= len {
                break;
            }
        }
        Ok(done)
    }

    /// One streamed `PUT` of `len` bytes (servers without ranged `PUT`).
    fn put_stream(&self, path: &VfsPath, data: &mut dyn Read, len: u64) -> Result<u64> {
        // A streamed body cannot be sent twice, so log in first: the PUT
        // then carries the credentials and gets no challenge.
        if let Some(parent) = path.parent() {
            let _ = self.stat(&parent);
        }
        let (tx, rx) = sync_channel::<io::Result<Vec<u8>>>(4);
        let mut req = Req::new(Method::PUT, self.inner.base.url_of(path, false));
        // The whole upload is one request: allow 1 s per MiB on top.
        req.timeout = Some(self.inner.timeout + std::time::Duration::from_secs(len >> 20));
        req.stream = Some(std::cell::Cell::new(Some(Body::sized(
            ChannelReader {
                rx,
                piece: Vec::new(),
                at: 0,
            },
            len,
        ))));
        std::thread::scope(|s| {
            let sender = s.spawn(move || self.simple(&req, path));
            let mut done = 0u64;
            let mut data_error = None;
            while done < len {
                let want = usize::try_from(len - done).unwrap_or(usize::MAX).min(STREAM_PIECE);
                let mut piece = vec![0u8; want];
                match fill(data, &mut piece) {
                    Ok(0) => break,
                    Ok(n) => {
                        piece.truncate(n);
                        done += n as u64;
                        if tx.send(Ok(piece)).is_err() {
                            break; // the request ended (an error reply)
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(io::Error::other("upload data failed")));
                        data_error = Some(e);
                        break;
                    }
                }
            }
            drop(tx);
            let status = sender
                .join()
                .unwrap_or_else(|_| Err(VfsError::new(ErrorKind::Other, "upload thread panicked")));
            if let Some(e) = data_error {
                return Err(e);
            }
            let status = status?;
            if ok(status) {
                Ok(done)
            } else {
                Err(put_error(status, path))
            }
        })
    }
}

/// `MKCOL`/`PUT` statuses that mean "not allowed here".
fn put_error(status: StatusCode, path: &VfsPath) -> VfsError {
    match status.as_u16() {
        405 => VfsError::new(ErrorKind::PermissionDenied, format!("{path}: HTTP {status}")),
        _ => status_error(status, path),
    }
}

impl VfsWrite for WebDavFs {
    fn create_dir(&self, path: &VfsPath) -> Result<()> {
        Self::check_target(path)?;
        let req = Req::new(method("MKCOL"), self.inner.base.url_of(path, true));
        let status = self.simple(&req, path)?;
        if ok(status) {
            return Ok(());
        }
        // 405: "the name is taken" by RFC 4918, "not allowed" in practice.
        if status == StatusCode::METHOD_NOT_ALLOWED && self.stat(path).is_ok() {
            return Err(VfsError::new(ErrorKind::AlreadyExists, path.to_string()));
        }
        Err(put_error(status, path))
    }

    fn write_file(&self, path: &VfsPath, offset: u64, data: &mut dyn Read, len: u64) -> Result<u64> {
        Self::check_target(path)?;
        let dir = path.parent().unwrap_or_default();
        if self.partial_put(&dir) {
            return self.put_chunks(path, offset, data, len);
        }
        if offset > 0 {
            return Err(VfsError::new(
                ErrorKind::Unsupported,
                format!("{path}: the server cannot append (no ranged PUT)"),
            ));
        }
        self.put_stream(path, data, len)
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        self.transfer("MOVE", from, to)
    }

    fn server_copy(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        self.transfer("COPY", from, to)
    }

    fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.delete(path, false)
    }

    fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.delete(path, true)
    }
}

impl WebDavFs {
    /// `MOVE` or `COPY` without overwriting.
    fn transfer(&self, verb: &str, from: &VfsPath, to: &VfsPath) -> Result<()> {
        Self::check_target(from)?;
        Self::check_target(to)?;
        let dir = self.stat(from)?.is_dir();
        let mut req = Req::new(method(verb), self.inner.base.url_of(from, dir));
        req.headers.push((
            "Destination",
            self.inner.base.url_of(to, dir).as_str().to_owned(),
        ));
        req.headers.push(("Overwrite", "F".to_owned()));
        if dir {
            req.headers.push(("Depth", "infinity".to_owned()));
        }
        let status = self.simple(&req, from)?;
        match status.as_u16() {
            _ if ok(status) => Ok(()),
            // The destination is on another server or volume.
            502 => Err(VfsError::new(
                ErrorKind::Unsupported,
                format!("{from}: {verb} refused (HTTP {status})"),
            )),
            _ => Err(put_error(status, from)),
        }
    }

    fn delete(&self, path: &VfsPath, dir: bool) -> Result<()> {
        Self::check_target(path)?;
        let req = Req::new(Method::DELETE, self.inner.base.url_of(path, dir));
        let status = self.simple(&req, path)?;
        if ok(status) {
            Ok(())
        } else {
            Err(put_error(status, path))
        }
    }
}

/// The body of a streamed `PUT`: pieces from the uploading thread.
struct ChannelReader {
    rx: Receiver<io::Result<Vec<u8>>>,
    piece: Vec<u8>,
    at: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.piece.len() {
            match self.rx.recv() {
                Ok(Ok(p)) => {
                    self.piece = p;
                    self.at = 0;
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.piece.len() - self.at);
        buf[..n].copy_from_slice(&self.piece[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Reads until `buf` is full or `data` ends.
fn fill(data: &mut dyn Read, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match data.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            // The data failed, not the connection: not a transient error.
            Err(e) => {
                return Err(
                    VfsError::new(ErrorKind::Other, format!("upload data: {e}")).with_source(e)
                );
            }
        }
    }
    Ok(n)
}
