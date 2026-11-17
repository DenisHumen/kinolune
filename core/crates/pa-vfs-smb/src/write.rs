// SPDX-License-Identifier: MPL-2.0
//! Writes on an SMB share (NET-023): libsmb2 `open`/`pwrite`, `mkdir`,
//! `rmdir`, `unlink`, `rename`.

use std::io::{self, Read};

use pa_smb2::FileType;
use pa_vfs::{ErrorKind, Result, VfsError, VfsPath, VfsWrite};

use crate::fs::SmbFs;
use crate::pool::map_error;

/// Largest single write request, whatever the server allows: keeps the
/// buffer small on phones and progress smooth.
const MAX_CHUNK: usize = 1024 * 1024;

impl SmbFs {
    /// Share path of an entry a write creates or removes: never the root,
    /// and no `\` or `:` (separators and streams on SMB) in any component.
    fn write_target(&self, path: &VfsPath) -> Result<String> {
        if path.is_root() {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                "the source root cannot be written",
            ));
        }
        if path.components().any(|c| c.contains(['\\', ':'])) {
            return Err(VfsError::new(
                ErrorKind::InvalidPath,
                format!("{path}: `\\` and `:` are not allowed on SMB"),
            ));
        }
        let p = self.share_path(path);
        self.inner.pool.forget_under(&p);
        Ok(p)
    }

    fn path_op(
        &self,
        path: &VfsPath,
        op: fn(&mut pa_smb2::Context, &str) -> std::result::Result<(), pa_smb2::Error>,
    ) -> Result<()> {
        let p = self.write_target(path)?;
        self.inner.pool.with_conn(&self.url(&p), |c| {
            c.forget_under(&p);
            op(&mut c.ctx, &p)
        })
    }
}

impl VfsWrite for SmbFs {
    fn create_dir(&self, path: &VfsPath) -> Result<()> {
        self.path_op(path, pa_smb2::Context::mkdir)
    }

    fn write_file(
        &self,
        path: &VfsPath,
        offset: u64,
        data: &mut dyn Read,
        len: u64,
    ) -> Result<u64> {
        let p = self.write_target(path)?;
        let url = self.url(&p);
        let smb = |e: pa_smb2::Error| map_error(&url, &e);
        self.inner.pool.with_conn_once(|c| {
            c.forget_under(&p);
            if offset > 0 {
                // A write-only handle may not read attributes: stat first.
                let size = c.ctx.stat(&p).map_err(smb)?.size;
                if size != offset {
                    return Err(VfsError::new(
                        ErrorKind::Other,
                        format!("{url}: {size} bytes, cannot append at {offset}"),
                    ));
                }
            }
            let file = c.ctx.open_write(&p, offset == 0).map_err(smb)?;
            let r = (|| {
                let max = match c.ctx.max_write_size() {
                    0 => 64 * 1024,
                    n => usize::try_from(n).unwrap_or(MAX_CHUNK),
                };
                let mut buf = vec![0u8; max.min(MAX_CHUNK)];
                let mut done = 0u64;
                while done < len {
                    let want = usize::try_from(len - done)
                        .unwrap_or(usize::MAX)
                        .min(buf.len());
                    let n = fill(data, &mut buf[..want])?;
                    if n == 0 {
                        break;
                    }
                    let mut sent = 0;
                    while sent < n {
                        let w = c
                            .ctx
                            .pwrite(file, offset + done + sent as u64, &buf[sent..n])
                            .map_err(smb)?;
                        if w == 0 {
                            return Err(VfsError::new(
                                ErrorKind::Other,
                                format!("{url}: the server wrote nothing"),
                            ));
                        }
                        sent += w;
                    }
                    done += n as u64;
                }
                Ok(done)
            })();
            // Close even after an error: the server keeps what was written.
            let closed = c.ctx.close(file).map_err(smb);
            let done = r?;
            closed?;
            Ok(done)
        })
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        let a = self.write_target(from)?;
        let b = self.write_target(to)?;
        self.inner.pool.with_conn(&self.url(&a), |c| {
            c.forget_under(&a);
            c.forget_under(&b);
            c.ctx.rename(&a, &b)
        })
    }

    fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.path_op(path, pa_smb2::Context::unlink)
    }

    fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.path_op(path, pa_smb2::Context::rmdir)
    }

    fn is_symlink(&self, path: &VfsPath) -> Result<bool> {
        let p = self.share_path(path);
        let st = self
            .inner
            .pool
            .with_conn(&self.url(&p), |c| c.ctx.stat(&p))?;
        Ok(st.kind == FileType::Link)
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
