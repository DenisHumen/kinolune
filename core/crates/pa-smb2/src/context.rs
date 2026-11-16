// SPDX-License-Identifier: MPL-2.0
//! Safe wrapper over one libsmb2 context (one TCP connection to one share).

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use crate::error::{Error, Op};
use crate::library::Smb2Lib;
use crate::sys;

/// SMB dialect to negotiate (`smb2_set_version`).
///
/// libsmb2 implements SMB 2.0.2 to 3.1.1 only: there is no SMB1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Version {
    /// Let the server pick the highest common dialect (default).
    #[default]
    Any,
    /// Any SMB 2.x dialect.
    Any2,
    /// Any SMB 3.x dialect (needed for encryption).
    Any3,
    /// SMB 2.0.2.
    V2_02,
    /// SMB 2.1.
    V2_10,
    /// SMB 3.0.
    V3_00,
    /// SMB 3.0.2.
    V3_02,
    /// SMB 3.1.1.
    V3_11,
}

impl Version {
    fn raw(self) -> c_int {
        match self {
            Self::Any => 0,
            Self::Any2 => 2,
            Self::Any3 => 3,
            Self::V2_02 => 0x0202,
            Self::V2_10 => 0x0210,
            Self::V3_00 => 0x0300,
            Self::V3_02 => 0x0302,
            Self::V3_11 => 0x0311,
        }
    }
}

/// Message signing (`smb2_set_security_mode`, `smb2_set_sign`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Signing {
    /// Sign when the server requires it (default).
    #[default]
    Auto,
    /// Require signing; fail if the server cannot sign.
    Required,
}

/// SMB3 encryption (`smb2_set_seal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Encryption {
    /// Advertise support and encrypt when the server or share requires it
    /// (libsmb2's default; works with shares that mandate encryption).
    #[default]
    Auto,
    /// Require SMB3 encryption for everything; fail if not negotiated.
    Required,
    /// Never encrypt (fails on shares that mandate encryption).
    Off,
}

/// What `stat`, `fstat` and directory listings report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stat {
    /// File, directory or link.
    pub kind: FileType,
    /// Size in bytes.
    pub size: u64,
    /// Last write time, Unix seconds.
    pub mtime_sec: u64,
    /// Nanoseconds part of the last write time.
    pub mtime_nsec: u64,
    /// File id the server reports (Samba: the inode), 0 if none. Two
    /// paths with the same non-zero id on one server are the same object.
    pub ino: u64,
}

/// Entry type as libsmb2 reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileType {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A reparse point (symlink) the server did not follow.
    Link,
    /// A type value this wrapper does not know.
    Other,
}

impl From<sys::smb2_stat_64> for Stat {
    fn from(st: sys::smb2_stat_64) -> Self {
        Self {
            kind: match st.smb2_type {
                sys::SMB2_TYPE_FILE => FileType::File,
                sys::SMB2_TYPE_DIRECTORY => FileType::Dir,
                sys::SMB2_TYPE_LINK => FileType::Link,
                _ => FileType::Other,
            },
            size: st.smb2_size,
            mtime_sec: st.smb2_mtime,
            mtime_nsec: st.smb2_mtime_nsec,
            ino: st.smb2_ino,
        }
    }
}

/// One directory entry (without `.` and `..`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirEntry {
    /// Entry name, UTF-8.
    pub name: String,
    /// Its metadata, from the same directory query.
    pub stat: Stat,
}

/// What kind of share a [`ShareInfo`] is (the low bits of its type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShareType {
    /// A folder share: the only kind that holds files.
    Disk,
    /// A printer.
    Printer,
    /// A device.
    Device,
    /// Interprocess communication (`IPC$`).
    Ipc,
}

/// One share of a server, as [`Context::list_shares`] reports it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShareInfo {
    /// Share name, the one to connect to.
    pub name: String,
    /// What the share is.
    pub kind: ShareType,
    /// The server marks it as an administrative share (`C$`, `ADMIN$`).
    pub hidden: bool,
    /// The server's comment ("Home directories"), may be empty.
    pub comment: String,
}

impl ShareInfo {
    /// A folder share a person would pick: a disk share that is not an
    /// administrative or special one (`IPC$`, `C$`, `ADMIN$`, `print$`;
    /// every name ending in `$` is hidden by convention).
    #[must_use]
    pub fn is_user_folder(&self) -> bool {
        self.kind == ShareType::Disk && !self.hidden && !self.name.ends_with('$')
    }
}

/// Where the share-enumeration callback leaves its result. Boxed, so its
/// address stays valid while libsmb2 holds it.
struct EnumReply {
    done: bool,
    status: c_int,
    data: *mut c_void,
}

unsafe extern "C" fn enum_cb(
    _smb2: *mut sys::smb2_context,
    status: c_int,
    data: *mut c_void,
    cb_data: *mut c_void,
) {
    // SAFETY: `cb_data` is the `EnumReply` passed to
    // `smb2_share_enum_async`, alive until the context is destroyed.
    let reply = unsafe { &mut *cb_data.cast::<EnumReply>() };
    reply.done = true;
    reply.status = status;
    reply.data = data;
}

/// An open file on a [`Context`]. Only valid with the context that opened it;
/// using it with another context, or after `close`, fails with an error
/// instead of touching a dangling handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    context: u64,
    id: u64,
}

/// One libsmb2 context: a connection to one share with its own session.
///
/// libsmb2 contexts are not thread safe, so `Context` is `Send` but not
/// `Sync`, and every call takes `&mut self`. Run several contexts for
/// parallel requests (one per thread, or a pool). All calls are blocking
/// and bounded by [`set_timeout`](Self::set_timeout).
///
/// Paths are relative to the share root, `/`-separated, without a leading
/// slash; `""` is the root.
pub struct Context {
    lib: &'static Smb2Lib,
    ptr: NonNull<sys::smb2_context>,
    files: HashMap<u64, NonNull<sys::smb2fh>>,
    next_file: u64,
    serial: u64,
    connected: bool,
    timeout: u32,
    /// Replies of share enumerations that did not finish; libsmb2 may
    /// still write to them, so they live until the context is destroyed.
    abandoned: Vec<*mut EnumReply>,
}

// SAFETY: a libsmb2 context is plain heap state plus a socket, with no
// thread affinity; `Context` only hands it out through `&mut self`, so it is
// used by one thread at a time.
unsafe impl Send for Context {}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("connected", &self.connected)
            .field("open_files", &self.files.len())
            .finish_non_exhaustive()
    }
}

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn cstring(op: Op, s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::new(op, 0, 0, "string contains a NUL byte".into()))
}

impl Context {
    /// Creates a disconnected context with NTLMSSP authentication.
    pub fn new(lib: &'static Smb2Lib) -> Result<Self, Error> {
        // SAFETY: no preconditions.
        let ptr = unsafe { (lib.functions.init_context)() };
        let ptr = NonNull::new(ptr)
            .ok_or_else(|| Error::new(Op::Init, 0, 0, "smb2_init_context failed".into()))?;
        let ctx = Self {
            lib,
            ptr,
            files: HashMap::new(),
            next_file: 1,
            serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            connected: false,
            timeout: 0,
            abandoned: Vec::new(),
        };
        // SAFETY: valid context.
        unsafe { (lib.functions.set_authentication)(ctx.raw(), sys::SMB2_SEC_NTLMSSP) };
        Ok(ctx)
    }

    fn raw(&self) -> *mut sys::smb2_context {
        self.ptr.as_ptr()
    }

    fn f(&self) -> &sys::Functions {
        &self.lib.functions
    }

    /// Per-request timeout in whole seconds (0 = libsmb2's default, none).
    pub fn set_timeout(&mut self, seconds: u32) {
        let s = c_int::try_from(seconds).unwrap_or(c_int::MAX);
        self.timeout = seconds;
        // SAFETY: valid context.
        unsafe { (self.f().set_timeout)(self.raw(), s) };
    }

    /// Dialect to negotiate; call before [`connect`](Self::connect).
    pub fn set_version(&mut self, version: Version) {
        // SAFETY: valid context; the value is a member of the C enum.
        unsafe { (self.f().set_version)(self.raw(), version.raw()) };
    }

    /// Signing policy; call before [`connect`](Self::connect).
    pub fn set_signing(&mut self, signing: Signing) {
        let (mode, sign) = match signing {
            Signing::Auto => (sys::SMB2_NEGOTIATE_SIGNING_ENABLED, 0),
            Signing::Required => (
                sys::SMB2_NEGOTIATE_SIGNING_ENABLED | sys::SMB2_NEGOTIATE_SIGNING_REQUIRED,
                1,
            ),
        };
        // SAFETY: valid context.
        unsafe {
            (self.f().set_security_mode)(self.raw(), mode);
            (self.f().set_sign)(self.raw(), sign);
        }
    }

    /// Encryption policy; call before [`connect`](Self::connect).
    /// [`Encryption::Auto`] leaves libsmb2's default untouched.
    pub fn set_encryption(&mut self, encryption: Encryption) {
        let seal = match encryption {
            Encryption::Auto => return,
            Encryption::Required => 1,
            Encryption::Off => 0,
        };
        // SAFETY: valid context.
        unsafe { (self.f().set_seal)(self.raw(), seal) };
    }

    /// NTLM credentials. An empty user and password log on as guest or
    /// anonymously, as the server allows. libsmb2 keeps its own copy of the
    /// password for the session; ours is wiped right after the call.
    pub fn set_credentials(
        &mut self,
        user: &str,
        password: &str,
        domain: Option<&str>,
    ) -> Result<(), Error> {
        let user_c = cstring(Op::Init, user)?;
        let mut pass = password.as_bytes().to_vec();
        if pass.contains(&0) {
            wipe(&mut pass);
            return Err(Error::new(
                Op::Init,
                0,
                0,
                "password contains a NUL byte".into(),
            ));
        }
        pass.push(0);
        // SAFETY: valid context and NUL-terminated strings; libsmb2 copies them.
        unsafe {
            (self.f().set_user)(self.raw(), user_c.as_ptr());
            (self.f().set_password)(self.raw(), pass.as_ptr().cast());
        }
        wipe(&mut pass);
        if let Some(domain) = domain {
            let d = cstring(Op::Init, domain)?;
            // SAFETY: as above.
            unsafe { (self.f().set_domain)(self.raw(), d.as_ptr()) };
        }
        Ok(())
    }

    /// Workstation name sent in the NTLM exchange.
    pub fn set_workstation(&mut self, name: &str) -> Result<(), Error> {
        let n = cstring(Op::Init, name)?;
        // SAFETY: valid context, NUL-terminated string copied by libsmb2.
        unsafe { (self.f().set_workstation)(self.raw(), n.as_ptr()) };
        Ok(())
    }

    /// Connects to `share` on `server` (`host` or `host:port`, IPv6 as
    /// `[addr]:port`), logging on as the user given to
    /// [`set_credentials`](Self::set_credentials).
    pub fn connect(&mut self, server: &str, share: &str, user: Option<&str>) -> Result<(), Error> {
        let s = cstring(Op::Connect, server)?;
        let sh = cstring(Op::Connect, share)?;
        let u = user.map(|u| cstring(Op::Connect, u)).transpose()?;
        self.clear_error();
        // SAFETY: valid context and strings.
        let rc = unsafe {
            (self.f().connect_share)(
                self.raw(),
                s.as_ptr(),
                sh.as_ptr(),
                u.as_ref().map_or(std::ptr::null(), |u| u.as_ptr()),
            )
        };
        if rc < 0 {
            return Err(self.error(Op::Connect, rc));
        }
        self.connected = true;
        Ok(())
    }

    /// `true` after a successful [`connect`](Self::connect).
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Negotiated dialect (`0x0202` … `0x0311`), 0 before connecting.
    #[must_use]
    pub fn dialect(&self) -> u16 {
        // SAFETY: valid context.
        unsafe { (self.f().get_dialect)(self.raw()) }
    }

    /// Largest read the server accepts in one request.
    #[must_use]
    pub fn max_read_size(&self) -> u32 {
        // SAFETY: valid context.
        unsafe { (self.f().get_max_read_size)(self.raw()) }
    }

    /// Metadata of `path`.
    pub fn stat(&mut self, path: &str) -> Result<Stat, Error> {
        let p = cstring(Op::Stat, path)?;
        let mut st = sys::smb2_stat_64::default();
        self.clear_error();
        // SAFETY: valid context, path and out-pointer.
        let rc = unsafe { (self.f().stat)(self.raw(), p.as_ptr(), &mut st) };
        if rc < 0 {
            return Err(self.error(Op::Stat, rc));
        }
        Ok(st.into())
    }

    /// Children of the directory `path`, without `.` and `..`. Names that
    /// are not valid UTF-8 are skipped.
    pub fn read_dir(&mut self, path: &str) -> Result<Vec<DirEntry>, Error> {
        let p = cstring(Op::ListDir, path)?;
        self.clear_error();
        // SAFETY: valid context and path.
        let dir = unsafe { (self.f().opendir)(self.raw(), p.as_ptr()) };
        let Some(dir) = NonNull::new(dir) else {
            return Err(self.error(Op::ListDir, 0));
        };
        let mut out = Vec::new();
        loop {
            // SAFETY: `dir` is open on this context.
            let ent = unsafe { (self.f().readdir)(self.raw(), dir.as_ptr()) };
            if ent.is_null() {
                break;
            }
            // SAFETY: libsmb2 returns a valid entry with a NUL-terminated
            // name, alive until the next readdir; both are copied here.
            let (name, st) = unsafe { (CStr::from_ptr((*ent).name), (*ent).st) };
            let Ok(name) = name.to_str() else { continue };
            if name == "." || name == ".." {
                continue;
            }
            out.push(DirEntry {
                name: name.to_owned(),
                stat: st.into(),
            });
        }
        // SAFETY: `dir` is open on this context and not used afterwards.
        unsafe { (self.f().closedir)(self.raw(), dir.as_ptr()) };
        Ok(out)
    }

    /// Opens `path` read-only. A directory fails with
    /// [`ErrorKind::IsADirectory`](crate::ErrorKind::IsADirectory).
    pub fn open(&mut self, path: &str) -> Result<FileId, Error> {
        let p = cstring(Op::Open, path)?;
        self.clear_error();
        // SAFETY: valid context and path.
        let fh = unsafe { (self.f().open)(self.raw(), p.as_ptr(), sys::O_RDONLY) };
        let Some(fh) = NonNull::new(fh) else {
            return Err(self.error(Op::Open, 0));
        };
        let id = self.next_file;
        self.next_file += 1;
        self.files.insert(id, fh);
        Ok(FileId {
            context: self.serial,
            id,
        })
    }

    fn handle(&self, op: Op, file: FileId) -> Result<NonNull<sys::smb2fh>, Error> {
        if file.context != self.serial {
            return Err(Error::new(
                op,
                0,
                0,
                "file handle belongs to another context".into(),
            ));
        }
        self.files
            .get(&file.id)
            .copied()
            .ok_or_else(|| Error::new(op, 0, 0, "file handle is closed".into()))
    }

    /// Metadata of an open file.
    pub fn fstat(&mut self, file: FileId) -> Result<Stat, Error> {
        let fh = self.handle(Op::Fstat, file)?;
        let mut st = sys::smb2_stat_64::default();
        self.clear_error();
        // SAFETY: open handle of this context, valid out-pointer.
        let rc = unsafe { (self.f().fstat)(self.raw(), fh.as_ptr(), &mut st) };
        if rc < 0 {
            return Err(self.error(Op::Fstat, rc));
        }
        Ok(st.into())
    }

    /// Reads up to `buf.len()` bytes at `offset` (at most
    /// [`max_read_size`](Self::max_read_size) per call). Returns 0 at the
    /// end of the file.
    pub fn pread(&mut self, file: FileId, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        let fh = self.handle(Op::Read, file)?;
        if buf.is_empty() {
            return Ok(0);
        }
        let max = match self.max_read_size() {
            0 => 64 * 1024,
            n => n,
        };
        let count = u32::try_from(buf.len()).unwrap_or(u32::MAX).min(max);
        self.clear_error();
        // SAFETY: open handle, `buf` has at least `count` writable bytes.
        let rc =
            unsafe { (self.f().pread)(self.raw(), fh.as_ptr(), buf.as_mut_ptr(), count, offset) };
        if rc < 0 {
            return Err(self.error(Op::Read, rc));
        }
        Ok(usize::try_from(rc).unwrap_or(0).min(buf.len()))
    }

    /// Closes an open file. The id is invalid afterwards.
    pub fn close(&mut self, file: FileId) -> Result<(), Error> {
        let fh = self.handle(Op::Close, file)?;
        self.files.remove(&file.id);
        self.clear_error();
        // SAFETY: open handle of this context, removed from the map first.
        let rc = unsafe { (self.f().close)(self.raw(), fh.as_ptr()) };
        if rc < 0 {
            return Err(self.error(Op::Close, rc));
        }
        Ok(())
    }

    /// Shares of the server (srvsvc `NetShareEnum`, level 1), in the
    /// server's order, including printers and administrative shares (see
    /// [`ShareInfo::is_user_folder`]). The context must be connected to the
    /// `IPC$` share. Bounded by [`set_timeout`](Self::set_timeout) (30 s if
    /// none was set).
    pub fn list_shares(&mut self) -> Result<Vec<ShareInfo>, Error> {
        // Owned through a raw pointer: libsmb2 writes to it from the
        // callback, so no Rust reference may be held across `service`.
        let reply: *mut EnumReply = Box::into_raw(Box::new(EnumReply {
            done: false,
            status: 0,
            data: std::ptr::null_mut(),
        }));
        self.clear_error();
        // SAFETY: valid context; `reply` stays allocated until the request
        // finished here, or until the context is destroyed (`abandoned`).
        let rc = unsafe {
            (self.f().share_enum_async)(
                self.raw(),
                sys::SHARE_INFO_1,
                enum_cb,
                reply.cast::<c_void>(),
            )
        };
        if rc < 0 {
            // SAFETY: libsmb2 did not take the pointer.
            drop(unsafe { Box::from_raw(reply) });
            return Err(self.error(Op::ShareEnum, rc));
        }
        let secs = if self.timeout == 0 { 30 } else { self.timeout };
        let deadline = Instant::now() + Duration::from_secs(secs.into());
        // SAFETY (all reads of `*reply`): the callback runs only inside
        // `service` on this thread, never concurrently with these reads.
        while !unsafe { std::ptr::read_volatile(&raw const (*reply).done) } {
            if Instant::now() >= deadline {
                self.abandoned.push(reply);
                return Err(Error::new(
                    Op::ShareEnum,
                    0,
                    0,
                    "share enumeration timed out".into(),
                ));
            }
            let raw = self.raw();
            // SAFETY: valid context.
            let (fd, events) = unsafe { ((self.f().get_fd)(raw), (self.f().which_events)(raw)) };
            let revents = wait_socket(fd, events, Duration::from_millis(100));
            if revents == 0 {
                continue;
            }
            // SAFETY: valid context; `revents` are the poll results.
            if unsafe { (self.f().service)(raw, revents) } < 0 {
                let err = self.error(Op::ShareEnum, 0);
                // The context is unusable now; keep the reply until it is
                // destroyed (a late callback may still write to it).
                self.abandoned.push(reply);
                return Err(err);
            }
        }
        // SAFETY: the request finished; libsmb2 no longer holds `reply`.
        let EnumReply { status, data, .. } = *unsafe { Box::from_raw(reply) };
        if status != 0 {
            if !data.is_null() {
                // SAFETY: data libsmb2 handed to the callback.
                unsafe { (self.f().free_data)(self.raw(), data) };
            }
            // Negative: -errno; positive: an NT status or WERROR.
            let (errno, nt) = if status < 0 {
                (status.saturating_neg(), 0)
            } else {
                (0, status.cast_unsigned())
            };
            let msg = self.error(Op::ShareEnum, 0).message().to_owned();
            return Err(Error::new(Op::ShareEnum, errno, nt, msg));
        }
        if data.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: on success `data` is a `srvsvc_NetrShareEnum_rep` decoded
        // by libsmb2, valid until `smb2_free_data`.
        let shares = unsafe { read_shares(&*data.cast::<sys::srvsvc_NetrShareEnum_rep>()) };
        // SAFETY: as above, freed once.
        unsafe { (self.f().free_data)(self.raw(), data) };
        Ok(shares)
    }

    /// Number of files currently open on this context.
    #[must_use]
    pub fn open_files(&self) -> usize {
        self.files.len()
    }

    fn clear_error(&mut self) {
        // SAFETY: valid context; an empty format string with no arguments.
        unsafe { (self.f().set_error)(self.raw(), c"".as_ptr()) };
    }

    fn error(&self, op: Op, rc: c_int) -> Error {
        // SAFETY: valid context; libsmb2 returns its own NUL-terminated buffer.
        let (msg, nt) = unsafe {
            let p = (self.f().get_error)(self.raw());
            let msg = if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().trim().to_owned()
            };
            (msg, (self.f().get_nterror)(self.raw()))
        };
        let msg = if msg.is_empty() {
            format!("libsmb2 returned {rc}")
        } else {
            msg
        };
        // Several sync paths return a bare -1 rather than -errno; -1 is
        // EPERM only by accident, so it carries no errno. EPERM from a real
        // server reply always comes with its NT status.
        let errno = if rc < -1 { rc.saturating_neg() } else { 0 };
        Error::new(op, errno, nt as u32, msg)
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        let raw = self.raw();
        let f = *self.f();
        for (_, fh) in self.files.drain() {
            // SAFETY: open handles of this context, each closed once.
            unsafe { (f.close)(raw, fh.as_ptr()) };
        }
        // SAFETY: valid context, destroyed once; destroy also disconnects.
        unsafe {
            if self.connected {
                (f.disconnect_share)(raw);
            }
            (f.destroy_context)(raw);
        }
        for reply in self.abandoned.drain(..) {
            // SAFETY: allocated by `list_shares` with `Box::into_raw`; the
            // context that could write to it is gone.
            drop(unsafe { Box::from_raw(reply) });
        }
    }
}

/// Copies the level-1 entries of a share enumeration reply.
///
/// # Safety
///
/// `rep` must be a reply decoded by libsmb2 (pointers valid or null).
unsafe fn read_shares(rep: &sys::srvsvc_NetrShareEnum_rep) -> Vec<ShareInfo> {
    let c = &rep.ses.share_info.level1;
    if rep.ses.share_info.level != 1 || c.buffer.is_null() {
        return Vec::new();
    }
    // SAFETY: non-null array header from the decoder.
    let arr = unsafe { &*c.buffer };
    if arr.share_info_1.is_null() {
        return Vec::new();
    }
    let n = c.entries_read.min(arr.max_count) as usize;
    let text = |s: &sys::dcerpc_utf16| -> String {
        if s.utf8.is_null() {
            return String::new();
        }
        // SAFETY: NUL-terminated UTF-8 made by the decoder.
        unsafe { CStr::from_ptr(s.utf8.cast::<c_char>()) }
            .to_string_lossy()
            .into_owned()
    };
    (0..n)
        .map(|i| {
            // SAFETY: `i < n <= max_count` entries were decoded.
            let e = unsafe { &*arr.share_info_1.add(i) };
            ShareInfo {
                name: text(&e.netname),
                kind: match e.share_type & 3 {
                    sys::SHARE_TYPE_DISKTREE => ShareType::Disk,
                    sys::SHARE_TYPE_PRINTQ => ShareType::Printer,
                    sys::SHARE_TYPE_DEVICE => ShareType::Device,
                    _ => ShareType::Ipc,
                },
                hidden: e.share_type & sys::SHARE_TYPE_HIDDEN != 0,
                comment: text(&e.remark),
            }
        })
        .filter(|s| !s.name.is_empty())
        .collect()
}

/// Waits until the socket is ready for `events` (libsmb2's poll flags) or
/// `wait` passes; returns the ready flags, 0 on timeout.
#[cfg(unix)]
fn wait_socket(fd: sys::t_socket, events: c_int, wait: Duration) -> c_int {
    let mut pfd = libc::pollfd {
        fd,
        events: i16::try_from(events).unwrap_or(0),
        revents: 0,
    };
    let ms = c_int::try_from(wait.as_millis()).unwrap_or(c_int::MAX);
    // SAFETY: one valid pollfd.
    let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
    if rc <= 0 { 0 } else { c_int::from(pfd.revents) }
}

/// Windows: `WSAPoll`, with the same flags libsmb2 itself polls with
/// (its mingw-w64 build maps `poll` to `WSAPoll`).
#[cfg(windows)]
fn wait_socket(fd: sys::t_socket, events: c_int, wait: Duration) -> c_int {
    crate::winsock::poll(fd, events, wait)
}

fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: valid exclusive reference; volatile so it is not elided.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}
