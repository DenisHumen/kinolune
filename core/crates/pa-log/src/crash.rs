// SPDX-License-Identifier: MPL-2.0
//! Crash capture: the panic hook and, on Linux and macOS, fatal signals of
//! native code (ADR-NNNN).
//!
//! A signal handler may only call async-signal-safe functions, so it does
//! not build a report: it writes a tiny marker (`crashes/native-pending`,
//! the signal number) with `open`/`write` to a path prepared beforehand,
//! puts the previous handler back and returns, so the previous handler
//! (Rust's stack-overflow guard, the default action) still runs. The next
//! start with the opt-in turns the marker into a crash report.
//!
//! Android and iOS are not covered here: their runtimes own the fatal
//! signals (ART's sigchain, the Apple crash reporter). On Android the app
//! reads the system's exit reasons instead and passes them to
//! [`Diagnostics::record_crash`](crate::Diagnostics::record_crash).

use std::fs;
use std::path::Path;

use crate::{Diagnostics, time};

pub(crate) const NATIVE_PENDING: &str = "native-pending";

/// Records Rust panics of every thread (also the ones a `catch_unwind`
/// recovers from: they are bugs too), then runs the previous hook.
pub(crate) fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(d) = crate::global().filter(|d| d.enabled()) {
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(non-string panic payload)".into());
            let at = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_default();
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>");
            let stack = std::backtrace::Backtrace::force_capture().to_string();
            d.record_crash(
                "rust-panic",
                &format!("thread '{name}' panicked at {at}:\n{msg}"),
                &stack,
            );
        }
        prev(info);
    }));
}

fn signal_name(sig: i32) -> &'static str {
    match sig {
        4 => "SIGILL",
        6 => "SIGABRT",
        7 | 10 => "SIGBUS",
        8 => "SIGFPE",
        11 => "SIGSEGV",
        _ => "signal",
    }
}

/// Turns the marker of a native crash of the previous run into a crash
/// report.
pub(crate) fn collect_native(d: &Diagnostics) {
    let pending = d.crashes_dir().join(NATIVE_PENDING);
    let Ok(text) = fs::read_to_string(&pending) else {
        return;
    };
    let when = fs::metadata(&pending)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|t| time::iso(t.as_secs()))
        .unwrap_or_else(|| "unknown".into());
    let sig: i32 = text
        .trim()
        .strip_prefix("signal=")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let _ = fs::remove_file(&pending);
    d.record_crash(
        "native-signal",
        &format!(
            "The previous run was stopped by {} ({sig}) at {when}.",
            signal_name(sig)
        ),
        "(no stack: native code crashed; see the log lines before the crash)",
    );
}

#[cfg(all(
    unix,
    not(any(target_os = "android", target_os = "ios", target_os = "tvos"))
))]
mod native {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::Once;
    use std::sync::atomic::{AtomicPtr, Ordering};

    use libc::{c_char, c_int, c_void};

    const SIGNALS: [c_int; 5] = [
        libc::SIGSEGV,
        libc::SIGBUS,
        libc::SIGILL,
        libc::SIGFPE,
        libc::SIGABRT,
    ];

    /// Path of the marker; null while the user has not opted in.
    static MARKER: AtomicPtr<c_char> = AtomicPtr::new(std::ptr::null_mut());
    static mut PREV: [MaybeUninit<libc::sigaction>; 5] = [const { MaybeUninit::uninit() }; 5];
    static INSTALL: Once = Once::new();

    extern "C" fn on_signal(sig: c_int, info: *mut libc::siginfo_t, _ctx: *mut c_void) {
        let path = MARKER.load(Ordering::Acquire);
        if !path.is_null() {
            let mut buf = *b"signal=0000000000\n";
            let mut n = sig.unsigned_abs();
            let mut i = 16;
            loop {
                buf[i] = b'0' + (n % 10) as u8;
                n /= 10;
                if n == 0 || i == 7 {
                    break;
                }
                i -= 1;
            }
            // `signal=` + digits + `\n`.
            buf.copy_within(i..18, 7);
            let len = 7 + (18 - i);
            // SAFETY: async-signal-safe calls on a NUL-terminated path that
            // is never freed.
            unsafe {
                let fd = libc::open(
                    path,
                    libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
                    0o600,
                );
                if fd >= 0 {
                    libc::write(fd, buf.as_ptr().cast(), len);
                    libc::close(fd);
                }
            }
        }
        // Give the signal back to whoever had it.
        if let Some(i) = SIGNALS.iter().position(|&s| s == sig) {
            // SAFETY: PREV[i] was filled by `install` before this handler
            // was set for `sig`.
            unsafe {
                libc::sigaction(sig, (&raw const PREV[i]).cast(), std::ptr::null_mut());
                // Sent with kill(): returning would not raise it again.
                if !info.is_null() && (*info).si_code <= 0 {
                    libc::raise(sig);
                }
            }
        }
    }

    pub(super) fn install() {
        INSTALL.call_once(|| {
            for (i, &sig) in SIGNALS.iter().enumerate() {
                // SAFETY: plain sigaction setup; PREV[i] receives the old
                // action before the handler for `sig` can run.
                unsafe {
                    let mut sa: libc::sigaction = std::mem::zeroed();
                    sa.sa_sigaction = on_signal as *const () as usize;
                    sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                    libc::sigemptyset(&mut sa.sa_mask);
                    libc::sigaction(sig, &sa, (&raw mut PREV[i]).cast());
                }
            }
        });
    }

    pub(super) fn arm(dir: &Path) {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let path = dir.join(super::NATIVE_PENDING);
        if let Ok(c) = CString::new(path.as_os_str().as_bytes()) {
            // Leaked on purpose: the handler may read it at any time.
            MARKER.store(c.into_raw(), Ordering::Release);
        }
    }

    pub(super) fn disarm() {
        MARKER.store(std::ptr::null_mut(), Ordering::Release);
    }
}

#[cfg(not(all(
    unix,
    not(any(target_os = "android", target_os = "ios", target_os = "tvos"))
)))]
mod native {
    pub(super) fn install() {}
    pub(super) fn arm(_dir: &std::path::Path) {}
    pub(super) fn disarm() {}
}

pub(crate) fn install_native() {
    native::install();
}

pub(crate) fn arm_native(dir: &Path) {
    native::arm(dir);
}

pub(crate) fn disarm_native() {
    native::disarm();
}
