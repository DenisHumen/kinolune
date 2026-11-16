// SPDX-License-Identifier: MPL-2.0
//! Winsock for libsmb2 on Windows (UI-029).
//!
//! libsmb2 does not initialise Winsock itself: without `WSAStartup` in the
//! process its `getaddrinfo` fails with `WSANOTINITIALISED`. Rust's
//! standard library only starts Winsock when it opens its own first
//! socket, so a process that talks SMB before anything else (a test, a
//! command-line tool, the app on a cold start) had no Winsock. Startup is
//! reference-counted per process and never undone here.

use std::ffi::c_int;
use std::sync::OnceLock;
use std::time::Duration;

use crate::sys::t_socket;

/// `WSADATA` is 408 bytes on x64; a larger aligned buffer is enough for
/// every Windows version and keeps us from declaring the struct.
#[repr(C, align(8))]
struct WsaData([u8; 1024]);

/// `WSAPOLLFD`.
#[repr(C)]
struct PollFd {
    fd: t_socket,
    events: i16,
    revents: i16,
}

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut WsaData) -> c_int;
    fn WSAPoll(fds: *mut PollFd, count: u32, timeout: c_int) -> c_int;
}

/// Starts Winsock 2.2 once per process; returns the `WSAStartup` result
/// (0 on success).
pub(crate) fn startup() -> c_int {
    static RESULT: OnceLock<c_int> = OnceLock::new();
    *RESULT.get_or_init(|| {
        let mut data = WsaData([0; 1024]);
        // SAFETY: valid out-pointer to a buffer larger than WSADATA.
        unsafe { WSAStartup(0x0202, &mut data) }
    })
}

/// Waits until the socket is ready for `events` (libsmb2's Winsock poll
/// flags) or `wait` passes; returns the ready flags, 0 on timeout.
pub(crate) fn poll(fd: t_socket, events: c_int, wait: Duration) -> c_int {
    let mut pfd = PollFd {
        fd,
        events: i16::try_from(events).unwrap_or(0),
        revents: 0,
    };
    let ms = c_int::try_from(wait.as_millis()).unwrap_or(c_int::MAX);
    // SAFETY: one valid WSAPOLLFD.
    let rc = unsafe { WSAPoll(&mut pfd, 1, ms) };
    if rc <= 0 { 0 } else { c_int::from(pfd.revents) }
}
