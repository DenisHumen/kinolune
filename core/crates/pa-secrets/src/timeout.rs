// SPDX-License-Identifier: MPL-2.0
//! Deadlines for keychain calls.
//!
//! A desktop keychain can block forever: a locked GNOME keyring waits for an
//! unlock prompt that a headless session never shows, a Secret Service
//! daemon may accept the D-Bus connection and never answer. A blocked call
//! cannot be cancelled, so each call runs on a thread of its own and the
//! caller waits at most a deadline. A call that missed its deadline keeps
//! its thread until the keychain answers (possibly never); while one is
//! pending the store refuses new calls at once instead of piling up threads
//! behind the same stuck connection.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use crate::{Backend, Result, SecretError, SecretKey, SecretStore, SecretString};

/// Default deadline of [`open`](crate::open) for connecting to the OS
/// keychain and checking that it answers.
pub const DEFAULT_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Default deadline of one keychain call (`get`, `set`, `delete`).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Result slot shared by the caller and the worker thread.
struct Slot<T> {
    result: Option<Result<T>>,
    /// The caller gave up; the worker must release the "stuck" count.
    abandoned: bool,
}

/// Runs `f` on a new thread and waits at most `timeout` for it.
///
/// On timeout returns [`SecretError::Timeout`] naming `what`; the thread
/// keeps running. `stuck` (if any) counts calls that missed their deadline
/// and have not returned yet.
pub(crate) fn run_with_timeout<T, F>(
    what: &str,
    timeout: Duration,
    stuck: Option<Arc<AtomicUsize>>,
    f: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let shared = Arc::new((
        Mutex::new(Slot {
            result: None,
            abandoned: false,
        }),
        Condvar::new(),
    ));
    let worker = Arc::clone(&shared);
    let worker_stuck = stuck.clone();
    thread::Builder::new()
        .name("pa-secrets-call".into())
        .spawn(move || {
            let result = f();
            let (lock, cvar) = &*worker;
            let mut slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
            if slot.abandoned {
                if let Some(stuck) = worker_stuck {
                    stuck.fetch_sub(1, Ordering::SeqCst);
                }
            } else {
                slot.result = Some(result);
                cvar.notify_all();
            }
        })?;

    let (lock, cvar) = &*shared;
    let slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut slot, _) = cvar
        .wait_timeout_while(slot, timeout, |s| s.result.is_none())
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(result) = slot.result.take() {
        return result;
    }
    slot.abandoned = true;
    if let Some(stuck) = stuck {
        stuck.fetch_add(1, Ordering::SeqCst);
    }
    Err(SecretError::Timeout(format!(
        "{what} took longer than {} s",
        timeout.as_secs_f32()
    )))
}

/// A store whose calls fail with [`SecretError::Timeout`] instead of
/// blocking past a deadline. [`open`](crate::open) wraps the OS keychain in
/// it.
pub struct TimeoutStore {
    inner: Arc<dyn SecretStore>,
    timeout: Duration,
    stuck: Arc<AtomicUsize>,
}

impl TimeoutStore {
    /// Wraps `inner`; each call waits at most `timeout`.
    pub fn new(inner: Arc<dyn SecretStore>, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            stuck: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Whether an earlier call missed its deadline and has not returned
    /// yet. New calls fail at once while this is true.
    pub fn is_stuck(&self) -> bool {
        self.stuck.load(Ordering::SeqCst) > 0
    }

    fn call<T, F>(&self, op: &str, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&dyn SecretStore) -> Result<T> + Send + 'static,
    {
        let backend = self.inner.backend();
        if self.is_stuck() {
            return Err(SecretError::Timeout(format!(
                "{backend} has not answered an earlier call yet"
            )));
        }
        let inner = Arc::clone(&self.inner);
        run_with_timeout(
            &format!("{backend} {op}"),
            self.timeout,
            Some(Arc::clone(&self.stuck)),
            move || f(inner.as_ref()),
        )
    }
}

impl fmt::Debug for TimeoutStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimeoutStore")
            .field("inner", &self.inner)
            .field("timeout", &self.timeout)
            .field("stuck", &self.stuck.load(Ordering::SeqCst))
            .finish()
    }
}

impl SecretStore for TimeoutStore {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>> {
        let key = key.clone();
        self.call("get", move |s| s.get(&key))
    }

    fn set(&self, key: &SecretKey, value: &SecretString) -> Result<()> {
        value.check_len()?;
        let (key, value) = (key.clone(), value.clone());
        self.call("set", move |s| s.set(&key, &value))
    }

    fn delete(&self, key: &SecretKey) -> Result<bool> {
        let key = key.clone();
        self.call("delete", move |s| s.delete(&key))
    }

    fn backend(&self) -> Backend {
        self.inner.backend()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::MemoryStore;

    /// Answers like [`MemoryStore`], but `set` of `hang` blocks until the
    /// test releases it.
    #[derive(Debug)]
    struct Gate {
        inner: MemoryStore,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl SecretStore for Gate {
        fn get(&self, key: &SecretKey) -> Result<Option<SecretString>> {
            self.inner.get(key)
        }
        fn set(&self, key: &SecretKey, value: &SecretString) -> Result<()> {
            if key.as_str() == "hang" {
                let (lock, cvar) = &*self.release;
                let open = lock.lock().unwrap();
                drop(cvar.wait_while(open, |open| !*open).unwrap());
            }
            self.inner.set(key, value)
        }
        fn delete(&self, key: &SecretKey) -> Result<bool> {
            self.inner.delete(key)
        }
        fn backend(&self) -> Backend {
            Backend::SecretService
        }
    }

    #[test]
    fn a_stuck_call_times_out_and_blocks_further_calls_until_it_returns() {
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let store = TimeoutStore::new(
            Arc::new(Gate {
                inner: MemoryStore::new(),
                release: Arc::clone(&release),
            }),
            Duration::from_millis(200),
        );
        let ok = SecretKey::new("ok").unwrap();
        store.set(&ok, &"v".into()).unwrap();
        assert_eq!(store.get(&ok).unwrap().unwrap().expose(), "v");

        let started = Instant::now();
        let err = store
            .set(&SecretKey::new("hang").unwrap(), &"x".into())
            .unwrap_err();
        assert!(matches!(err, SecretError::Timeout(_)), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(store.is_stuck());

        // Fails at once while the first call is still pending.
        let started = Instant::now();
        assert!(matches!(store.get(&ok), Err(SecretError::Timeout(_))));
        assert!(started.elapsed() < Duration::from_millis(100));

        // The keychain answers after all: the store works again.
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        let deadline = Instant::now() + Duration::from_secs(5);
        while store.is_stuck() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!store.is_stuck());
        assert_eq!(store.get(&ok).unwrap().unwrap().expose(), "v");
    }

    #[test]
    fn errors_of_the_inner_store_pass_through() {
        let store = TimeoutStore::new(Arc::new(MemoryStore::new()), Duration::from_secs(5));
        let big = SecretString::new("x".repeat(crate::MAX_SECRET_LEN + 1));
        assert!(matches!(
            store.set(&SecretKey::new("k").unwrap(), &big),
            Err(SecretError::TooLarge { .. })
        ));
        assert_eq!(store.backend(), Backend::Memory);
    }
}
