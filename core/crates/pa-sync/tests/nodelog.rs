// SPDX-License-Identifier: MPL-2.0
//! SYNC-017 done-when: five nodes with random network partitions, reordered
//! and duplicated deliveries, lost frames, damaged entries and restarts end
//! with equal version vectors and equal states; a damaged entry is seen and
//! fetched again; digests find a state that went wrong.

use std::collections::BTreeSet;

use pa_model::{MediaKey, MediaKind};
use pa_sync::nodelog::{Entry, Inserted, NodeLog, Range, VersionVector};
use pa_sync::proto::Payload;
use pa_sync::{Hlc, Op, OpBody, OpId, SyncState};
use proptest::prelude::*;
use uuid::Uuid;

const N: usize = 5;
const NAMES: [&str; N] = ["srv-a", "srv-b", "pc", "tv", "phone"];
const PROFILES: [&str; 2] = ["default", "kid"];
const KEYS: [&str; 3] = ["tmdb:movie:603", "tmdb:episode:1399:1:1", "twitch:vod:123"];

fn key(i: usize) -> MediaKey {
    KEYS[i].parse().unwrap()
}

fn body() -> impl Strategy<Value = OpBody> {
    let p = (0..PROFILES.len()).prop_map(|i| PROFILES[i].to_owned());
    let k = (0..KEYS.len()).prop_map(key);
    prop_oneof![
        (p.clone(), k.clone(), 0..4u64).prop_map(|(profile, key, pos)| {
            OpBody::ProgressUpsert {
                profile,
                key,
                kind: MediaKind::Movie,
                position_ms: pos * 1000,
                duration_ms: None,
            }
        }),
        (p.clone(), k.clone()).prop_map(|(profile, key)| OpBody::Watched { profile, key }),
        (p.clone(), k).prop_map(|(profile, key)| OpBody::Unwatch { profile, key }),
        (p.clone(), "tmdb:movie:60[34]", any::<bool>()).prop_map(|(profile, target, favorite)| {
            OpBody::Favorite {
                profile,
                target,
                favorite,
            }
        }),
        ("player\\.[ab]", proptest::option::of("[0-9]"))
            .prop_map(|(key, value)| OpBody::Setting { key, value }),
        (p, "c[12]", "tmdb:movie:60[34]", any::<bool>()).prop_map(
            |(profile, category, target, member)| OpBody::CategoryItem {
                profile,
                category,
                target,
                member,
            }
        ),
    ]
}

#[derive(Debug, Clone)]
enum Step {
    /// Node writes an op; its wall clock is `wall` (clocks disagree freely).
    Write { node: usize, wall: u64, body: OpBody },
    /// One anti-entropy pass `from` → `to`: the frames go out shuffled by
    /// `seed`, some lost, some twice, one maybe damaged.
    Sync {
        from: usize,
        to: usize,
        seed: u64,
        lose: u8,
        damage: bool,
    },
    /// The link between two nodes goes down or up.
    Cut { a: usize, b: usize },
    /// A node restarts: what waited for a hole is gone, the gapless log
    /// stays, the state is rebuilt from it.
    Restart { node: usize },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0..N, 0..50u64, body()).prop_map(|(node, wall, body)| Step::Write { node, wall, body }),
        5 => (0..N, 0..N, any::<u64>(), 0..4u8, proptest::bool::weighted(0.1))
            .prop_map(|(from, to, seed, lose, damage)| Step::Sync { from, to, seed, lose, damage }),
        1 => (0..N, 0..N).prop_map(|(a, b)| Step::Cut { a, b }),
        1 => (0..N).prop_map(|node| Step::Restart { node }),
    ]
}

struct Node {
    log: NodeLog,
    state: SyncState,
}

impl Node {
    fn new(i: usize) -> Self {
        Self {
            log: NodeLog::new(NAMES[i]).unwrap(),
            state: SyncState::default(),
        }
    }

    fn restart(&mut self) {
        let me = self.log.me().to_owned();
        let kept: Vec<Entry> = self.log.gapless().cloned().collect();
        let (log, refused) = NodeLog::restore(me, kept).unwrap();
        assert_eq!(refused, 0, "a stored gapless log restores whole");
        self.log = log;
        self.state = SyncState::default();
        for e in self.log.entries() {
            self.state.apply(&e.open().unwrap());
        }
    }

    /// Takes one frame's entry; returns what to ask for again.
    fn take(&mut self, entry: Entry, hash: &str) -> Option<Range> {
        let op = entry.open();
        match self.log.insert(entry, hash) {
            Inserted::New { .. } | Inserted::Gap { .. } => {
                // Ops apply at once, in any order (LWW); the log keeps
                // completeness.
                if let Some(op) = op {
                    self.state.apply(&op);
                }
                None
            }
            Inserted::Duplicate => None,
            Inserted::Corrupt { want } => Some(want),
            other => panic!("no forks or overflows here: {other:?}"),
        }
    }
}

/// Deterministic shuffle (xorshift) so a failing case replays exactly.
fn shuffle<T>(items: &mut [T], mut seed: u64) {
    seed |= 1;
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

/// What `from` sends `to` in one pass: what the vector says `to` lacks and
/// the holes `to` asked about.
fn outgoing(from: &Node, to: &Node) -> Vec<Entry> {
    let mut out = from.log.missing_for(&to.log.vector());
    for gap in to.log.gaps() {
        out.extend(from.log.range(&gap));
    }
    out
}

struct World {
    nodes: Vec<Node>,
    cut: BTreeSet<(usize, usize)>,
    written: Vec<Op>,
    next_id: u128,
    damaged: usize,
}

impl World {
    fn new() -> Self {
        Self {
            nodes: (0..N).map(Node::new).collect(),
            cut: BTreeSet::new(),
            written: Vec::new(),
            next_id: 1,
            damaged: 0,
        }
    }

    fn link(a: usize, b: usize) -> (usize, usize) {
        (a.min(b), a.max(b))
    }

    fn run(&mut self, step: Step) {
        match step {
            Step::Write { node, wall, body } => {
                let op = Op {
                    op_id: OpId(Uuid::from_u128(self.next_id)),
                    hlc: Hlc {
                        wall_ms: wall,
                        counter: 0,
                        device: NAMES[node].to_owned(),
                    },
                    aliases: Vec::new(),
                    body,
                };
                self.next_id += 1;
                let n = &mut self.nodes[node];
                n.log.append_local(op.op_id, Payload::plain(&op).unwrap());
                n.state.apply(&op);
                self.written.push(op);
            }
            Step::Sync {
                from,
                to,
                seed,
                lose,
                damage,
            } => {
                if from == to || self.cut.contains(&Self::link(from, to)) {
                    return;
                }
                let mut frames: Vec<(Entry, String)> = outgoing(&self.nodes[from], &self.nodes[to])
                    .into_iter()
                    .map(|e| {
                        let h = e.hash();
                        (e, h)
                    })
                    .collect();
                // Duplicates: every third frame goes twice.
                let dups: Vec<_> = frames.iter().step_by(3).cloned().collect();
                frames.extend(dups);
                shuffle(&mut frames, seed);
                // Losses: drop every `lose`-th frame (0: none).
                if lose > 0 {
                    frames = frames
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| (i + 1) % (lose as usize + 1) != 0)
                        .map(|(_, f)| f)
                        .collect();
                }
                if damage && let Some((e, _)) = frames.first_mut() {
                    e.payload.body.push(' ');
                    self.damaged += 1;
                }
                let mut wants = Vec::new();
                for (e, h) in frames {
                    wants.extend(self.nodes[to].take(e, &h));
                }
                // A damaged entry is asked for again — here from the same
                // peer; any other member would do.
                for want in wants {
                    let again = self.nodes[from].log.range(&want);
                    for e in again {
                        let h = e.hash();
                        assert!(self.nodes[to].take(e, &h).is_none());
                    }
                }
            }
            Step::Cut { a, b } => {
                let l = Self::link(a, b);
                if !self.cut.remove(&l) {
                    self.cut.insert(l);
                }
            }
            Step::Restart { node } => self.nodes[node].restart(),
        }
    }

    /// The network heals: every pair syncs until nothing moves.
    fn heal(&mut self) {
        self.cut.clear();
        for _ in 0..N + 2 {
            for from in 0..N {
                for to in 0..N {
                    if from != to {
                        self.run(Step::Sync {
                            from,
                            to,
                            seed: 7,
                            lose: 0,
                            damage: false,
                        });
                    }
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn five_nodes_converge(steps in proptest::collection::vec(step(), 0..120)) {
        let mut world = World::new();
        for s in steps {
            world.run(s);
        }
        world.heal();
        let mut expected = SyncState::default();
        for op in &world.written {
            expected.apply(op);
        }
        let mut full = VersionVector::default();
        for (i, name) in NAMES.iter().enumerate() {
            let own = world.written.iter().filter(|o| o.hlc.device == NAMES[i]).count();
            if own > 0 {
                full.raise(name, own as u64);
            }
        }
        for n in &world.nodes {
            prop_assert_eq!(n.log.vector(), full.clone());
            prop_assert!(n.log.gaps().is_empty());
            prop_assert_eq!(&n.state, &expected);
            // The digest of the logs is the digest of the state.
            prop_assert_eq!(n.log.digest(), n.state.digest());
        }
    }
}

fn op(n: u128, device: &str, key: usize) -> Op {
    Op {
        op_id: OpId(Uuid::from_u128(n)),
        hlc: Hlc {
            wall_ms: 1_763_000_000_000 + n as u64,
            counter: 0,
            device: device.into(),
        },
        aliases: Vec::new(),
        body: OpBody::Watched {
            profile: "default".into(),
            key: self::key(key),
        },
    }
}

/// A damaged entry is refused, the hole stays visible, and the range is
/// fetched from another member.
#[test]
fn a_damaged_entry_is_detected_and_fetched_again() {
    let mut a = NodeLog::new("a").unwrap();
    let entries: Vec<Entry> = (1..=3)
        .map(|n| {
            let o = op(n, "a", n as usize - 1);
            a.append_local(o.op_id, Payload::plain(&o).unwrap())
        })
        .collect();
    // b got everything intact; c gets entry 2 damaged on the way.
    let (b, _) = NodeLog::restore("b", entries.clone()).unwrap();
    let mut c = NodeLog::new("c").unwrap();
    let mut damaged = entries[1].clone();
    damaged.payload.body = damaged.payload.body.replace("default", "dEfault");
    let r1 = c.insert(entries[0].clone(), &entries[0].hash());
    assert!(matches!(r1, Inserted::New { .. }));
    let want = match c.insert(damaged, &entries[1].hash()) {
        Inserted::Corrupt { want } => want,
        other => panic!("{other:?}"),
    };
    assert_eq!((want.origin.as_str(), want.from, want.to), ("a", 2, 2));
    assert!(matches!(
        c.insert(entries[2].clone(), &entries[2].hash()),
        Inserted::Gap { .. }
    ));
    assert_eq!(c.vector().get("a"), 1);
    assert_eq!(c.gaps(), [want.clone()]);
    // From any other member: b.
    for e in b.range(&want) {
        let h = e.hash();
        assert!(matches!(c.insert(e, &h), Inserted::New { ready } if ready.len() == 2));
    }
    assert_eq!(c.vector(), a.vector());
    assert!(c.gaps().is_empty());

    // The same over the wire: a frame whose payload was changed after the
    // hash was taken does not decode into a stored entry.
    let mut frame = entries[1].frame();
    frame.payload.body.push(' ');
    let mut d = NodeLog::new("d").unwrap();
    assert!(matches!(
        d.insert_frame(&frame),
        Some(Inserted::Corrupt { .. })
    ));
    assert!(d.is_empty());
}

/// An op applied wrongly (here: lost from the state after it was logged)
/// shows up as a differing partition; re-applying that partition's ops
/// repairs it.
#[test]
fn a_digest_finds_and_repairs_a_wrong_state() {
    let mut a = NodeLog::new("a").unwrap();
    let mut good = SyncState::default();
    let mut bad = SyncState::default();
    for n in 1..=12u128 {
        let o = op(n, "a", (n % 3) as usize);
        let o = Op {
            body: OpBody::ProgressUpsert {
                profile: PROFILES[(n % 2) as usize].into(),
                key: key((n % 3) as usize),
                kind: MediaKind::Movie,
                position_ms: n as u64,
                duration_ms: None,
            },
            ..o
        };
        a.append_local(o.op_id, Payload::plain(&o).unwrap());
        good.apply(&o);
        if n != 12 {
            bad.apply(&o);
        }
    }
    assert_eq!(a.digest(), good.digest());
    let diff = good.digest().diff(&bad.digest());
    assert_eq!(diff.len(), 1, "{diff:?}");
    for e in a.entries_in(&diff) {
        bad.apply(&e.open().unwrap());
    }
    assert_eq!(bad, good);
    assert!(good.digest().diff(&bad.digest()).is_empty());
}

/// Every op kind names the register its state keeps (digest leaves of the
/// state and of the logs agree).
#[test]
fn state_registers_match_op_registers() {
    let bodies = [
        OpBody::Rating {
            profile: "default".into(),
            target: "tmdb:movie:603".into(),
            rating: Some(7),
        },
        OpBody::PlaylistUpsert {
            profile: "default".into(),
            playlist: "pl".into(),
            name: "Mix".into(),
            video: true,
        },
        OpBody::PlaylistItems {
            profile: "default".into(),
            playlist: "pl".into(),
            items: vec![key(0)],
        },
        OpBody::FolderRule {
            rule_key: "r1".into(),
            rule: Some("x".into()),
        },
        OpBody::TwitchFollow {
            profile: "default".into(),
            channel_id: "1".into(),
            followed: true,
        },
        OpBody::SourceDelete { source: "s1".into() },
        OpBody::ProfileDelete {
            profile: "kid".into(),
        },
        OpBody::CategoryUpsert {
            profile: "default".into(),
            category: "c1".into(),
            name: "To watch".into(),
        },
        OpBody::TagSet {
            profile: "default".into(),
            tag: "Sci-Fi".into(),
            target: "tmdb:movie:603".into(),
            tagged: true,
        },
    ];
    for (i, body) in bodies.into_iter().enumerate() {
        let o = Op {
            body,
            ..op(i as u128 + 1, "a", 0)
        };
        let mut s = SyncState::default();
        assert!(s.apply(&o));
        let regs: Vec<String> = s.registers().into_iter().map(|(r, _)| r).collect();
        assert_eq!(regs, [o.body.register()], "{:?}", o.body);
    }
}
