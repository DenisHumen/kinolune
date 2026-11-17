// SPDX-License-Identifier: MPL-2.0
//! Raw C ABI of libsmb2 6.x (`include/smb2/libsmb2.h`): the types and the
//! function table this crate loads. Only what [`Context`](crate::Context)
//! uses; the list is mirrored in `tools/libsmb2/lib/smb2.sh`
//! (`SMB2_REQUIRED_SYMBOLS`), and the licence gate checks every artifact
//! exports it.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void};
use std::marker::{PhantomData, PhantomPinned};

macro_rules! opaque {
    ($($name:ident),+) => {$(
        /// Opaque libsmb2 handle.
        #[repr(C)]
        pub struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    )+};
}

opaque!(smb2_context, smb2dir, smb2fh);

/// `SMB2_TYPE_FILE`.
pub const SMB2_TYPE_FILE: u32 = 0;
/// `SMB2_TYPE_DIRECTORY`.
pub const SMB2_TYPE_DIRECTORY: u32 = 1;
/// `SMB2_TYPE_LINK`: a reparse point the server did not follow.
pub const SMB2_TYPE_LINK: u32 = 2;

/// `SMB2_NEGOTIATE_SIGNING_ENABLED`.
pub const SMB2_NEGOTIATE_SIGNING_ENABLED: u16 = 0x0001;
/// `SMB2_NEGOTIATE_SIGNING_REQUIRED`.
pub const SMB2_NEGOTIATE_SIGNING_REQUIRED: u16 = 0x0002;

/// `SMB2_SEC_NTLMSSP` (`enum smb2_sec`): libsmb2 is built without Kerberos.
pub const SMB2_SEC_NTLMSSP: c_int = 1;

/// `SHARE_INFO_1` (`enum SHARE_INFO_enum`): name, type and comment.
pub const SHARE_INFO_1: c_int = 1;
/// Low two bits of a share type: a disk share (`SHARE_TYPE_DISKTREE`).
pub const SHARE_TYPE_DISKTREE: u32 = 0;
/// Low two bits of a share type: a print queue.
pub const SHARE_TYPE_PRINTQ: u32 = 1;
/// Low two bits of a share type: a device.
pub const SHARE_TYPE_DEVICE: u32 = 2;
/// Low two bits of a share type: interprocess communication (`IPC$`).
pub const SHARE_TYPE_IPC: u32 = 3;
/// Flag of a share type: an administrative share (`C$`, `ADMIN$`).
pub const SHARE_TYPE_HIDDEN: u32 = 0x8000_0000;

/// `O_RDONLY`, the same (0) on every supported OS.
pub const O_RDONLY: c_int = 0;

// Open flags for writes (NET-023). libsmb2 reads them with the platform's
// <fcntl.h>, so they are the platform's values.
/// `O_WRONLY`.
pub const O_WRONLY: c_int = 1;
/// `O_CREAT`.
#[cfg(unix)]
pub const O_CREAT: c_int = libc::O_CREAT;
/// `O_TRUNC`.
#[cfg(unix)]
pub const O_TRUNC: c_int = libc::O_TRUNC;
/// `O_CREAT` (mingw-w64 / MSVCRT `<fcntl.h>`).
#[cfg(windows)]
pub const O_CREAT: c_int = 0x0100;
/// `O_TRUNC` (mingw-w64 / MSVCRT `<fcntl.h>`).
#[cfg(windows)]
pub const O_TRUNC: c_int = 0x0200;

/// `struct smb2_stat_64`. Times are Unix seconds plus nanoseconds.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct smb2_stat_64 {
    pub smb2_type: u32,
    pub smb2_nlink: u32,
    pub smb2_ino: u64,
    pub smb2_size: u64,
    pub smb2_atime: u64,
    pub smb2_atime_nsec: u64,
    pub smb2_mtime: u64,
    pub smb2_mtime_nsec: u64,
    pub smb2_ctime: u64,
    pub smb2_ctime_nsec: u64,
    pub smb2_btime: u64,
    pub smb2_btime_nsec: u64,
}

/// `struct smb2dirent`; valid until the next `smb2_readdir`/`smb2_closedir`.
#[repr(C)]
pub struct smb2dirent {
    pub name: *const c_char,
    pub st: smb2_stat_64,
}

/// `struct dcerpc_utf16` (`libsmb2-dcerpc.h`): a decoded string keeps its
/// UTF-8 copy in `utf8`.
#[repr(C)]
pub struct dcerpc_utf16 {
    pub max_count: u32,
    pub offset: u32,
    pub actual_count: u32,
    pub utf16: *mut c_void,
    pub utf8: *const c_char,
}

/// `struct srvsvc_SHARE_INFO_1`.
#[repr(C)]
pub struct srvsvc_SHARE_INFO_1 {
    pub netname: dcerpc_utf16,
    pub share_type: u32,
    pub remark: dcerpc_utf16,
}

/// `struct srvsvc_SHARE_INFO_1_carray`.
#[repr(C)]
pub struct srvsvc_SHARE_INFO_1_carray {
    pub max_count: u32,
    pub share_info_1: *mut srvsvc_SHARE_INFO_1,
}

/// `struct srvsvc_SHARE_INFO_1_CONTAINER`; level 0 has the same layout.
#[repr(C)]
pub struct srvsvc_SHARE_INFO_1_CONTAINER {
    pub entries_read: u32,
    pub buffer: *mut srvsvc_SHARE_INFO_1_carray,
}

/// `struct srvsvc_SHARE_ENUM_UNION` (the union holds one container).
#[repr(C)]
pub struct srvsvc_SHARE_ENUM_UNION {
    pub level: u32,
    pub level1: srvsvc_SHARE_INFO_1_CONTAINER,
}

/// `struct srvsvc_SHARE_ENUM_STRUCT`.
#[repr(C)]
pub struct srvsvc_SHARE_ENUM_STRUCT {
    pub level: u32,
    pub share_info: srvsvc_SHARE_ENUM_UNION,
}

/// `struct srvsvc_NetrShareEnum_rep`, freed with `smb2_free_data`.
#[repr(C)]
pub struct srvsvc_NetrShareEnum_rep {
    pub status: u32,
    pub ses: srvsvc_SHARE_ENUM_STRUCT,
    pub total_entries: u32,
    pub resume_handle: u32,
}

/// `smb2_command_cb`.
pub type smb2_command_cb = unsafe extern "C" fn(*mut smb2_context, c_int, *mut c_void, *mut c_void);

/// `t_socket`: a file descriptor, or a `SOCKET` on Windows.
#[cfg(windows)]
pub type t_socket = usize;
/// `t_socket`: a file descriptor, or a `SOCKET` on Windows.
#[cfg(not(windows))]
pub type t_socket = c_int;

type Ctx = *mut smb2_context;

/// Every libsmb2 function the wrapper calls, resolved by name at load time.
#[allow(missing_docs)]
#[derive(Clone, Copy)]
pub struct Functions {
    pub init_context: unsafe extern "C" fn() -> Ctx,
    pub destroy_context: unsafe extern "C" fn(Ctx),
    pub set_timeout: unsafe extern "C" fn(Ctx, c_int),
    pub set_version: unsafe extern "C" fn(Ctx, c_int),
    pub set_security_mode: unsafe extern "C" fn(Ctx, u16),
    pub set_seal: unsafe extern "C" fn(Ctx, c_int),
    pub set_sign: unsafe extern "C" fn(Ctx, c_int),
    pub set_authentication: unsafe extern "C" fn(Ctx, c_int),
    pub set_user: unsafe extern "C" fn(Ctx, *const c_char),
    pub set_password: unsafe extern "C" fn(Ctx, *const c_char),
    pub set_domain: unsafe extern "C" fn(Ctx, *const c_char),
    pub set_workstation: unsafe extern "C" fn(Ctx, *const c_char),
    pub connect_share:
        unsafe extern "C" fn(Ctx, *const c_char, *const c_char, *const c_char) -> c_int,
    pub disconnect_share: unsafe extern "C" fn(Ctx) -> c_int,
    pub get_error: unsafe extern "C" fn(Ctx) -> *const c_char,
    /// Variadic `printf`-style; called only with `""`, which also resets
    /// the NT status, so the status read after a call belongs to it.
    pub set_error: unsafe extern "C" fn(Ctx, *const c_char, ...),
    pub get_nterror: unsafe extern "C" fn(Ctx) -> c_int,
    pub get_dialect: unsafe extern "C" fn(Ctx) -> u16,
    pub get_max_read_size: unsafe extern "C" fn(Ctx) -> u32,
    pub opendir: unsafe extern "C" fn(Ctx, *const c_char) -> *mut smb2dir,
    pub readdir: unsafe extern "C" fn(Ctx, *mut smb2dir) -> *mut smb2dirent,
    pub closedir: unsafe extern "C" fn(Ctx, *mut smb2dir),
    pub open: unsafe extern "C" fn(Ctx, *const c_char, c_int) -> *mut smb2fh,
    pub close: unsafe extern "C" fn(Ctx, *mut smb2fh) -> c_int,
    pub pread: unsafe extern "C" fn(Ctx, *mut smb2fh, *mut u8, u32, u64) -> c_int,
    pub fstat: unsafe extern "C" fn(Ctx, *mut smb2fh, *mut smb2_stat_64) -> c_int,
    pub pwrite: unsafe extern "C" fn(Ctx, *mut smb2fh, *const u8, u32, u64) -> c_int,
    pub get_max_write_size: unsafe extern "C" fn(Ctx) -> u32,
    pub mkdir: unsafe extern "C" fn(Ctx, *const c_char) -> c_int,
    pub rmdir: unsafe extern "C" fn(Ctx, *const c_char) -> c_int,
    pub unlink: unsafe extern "C" fn(Ctx, *const c_char) -> c_int,
    pub rename: unsafe extern "C" fn(Ctx, *const c_char, *const c_char) -> c_int,
    pub stat: unsafe extern "C" fn(Ctx, *const c_char, *mut smb2_stat_64) -> c_int,
    pub get_fd: unsafe extern "C" fn(Ctx) -> t_socket,
    pub which_events: unsafe extern "C" fn(Ctx) -> c_int,
    pub service: unsafe extern "C" fn(Ctx, c_int) -> c_int,
    pub share_enum_async: unsafe extern "C" fn(Ctx, c_int, smb2_command_cb, *mut c_void) -> c_int,
    pub free_data: unsafe extern "C" fn(Ctx, *mut c_void),
}

/// Names of [`Functions`] fields in the library, in declaration order.
pub const SYMBOLS: &[&str] = &[
    "smb2_init_context",
    "smb2_destroy_context",
    "smb2_set_timeout",
    "smb2_set_version",
    "smb2_set_security_mode",
    "smb2_set_seal",
    "smb2_set_sign",
    "smb2_set_authentication",
    "smb2_set_user",
    "smb2_set_password",
    "smb2_set_domain",
    "smb2_set_workstation",
    "smb2_connect_share",
    "smb2_disconnect_share",
    "smb2_get_error",
    "smb2_set_error",
    "smb2_get_nterror",
    "smb2_get_dialect",
    "smb2_get_max_read_size",
    "smb2_opendir",
    "smb2_readdir",
    "smb2_closedir",
    "smb2_open",
    "smb2_close",
    "smb2_pread",
    "smb2_fstat",
    "smb2_pwrite",
    "smb2_get_max_write_size",
    "smb2_mkdir",
    "smb2_rmdir",
    "smb2_unlink",
    "smb2_rename",
    "smb2_stat",
    "smb2_get_fd",
    "smb2_which_events",
    "smb2_service",
    "smb2_share_enum_async",
    "smb2_free_data",
];

impl Functions {
    /// Resolves every function from `lib`.
    ///
    /// # Safety
    ///
    /// `lib` must be libsmb2 6.x (the signatures above are not checked).
    pub(crate) unsafe fn load(lib: &libloading::Library) -> Result<Self, String> {
        macro_rules! sym {
            // SAFETY: the caller guarantees `lib` is libsmb2, whose function
            // has the C type of the field it is assigned to.
            ($name:literal) => {
                unsafe { get(lib, concat!($name, "\0")) }?
            };
        }
        Ok(Self {
            init_context: sym!("smb2_init_context"),
            destroy_context: sym!("smb2_destroy_context"),
            set_timeout: sym!("smb2_set_timeout"),
            set_version: sym!("smb2_set_version"),
            set_security_mode: sym!("smb2_set_security_mode"),
            set_seal: sym!("smb2_set_seal"),
            set_sign: sym!("smb2_set_sign"),
            set_authentication: sym!("smb2_set_authentication"),
            set_user: sym!("smb2_set_user"),
            set_password: sym!("smb2_set_password"),
            set_domain: sym!("smb2_set_domain"),
            set_workstation: sym!("smb2_set_workstation"),
            connect_share: sym!("smb2_connect_share"),
            disconnect_share: sym!("smb2_disconnect_share"),
            get_error: sym!("smb2_get_error"),
            set_error: sym!("smb2_set_error"),
            get_nterror: sym!("smb2_get_nterror"),
            get_dialect: sym!("smb2_get_dialect"),
            get_max_read_size: sym!("smb2_get_max_read_size"),
            opendir: sym!("smb2_opendir"),
            readdir: sym!("smb2_readdir"),
            closedir: sym!("smb2_closedir"),
            open: sym!("smb2_open"),
            close: sym!("smb2_close"),
            pread: sym!("smb2_pread"),
            fstat: sym!("smb2_fstat"),
            pwrite: sym!("smb2_pwrite"),
            get_max_write_size: sym!("smb2_get_max_write_size"),
            mkdir: sym!("smb2_mkdir"),
            rmdir: sym!("smb2_rmdir"),
            unlink: sym!("smb2_unlink"),
            rename: sym!("smb2_rename"),
            stat: sym!("smb2_stat"),
            get_fd: sym!("smb2_get_fd"),
            which_events: sym!("smb2_which_events"),
            service: sym!("smb2_service"),
            share_enum_async: sym!("smb2_share_enum_async"),
            free_data: sym!("smb2_free_data"),
        })
    }
}

/// Resolves one symbol as a value of type `T` (a function pointer).
///
/// # Safety
///
/// The symbol must have type `T`.
unsafe fn get<T: Copy>(lib: &libloading::Library, name: &str) -> Result<T, String> {
    // SAFETY: forwarded to the caller.
    match unsafe { lib.get::<T>(name.as_bytes()) } {
        Ok(symbol) => Ok(*symbol),
        Err(e) => Err(format!("{}: {e}", name.trim_end_matches('\0'))),
    }
}
