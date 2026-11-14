// SPDX-License-Identifier: MPL-2.0
//! The OS keychain never blocks `open` or a secret call past its deadline;
//! a missing, locked or silent keychain falls back to the encrypted file,
//! and the file moves into the keychain once it works.
//!
//! Most tests inject a fake keychain through `OpenOptions::native_connect`.
//! On Linux two more run the real Secret Service client in a child process
//! (the D-Bus address is read from the environment): one without any
//! session bus, one against a fake bus that accepts the connection and
//! never answers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_secrets::{
    Backend, EncryptedFileStore, FallbackKind, NativeConnect, OpenOptions, SecretError, SecretKey,
    SecretStore, SecretString, open,
};

const OPEN_TIMEOUT: Duration = Duration::from_millis(300);
const CALL_TIMEOUT: Duration = Duration::from_millis(300);
/// Upper bound of a call that must not hang: the deadline plus slack for a
/// loaded CI machine.
const SLACK: Duration = Duration::from_secs(3);

/// A fake OS keychain. `hang` makes the named operations block forever
/// (`connect`, `get`, `set`, `delete`); `fail` makes connecting fail.
#[derive(Debug, Default)]
struct FakeKeychain {
    items: Mutex<HashMap<String, String>>,
    hang: Mutex<Vec<&'static str>>,
}

impl FakeKeychain {
    fn hangs(&self, op: &'static str) -> bool {
        self.hang.lock().unwrap().contains(&op)
    }

    fn maybe_hang(&self, op: &'static str) {
        if self.hangs(op) {
            loop {
                std::thread::park();
            }
        }
    }
}

/// The store handed out by the fake keychain.
#[derive(Debug)]
struct FakeStore(Arc<FakeKeychain>);

impl SecretStore for FakeStore {
    fn get(&self, key: &SecretKey) -> pa_secrets::Result<Option<SecretString>> {
        self.0.maybe_hang("get");
        let items = self.0.items.lock().unwrap();
        Ok(items
            .get(key.as_str())
            .map(|v| SecretString::new(v.clone())))
    }

    fn set(&self, key: &SecretKey, value: &SecretString) -> pa_secrets::Result<()> {
        self.0.maybe_hang("set");
        let mut items = self.0.items.lock().unwrap();
        items.insert(key.as_str().into(), value.expose().into());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> pa_secrets::Result<bool> {
        self.0.maybe_hang("delete");
        Ok(self.0.items.lock().unwrap().remove(key.as_str()).is_some())
    }

    fn backend(&self) -> Backend {
        Backend::SecretService
    }
}

fn connect_to(keychain: &Arc<FakeKeychain>) -> NativeConnect {
    let keychain = Arc::clone(keychain);
    Arc::new(move |_service: &str| {
        keychain.maybe_hang("connect");
        Ok(Arc::new(FakeStore(Arc::clone(&keychain))) as Arc<dyn SecretStore>)
    })
}

fn failing(error: fn() -> SecretError) -> NativeConnect {
    Arc::new(move |_service: &str| Err(error()))
}

fn options(dir: &std::path::Path, connect: NativeConnect) -> OpenOptions {
    OpenOptions {
        native: true,
        native_connect: Some(connect),
        open_timeout: OPEN_TIMEOUT,
        call_timeout: CALL_TIMEOUT,
        allow_fallback: true,
        ..OpenOptions::new(Some(dir.to_path_buf()))
    }
}

#[test]
fn a_keychain_that_never_answers_falls_back_within_the_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let keychain = Arc::new(FakeKeychain::default());
    keychain.hang.lock().unwrap().push("connect");

    let started = Instant::now();
    let opened = open(options(tmp.path(), connect_to(&keychain))).unwrap();
    assert!(
        started.elapsed() < OPEN_TIMEOUT + SLACK,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(opened.backend(), Backend::EncryptedFile);
    assert_eq!(opened.fallback_kind, Some(FallbackKind::TimedOut));
    assert!(
        opened
            .fallback_reason
            .as_deref()
            .unwrap()
            .contains("did not answer"),
        "{:?}",
        opened.fallback_reason
    );
    assert_eq!(
        opened.fallback_file.as_deref(),
        Some(tmp.path().join(pa_secrets::SECRETS_FILE).as_path())
    );

    // The source password is stored at once, in the file.
    let key = SecretKey::source_password("7").unwrap();
    opened.store.set(&key, &"Sm8Pass".into()).unwrap();
    assert!(EncryptedFileStore::exists(tmp.path()));
}

#[test]
fn missing_and_locked_keychains_are_told_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let cases: [(fn() -> SecretError, FallbackKind); 3] = [
        (
            || SecretError::Unavailable("no session bus".into()),
            FallbackKind::Unavailable,
        ),
        (
            || SecretError::Locked("default keyring is locked".into()),
            FallbackKind::Locked,
        ),
        (
            || SecretError::Backend("D-Bus error".into()),
            FallbackKind::Failed,
        ),
    ];
    for (error, kind) in cases {
        let opened = open(options(tmp.path(), failing(error))).unwrap();
        assert_eq!(opened.backend(), Backend::EncryptedFile);
        assert_eq!(opened.fallback_kind, Some(kind));
    }

    // Without the fallback the kind stays visible in the error.
    let mut no_fallback = options(tmp.path(), failing(|| SecretError::Locked("x".into())));
    no_fallback.allow_fallback = false;
    assert!(matches!(open(no_fallback), Err(SecretError::Locked(_))));
    let keychain = Arc::new(FakeKeychain::default());
    keychain.hang.lock().unwrap().push("connect");
    let mut no_fallback = options(tmp.path(), connect_to(&keychain));
    no_fallback.allow_fallback = false;
    assert!(matches!(open(no_fallback), Err(SecretError::Timeout(_))));
}

#[test]
fn a_keychain_call_that_hangs_fails_with_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let keychain = Arc::new(FakeKeychain::default());
    let opened = open(options(tmp.path(), connect_to(&keychain))).unwrap();
    assert_eq!(opened.backend(), Backend::SecretService);
    assert_eq!(opened.fallback_kind, None);

    let key = SecretKey::source_password("1").unwrap();
    opened.store.set(&key, &"first".into()).unwrap();

    // The keyring gets locked and writing waits for an unseen prompt.
    keychain.hang.lock().unwrap().push("set");
    let started = Instant::now();
    let err = opened.store.set(&key, &"second".into()).unwrap_err();
    assert!(matches!(err, SecretError::Timeout(_)), "{err:?}");
    assert!(started.elapsed() < CALL_TIMEOUT + SLACK);

    // The stuck call is still pending: further calls fail at once.
    let started = Instant::now();
    assert!(matches!(
        opened.store.get(&key),
        Err(SecretError::Timeout(_))
    ));
    assert!(started.elapsed() < SLACK);
}

#[test]
fn the_fallback_file_round_trips_across_restarts() {
    let tmp = tempfile::tempdir().unwrap();
    let unavailable = || failing(|| SecretError::Unavailable("no Secret Service".into()));
    let key = SecretKey::twitch_tokens("99").unwrap();
    let value = SecretString::new(r#"{"access":"a-tok","refresh":"r-tok"}"#);
    {
        let opened = open(options(tmp.path(), unavailable())).unwrap();
        opened.store.set(&key, &value).unwrap();
        // Encrypted on disk.
        let raw = std::fs::read(tmp.path().join(pa_secrets::SECRETS_FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("a-tok"));
    }
    let opened = open(options(tmp.path(), unavailable())).unwrap();
    assert_eq!(opened.backend(), Backend::EncryptedFile);
    assert_eq!(opened.store.get(&key).unwrap().unwrap(), value);
    assert!(opened.store.delete(&key).unwrap());
    assert_eq!(opened.store.get(&key).unwrap(), None);
}

#[test]
fn secrets_move_into_the_keychain_once_it_works() {
    let tmp = tempfile::tempdir().unwrap();
    let keychain = Arc::new(FakeKeychain::default());
    let a = SecretKey::source_password("a").unwrap();
    let b = SecretKey::tmdb_api_key();

    // First start: the keyring is locked.
    {
        let locked = failing(|| SecretError::Locked("default keyring is locked".into()));
        let opened = open(options(tmp.path(), locked)).unwrap();
        assert_eq!(opened.fallback_kind, Some(FallbackKind::Locked));
        opened.store.set(&a, &"pass-a".into()).unwrap();
        opened.store.set(&b, &"key-b".into()).unwrap();
    }

    // Next start: the keychain answers; the file moves into it.
    let opened = open(options(tmp.path(), connect_to(&keychain))).unwrap();
    assert_eq!(opened.backend(), Backend::SecretService);
    assert_eq!(opened.fallback_reason, None);
    assert_eq!(opened.migrated, 2);
    assert!(!EncryptedFileStore::exists(tmp.path()));
    assert!(!tmp.path().join(pa_secrets::KEY_FILE).exists());
    assert_eq!(opened.store.get(&a).unwrap().unwrap().expose(), "pass-a");
    assert_eq!(
        keychain.items.lock().unwrap()["provider.api_key:tmdb"],
        "key-b"
    );
}

#[test]
fn a_keychain_that_hangs_during_migration_keeps_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let a = SecretKey::source_password("a").unwrap();
    {
        let file = EncryptedFileStore::open(tmp.path()).unwrap();
        file.set(&a, &"pass-a".into()).unwrap();
    }
    let keychain = Arc::new(FakeKeychain::default());
    keychain.hang.lock().unwrap().push("set");

    let started = Instant::now();
    let opened = open(options(tmp.path(), connect_to(&keychain))).unwrap();
    assert!(started.elapsed() < OPEN_TIMEOUT + CALL_TIMEOUT + SLACK);
    assert_eq!(opened.backend(), Backend::EncryptedFile);
    assert_eq!(opened.fallback_kind, Some(FallbackKind::TimedOut));
    assert_eq!(opened.store.get(&a).unwrap().unwrap().expose(), "pass-a");
}

/// The real Secret Service client in a child process, with the D-Bus
/// environment of the parent test.
#[cfg(target_os = "linux")]
mod real_client {
    use std::os::unix::net::UnixListener;
    use std::process::{Command, Stdio};

    use super::*;

    const CHILD_ENV: &str = "PA_SECRETS_HANG_CHILD";

    /// Runs [`child_opens_the_os_keychain`] in a child process with `env`
    /// changes and returns its `RESULT` line.
    fn run_child(configure: impl FnOnce(&mut Command)) -> String {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "real_client::child_opens_the_os_keychain",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1");
        configure(&mut cmd);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = cmd.spawn().unwrap();
        // The bug this guards against is a hang: kill the child instead of
        // hanging the test run.
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(60) {
                child.kill().unwrap();
                let out = child.wait_with_output().unwrap();
                panic!(
                    "the child hung for 60 s:\n{}",
                    String::from_utf8_lossy(&out.stdout)
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let out = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "child failed after {:?}:\n{stdout}\n{}",
            started.elapsed(),
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
            .split("RESULT ")
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .unwrap_or_else(|| panic!("no RESULT line:\n{stdout}"))
            .to_owned()
    }

    /// Not a test of its own: does the work when started by [`run_child`].
    #[test]
    fn child_opens_the_os_keychain() {
        if std::env::var(CHILD_ENV).is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let options = OpenOptions {
            service: format!("pa-secrets-hang-test-{}", std::process::id()),
            open_timeout: Duration::from_secs(1),
            call_timeout: Duration::from_secs(1),
            ..OpenOptions::new(Some(tmp.path().to_path_buf()))
        };
        let started = Instant::now();
        let opened = open(options).unwrap();
        let elapsed = started.elapsed();
        // A write must not hang either.
        let key = SecretKey::source_password("child").unwrap();
        opened.store.set(&key, &"child-pass".into()).unwrap();
        println!(
            "\nRESULT backend={} kind={} open_ms={}",
            opened.backend(),
            opened.fallback_kind.map_or("none", FallbackKind::as_str),
            elapsed.as_millis()
        );
    }

    fn open_ms(result: &str) -> u64 {
        result
            .split("open_ms=")
            .nth(1)
            .and_then(|v| v.trim().parse().ok())
            .unwrap()
    }

    #[test]
    fn no_session_bus_means_unavailable() {
        let runtime = tempfile::tempdir().unwrap();
        let result = run_child(|cmd| {
            // zbus falls back to $XDG_RUNTIME_DIR/bus without the variable.
            cmd.env_remove("DBUS_SESSION_BUS_ADDRESS")
                .env("XDG_RUNTIME_DIR", runtime.path());
        });
        assert!(result.contains("backend=encrypted-file"), "{result}");
        assert!(result.contains("kind=unavailable"), "{result}");
        assert!(
            open_ms(&result) < 1000 + SLACK.as_millis() as u64,
            "{result}"
        );
    }

    #[test]
    fn a_session_bus_that_never_answers_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("bus");
        let listener = UnixListener::bind(&socket).unwrap();
        // Accepts every connection and never says a word (not even the
        // SASL handshake): a hung D-Bus daemon.
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                held.push(stream);
            }
        });
        let result = run_child(|cmd| {
            cmd.env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", socket.display()),
            );
        });
        assert!(result.contains("backend=encrypted-file"), "{result}");
        assert!(result.contains("kind=timeout"), "{result}");
        assert!(
            open_ms(&result) < 1000 + SLACK.as_millis() as u64,
            "{result}"
        );
    }
}
