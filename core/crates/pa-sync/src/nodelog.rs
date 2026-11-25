// SPDX-License-Identifier: MPL-2.0
//! Per-node logs, version vectors and anti-entropy (SYNC-017, ADR-0157 §1–2).
//!
//! Every node (server, desktop hub, phone, a journal in a shared folder)
//! writes its ops into its own append-only log:
//!
//! - `seq` goes 1, 2, 3, … without holes. It is a counter, not a time, so
//!   completeness of delivery never depends on clocks.
//! - Each [`Entry`] names the hash of the previous entry of the same origin
//!   (`prev`, [`GENESIS`] for `seq` 1) and carries its own hash: a damaged
//!   entry is seen at once, a hole by the next `seq`.
//! - Ops of other origins are kept in their origin's log unchanged and
//!   passed on as they are.
//!
//! A node knows its [`VersionVector`] `{origin: last seq it holds without
//! holes}`. Two nodes exchange vectors (`hello.vv`, `vv`) and each sends the
//! other the ranges it lacks ([`NodeLog::missing_for`]) in any order and in
//! batches; an entry that arrives early waits ([`Inserted::Gap`]) until the
//! hole before it is filled, from this peer or any other (`want`). A damaged
//! entry ([`Inserted::Corrupt`]) is not stored and is asked for again.
//!
//! The [`Digest`] of the materialised state (a hash tree by partition:
//! data kind, profile, key bucket) catches what vectors cannot: an op that
//! was stored but applied wrongly, or an op that reached a node outside the
//! logs (an older peer). Nodes compare digests every
//! [`DIGEST_INTERVAL_MS`] and after a long break; the ops of differing
//! partitions are applied again and sent to the peer.
//!
//! The module is sans-IO and in memory; the server hub persists the same
//! entries in SQLite, the shared folder in its journal lines.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::Error;
use crate::hlc::is_valid_device_id;
use crate::op::{Op, OpId};
use crate::proto::{ENC_PLAIN, OpFrame, Payload};
use crate::state::Stamp;

/// `prev` of the first entry of a log.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
/// Length of a hash in hex characters.
pub const HASH_LEN: usize = 64;
/// Most entries of one origin kept out of order, waiting for a hole.
pub const MAX_PENDING: usize = 10_000;
/// Most origins in a vector on the wire.
pub const MAX_VECTOR_LEN: usize = 4_096;
/// Version of the [`Digest`] layout; digests of other versions are not
/// compared.
pub const DIGEST_VERSION: u32 = 1;
/// Most partitions in a digest on the wire.
pub const MAX_DIGEST_PARTS: usize = 8_192;
/// Key buckets per `(kind, profile)` partition.
pub const BUCKETS: u8 = 16;
/// How often nodes compare digests (ADR-0157 §2: every 10 minutes).
pub const DIGEST_INTERVAL_MS: u64 = 10 * 60 * 1000;

const DOMAIN: &[u8] = b"kinolune.log.v1\0";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Whether `s` looks like a hash: 64 lowercase hex characters.
pub fn is_hash(s: &str) -> bool {
    s.len() == HASH_LEN && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Hash of an entry: SHA-256 over the origin, `seq`, `prev`, `op_id` and the
/// payload exactly as stored (an opaque payload is hashed unread).
pub fn entry_hash(origin: &str, seq: u64, prev: &str, op_id: OpId, payload: &Payload) -> String {
    let mut h = Sha256::new();
    h.update(DOMAIN);
    h.update(origin.as_bytes());
    h.update([0]);
    h.update(seq.to_be_bytes());
    h.update(prev.as_bytes());
    h.update([0]);
    h.update(op_id.to_string().as_bytes());
    h.update([0]);
    h.update(payload.enc.as_bytes());
    h.update([0]);
    h.update(payload.body.as_bytes());
    hex(&h.finalize())
}

/// Where an op sits in its origin's log. Travels in `op` frames from
/// protocol 2 on; older peers ignore it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogMeta {
    /// Node whose log holds the op.
    pub origin: String,
    /// Position in that log, from 1 without holes.
    pub oseq: u64,
    /// Hash of the previous entry of the origin ([`GENESIS`] for 1).
    pub prev: String,
    /// Hash of this entry ([`entry_hash`]).
    pub hash: String,
}

impl LogMeta {
    /// Checks ids and hash formats.
    pub fn validate(&self) -> Result<(), Error> {
        if !is_valid_device_id(&self.origin) {
            return Err(Error::Invalid("origin"));
        }
        if self.oseq == 0 || !is_hash(&self.prev) || !is_hash(&self.hash) {
            return Err(Error::Invalid("log entry"));
        }
        Ok(())
    }
}

/// One entry of a node log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub origin: String,
    pub seq: u64,
    pub prev: String,
    pub op_id: OpId,
    pub payload: Payload,
}

impl Entry {
    /// Its hash ([`entry_hash`]).
    pub fn hash(&self) -> String {
        entry_hash(
            &self.origin,
            self.seq,
            &self.prev,
            self.op_id,
            &self.payload,
        )
    }

    /// The `op` frame that carries it.
    pub fn frame(&self) -> OpFrame {
        OpFrame {
            op_id: self.op_id,
            seq: None,
            payload: self.payload.clone(),
            log: Some(LogMeta {
                origin: self.origin.clone(),
                oseq: self.seq,
                prev: self.prev.clone(),
                hash: self.hash(),
            }),
        }
    }

    /// The entry of a frame and the hash the frame claims for it; `None`
    /// for a frame without log position (an older peer, or an op of the
    /// app that is not logged yet).
    pub fn from_frame(frame: &OpFrame) -> Option<(Self, String)> {
        let meta = frame.log.as_ref()?;
        Some((
            Self {
                origin: meta.origin.clone(),
                seq: meta.oseq,
                prev: meta.prev.clone(),
                op_id: frame.op_id,
                payload: frame.payload.clone(),
            },
            meta.hash.clone(),
        ))
    }

    /// The op inside, if this build reads it.
    pub fn open(&self) -> Option<Op> {
        if self.payload.enc != ENC_PLAIN {
            return None;
        }
        crate::outbox::decode(&self.payload.body)
            .ok()
            .filter(|op| op.op_id == self.op_id)
    }
}

/// A range of one origin's log, both ends included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub origin: String,
    pub from: u64,
    pub to: u64,
}

impl Range {
    /// Checks the origin and the bounds.
    pub fn validate(&self) -> Result<(), Error> {
        if !is_valid_device_id(&self.origin) {
            return Err(Error::Invalid("origin"));
        }
        if self.from == 0 || self.from > self.to {
            return Err(Error::Invalid("range"));
        }
        Ok(())
    }
}

/// `{origin: last seq held without holes}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionVector(pub BTreeMap<String, u64>);

impl VersionVector {
    /// The `seq` held of `origin` (0: nothing).
    pub fn get(&self, origin: &str) -> u64 {
        self.0.get(origin).copied().unwrap_or(0)
    }

    /// Raises `origin` to at least `seq`.
    pub fn raise(&mut self, origin: &str, seq: u64) {
        if seq > self.get(origin) {
            self.0.insert(origin.to_owned(), seq);
        }
    }

    /// Whether this vector holds everything `other` holds.
    pub fn covers(&self, other: &Self) -> bool {
        other.0.iter().all(|(o, s)| self.get(o) >= *s)
    }

    /// The ranges `other` holds and this vector does not.
    pub fn lacks(&self, other: &Self) -> Vec<Range> {
        other
            .0
            .iter()
            .filter(|(o, s)| self.get(o) < **s)
            .map(|(o, s)| Range {
                origin: o.clone(),
                from: self.get(o) + 1,
                to: *s,
            })
            .collect()
    }

    /// Checks size and ids.
    pub fn validate(&self) -> Result<(), Error> {
        if self.0.len() > MAX_VECTOR_LEN {
            return Err(Error::Invalid("version vector size"));
        }
        if self.0.keys().all(|o| is_valid_device_id(o)) {
            Ok(())
        } else {
            Err(Error::Invalid("origin"))
        }
    }
}

/// What [`NodeLog::insert`] did with an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inserted {
    /// Stored. `ready` are the entries that joined the gapless part of
    /// their log now, in `seq` order: this one and the ones that waited
    /// behind it.
    New { ready: Vec<Entry> },
    /// Stored out of order; `missing` is the hole before it.
    Gap { missing: Range },
    /// Already held.
    Duplicate,
    /// The entry does not match its hash: not stored. Ask for `want` again,
    /// from this peer or any other.
    Corrupt { want: Range },
    /// The entry is intact but does not follow the log held here (another
    /// entry at the same `seq`, or `prev` is not the hash before it): the
    /// origin forked its log. Not stored; the held log wins.
    Conflict { at: Range },
    /// Not stored: too many entries wait for a hole, or the origin id is
    /// not valid.
    Dropped,
}

#[derive(Debug, Clone, Default)]
struct OriginLog {
    /// Entries and their hashes by `seq`: `1..=contiguous` and waiting ones.
    entries: BTreeMap<u64, (Entry, String)>,
    contiguous: u64,
}

impl OriginLog {
    fn head(&self) -> &str {
        self.entries
            .get(&self.contiguous)
            .map_or(GENESIS, |(_, h)| h.as_str())
    }

    fn pending(&self) -> usize {
        self.entries.len() - usize::try_from(self.contiguous).unwrap_or(usize::MAX)
    }
}

/// The logs a node holds: its own and every other origin's.
#[derive(Debug, Clone)]
pub struct NodeLog {
    me: String,
    logs: BTreeMap<String, OriginLog>,
    /// Newest stamp per register among the readable stored entries: the
    /// state the logs materialise, for [`NodeLog::digest`].
    winners: HashMap<String, Stamp>,
}

impl NodeLog {
    /// An empty set of logs for node `me`.
    pub fn new(me: impl Into<String>) -> Result<Self, Error> {
        let me = me.into();
        if !is_valid_device_id(&me) {
            return Err(Error::Invalid("device id"));
        }
        Ok(Self {
            me,
            logs: BTreeMap::new(),
            winners: HashMap::new(),
        })
    }

    /// Logs restored from stored entries (in any order). Returns the logs
    /// and how many entries were refused (damaged or forked); those are
    /// fetched again from peers.
    pub fn restore(
        me: impl Into<String>,
        entries: impl IntoIterator<Item = Entry>,
    ) -> Result<(Self, usize), Error> {
        let mut log = Self::new(me)?;
        let mut refused = 0;
        for e in entries {
            let hash = e.hash();
            if matches!(
                log.insert(e, &hash),
                Inserted::Corrupt { .. } | Inserted::Conflict { .. } | Inserted::Dropped
            ) {
                refused += 1;
            }
        }
        Ok((log, refused))
    }

    /// Id of this node (origin of its own entries).
    pub fn me(&self) -> &str {
        &self.me
    }

    /// Appends an op of this node to its own log.
    pub fn append_local(&mut self, op_id: OpId, payload: Payload) -> Entry {
        let me = self.me.clone();
        let log = self.logs.entry(me.clone()).or_default();
        // Waiting entries of our own origin (a peer echoing a log we lost)
        // are dropped: this node writes its log, nobody else.
        let keep = log.contiguous;
        log.entries.retain(|s, _| *s <= keep);
        let entry = Entry {
            origin: me,
            seq: log.contiguous + 1,
            prev: log.head().to_owned(),
            op_id,
            payload,
        };
        let hash = entry.hash();
        log.entries.insert(entry.seq, (entry.clone(), hash));
        log.contiguous = entry.seq;
        self.note_winner(&entry);
        entry
    }

    /// [`append_local`](Self::append_local) of a frame's op.
    pub fn append_frame(&mut self, frame: &OpFrame) -> Entry {
        self.append_local(frame.op_id, frame.payload.clone())
    }

    fn note_winner(&mut self, entry: &Entry) {
        let Some(op) = entry.open() else { return };
        let stamp: Stamp = (op.hlc, op.op_id);
        let register = op.body.register();
        match self.winners.get(&register) {
            Some(kept) if *kept >= stamp => {}
            _ => {
                self.winners.insert(register, stamp);
            }
        }
    }

    /// Stores an entry received with `hash` (the hash the sender claims).
    pub fn insert(&mut self, entry: Entry, hash: &str) -> Inserted {
        if !is_valid_device_id(&entry.origin) || entry.seq == 0 {
            return Inserted::Dropped;
        }
        let one = Range {
            origin: entry.origin.clone(),
            from: entry.seq,
            to: entry.seq,
        };
        let real = entry.hash();
        if real != hash {
            return Inserted::Corrupt { want: one };
        }
        let log = self.logs.entry(entry.origin.clone()).or_default();
        if let Some((_, held)) = log.entries.get(&entry.seq) {
            return if *held == real {
                Inserted::Duplicate
            } else {
                Inserted::Conflict { at: one }
            };
        }
        if entry.seq > log.contiguous + 1 {
            if log.pending() >= MAX_PENDING {
                return Inserted::Dropped;
            }
            let missing = Range {
                origin: entry.origin.clone(),
                from: log.contiguous + 1,
                to: entry.seq - 1,
            };
            // The waiting entry right after it must point at it.
            if let Some((next, _)) = log.entries.get(&(entry.seq + 1))
                && next.prev != real
            {
                return Inserted::Conflict { at: one };
            }
            log.entries.insert(entry.seq, (entry.clone(), real));
            self.note_winner(&entry);
            return Inserted::Gap { missing };
        }
        // seq == contiguous + 1
        if entry.prev != log.head() {
            return Inserted::Conflict { at: one };
        }
        log.entries.insert(entry.seq, (entry, real));
        log.contiguous += 1;
        let mut ready = vec![log.entries[&log.contiguous].0.clone()];
        // Entries that waited behind the hole join, while their chain holds.
        loop {
            let next = log.contiguous + 1;
            let Some((e, _)) = log.entries.get(&next) else {
                break;
            };
            if e.prev != log.head() {
                // A forked entry waited there: drop it and everything after
                // it that waited, they are fetched again.
                let cut = log.contiguous;
                log.entries.retain(|s, _| *s <= cut);
                break;
            }
            ready.push(e.clone());
            log.contiguous = next;
        }
        // The waiting ones were counted when they were stored.
        let first = ready[0].clone();
        self.note_winner(&first);
        Inserted::New { ready }
    }

    /// [`insert`](Self::insert) of a frame; `None` for a frame without log
    /// position.
    pub fn insert_frame(&mut self, frame: &OpFrame) -> Option<Inserted> {
        let (entry, hash) = Entry::from_frame(frame)?;
        Some(self.insert(entry, &hash))
    }

    /// The vector of this node.
    pub fn vector(&self) -> VersionVector {
        VersionVector(
            self.logs
                .iter()
                .filter(|(_, l)| l.contiguous > 0)
                .map(|(o, l)| (o.clone(), l.contiguous))
                .collect(),
        )
    }

    /// The `seq` held without holes of `origin`.
    pub fn contiguous(&self, origin: &str) -> u64 {
        self.logs.get(origin).map_or(0, |l| l.contiguous)
    }

    /// Entries a peer with vector `theirs` lacks, by origin and `seq`. Only
    /// the gapless part is sent: the peer could not place the rest yet.
    pub fn missing_for(&self, theirs: &VersionVector) -> Vec<Entry> {
        let mut out = Vec::new();
        for (origin, log) in &self.logs {
            let from = theirs.get(origin) + 1;
            if from > log.contiguous {
                continue;
            }
            out.extend(
                log.entries
                    .range(from..=log.contiguous)
                    .map(|(_, (e, _))| e.clone()),
            );
        }
        out
    }

    /// Held entries of a range (gapless or waiting).
    pub fn range(&self, range: &Range) -> Vec<Entry> {
        let Some(log) = self.logs.get(&range.origin) else {
            return Vec::new();
        };
        if range.from > range.to {
            return Vec::new();
        }
        log.entries
            .range(range.from..=range.to)
            .map(|(_, (e, _))| e.clone())
            .collect()
    }

    /// The holes behind waiting entries: what to ask peers for.
    pub fn gaps(&self) -> Vec<Range> {
        self.logs
            .iter()
            .filter_map(|(origin, log)| {
                let (first, _) = log.entries.range(log.contiguous + 1..).next()?;
                Some(Range {
                    origin: origin.clone(),
                    from: log.contiguous + 1,
                    to: first - 1,
                })
            })
            .collect()
    }

    /// Every held entry, by origin and `seq`.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.logs
            .values()
            .flat_map(|l| l.entries.values().map(|(e, _)| e))
    }

    /// The gapless entries only (what a node persists and restores).
    pub fn gapless(&self) -> impl Iterator<Item = &Entry> {
        self.logs
            .values()
            .flat_map(|l| l.entries.range(..=l.contiguous).map(|(_, (e, _))| e))
    }

    /// Number of held entries.
    pub fn len(&self) -> usize {
        self.logs.values().map(|l| l.entries.len()).sum()
    }

    /// Whether no entry is held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Digest of the state the held entries materialise.
    pub fn digest(&self) -> Digest {
        Digest::from_leaves(self.winners.iter().map(|(r, s)| (r.as_str(), s.1)))
    }

    /// Held entries whose op falls in one of `parts` (digest repair).
    pub fn entries_in(&self, parts: &BTreeSet<String>) -> Vec<Entry> {
        if parts.is_empty() {
            return Vec::new();
        }
        self.entries()
            .filter(|e| {
                e.open()
                    .is_some_and(|op| parts.contains(&partition(&op.body.register())))
            })
            .cloned()
            .collect()
    }
}

/// Partition of a register: `kind/profile/bucket` (the bucket is the first
/// hex digit of the register's SHA-256).
pub fn partition(register: &str) -> String {
    let mut parts = register.split('\u{1f}');
    let kind = parts.next().unwrap_or("");
    let profile = parts.next().unwrap_or("");
    let bucket = Sha256::digest(register.as_bytes())[0] >> 4;
    format!("{kind}/{profile}/{bucket:x}")
}

/// Digest of a materialised state: a hash per partition over its registers
/// and the op that won each, and a root hash over the partitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    /// [`DIGEST_VERSION`].
    pub v: u32,
    pub root: String,
    /// Hash by partition ([`partition`]).
    pub parts: BTreeMap<String, String>,
}

impl Digest {
    /// The digest of `(register, winning op)` pairs (any order).
    pub fn from_leaves<'a>(leaves: impl IntoIterator<Item = (&'a str, OpId)>) -> Self {
        let mut by_part: BTreeMap<String, BTreeMap<&'a str, OpId>> = BTreeMap::new();
        for (register, op_id) in leaves {
            by_part
                .entry(partition(register))
                .or_default()
                .insert(register, op_id);
        }
        let parts: BTreeMap<String, String> = by_part
            .into_iter()
            .map(|(p, leaves)| {
                let mut h = Sha256::new();
                for (register, op_id) in leaves {
                    h.update(register.as_bytes());
                    h.update([0]);
                    h.update(op_id.to_string().as_bytes());
                    h.update([0]);
                }
                (p, hex(&h.finalize()))
            })
            .collect();
        let mut h = Sha256::new();
        for (p, hash) in &parts {
            h.update(p.as_bytes());
            h.update([0]);
            h.update(hash.as_bytes());
            h.update([0]);
        }
        Self {
            v: DIGEST_VERSION,
            root: hex(&h.finalize()),
            parts,
        }
    }

    /// Partitions that differ (present on one side only, or with another
    /// hash). Empty when the roots match or the versions differ.
    pub fn diff(&self, other: &Self) -> BTreeSet<String> {
        if self.v != other.v || self.root == other.root {
            return BTreeSet::new();
        }
        let keys: BTreeSet<&String> = self.parts.keys().chain(other.parts.keys()).collect();
        keys.into_iter()
            .filter(|k| self.parts.get(*k) != other.parts.get(*k))
            .cloned()
            .collect()
    }

    /// Checks sizes and hash formats.
    pub fn validate(&self) -> Result<(), Error> {
        let ok = is_hash(&self.root)
            && self.parts.len() <= MAX_DIGEST_PARTS
            && self.parts.iter().all(|(p, h)| p.len() <= 512 && is_hash(h));
        ok.then_some(()).ok_or(Error::Invalid("digest"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::Hlc;
    use crate::op::OpBody;

    fn op(n: u128, origin: &str) -> Op {
        Op {
            op_id: OpId(uuid::Uuid::from_u128(n)),
            hlc: Hlc {
                wall_ms: 1_763_000_000_000 + n as u64,
                counter: 0,
                device: origin.into(),
            },
            aliases: Vec::new(),
            body: OpBody::Watched {
                profile: "default".into(),
                key: format!("tmdb:movie:{n}").parse().unwrap(),
            },
        }
    }

    fn log_of(origin: &str, n: u128) -> (NodeLog, Vec<Entry>) {
        let mut log = NodeLog::new(origin).unwrap();
        let entries = (1..=n)
            .map(|i| {
                let o = op(i, origin);
                log.append_local(o.op_id, Payload::plain(&o).unwrap())
            })
            .collect();
        (log, entries)
    }

    #[test]
    fn entries_chain_and_out_of_order_ones_wait() {
        let (_, entries) = log_of("a", 4);
        assert_eq!(entries[0].prev, GENESIS);
        assert_eq!(entries[1].prev, entries[0].hash());
        let mut b = NodeLog::new("b").unwrap();
        let ins = |b: &mut NodeLog, e: &Entry| b.insert(e.clone(), &e.hash());
        assert_eq!(
            ins(&mut b, &entries[2]),
            Inserted::Gap {
                missing: Range {
                    origin: "a".into(),
                    from: 1,
                    to: 2
                }
            }
        );
        assert_eq!(b.vector().get("a"), 0);
        assert_eq!(b.gaps().len(), 1);
        assert!(matches!(ins(&mut b, &entries[0]), Inserted::New { ready } if ready.len() == 1));
        assert!(matches!(ins(&mut b, &entries[1]), Inserted::New { ready } if ready.len() == 2));
        assert_eq!(b.vector().get("a"), 3);
        assert_eq!(ins(&mut b, &entries[1]), Inserted::Duplicate);
        assert!(b.gaps().is_empty());
    }

    #[test]
    fn damaged_and_forked_entries_are_refused() {
        let (_, entries) = log_of("a", 2);
        let mut b = NodeLog::new("b").unwrap();
        let mut bad = entries[0].clone();
        bad.payload.body = bad.payload.body.replace("default", "defaulT");
        let r = b.insert(bad, &entries[0].hash());
        assert!(matches!(r, Inserted::Corrupt { want } if want.from == 1));
        assert!(b.is_empty());
        // A fork: a valid entry 1 of another chain after ours.
        b.insert(entries[0].clone(), &entries[0].hash());
        let (_, other) = log_of("a", 3);
        let mut fork = other[2].clone();
        fork.seq = 2;
        fork.prev = GENESIS.into();
        let h = fork.hash();
        assert!(matches!(b.insert(fork, &h), Inserted::Conflict { .. }));
        assert_eq!(b.vector().get("a"), 1);
    }

    #[test]
    fn missing_ranges_follow_the_vectors() {
        let (a, _) = log_of("a", 5);
        let mut theirs = VersionVector::default();
        theirs.raise("a", 2);
        let missing = a.missing_for(&theirs);
        assert_eq!(missing.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4, 5]);
        assert_eq!(theirs.lacks(&a.vector()).len(), 1);
        assert!(a.vector().covers(&theirs));
    }

    #[test]
    fn digests_point_at_the_differing_partition() {
        let (a, _) = log_of("a", 6);
        let (mut b, _) = log_of("a", 5);
        assert_eq!(a.digest().diff(&a.digest()), BTreeSet::new());
        let diff = a.digest().diff(&b.digest());
        assert_eq!(diff.len(), 1);
        let repair = a.entries_in(&diff);
        assert!(repair.iter().any(|e| e.seq == 6));
        b = NodeLog::restore("a", a.entries().cloned()).unwrap().0;
        assert!(a.digest().diff(&b.digest()).is_empty());
        a.digest().validate().unwrap();
    }
}
