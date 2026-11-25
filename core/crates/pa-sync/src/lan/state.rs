// SPDX-License-Identifier: MPL-2.0
//! State of a LAN node shared by its links: the journal, the links, cursors
//! and the routing of ops and playback messages between them.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use pa_discovery::DeviceClass;
use pa_model::MediaKey;
use pa_pairing::{Family, FamilyUpdate};
use tokio::sync::mpsc;

use super::election::{Backoff, Election};
use super::{LanConfig, LanEvent};
use crate::OpId;
use crate::hlc::Hlc;
use crate::input::{InputAction, InputFrame};
use crate::music::MusicFrame;
use crate::nodelog::{Inserted, NodeLog, Range};
use crate::proto::{
    Ack, Change, Cursor, DigestFrame, ErrorFrame, Hello, MAX_PULL_LIMIT, Message, OpFrame,
    PlaybackUpdate, ProtoRange, Pull, Role, VvFrame, code, encode, negotiate,
};

/// Frames to a link's writer; `None` closes the link.
pub(crate) type LinkTx = mpsc::UnboundedSender<Option<String>>;

/// Where a message came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    App,
    Link(u64),
}

/// Which way a link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Side {
    /// We dialed the hub `hub_id`.
    Up { hub_id: String },
    /// A device dialed us.
    Down,
}

struct Link {
    device_id: String,
    side: Side,
    tx: LinkTx,
    /// Journal id of the hub (upstream links only).
    log: Option<String>,
    /// The other side accepted the link. A dialer checks the hub's `auth`
    /// before the hub checks its own, so an upstream link counts only once
    /// the hub has sent something other than an `error` (it always answers
    /// our `pull`); a downstream link is accepted by us.
    accepted: bool,
    /// Protocol version agreed on the link; 2 and up run anti-entropy.
    version: u16,
}

pub(crate) struct Shared {
    pub(crate) config: LanConfig,
    pub(crate) device_id: String,
    pub(crate) tls_server: Arc<rustls::ServerConfig>,
    pub(crate) tls_client: Arc<rustls::ClientConfig>,
    state: Mutex<State>,
}

/// A `pb.update` as received.
struct Cached {
    at: Instant,
    raw: String,
    update: PlaybackUpdate,
}

pub(crate) struct State {
    pub(crate) family: Family,
    pub(crate) election: Election,
    pub(crate) hub: Option<String>,
    revoked: bool,
    playback_ttl: std::time::Duration,
    events: mpsc::UnboundedSender<LanEvent>,
    /// Every op this node has seen; also what it serves as a hub.
    journal: crate::proto::Journal,
    /// The node logs (SYNC-017). In memory like the journal, so this
    /// node's own origin is the journal id of this run, not the device id:
    /// a restarted node starts a new log instead of forking its old one.
    nodes: NodeLog,
    links: HashMap<u64, Link>,
    next_link: u64,
    /// Our cursor in each hub's journal, by hub device id.
    cursors: HashMap<String, Cursor>,
    /// Ops each hub journal (by log id) is known to hold.
    acked: HashMap<String, HashSet<OpId>>,
    /// Last `pb.update` per title (and its frame), replayed to new links.
    playback: HashMap<(String, MediaKey), Cached>,
    /// Waiting for the hub's answer to a `pull` (SYNC-003, before playing).
    pull_waiters: Vec<tokio::sync::oneshot::Sender<()>>,
    /// Newest stamp per message type and title: drops repeats and loops.
    seen: HashMap<(bool, String, MediaKey), Hlc>,
    /// Open input requests by session (UI-027), replayed to new links.
    inputs: HashMap<String, CachedInput>,
    /// `input` frames already routed: drops repeats and loops.
    seen_inputs: HashSet<(String, Hlc)>,
    /// `music` frames already routed (MUS-015): drops repeats and loops.
    seen_music: HashSet<(String, Hlc)>,
}

/// An open `input` request as received.
struct CachedInput {
    raw: String,
    frame: InputFrame,
}

/// Most entries of [`State::seen_inputs`] before it starts over.
const MAX_SEEN_INPUTS: usize = 4096;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn frame(msg: &Message) -> Option<String> {
    encode(msg).ok()
}

impl Shared {
    pub(crate) fn new(
        config: LanConfig,
        family: Family,
        events: mpsc::UnboundedSender<LanEvent>,
    ) -> std::io::Result<Self> {
        let device_id = family.identity().device_id().to_owned();
        let log = uuid::Uuid::now_v7().to_string();
        let nodes = NodeLog::new(log.clone()).map_err(super::tls::io_err)?;
        let journal = crate::proto::Journal::new(log).map_err(super::tls::io_err)?;
        let backoff = Backoff {
            first: config.dead_retry,
            max: config.dead_cooldown,
        };
        let state = State {
            election: Election::new(device_id.clone(), config.class).with_backoff(backoff),
            family,
            hub: None,
            revoked: false,
            playback_ttl: config.playback_ttl,
            events,
            journal,
            nodes,
            links: HashMap::new(),
            next_link: 1,
            cursors: HashMap::new(),
            acked: HashMap::new(),
            playback: HashMap::new(),
            pull_waiters: Vec::new(),
            seen: HashMap::new(),
            inputs: HashMap::new(),
            seen_inputs: HashSet::new(),
            seen_music: HashSet::new(),
        };
        Ok(Self {
            tls_server: Arc::new(super::tls::server_config()?),
            tls_client: Arc::new(super::tls::client_config()?),
            config,
            device_id,
            state: Mutex::new(state),
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    pub(crate) fn with_family<R>(&self, f: impl FnOnce(&Family) -> R) -> R {
        f(&self.lock().family)
    }

    pub(crate) fn peer_up(&self, device_id: &str, class: DeviceClass, addr: SocketAddr) {
        self.lock().election.upsert(device_id, class, addr);
    }

    pub(crate) fn peer_down(&self, device_id: &str) {
        self.lock().election.remove(device_id);
    }
}

impl State {
    fn emit(&self, event: LanEvent) {
        let _ = self.events.send(event);
    }

    pub(crate) fn set_hub(&mut self, hub: Option<String>) {
        if self.hub != hub {
            self.hub.clone_from(&hub);
            self.emit(LanEvent::HubChanged(hub));
        }
    }

    pub(crate) fn journal_frames(&self) -> Vec<String> {
        self.journal
            .iter()
            .filter_map(|f| serde_json::to_string(f).ok())
            .collect()
    }

    pub(crate) fn link_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.links.values().map(|l| l.device_id.clone()).collect();
        ids.sort();
        ids
    }

    /// Our `hello` on a link of `side`.
    pub(crate) fn hello(&self, side: &Side, nonce: String) -> Hello {
        let (role, cursor) = match side {
            Side::Up { hub_id } => (Role::Device, self.cursors.get(hub_id).cloned()),
            Side::Down => (Role::Hub, None),
        };
        Hello {
            proto: ProtoRange::LOCAL,
            device_id: self.family.identity().device_id().to_owned(),
            role,
            cursor,
            head: Some(self.journal.head()),
            nonce: Some(nonce),
            vv: Some(self.nodes.vector()),
        }
    }

    /// Checks a peer's `auth` against the roster.
    pub(crate) fn verify(
        &self,
        peer: &str,
        peer_nonce: &str,
        own_nonce: &str,
        sig: &str,
    ) -> Result<(), pa_pairing::Error> {
        if self.revoked {
            return Err(pa_pairing::Error::Revoked(peer.to_owned()));
        }
        let me = self.family.identity().device_id();
        pa_pairing::auth::verify(self.family.roster(), peer, me, peer_nonce, own_nonce, sig)
            .map(|_| ())
    }

    /// Adds a link that passed the handshake and sends what it needs first.
    pub(crate) fn register(&mut self, side: Side, peer: &Hello, tx: LinkTx) -> u64 {
        let id = self.next_link;
        self.next_link += 1;
        let log = match &side {
            Side::Up { .. } => peer.head.as_ref().map(|h| h.log.clone()),
            Side::Down => None,
        };
        let link = Link {
            device_id: peer.device_id.clone(),
            side: side.clone(),
            tx,
            log,
            accepted: side == Side::Down,
            version: negotiate(ProtoRange::LOCAL, peer.proto).unwrap_or(1),
        };
        // SYNC-017: the log entries the peer lacks, by its vector.
        if link.version >= 2
            && let Some(vv) = &peer.vv
        {
            for e in self.nodes.missing_for(vv) {
                Self::send(&link.tx, &Message::Op(e.frame()));
            }
        }
        match &side {
            Side::Up { hub_id } => {
                // Push what this hub journal lacks, then pull what we lack.
                let acked = link.log.as_ref().and_then(|l| self.acked.get(l));
                for f in self.journal.iter() {
                    if acked.is_none_or(|a| !a.contains(&f.op_id)) {
                        let up = OpFrame {
                            seq: None,
                            ..f.clone()
                        };
                        Self::send(&link.tx, &Message::Op(up));
                    }
                }
                let cursor = self
                    .cursors
                    .get(hub_id)
                    .filter(|c| Some(&c.log) == link.log.as_ref())
                    .cloned();
                Self::send(
                    &link.tx,
                    &Message::Pull(Pull {
                        cursor,
                        limit: MAX_PULL_LIMIT,
                    }),
                );
                // The hub becomes ours once it answers (`accept`).
                self.links.insert(id, link);
            }
            Side::Down => {
                // The device dialed us and passed `auth`: it is up. If it
                // was marked dead (say it restarted at the same address),
                // it is electable again.
                self.election.revive(&peer.device_id, Instant::now());
                if let Ok(update) = self.family.update_for(&peer.device_id) {
                    Self::send(
                        &link.tx,
                        &Message::Family(crate::proto::Family {
                            body: update.to_json(),
                        }),
                    );
                }
                let ttl = self.playback_ttl;
                self.playback.retain(|_, c| c.at.elapsed() < ttl);
                for c in self.playback.values() {
                    let _ = link.tx.send(Some(c.raw.clone()));
                }
                // UI-027: a phone that connects (back from the background)
                // still gets the open request.
                let now = crate::hlc::system_ms();
                self.inputs.retain(|_, c| c.frame.is_current(now));
                for c in self.inputs.values() {
                    let _ = link.tx.send(Some(c.raw.clone()));
                }
                let head = self.journal.head();
                if peer.cursor.as_ref() != Some(&head) {
                    Self::send(&link.tx, &Message::Change(Change { head }));
                }
                self.links.insert(id, link);
            }
        }
        id
    }

    pub(crate) fn unregister(&mut self, id: u64) {
        if let Some(link) = self.links.remove(&id)
            && let Side::Up { hub_id } = link.side
        {
            // The answer will not come: waiters see a closed channel.
            self.pull_waiters.clear();
            // A replaced link may end after the new one is up: to the same
            // hub at a new address, or to a dead hub that answered a retry.
            // The hub is then whichever upstream link is left.
            if self.hub.as_ref() == Some(&hub_id) {
                let ups: Vec<&String> = self
                    .links
                    .values()
                    .filter_map(|l| match &l.side {
                        Side::Up { hub_id } if l.accepted => Some(hub_id),
                        _ => None,
                    })
                    .collect();
                if !ups.contains(&&hub_id) {
                    let left = ups.first().map(|h| (*h).clone());
                    self.set_hub(left);
                }
            }
        }
    }

    /// Asks the hub for what this device lacks (SYNC-003: before playing).
    /// The receiver fires when the hub has answered in full; `None` when
    /// there is no hub link (this device is the hub, or alone).
    pub(crate) fn request_pull(&mut self) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let (hub_id, tx, log) = self.links.values().find_map(|l| match &l.side {
            Side::Up { hub_id } => Some((hub_id.clone(), l.tx.clone(), l.log.clone())),
            Side::Down => None,
        })?;
        let cursor = self
            .cursors
            .get(&hub_id)
            .filter(|c| Some(&c.log) == log.as_ref())
            .cloned();
        let (done, rx) = tokio::sync::oneshot::channel();
        self.pull_waiters.push(done);
        Self::send(
            &tx,
            &Message::Pull(Pull {
                cursor,
                limit: MAX_PULL_LIMIT,
            }),
        );
        Some(rx)
    }

    /// The last `pb.update` of another device per title, newest replay
    /// window only.
    pub(crate) fn playback_updates(&self) -> Vec<PlaybackUpdate> {
        let me = self.family.identity().device_id();
        self.playback
            .values()
            .filter(|c| c.at.elapsed() < self.playback_ttl && c.update.hlc.device != me)
            .map(|c| c.update.clone())
            .collect()
    }

    fn send(tx: &LinkTx, msg: &Message) {
        if let Some(text) = frame(msg) {
            let _ = tx.send(Some(text));
        }
    }

    fn send_error(tx: &LinkTx, code: &str, op_id: Option<OpId>) {
        Self::send(
            tx,
            &Message::Error(ErrorFrame {
                code: code.to_owned(),
                message: None,
                op_id,
            }),
        );
    }

    /// Stores an op and routes it to every other link. Returns its `seq`
    /// in our journal, or `None` when the frame was refused (a damaged log
    /// entry: not acked, asked for again).
    pub(crate) fn on_op(&mut self, frame: OpFrame, source: Source) -> Option<u64> {
        let mut frame = OpFrame { seq: None, ..frame };
        // SYNC-017: the place of the op in its origin's log.
        let mut entry_new = false;
        match self.nodes.insert_frame(&frame) {
            None | Some(Inserted::Duplicate) => {}
            Some(Inserted::New { .. }) => entry_new = true,
            Some(Inserted::Gap { missing }) => {
                entry_new = true;
                self.want(&missing);
            }
            Some(Inserted::Corrupt { want }) => {
                self.want(&want);
                return None;
            }
            // The op itself is fine; only its claimed place is not.
            Some(Inserted::Conflict { .. } | Inserted::Dropped) => frame.log = None,
        }
        if frame.log.is_none() && !self.journal.contains(&frame.op_id) {
            // An op of the app or of a protocol-1 peer: this node vouches
            // for it in its own log.
            frame.log = self.nodes.append_frame(&frame).frame().log;
            entry_new = true;
        }
        let (seq, new) = self.journal.append(&frame);
        if !new && !entry_new {
            return Some(seq);
        }
        if new
            && source != Source::App
            && let Ok(op) = frame.open()
        {
            self.emit(LanEvent::Op(op));
        }
        for (id, link) in &self.links {
            if Source::Link(*id) == source {
                continue;
            }
            let out = match link.side {
                Side::Down => OpFrame {
                    seq: Some(seq),
                    ..frame.clone()
                },
                Side::Up { .. } => frame.clone(),
            };
            Self::send(&link.tx, &Message::Op(out));
        }
        Some(seq)
    }

    /// Asks every link of protocol 2 for a range of a log (a hole or a
    /// damaged entry): any member may have it.
    fn want(&self, range: &Range) {
        for link in self.links.values().filter(|l| l.version >= 2) {
            Self::send(&link.tx, &Message::Want(range.clone()));
        }
    }

    /// Periodic anti-entropy on link `id` (SYNC-017): our vector (the peer
    /// sends what we lack), the holes still open and our state digest.
    pub(crate) fn anti_entropy(&self, id: u64) {
        let Some(link) = self.links.get(&id).filter(|l| l.version >= 2) else {
            return;
        };
        Self::send(
            &link.tx,
            &Message::Vv(VvFrame {
                vv: self.nodes.vector(),
            }),
        );
        for gap in self.nodes.gaps() {
            Self::send(&link.tx, &Message::Want(gap));
        }
        Self::send(
            &link.tx,
            &Message::Digest(DigestFrame {
                digest: self.nodes.digest(),
                reply: false,
            }),
        );
    }

    /// A peer's digest: the ops of differing partitions go to the peer and
    /// to the app again (an op applied wrongly is applied once more), and
    /// a first digest gets ours back.
    fn on_digest(&mut self, tx: &LinkTx, frame: &DigestFrame) {
        let mine = self.nodes.digest();
        let diff = mine.diff(&frame.digest);
        if diff.is_empty() {
            return;
        }
        for e in self.nodes.entries_in(&diff) {
            if let Some(op) = e.open() {
                self.emit(LanEvent::Op(op));
            }
            Self::send(tx, &Message::Op(e.frame()));
        }
        if !frame.reply {
            Self::send(
                tx,
                &Message::Digest(DigestFrame {
                    digest: mine,
                    reply: true,
                }),
            );
        }
    }

    /// Routes `pb.update` / `pb.takeover` (as the original `raw` frame) to
    /// every other link, once per stamp.
    pub(crate) fn on_playback(&mut self, raw: &str, msg: Message, source: Source) {
        let (is_update, profile, key, hlc) = match &msg {
            Message::PbUpdate(u) => (true, &u.profile, &u.key, &u.hlc),
            Message::PbTakeover(t) => (false, &t.profile, &t.key, &t.hlc),
            _ => return,
        };
        let slot = (is_update, profile.clone(), key.clone());
        if self.seen.get(&slot).is_some_and(|seen| seen >= hlc) {
            return;
        }
        self.seen.insert(slot, hlc.clone());
        if let Message::PbUpdate(u) = &msg {
            self.playback.insert(
                (profile.clone(), key.clone()),
                Cached {
                    at: Instant::now(),
                    raw: raw.to_owned(),
                    update: u.clone(),
                },
            );
        }
        for (id, link) in &self.links {
            if Source::Link(*id) != source {
                let _ = link.tx.send(Some(raw.to_owned()));
            }
        }
        if source != Source::App {
            self.emit(match msg {
                Message::PbUpdate(u) => LanEvent::Playback(u),
                Message::PbTakeover(t) => LanEvent::Takeover(t),
                _ => unreachable!("matched above"),
            });
        }
    }

    /// Routes an `input` frame (as the original `raw` frame) to every other
    /// link, once per sender stamp, and keeps open requests for new links
    /// (UI-027). Never stored in the journal.
    pub(crate) fn on_input(&mut self, raw: &str, frame: InputFrame, source: Source) {
        if self.seen_inputs.len() >= MAX_SEEN_INPUTS {
            self.seen_inputs.clear();
        }
        if !self
            .seen_inputs
            .insert((frame.session.clone(), frame.hlc.clone()))
        {
            return;
        }
        match frame.action {
            InputAction::Request => {
                self.inputs.insert(
                    frame.session.clone(),
                    CachedInput {
                        raw: raw.to_owned(),
                        frame: frame.clone(),
                    },
                );
            }
            InputAction::Cancel | InputAction::Submit => {
                self.inputs.remove(&frame.session);
            }
            _ => {}
        }
        for (id, link) in &self.links {
            if Source::Link(*id) != source {
                let _ = link.tx.send(Some(raw.to_owned()));
            }
        }
        if source != Source::App && frame.hlc.device != self.family.identity().device_id() {
            self.emit(LanEvent::Input(frame));
        }
    }

    /// Routes a `music` frame (MUS-015) to every other link, once per
    /// sender stamp. Never stored and never replayed: a handoff is for a
    /// device that is online now.
    pub(crate) fn on_music(&mut self, raw: &str, frame: MusicFrame, source: Source) {
        if self.seen_music.len() >= MAX_SEEN_INPUTS {
            self.seen_music.clear();
        }
        if !self
            .seen_music
            .insert((frame.session.clone(), frame.hlc.clone()))
        {
            return;
        }
        for (id, link) in &self.links {
            if Source::Link(*id) != source {
                let _ = link.tx.send(Some(raw.to_owned()));
            }
        }
        if source != Source::App && frame.hlc.device != self.family.identity().device_id() {
            self.emit(LanEvent::Music(frame));
        }
    }

    /// Handles one message of link `id`.
    pub(crate) fn on_message(&mut self, id: u64, raw: &str, msg: Message) {
        let Some(link) = self.links.get_mut(&id) else {
            return;
        };
        // A hub that refuses us sends an `error` before it closes.
        if !link.accepted && !matches!(msg, Message::Error(_)) {
            link.accepted = true;
            if let Side::Up { hub_id } = &link.side {
                // The hub accepted us: it is up (a background retry of a
                // dead hub got through) and it is our hub now.
                let hub = hub_id.clone();
                self.election.revive(&hub, Instant::now());
                self.set_hub(Some(hub));
            }
        }
        let Some(link) = self.links.get(&id) else {
            return;
        };
        let (tx, side, log) = (link.tx.clone(), link.side.clone(), link.log.clone());
        match msg {
            Message::Op(f) => {
                if let (Side::Up { hub_id }, Some(log), Some(seq)) = (&side, &log, f.seq) {
                    self.acked.entry(log.clone()).or_default().insert(f.op_id);
                    let cursor = self.cursors.entry(hub_id.clone()).or_insert(Cursor {
                        log: log.clone(),
                        seq: 0,
                    });
                    if cursor.log == *log && cursor.seq + 1 == seq {
                        cursor.seq = seq;
                    }
                }
                let op_id = f.op_id;
                let Some(seq) = self.on_op(f, Source::Link(id)) else {
                    return;
                };
                if side == Side::Down {
                    Self::send(
                        &tx,
                        &Message::Ack(Ack {
                            op_id,
                            seq: Some(seq),
                        }),
                    );
                }
            }
            Message::Ack(a) => {
                if let Some(log) = log {
                    self.acked.entry(log).or_default().insert(a.op_id);
                    self.emit(LanEvent::Acked(a.op_id));
                }
            }
            Message::Change(c) => {
                if let Side::Up { hub_id } = &side {
                    let cursor = self.cursors.get(hub_id).filter(|k| k.log == c.head.log);
                    if cursor.is_none_or(|k| k.seq < c.head.seq) {
                        Self::send(
                            &tx,
                            &Message::Pull(Pull {
                                cursor: cursor.cloned(),
                                limit: MAX_PULL_LIMIT,
                            }),
                        );
                    }
                }
            }
            Message::Pull(p) => {
                let (frames, end) = self.journal.pull(&p);
                for f in frames {
                    Self::send(&tx, &Message::Op(f));
                }
                Self::send(&tx, &Message::PullEnd(end));
            }
            Message::PullEnd(end) => {
                if let Side::Up { hub_id } = &side {
                    let more = end.more;
                    let cursor = end.cursor;
                    self.cursors.insert(hub_id.clone(), cursor.clone());
                    if !more {
                        self.emit(LanEvent::Pulled);
                        for done in self.pull_waiters.drain(..) {
                            let _ = done.send(());
                        }
                    }
                    if more {
                        Self::send(
                            &tx,
                            &Message::Pull(Pull {
                                cursor: Some(cursor),
                                limit: MAX_PULL_LIMIT,
                            }),
                        );
                    }
                }
            }
            msg @ (Message::PbUpdate(_) | Message::PbTakeover(_)) => {
                self.on_playback(raw, msg, Source::Link(id));
            }
            Message::Family(f) => self.on_family(id, &f.body),
            Message::Vv(v) => {
                for e in self.nodes.missing_for(&v.vv) {
                    Self::send(&tx, &Message::Op(e.frame()));
                }
            }
            Message::Want(r) => {
                for e in self.nodes.range(&r) {
                    Self::send(&tx, &Message::Op(e.frame()));
                }
            }
            Message::Digest(d) => self.on_digest(&tx, &d),
            Message::Input(f) => self.on_input(raw, f, Source::Link(id)),
            Message::Music(f) => self.on_music(raw, f, Source::Link(id)),
            Message::Error(e) if e.code == code::UNAUTHORIZED => {
                let _ = tx.send(None);
            }
            _ => {}
        }
    }

    fn on_family(&mut self, from: u64, body: &str) {
        let Ok(update) = FamilyUpdate::from_json(body) else {
            return;
        };
        match self.family.apply_update(&update) {
            Ok(report) if report.roster.changed() || report.key_rotated => {
                self.emit(LanEvent::FamilyChanged);
                // Spread the change and drop links of revoked devices.
                for (id, link) in &self.links {
                    match self.family.update_for(&link.device_id) {
                        Ok(update) if *id != from => Self::send(
                            &link.tx,
                            &Message::Family(crate::proto::Family {
                                body: update.to_json(),
                            }),
                        ),
                        Ok(_) => {}
                        Err(_) => {
                            Self::send_error(&link.tx, code::UNAUTHORIZED, None);
                            let _ = link.tx.send(None);
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(pa_pairing::Error::Revoked(_)) => {
                self.revoked = true;
                self.emit(LanEvent::Revoked);
                for link in self.links.values() {
                    let _ = link.tx.send(None);
                }
            }
            Err(e) => tracing::debug!(error = %e, "lan sync: family update rejected"),
        }
    }
}

#[cfg(test)]
mod tests {
    use pa_pairing::{DeviceIdentity, DeviceKind};

    use super::*;
    use crate::proto::{ProtoRange, decode};
    use crate::{Op, OpBody};

    fn hello(device_id: &str) -> Hello {
        Hello {
            proto: ProtoRange::LOCAL,
            device_id: device_id.into(),
            role: Role::Device,
            cursor: None,
            head: None,
            nonce: None,
            vv: None,
        }
    }

    fn op(n: u128, body: OpBody) -> Op {
        Op {
            op_id: OpId(uuid::Uuid::from_u128(n)),
            hlc: Hlc {
                wall_ms: 1_763_000_000_000,
                counter: 0,
                device: "phone".into(),
            },
            aliases: Vec::new(),
            body,
        }
    }

    /// The messages a link was sent.
    fn sent(rx: &mut mpsc::UnboundedReceiver<Option<String>>) -> Vec<Message> {
        let mut out = Vec::new();
        while let Ok(Some(frame)) = rx.try_recv() {
            out.push(decode(&frame).unwrap());
        }
        out
    }

    /// SYNC-013: a node of an older build does not know the new op kinds.
    /// It keeps such an op in its journal, passes it on to the other
    /// devices and acknowledges it, but never hands it to its app; the ops
    /// it knows keep flowing. An unknown kind here is exactly what an older
    /// build sees of `category.upsert`.
    #[test]
    fn an_op_of_an_unknown_kind_is_relayed_and_acked_but_not_applied() {
        let identity = DeviceIdentity::generate("pc").unwrap();
        let family = Family::found(identity, "pc", DeviceKind::Desktop, 1).unwrap();
        let (events, mut app) = mpsc::unbounded_channel();
        let shared = Shared::new(LanConfig::new(DeviceClass::Desktop), family, events).unwrap();
        let mut state = shared.lock();
        let (phone_tx, mut phone) = mpsc::unbounded_channel();
        let (tv_tx, mut tv) = mpsc::unbounded_channel();
        let from_phone = state.register(Side::Down, &hello("phone"), phone_tx);
        state.register(Side::Down, &hello("tv"), tv_tx);
        sent(&mut phone);
        sent(&mut tv);

        let label = op(
            1,
            OpBody::CategoryUpsert {
                profile: "default".into(),
                category: "c1".into(),
                name: "To watch".into(),
            },
        );
        let mut frame = OpFrame::plain(&label).unwrap();
        frame.payload.body = frame
            .payload
            .body
            .replace("\"op\":\"category.upsert\"", "\"op\":\"category.v9\"");
        assert!(frame.open().is_err(), "this build cannot read it");
        let raw = encode(&Message::Op(frame.clone())).unwrap();
        state.on_message(from_phone, &raw, decode(&raw).unwrap());

        let to_phone = sent(&mut phone);
        assert!(
            matches!(to_phone.as_slice(), [Message::Ack(a)] if a.op_id == label.op_id),
            "{to_phone:?}"
        );
        let to_tv = sent(&mut tv);
        assert!(
            matches!(to_tv.as_slice(), [Message::Op(f)] if f.payload == frame.payload),
            "relayed unchanged: {to_tv:?}"
        );
        assert_eq!(state.journal_frames().len(), 1);
        while let Ok(event) = app.try_recv() {
            assert!(!matches!(event, LanEvent::Op(_)), "not applied: {event:?}");
        }

        // A known op after it reaches the app as before.
        let watched = op(
            2,
            OpBody::Watched {
                profile: "default".into(),
                key: "tmdb:movie:603".parse().unwrap(),
            },
        );
        let raw = encode(&Message::Op(OpFrame::plain(&watched).unwrap())).unwrap();
        state.on_message(from_phone, &raw, decode(&raw).unwrap());
        let mut got = Vec::new();
        while let Ok(event) = app.try_recv() {
            if let LanEvent::Op(op) = event {
                got.push(op);
            }
        }
        assert_eq!(got, [watched]);
        assert_eq!(sent(&mut tv).len(), 1);
    }

    /// SYNC-017: a link of protocol 2 gets what its `hello.vv` lacks, as
    /// log entries; a link of protocol 1 keeps the cursor path; a damaged
    /// entry is not acked and is asked for again; an intact one reaches a
    /// protocol-1 device as a plain op.
    #[test]
    fn protocol_2_links_run_anti_entropy_and_protocol_1_links_keep_working() {
        use crate::nodelog::{NodeLog, VersionVector};
        use crate::proto::Payload;

        let identity = DeviceIdentity::generate("pc").unwrap();
        let family = Family::found(identity, "pc", DeviceKind::Desktop, 1).unwrap();
        let (events, _app) = mpsc::unbounded_channel();
        let shared = Shared::new(LanConfig::new(DeviceClass::Desktop), family, events).unwrap();
        let mut state = shared.lock();
        let watched = |n: u128, key: &str| {
            op(
                n,
                OpBody::Watched {
                    profile: "default".into(),
                    key: key.parse().unwrap(),
                },
            )
        };
        for n in 1..=2 {
            let o = watched(n, "tmdb:movie:603");
            state.on_op(OpFrame::plain(&o).unwrap(), Source::App);
        }
        assert_eq!(state.nodes.len(), 2, "the node vouches for its app's ops");

        let (phone_tx, mut phone) = mpsc::unbounded_channel();
        let mut v2 = hello("phone");
        v2.vv = Some(VersionVector::default());
        let from_phone = state.register(Side::Down, &v2, phone_tx);
        let logged = sent(&mut phone)
            .into_iter()
            .filter(|m| matches!(m, Message::Op(f) if f.log.is_some()))
            .count();
        assert_eq!(logged, 2);

        let (tv_tx, mut tv) = mpsc::unbounded_channel();
        let mut v1 = hello("tv");
        v1.proto = ProtoRange { min: 1, max: 1 };
        state.register(Side::Down, &v1, tv_tx);
        let to_tv = sent(&mut tv);
        assert!(
            to_tv.iter().any(|m| matches!(m, Message::Change(_))),
            "{to_tv:?}"
        );
        assert!(!to_tv.iter().any(|m| matches!(m, Message::Op(_))));

        // The phone's own log; entry 1 arrives damaged.
        let mut phone_log = NodeLog::new("phone-run").unwrap();
        let o = watched(3, "tmdb:movie:604");
        let entry = phone_log.append_local(o.op_id, Payload::plain(&o).unwrap());
        let mut bad = entry.frame();
        bad.payload.body = bad.payload.body.replace("604", "605");
        let raw = encode(&Message::Op(bad)).unwrap();
        state.on_message(from_phone, &raw, decode(&raw).unwrap());
        let to_phone = sent(&mut phone);
        assert!(!to_phone.iter().any(|m| matches!(m, Message::Ack(_))));
        assert!(
            to_phone
                .iter()
                .any(|m| matches!(m, Message::Want(r) if r.origin == "phone-run" && r.from == 1)),
            "{to_phone:?}"
        );
        assert!(sent(&mut tv).is_empty(), "nothing stored, nothing relayed");

        let raw = encode(&Message::Op(entry.frame())).unwrap();
        state.on_message(from_phone, &raw, decode(&raw).unwrap());
        assert!(
            sent(&mut phone)
                .iter()
                .any(|m| matches!(m, Message::Ack(a) if a.op_id == o.op_id))
        );
        let to_tv = sent(&mut tv);
        assert!(
            matches!(to_tv.as_slice(), [Message::Op(f)] if f.open().unwrap() == o),
            "{to_tv:?}"
        );
        assert_eq!(state.nodes.vector().get("phone-run"), 1);

        // Periodic anti-entropy on the protocol-2 link only.
        state.anti_entropy(from_phone);
        let msgs = sent(&mut phone);
        assert!(msgs.iter().any(|m| matches!(m, Message::Vv(_))));
        assert!(msgs.iter().any(|m| matches!(m, Message::Digest(_))));
    }
}
