// SPDX-License-Identifier: MPL-2.0
//! Background workers that fetch read-ahead blocks in parallel.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::thread;

use crate::source::SourceShared;

/// One block to fetch: the source (weak, so a closed file cancels its jobs)
/// and the reader generation it was scheduled for.
pub(crate) struct Job {
    pub(crate) source: Weak<SourceShared>,
    pub(crate) chunk: u64,
    pub(crate) generation: u64,
}

struct Queue {
    jobs: VecDeque<Job>,
    closed: bool,
    workers: usize,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    /// Worker limit, set by the tuning (NET-021); extra workers exit when
    /// they would wait for a job.
    max_workers: AtomicUsize,
}

/// Worker threads per [`SmbFs`](crate::SmbFs), started on demand up to
/// the current limit and stopped when the file system is dropped.
pub(crate) struct Prefetcher {
    shared: Arc<Shared>,
    enabled: bool,
}

impl Prefetcher {
    pub(crate) fn new(max_workers: usize) -> Self {
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    closed: false,
                    workers: 0,
                }),
                ready: Condvar::new(),
                max_workers: AtomicUsize::new(max_workers),
            }),
            enabled: max_workers > 0,
        }
    }

    /// `false` when background prefetch is off.
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Changes the number of workers (blocks fetched in parallel).
    pub(crate) fn set_workers(&self, n: usize) {
        if !self.enabled {
            return;
        }
        self.shared.max_workers.store(n.max(1), Ordering::Relaxed);
        let q = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let more = q.jobs.len();
        drop(q);
        // Extra workers notice the lower limit; new jobs start more.
        self.spawn(more);
        self.shared.ready.notify_all();
    }

    /// Starts workers up to the limit for `pending` queued jobs.
    fn spawn(&self, pending: usize) {
        let mut q = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let max = self.shared.max_workers.load(Ordering::Relaxed);
        while !q.closed && q.workers < max.min(pending) {
            let shared = Arc::clone(&self.shared);
            let spawned = thread::Builder::new()
                .name("pa-smb-prefetch".into())
                .spawn(move || worker(&shared));
            if spawned.is_err() {
                break; // Readers fetch synchronously instead.
            }
            q.workers += 1;
        }
    }

    /// Queues `jobs`, starting workers up to the limit.
    pub(crate) fn submit(&self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        let mut q = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if q.closed {
            return;
        }
        q.jobs.extend(jobs);
        let pending = q.jobs.len();
        drop(q);
        self.spawn(pending);
        self.shared.ready.notify_all();
    }

    /// Stops the workers after their current job; queued jobs are dropped.
    pub(crate) fn close(&self) {
        let mut q = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        q.closed = true;
        let dropped: Vec<Job> = q.jobs.drain(..).collect();
        drop(q);
        self.shared.ready.notify_all();
        for job in dropped {
            if let Some(src) = job.source.upgrade() {
                src.cancel(job.chunk);
            }
        }
    }
}

fn worker(shared: &Shared) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if q.closed || q.workers > shared.max_workers.load(Ordering::Relaxed) {
                    q.workers -= 1;
                    return;
                }
                if let Some(job) = q.jobs.pop_front() {
                    break job;
                }
                q = shared.ready.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        if let Some(src) = job.source.upgrade() {
            src.prefetch(job.chunk, job.generation);
        }
    }
}
