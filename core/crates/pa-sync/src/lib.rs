// SPDX-License-Identifier: MPL-2.0
//! Serverless sync of user state between devices (ADR-0004).
//!
//! - [`hlc`]: hybrid logical clock; timestamps order all ops without a server.
//! - [`op`]: sync operations with a UUIDv7 [`OpId`] (progress, watched,
//!   favorites, ratings, playlists, folder rules, settings without secrets,
//!   Twitch follows, sources, profiles, user categories and tags).
//! - [`SyncState`]: last-writer-wins register per field; any order of the
//!   same ops gives the same state.
//! - [`outbox`]: the SQLite outbox with idempotent enqueue and retries.
//! - [`Replica`]: ties them together for one device.
//! - [`proto`]: the wire protocol (`docs/api/sync-protocol.md`): handshake
//!   with version negotiation, `pb.update`/`pb.takeover` for instant resume,
//!   op exchange with opaque versioned payloads, acks and journal cursors.
//! - [`library`]: `Replica` as the sync sink of `pa_library::progress`
//!   (LIB-011) and of Twitch favourites (`pa_library::twitch`, TWI-007),
//!   and writing received progress and follow ops into the library.
//!
//! - [`seal`]: the E2E container (NaCl secretbox, key from the pairing
//!   family key) of source passwords in `source.upsert` ops (SYNC-010).
//! - [`handoff`]: rules of the ephemeral playback session (SYNC-003):
//!   heartbeat TTLs, the newest resume point by HLC.
//! - [`input`]: typing on a TV from a phone (UI-027): the `input`
//!   message, its text sealed with the family key.
//! - [`matching`]: the keys of one item across devices (SYNC-004): the
//!   `key` and `aliases` of ops and `pb.*` messages, when a position
//!   carries over to another version.
//! - `lan` (feature `lan`): the LAN P2P transport (SYNC-006): mDNS, hub
//!   election, WebSocket over TLS 1.3 bound to the pairing keys.
//!
//! - [`folder`]: sync through a shared folder of a source (SYNC-007): an
//!   append-only journal per device in `/.kinolune/state/`, read by the
//!   others from their cursors, compacted into snapshots.
//!
//! Other transports (the user's own server, iroh) move the same [`proto`]
//! frames and call [`Replica::receive`].
//!
//! ```
//! use pa_model::{MediaKey, MediaKind};
//! use pa_sync::{OpBody, Replica};
//!
//! let conn = pa_library::open_in_memory().unwrap();
//! let mut phone = Replica::new("phone").unwrap();
//! let mut tv = Replica::new("tv").unwrap();
//! let key: MediaKey = "tmdb:movie:603".parse().unwrap();
//! let op = phone
//!     .record(&conn, OpBody::ProgressUpsert {
//!         profile: "default".into(),
//!         key: key.clone(),
//!         kind: MediaKind::Movie,
//!         position_ms: 2_471_200,
//!         duration_ms: Some(8_160_000),
//!     }, 1_763_000_000_000)
//!     .unwrap();
//! tv.receive(&op, 1_763_000_000_100).unwrap();
//! let pos = &tv.state().progress[&("default".to_owned(), key)];
//! assert_eq!(pos.value.position_ms, 2_471_200);
//! assert_eq!(pa_sync::outbox::len(&conn).unwrap(), 1);
//! ```

pub mod folder;
pub mod handoff;
pub mod hlc;
pub mod input;
#[cfg(feature = "lan")]
pub mod lan;
pub mod library;
pub mod matching;
pub mod op;
pub mod outbox;
pub mod profiles;
pub mod proto;
pub mod seal;
mod state;

pub use hlc::{Hlc, HlcClock};
pub use op::{Op, OpBody, OpId};
pub use state::{LabelKey, PlaylistMeta, Position, ProfileKey, Reg, Stamp, SyncState};

use pa_library::rusqlite::{self, Connection};

/// Error of pa-sync.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A field of an op or an id is not acceptable.
    #[error("invalid {0}")]
    Invalid(&'static str),
    /// A peer's clock is too far ahead ([`hlc::MAX_DRIFT_MS`]).
    #[error("remote clock {remote_ms} ms is too far ahead of local {local_ms} ms")]
    ClockDrift { remote_ms: u64, local_ms: u64 },
    /// No protocol version in common (`(min, max)` of each side).
    #[error("no common protocol version: local {local:?}, remote {remote:?}")]
    Incompatible {
        local: (u16, u16),
        remote: (u16, u16),
    },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Writing a received change into the library failed.
    #[error(transparent)]
    Library(#[from] pa_library::Error),
}

/// Sync state of one device: its clock and the merged state. The state is
/// in memory; persisting it into library tables is up to the caller.
#[derive(Debug, Clone)]
pub struct Replica {
    clock: HlcClock,
    state: SyncState,
}

impl Replica {
    /// A replica for `device_id` with an empty state.
    pub fn new(device_id: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            clock: HlcClock::new(device_id)?,
            state: SyncState::default(),
        })
    }

    /// Device id of this replica.
    pub fn device_id(&self) -> &str {
        self.clock.device()
    }

    /// The merged state.
    pub fn state(&self) -> &SyncState {
        &self.state
    }

    /// The clock (e.g. to restore it after restart with
    /// [`HlcClock::observe`] of the last saved stamp).
    pub fn clock_mut(&mut self) -> &mut HlcClock {
        &mut self.clock
    }

    /// Records a local change at `now_ms` (Unix ms): stamps it, applies it
    /// and queues it in the outbox of `conn`. To keep a library table and the
    /// outbox consistent, call it inside the caller's transaction.
    pub fn record(&mut self, conn: &Connection, body: OpBody, now_ms: i64) -> Result<Op, Error> {
        self.record_aliased(conn, body, Vec::new(), now_ms)
    }

    /// [`Replica::record`] with the other keys of the op's item
    /// (`Op::aliases`, SYNC-004); at most [`matching::MAX_ALIASES`] are
    /// kept.
    pub fn record_aliased(
        &mut self,
        conn: &Connection,
        body: OpBody,
        mut aliases: Vec<String>,
        now_ms: i64,
    ) -> Result<Op, Error> {
        body.validate()?;
        aliases.truncate(matching::MAX_ALIASES);
        let hlc = self.clock.tick(u64::try_from(now_ms).unwrap_or(0));
        let op = Op {
            op_id: OpId::new_v7(),
            hlc,
            aliases,
            body,
        };
        outbox::enqueue(conn, &op, now_ms)?;
        self.state.apply(&op);
        Ok(op)
    }

    /// Applies an op from another device. Returns true if the state changed;
    /// duplicates and stale ops return false.
    pub fn receive(&mut self, op: &Op, now_ms: i64) -> Result<bool, Error> {
        op.body.validate()?;
        if !hlc::is_valid_device_id(&op.hlc.device) {
            return Err(Error::Invalid("device id"));
        }
        self.clock
            .observe(&op.hlc, u64::try_from(now_ms).unwrap_or(0))?;
        Ok(self.state.apply(op))
    }
}
