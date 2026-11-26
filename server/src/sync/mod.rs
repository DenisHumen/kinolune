// SPDX-License-Identifier: MPL-2.0
//! Sync hub (SYNC-008): the sync protocol (`docs/api/sync-protocol.md`, §8) over
//! WebSocket at [`PATH`].
//!
//! The server is the hub for devices that know it (ADR-0004, mode B):
//!
//! - **ops** go into a SQLite journal ([`journal`], `sync.sqlite3` in the
//!   data directory), are acked with their `seq` and fanned out to the other
//!   connected devices; duplicates (the same op also arriving over P2P or a
//!   resend from an outbox) are recognised by `op_id` and get the same `seq`.
//!   Plain progress ops are also written into the server library
//!   (`pa_library::progress::apply_remote`), so the server knows where each
//!   profile stopped in its own library.
//! - **`pb.update` / `pb.takeover`** are forwarded to the other devices at
//!   once and never stored on disk. The last `pb.update` per
//!   `(profile, key)` stays in memory for [`SyncConfig::playback_ttl_secs`]
//!   and is sent to each device right after the handshake (instant resume).
//!
//! - **Authentication** (SYNC-005): the hub's `hello` carries a nonce; a
//!   device answers with `auth`, a signature of its paired device key, and
//!   the hub proves itself the same way. Unknown and revoked devices get
//!   `error{unauthorized}` and are disconnected; a device revoked while
//!   connected is disconnected at once. Paired devices get a `family`
//!   update (roster and family key) after the handshake and after every
//!   change. [`SyncConfig::require_pairing`] = false lets unpaired devices
//!   in (tests, a trusted loopback setup).
//! - **Node logs** (SYNC-017, protocol 2): the hub keeps every origin's log
//!   ([`journal`], `node_log`) and its version vector. A device of
//!   protocol 2 sends its vector in `hello.vv` and gets the entries it
//!   lacks; `op` frames carry their log place, a damaged entry is not
//!   acked and is asked for again (`want`); `vv`, `want` and `digest` run
//!   anti-entropy. Devices of protocol 1 keep the cursor path unchanged;
//!   their ops go into the hub's own log.

pub mod journal;
mod playback;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message as WsMessage, Utf8Bytes, WebSocket};
use pa_sync::nodelog::{Inserted, Range};
use pa_sync::proto::{
    self, Ack, Auth, Change, DigestFrame, ErrorFrame, Family, Hello, Message, OpFrame, ProtoRange,
    Role, code,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, watch};

pub use crate::config::SyncConfig;
use crate::pairing::{FamilyEvent, Pairing};
use crate::{Error, Library};
use journal::{JournalError, SqliteJournal};
use playback::PlaybackCache;

/// WebSocket endpoint of the hub.
pub const PATH: &str = "/api/v1/sync";

/// Journal file inside the data directory.
pub const JOURNAL_FILE: &str = "sync.sqlite3";

/// How long a device has to send its `hello`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// The hub pings idle connections this often…
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// …and drops a connection that sent nothing (not even a pong) this long.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Fan-out queue per connection; a device that falls further behind gets
/// `change{head}` and pulls the ops it missed.
const FANOUT_CAPACITY: usize = 4096;

/// One frame to fan out, already encoded once for every receiver.
#[derive(Clone)]
struct Event {
    /// Connection that sent it (it does not get its own frame back).
    from: u64,
    frame: Utf8Bytes,
}

struct Inner {
    journal: Mutex<SqliteJournal>,
    hub_id: String,
    library: Library,
    playback: Mutex<PlaybackCache>,
    events: broadcast::Sender<Event>,
    slots: Arc<Semaphore>,
    max_connections: u32,
    shutdown: watch::Sender<bool>,
    next_conn: std::sync::atomic::AtomicU64,
    pairing: Pairing,
    require_pairing: bool,
}

/// The sync hub; cheap to clone.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

impl Hub {
    /// A hub with the journal at `journal` that writes progress into
    /// `library`, with the device family in `data_dir` (founded on first
    /// start; `addr` goes into pairing QR codes). Blocking (opens SQLite).
    pub fn open(
        journal: &std::path::Path,
        library: Library,
        config: &SyncConfig,
        data_dir: &std::path::Path,
        addr: Option<String>,
    ) -> Result<Self, Error> {
        let journal = SqliteJournal::open(journal).map_err(|source| Error::Sync {
            path: journal.to_owned(),
            source,
        })?;
        let pairing = Pairing::open(data_dir, journal.hub_id(), addr)?;
        let (events, _) = broadcast::channel(FANOUT_CAPACITY);
        let max_connections = config.max_connections.max(1);
        Ok(Self {
            inner: Arc::new(Inner {
                hub_id: journal.hub_id().to_owned(),
                journal: Mutex::new(journal),
                library,
                playback: Mutex::new(PlaybackCache::new(
                    Duration::from_secs(config.playback_ttl_secs),
                    config.playback_max_entries,
                )),
                events,
                slots: Arc::new(Semaphore::new(max_connections as usize)),
                max_connections,
                shutdown: watch::channel(false).0,
                next_conn: std::sync::atomic::AtomicU64::new(1),
                pairing,
                require_pairing: config.require_pairing,
            }),
        })
    }

    /// Titles that devices are watching right now: their last `pb.update`
    /// is `playing` or `paused` and within its TTL (SRV-019).
    pub fn watching_now(&self) -> usize {
        self.inner
            .playback
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .watching(Instant::now())
    }

    /// `device_id` of the hub.
    pub fn hub_id(&self) -> &str {
        &self.inner.hub_id
    }

    /// The device family of the server.
    pub fn pairing(&self) -> &Pairing {
        &self.inner.pairing
    }

    /// A connection slot, or `None` when [`SyncConfig::max_connections`]
    /// devices are connected or the hub is shutting down.
    pub(crate) fn try_slot(&self) -> Option<OwnedSemaphorePermit> {
        if *self.inner.shutdown.borrow() {
            return None;
        }
        Arc::clone(&self.inner.slots).try_acquire_owned().ok()
    }

    /// Tells every connection to close.
    pub(crate) fn close(&self) {
        self.inner.shutdown.send_replace(true);
    }

    /// Waits (up to `timeout`) for the connections to close, then
    /// checkpoints the journal.
    pub(crate) async fn drain(&self, timeout: Duration) {
        let all = Arc::clone(&self.inner.slots).acquire_many_owned(self.inner.max_connections);
        if tokio::time::timeout(timeout, all).await.is_err() {
            tracing::warn!("sync connections still open at shutdown");
        }
        let hub = self.clone();
        let done = tokio::task::spawn_blocking(move || hub.journal().checkpoint()).await;
        if let Ok(Err(e)) = done {
            tracing::warn!(error = %e, "sync journal checkpoint failed");
        }
    }

    fn journal(&self) -> std::sync::MutexGuard<'_, SqliteJournal> {
        self.inner.journal.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn blocking<T, F>(&self, f: F) -> Result<T, JournalError>
    where
        T: Send + 'static,
        F: FnOnce(&mut SqliteJournal) -> Result<T, JournalError> + Send + 'static,
    {
        let hub = self.clone();
        tokio::task::spawn_blocking(move || f(&mut hub.journal()))
            .await
            .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
    }

    fn head(&self) -> proto::Cursor {
        self.journal().head()
    }

    fn hello(&self, nonce: &str) -> Message {
        let (head, vv) = {
            let j = self.journal();
            (j.head(), j.vector())
        };
        Message::Hello(Hello {
            proto: ProtoRange::LOCAL,
            device_id: self.inner.hub_id.clone(),
            role: Role::Hub,
            cursor: None,
            head: Some(head),
            nonce: Some(nonce.to_owned()),
            // Protocol 1 devices ignore it.
            vv: Some(vv),
        })
    }

    fn publish(&self, from: u64, frame: Utf8Bytes) {
        // No receivers is fine: nobody else is connected.
        let _ = self.inner.events.send(Event { from, frame });
    }

    /// Serves one device until it disconnects or the hub shuts down.
    pub(crate) async fn serve(self, socket: WebSocket, slot: OwnedSemaphorePermit) {
        let id = self
            .inner
            .next_conn
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut conn = Connection {
            hub: self,
            socket,
            id,
            device: None,
            authenticated: false,
            last_seen: Instant::now(),
            version: 1,
        };
        match conn.run().await {
            Ok(()) => tracing::debug!(conn = id, device = conn.device(), "sync: closed"),
            Err(e) => {
                tracing::info!(conn = id, device = conn.device(), reason = %e, "sync: dropped")
            }
        }
        drop(slot);
    }
}

/// Why a connection ended early.
#[derive(Debug, thiserror::Error)]
enum ConnError {
    #[error("socket: {0}")]
    Socket(#[from] axum::Error),
    #[error("{0}")]
    Protocol(&'static str),
    #[error("random generator: {0}")]
    Random(String),
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn encode(msg: &Message) -> Utf8Bytes {
    // Messages built by the hub are always valid.
    proto::encode(msg).expect("hub messages encode").into()
}

fn error(code: &str, message: Option<&str>, op_id: Option<pa_sync::OpId>) -> Message {
    Message::Error(ErrorFrame {
        code: code.to_owned(),
        message: message.map(str::to_owned),
        op_id,
    })
}

struct Connection {
    hub: Hub,
    socket: WebSocket,
    id: u64,
    device: Option<String>,
    /// The device proved its paired key.
    authenticated: bool,
    last_seen: Instant,
    /// Protocol version agreed with the device.
    version: u16,
}

impl Connection {
    fn device(&self) -> &str {
        self.device.as_deref().unwrap_or("-")
    }

    async fn send(&mut self, msg: &Message) -> Result<(), ConnError> {
        self.send_raw(encode(msg)).await
    }

    async fn send_raw(&mut self, frame: Utf8Bytes) -> Result<(), ConnError> {
        Ok(self.socket.send(WsMessage::Text(frame)).await?)
    }

    /// Sends an error and closes the connection.
    async fn fail(&mut self, code: &'static str, message: &str) -> Result<(), ConnError> {
        self.send(&error(code, Some(message), None)).await?;
        let _ = self.socket.send(WsMessage::Close(None)).await;
        Err(ConnError::Protocol(code))
    }

    /// Next text frame; `None` when the peer closed. Pings, pongs and
    /// binary frames are handled here.
    async fn recv(&mut self) -> Result<Option<Utf8Bytes>, ConnError> {
        loop {
            let Some(msg) = self.socket.recv().await else {
                return Ok(None);
            };
            self.last_seen = Instant::now();
            match msg? {
                WsMessage::Text(text) => return Ok(Some(text)),
                WsMessage::Binary(_) => {
                    self.send(&error(code::BAD_MESSAGE, Some("text frames only"), None))
                        .await?;
                }
                WsMessage::Close(_) => return Ok(None),
                WsMessage::Ping(_) | WsMessage::Pong(_) => {}
            }
        }
    }

    async fn run(&mut self) -> Result<(), ConnError> {
        // Subscribe before the handshake so nothing published meanwhile is
        // lost: it is delivered right after the handshake.
        let mut events = self.hub.inner.events.subscribe();
        let mut family = self.hub.inner.pairing.events();
        let mut shutdown = self.hub.inner.shutdown.subscribe();
        let nonce = pa_pairing::auth::nonce().map_err(|e| ConnError::Random(e.to_string()))?;
        let hello = self.hub.hello(&nonce);
        self.send(&hello).await?;

        // Handshake: both sides send `hello` first (spec §4.1).
        let remote = match tokio::time::timeout(HANDSHAKE_TIMEOUT, self.handshake()).await {
            Ok(Ok(Some(hello))) => hello,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(e)) => return Err(e),
            Err(_) => return self.fail(code::HANDSHAKE, "no hello").await,
        };
        let version = match proto::negotiate(ProtoRange::LOCAL, remote.proto) {
            Ok(v) => v,
            Err(e) => return self.fail(code::PROTO_UNSUPPORTED, &e.to_string()).await,
        };
        self.device = Some(remote.device_id.clone());
        self.version = version;
        if let Some(device_nonce) = &remote.nonce {
            // Prove the hub to the device, then check the device.
            let sig = self
                .hub
                .inner
                .pairing
                .prove(&remote.device_id, &nonce, device_nonce);
            self.send(&Message::Auth(Auth { sig })).await?;
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, self.next_message()).await {
                Ok(Ok(Some(Message::Auth(auth)))) => {
                    match self.hub.inner.pairing.verify(
                        &remote.device_id,
                        device_nonce,
                        &nonce,
                        &auth.sig,
                    ) {
                        Ok(()) => self.authenticated = true,
                        Err(e) if self.hub.inner.require_pairing => {
                            tracing::info!(conn = self.id, device = %remote.device_id, reason = %e, "sync: unauthorized");
                            return self.fail(code::UNAUTHORIZED, "device is not paired").await;
                        }
                        Err(e) => {
                            tracing::debug!(conn = self.id, reason = %e, "sync: unpaired device admitted");
                        }
                    }
                }
                Ok(Ok(None)) => return Ok(()),
                Ok(Err(e)) => return Err(e),
                Ok(Ok(Some(_))) | Err(_) => {
                    return self.fail(code::UNAUTHORIZED, "auth expected").await;
                }
            }
        } else if self.hub.inner.require_pairing {
            return self.fail(code::UNAUTHORIZED, "pairing required").await;
        }
        tracing::info!(conn = self.id, device = %remote.device_id, version, paired = self.authenticated, "sync: connected");
        if self.authenticated {
            self.send_family().await?;
        }

        // Instant resume: what is playing right now on other devices.
        let recent = {
            let mut cache = self
                .hub
                .inner
                .playback
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            cache.fresh(Instant::now())
        };
        for frame in recent {
            self.send_raw(frame).await?;
        }
        // SYNC-017: what the device's vector lacks, and the holes the hub
        // would like filled.
        if version >= 2
            && let Some(vv) = remote.vv.clone()
        {
            let (missing, gaps) = {
                let j = self.hub.journal();
                (j.missing_for(&vv), j.gaps())
            };
            for e in missing {
                self.send(&Message::Op(e.frame())).await?;
            }
            for gap in gaps {
                self.send(&Message::Want(gap)).await?;
            }
        }
        // Tell a device that is behind to pull (it may pull anyway).
        let head = self.hub.head();
        let behind = match &remote.cursor {
            Some(c) => c.log != head.log || c.seq < head.seq,
            None => head.seq > 0,
        };
        if behind {
            self.send(&Message::Change(Change { head })).await?;
        }

        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.tick().await;
        loop {
            tokio::select! {
                incoming = self.recv() => {
                    let Some(text) = incoming? else { return Ok(()) };
                    self.handle(text).await?;
                }
                event = events.recv() => match event {
                    Ok(event) if event.from == self.id => {}
                    Ok(event) => self.send_raw(event.frame).await?,
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::debug!(conn = self.id, missed, "sync: fan-out lagged");
                        let head = self.hub.head();
                        self.send(&Message::Change(Change { head })).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
                event = family.recv() => match event {
                    Ok(FamilyEvent::Revoked(ids)) if ids.iter().any(|id| Some(id) == self.device.as_ref()) => {
                        tracing::info!(conn = self.id, device = self.device(), "sync: device revoked");
                        return self.fail(code::UNAUTHORIZED, "device revoked").await;
                    }
                    Ok(FamilyEvent::Changed) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        if self.authenticated {
                            self.send_family().await?;
                        }
                    }
                    Ok(FamilyEvent::Revoked(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
                _ = ping.tick() => {
                    if self.last_seen.elapsed() > IDLE_TIMEOUT {
                        return Err(ConnError::Protocol("idle"));
                    }
                    self.socket.send(WsMessage::Ping(Default::default())).await?;
                }
                () = async { let _ = shutdown.wait_for(|stop| *stop).await; } => {
                    let _ = self.socket.send(WsMessage::Close(None)).await;
                    return Ok(());
                }
            }
        }
    }

    /// Sends the device its `family` update (roster and sealed key).
    async fn send_family(&mut self) -> Result<(), ConnError> {
        let device = self.device().to_owned();
        let pairing = self.hub.inner.pairing.clone();
        let body = tokio::task::spawn_blocking(move || pairing.update_for(&device))
            .await
            .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
        match body {
            Some(body) => self.send(&Message::Family(Family { body })).await,
            None => Ok(()),
        }
    }

    /// Next message that decodes; bad frames get `bad_message`. `None`:
    /// the device closed.
    async fn next_message(&mut self) -> Result<Option<Message>, ConnError> {
        loop {
            let Some(text) = self.recv().await? else {
                return Ok(None);
            };
            match proto::decode(text.as_str()) {
                Ok(msg) => return Ok(Some(msg)),
                Err(e) => {
                    self.send(&error(code::BAD_MESSAGE, Some(&e.to_string()), None))
                        .await?;
                }
            }
        }
    }

    /// Waits for the device's `hello`. `None`: the device closed.
    async fn handshake(&mut self) -> Result<Option<Hello>, ConnError> {
        loop {
            let Some(text) = self.recv().await? else {
                return Ok(None);
            };
            match proto::decode(text.as_str()) {
                Ok(Message::Hello(hello)) => return Ok(Some(hello)),
                Ok(_) => {
                    self.fail(code::HANDSHAKE, "hello expected").await?;
                }
                Err(e) => {
                    self.send(&error(code::BAD_MESSAGE, Some(&e.to_string()), None))
                        .await?;
                }
            }
        }
    }

    async fn handle(&mut self, text: Utf8Bytes) -> Result<(), ConnError> {
        let msg = match proto::decode(text.as_str()) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::debug!(conn = self.id, error = %e, "sync: bad frame");
                return self
                    .send(&error(code::BAD_MESSAGE, Some(&e.to_string()), None))
                    .await;
            }
        };
        match msg {
            Message::PbUpdate(update) => {
                {
                    let mut cache = self
                        .hub
                        .inner
                        .playback
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    cache.put(&update, text.clone(), Instant::now());
                }
                // The original frame: unknown fields of newer clients survive.
                self.hub.publish(self.id, text);
            }
            // UI-027: remote text input, sealed; MUS-015: a music queue
            // handed to another device, sealed; LIB-028: live upload
            // progress. Relayed unread, never stored.
            Message::PbTakeover(_) | Message::Input(_) | Message::Music(_) | Message::Upload(_) => {
                self.hub.publish(self.id, text)
            }
            Message::Op(frame) => self.store(frame).await?,
            Message::Pull(pull) => {
                let reply = self.hub.blocking(move |j| j.pull(&pull)).await;
                match reply {
                    Ok((frames, end)) => {
                        for frame in frames {
                            self.send(&Message::Op(frame)).await?;
                        }
                        self.send(&Message::PullEnd(end)).await?;
                    }
                    Err(e) => tracing::error!(conn = self.id, error = %e, "sync: pull failed"),
                }
            }
            Message::Vv(v) => {
                let missing = self.hub.journal().missing_for(&v.vv);
                for e in missing {
                    self.send(&Message::Op(e.frame())).await?;
                }
            }
            Message::Want(range) => {
                let found = self.hub.journal().range(&range);
                for e in found {
                    self.send(&Message::Op(e.frame())).await?;
                }
            }
            Message::Digest(d) => {
                let (mine, repair) = self.hub.journal().digest_diff(&d.digest);
                let differs = !mine.diff(&d.digest).is_empty();
                if differs {
                    tracing::info!(
                        conn = self.id,
                        entries = repair.len(),
                        "sync: digests differ"
                    );
                }
                for e in repair {
                    self.send(&Message::Op(e.frame())).await?;
                }
                if differs && !d.reply {
                    self.send(&Message::Digest(DigestFrame {
                        digest: mine,
                        reply: true,
                    }))
                    .await?;
                }
            }
            Message::Family(update) if self.authenticated => {
                let pairing = self.hub.inner.pairing.clone();
                let device = self.device().to_owned();
                tokio::task::spawn_blocking(move || pairing.apply_update(&device, &update.body))
                    .await
                    .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
            }
            // A second hello or auth, acks of fanned-out ops, frames meant
            // for devices, family updates of unpaired devices and types of
            // newer versions: nothing to do.
            Message::Hello(_)
            | Message::Auth(_)
            | Message::Family(_)
            | Message::Ack(_)
            | Message::Change(_)
            | Message::PullEnd(_)
            | Message::Unknown => {}
            Message::Error(e) => {
                tracing::debug!(conn = self.id, code = %e.code, "sync: device reported an error");
            }
        }
        Ok(())
    }

    /// Journals an op, acks it and fans it out if it is new. A log entry
    /// that does not match its hash is not acked and is asked for again.
    async fn store(&mut self, frame: OpFrame) -> Result<(), ConnError> {
        let now = now_ms();
        let stored = {
            let library = self.hub.inner.library.clone();
            self.hub
                .blocking(move |j| {
                    let mut frame = OpFrame { seq: None, ..frame };
                    let mut entry_new = false;
                    let mut gap = None;
                    match j.insert_entry(&frame)? {
                        None | Some(Inserted::Duplicate) => {}
                        Some(Inserted::New { .. }) => entry_new = true,
                        Some(Inserted::Gap { missing }) => {
                            entry_new = true;
                            gap = Some(missing);
                        }
                        Some(Inserted::Corrupt { want }) => return Ok(Stored::Refused(want)),
                        // The op is fine; only its claimed place is not.
                        Some(Inserted::Conflict { .. } | Inserted::Dropped) => frame.log = None,
                    }
                    if frame.log.is_none() && !j.contains(frame.op_id)? {
                        // A device of protocol 1: the hub vouches for its op.
                        frame.log = j.vouch(&frame)?.frame().log;
                        entry_new = true;
                    }
                    let (seq, new) = j.append(&frame, now)?;
                    if new {
                        apply_to_library(&library, &frame);
                    }
                    Ok(Stored::Kept {
                        frame,
                        seq,
                        fan_out: new || entry_new,
                        gap,
                    })
                })
                .await
        };
        let (frame, seq, fan_out, gap) = match stored {
            Ok(Stored::Kept {
                frame,
                seq,
                fan_out,
                gap,
            }) => (frame, seq, fan_out, gap),
            Ok(Stored::Refused(want)) => {
                tracing::info!(conn = self.id, origin = %want.origin, seq = want.from, "sync: damaged log entry");
                return self.send(&Message::Want(want)).await;
            }
            Err(e) => {
                // No ack: the device keeps the op in its outbox and retries.
                tracing::error!(conn = self.id, error = %e, "sync: journal write failed");
                return Ok(());
            }
        };
        self.send(&Message::Ack(Ack {
            op_id: frame.op_id,
            seq: Some(seq),
        }))
        .await?;
        if let Some(gap) = gap
            && self.version >= 2
        {
            self.send(&Message::Want(gap)).await?;
        }
        if fan_out {
            let frame = OpFrame {
                seq: Some(seq),
                ..frame
            };
            self.hub.publish(self.id, encode(&Message::Op(frame)));
        }
        Ok(())
    }
}

/// What [`Connection::store`] did.
enum Stored {
    Kept {
        frame: OpFrame,
        seq: u64,
        fan_out: bool,
        gap: Option<Range>,
    },
    Refused(Range),
}

/// The server is authoritative for its own library: progress ops of known
/// profiles go into its `progress` table (LWW by op time). Ops it cannot
/// read (encrypted, newer schema) are only journaled. Runs on a blocking
/// thread.
fn apply_to_library(library: &Library, frame: &OpFrame) {
    if frame.payload.enc != proto::ENC_PLAIN {
        return;
    }
    let Ok(op) = frame.open() else {
        return;
    };
    let Some(change) = pa_sync::library::progress_change(&op.body) else {
        return;
    };
    let Ok(at_ms) = i64::try_from(op.hlc.wall_ms) else {
        return;
    };
    let conn = library.conn.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = pa_library::progress::apply_remote(&conn, &change, at_ms, &op.hlc.device) {
        tracing::warn!(error = %e, "sync: progress not written to the library");
    }
}
