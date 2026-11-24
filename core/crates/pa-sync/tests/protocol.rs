// SPDX-License-Identifier: MPL-2.0
//! Message codec, version compatibility and op exchange (SYNC-002).

use pa_model::{MediaKey, MediaKind, SCHEMA_VERSION};
use pa_sync::proto::{
    self, Ack, Auth, Change, Cursor, ErrorFrame, Family, Hello, Journal, Message, OpFrame, Payload,
    PlayState, PlaybackUpdate, ProtoRange, Pull, PullEnd, Role, Takeover,
};
use pa_sync::nodelog::{self, Digest, NodeLog, Range, VersionVector};
use pa_sync::proto::{DigestFrame, VvFrame};
use pa_sync::{Error, Hlc, Op, OpBody, OpId, Replica, outbox};
use uuid::Uuid;

const T0: i64 = 1_763_000_000_000;

fn movie() -> MediaKey {
    "tmdb:movie:603".parse().unwrap()
}

fn hlc(device: &str) -> Hlc {
    Hlc {
        wall_ms: 1_763_000_000_000,
        counter: 2,
        device: device.into(),
    }
}

fn op(n: u128) -> Op {
    Op {
        aliases: Vec::new(),
        op_id: OpId(Uuid::from_u128(n)),
        hlc: hlc("phone"),
        body: OpBody::Watched {
            profile: "default".into(),
            key: movie(),
        },
    }
}

fn cursor(seq: u64) -> Cursor {
    Cursor {
        log: "j1".into(),
        seq,
    }
}

fn update(state: PlayState, position_ms: u64) -> PlaybackUpdate {
    PlaybackUpdate {
        aliases: Vec::new(),
        profile: "default".into(),
        key: movie(),
        position_ms,
        duration_ms: Some(8_160_000),
        rate: 1.0,
        state,
        hlc: hlc("phone"),
        ttl_ms: None,
    }
}

/// Exact frames of protocol v1. Changing any of them breaks older peers.
fn golden() -> Vec<(Message, String)> {
    let h = r#""hlc":{"wall_ms":1763000000000,"counter":2,"device":"phone"}"#;
    let id = "00000000-0000-0000-0000-000000000007";
    let payload = serde_json::to_string(&outbox::encode(&op(7)).unwrap()).unwrap();
    vec![
        (
            Message::Hello(Hello {
                proto: ProtoRange { min: 1, max: 1 },
                device_id: "phone".into(),
                role: Role::Device,
                cursor: Some(cursor(41)),
                head: None,
                nonce: None,
                vv: None,
            }),
            r#"{"t":"hello","proto":{"min":1,"max":1},"device_id":"phone","role":"device","cursor":{"log":"j1","seq":41}}"#.into(),
        ),
        (
            Message::Hello(Hello {
                proto: ProtoRange { min: 1, max: 1 },
                device_id: "tv".into(),
                role: Role::Device,
                cursor: None,
                head: None,
                nonce: Some("0a1b".into()),
                vv: None,
            }),
            r#"{"t":"hello","proto":{"min":1,"max":1},"device_id":"tv","role":"device","nonce":"0a1b"}"#.into(),
        ),
        (
            Message::Auth(Auth { sig: "c0ffee".into() }),
            r#"{"t":"auth","sig":"c0ffee"}"#.into(),
        ),
        (
            Message::Family(Family {
                body: r#"{"roster":{}}"#.into(),
            }),
            r#"{"t":"family","body":"{\"roster\":{}}"}"#.into(),
        ),
        (
            Message::PbUpdate(update(PlayState::Playing, 2_471_200)),
            format!(
                r#"{{"t":"pb.update","profile":"default","key":"tmdb:movie:603","position_ms":2471200,"duration_ms":8160000,"state":"playing",{h}}}"#
            ),
        ),
        (
            Message::PbTakeover(Takeover {
                aliases: Vec::new(),
                profile: "default".into(),
                key: movie(),
                hlc: hlc("phone"),
            }),
            format!(r#"{{"t":"pb.takeover","profile":"default","key":"tmdb:movie:603",{h}}}"#),
        ),
        (
            Message::Op(OpFrame::plain(&op(7)).unwrap()),
            format!(r#"{{"t":"op","op_id":"{id}","payload":{{"enc":"plain","body":{payload}}}}}"#),
        ),
        (
            Message::Ack(Ack {
                op_id: op(7).op_id,
                seq: Some(42),
            }),
            format!(r#"{{"t":"ack","op_id":"{id}","seq":42}}"#),
        ),
        (
            Message::Change(Change { head: cursor(42) }),
            r#"{"t":"change","head":{"log":"j1","seq":42}}"#.into(),
        ),
        (
            Message::Pull(Pull {
                cursor: Some(cursor(41)),
                limit: 500,
            }),
            r#"{"t":"pull","cursor":{"log":"j1","seq":41},"limit":500}"#.into(),
        ),
        (
            Message::PullEnd(PullEnd {
                cursor: cursor(42),
                more: false,
            }),
            r#"{"t":"pull.end","cursor":{"log":"j1","seq":42},"more":false}"#.into(),
        ),
        (
            Message::Error(ErrorFrame {
                code: proto::code::PROTO_UNSUPPORTED.into(),
                message: None,
                op_id: None,
            }),
            r#"{"t":"error","code":"proto.unsupported"}"#.into(),
        ),
    ]
}

#[test]
fn every_message_matches_its_golden_frame() {
    for (msg, frame) in golden().into_iter().chain(golden_v2()) {
        assert_eq!(proto::encode(&msg).unwrap(), frame);
        assert_eq!(proto::decode(&frame).unwrap(), msg);
    }
}

/// Frames added by protocol 2 (SYNC-017). Changing any of them breaks
/// peers of protocol 2.
fn golden_v2() -> Vec<(Message, String)> {
    let mut vv = VersionVector::default();
    vv.raise("phone", 7);
    vv.raise("srv", 3);
    let id = "00000000-0000-0000-0000-000000000007";
    let payload = serde_json::to_string(&outbox::encode(&op(7)).unwrap()).unwrap();
    let mut log = NodeLog::new("phone").unwrap();
    let entry = log.append_local(op(7).op_id, Payload::plain(&op(7)).unwrap());
    let hash = entry.hash();
    let genesis = nodelog::GENESIS;
    let digest = Digest {
        v: 1,
        root: "a".repeat(64),
        parts: [("watched/default/3".to_owned(), "b".repeat(64))].into(),
    };
    vec![
        (
            Message::Hello(Hello {
                proto: ProtoRange { min: 1, max: 2 },
                device_id: "phone".into(),
                role: Role::Device,
                cursor: None,
                head: None,
                nonce: None,
                vv: Some(vv.clone()),
            }),
            r#"{"t":"hello","proto":{"min":1,"max":2},"device_id":"phone","role":"device","vv":{"phone":7,"srv":3}}"#.into(),
        ),
        (
            Message::Op(entry.frame()),
            format!(
                r#"{{"t":"op","op_id":"{id}","log":{{"origin":"phone","oseq":1,"prev":"{genesis}","hash":"{hash}"}},"payload":{{"enc":"plain","body":{payload}}}}}"#
            ),
        ),
        (
            Message::Vv(VvFrame { vv }),
            r#"{"t":"vv","vv":{"phone":7,"srv":3}}"#.into(),
        ),
        (
            Message::Want(Range {
                origin: "srv".into(),
                from: 2,
                to: 3,
            }),
            r#"{"t":"want","origin":"srv","from":2,"to":3}"#.into(),
        ),
        (
            Message::Digest(DigestFrame {
                digest,
                reply: true,
            }),
            format!(
                r#"{{"t":"digest","digest":{{"v":1,"root":"{}","parts":{{"watched/default/3":"{}"}}}},"reply":true}}"#,
                "a".repeat(64),
                "b".repeat(64)
            ),
        ),
    ]
}

/// A peer of protocol 1 reads frames of protocol 2: the new fields are
/// ignored, the new messages are unknown to it.
#[test]
fn protocol_1_reads_protocol_2_frames() {
    #[derive(serde::Deserialize)]
    struct OpV1 {
        #[allow(dead_code)]
        op_id: OpId,
        payload: Payload,
    }
    for (msg, frame) in golden_v2() {
        let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
        match msg {
            Message::Op(f) => {
                let v1: OpV1 = serde_json::from_value(value).unwrap();
                assert_eq!(v1.payload, f.payload);
            }
            Message::Hello(_) => {
                let h: Hello = serde_json::from_value(value).unwrap();
                assert_eq!(h.device_id, "phone");
            }
            _ => assert!(["vv", "want", "digest"].contains(&value["t"].as_str().unwrap())),
        }
    }
}

#[test]
fn plain_payload_opens_to_the_same_op() {
    let frame = OpFrame::plain(&op(7)).unwrap();
    assert_eq!(frame.open().unwrap(), op(7));
    let body: serde_json::Value = serde_json::from_str(&frame.payload.body).unwrap();
    assert_eq!(body["schema_version"], SCHEMA_VERSION);
}

#[test]
fn versions_one_apart_are_compatible() {
    for a in 1..=6u16 {
        for b in 1..=6u16 {
            let got = proto::negotiate(ProtoRange::for_version(a), ProtoRange::for_version(b));
            if a.abs_diff(b) <= 1 {
                assert_eq!(got.unwrap(), a.min(b), "v{a} with v{b}");
            } else {
                assert!(
                    matches!(got, Err(Error::Incompatible { .. })),
                    "v{a} with v{b}"
                );
            }
        }
    }
    assert_eq!(ProtoRange::LOCAL.max, proto::PROTOCOL_VERSION);
    assert_eq!(ProtoRange::LOCAL.min, proto::MIN_PROTOCOL_VERSION);
}

#[test]
fn frames_of_the_next_version_still_decode() {
    // v3 adds a message type, fields and enum values; v2 ignores them.
    let unknown = r#"{"t":"pb.queue","items":["tmdb:movie:603"]}"#;
    assert_eq!(proto::decode(unknown).unwrap(), Message::Unknown);

    let hello = r#"{"t":"hello","proto":{"min":2,"max":3},"device_id":"tv","role":"relay","caps":["zstd"]}"#;
    let Message::Hello(h) = proto::decode(hello).unwrap() else {
        panic!("not a hello");
    };
    assert_eq!(h.role, Role::Other);
    assert_eq!(proto::negotiate(ProtoRange::LOCAL, h.proto).unwrap(), 2);

    let pb = r#"{"t":"pb.update","profile":"default","key":"tmdb:movie:603","position_ms":5,"state":"buffering","audio":"eng","hlc":{"wall_ms":1,"counter":0,"device":"tv"}}"#;
    let Message::PbUpdate(u) = proto::decode(pb).unwrap() else {
        panic!("not a pb.update");
    };
    assert_eq!(
        (u.state, u.rate, u.duration_ms),
        (PlayState::Other, 1.0, None)
    );
}

#[test]
fn ops_of_a_newer_schema_are_relayed_unread() {
    let mut frame = OpFrame::plain(&op(1)).unwrap();
    frame.payload.body = frame.payload.body.replacen(
        &format!("\"schema_version\":{SCHEMA_VERSION}"),
        &format!("\"schema_version\":{}", SCHEMA_VERSION + 1),
        1,
    );
    assert!(matches!(
        frame.open(),
        Err(Error::Invalid("schema version"))
    ));
    let sealed = OpFrame {
        payload: Payload {
            enc: "secretbox.v1".into(),
            body: "bm9uY2UrY2lwaGVydGV4dA==".into(),
        },
        ..frame.clone()
    };
    assert!(matches!(
        sealed.open(),
        Err(Error::Invalid("payload encryption"))
    ));

    // The hub stores both without reading them.
    let mut hub = Journal::new("j1").unwrap();
    let wire = proto::encode(&Message::Op(sealed.clone())).unwrap();
    let Message::Op(got) = proto::decode(&wire).unwrap() else {
        panic!("not an op");
    };
    assert_eq!(hub.append(&got), (1, true));
}

#[test]
fn bad_frames_are_rejected() {
    let bad = [
        r#"{"t":"hello","proto":{"min":2,"max":1},"device_id":"tv","role":"device"}"#,
        r#"{"t":"hello","proto":{"min":1,"max":1},"device_id":"bad id","role":"device"}"#,
        r#"{"t":"pb.update","profile":"default","key":"tmdb:movie:603","position_ms":5,"rate":0,"state":"playing","hlc":{"wall_ms":1,"counter":0,"device":"tv"}}"#,
        r#"{"t":"pb.takeover","profile":"","key":"tmdb:movie:603","hlc":{"wall_ms":1,"counter":0,"device":"tv"}}"#,
        r#"{"t":"pull","limit":0}"#,
        r#"{"t":"pull","limit":1001}"#,
        r#"{"t":"change","head":{"log":"","seq":1}}"#,
        r#"{"t":"op","op_id":"x","payload":{"enc":"plain","body":"{}"}}"#,
        r#"{"t":"error","code":""}"#,
        r#"{"t":"hello","proto":{"min":1,"max":1},"device_id":"tv","role":"device","nonce":"zz"}"#,
        r#"{"t":"auth","sig":""}"#,
        r#"{"t":"auth","sig":"not hex"}"#,
        r#"{"t":"family","body":""}"#,
        r#"{"proto":{"min":1,"max":1}}"#,
        "not json",
    ];
    for frame in bad {
        assert!(proto::decode(frame).is_err(), "{frame}");
    }
    let huge = format!(
        r#"{{"t":"error","code":"x","pad":"{}"}}"#,
        "a".repeat(proto::MAX_FRAME_LEN)
    );
    assert!(proto::decode(&huge).is_err());
    assert!(proto::encode(&Message::Unknown).is_err());

    let mut frame = OpFrame::plain(&op(1)).unwrap();
    frame.op_id = op(2).op_id;
    assert!(matches!(frame.open(), Err(Error::Invalid("op id"))));
}

#[test]
fn journal_dedups_and_pages_by_cursor() {
    let mut j = Journal::new("j1").unwrap();
    for n in 1..=5 {
        assert_eq!(j.append(&OpFrame::plain(&op(n)).unwrap()), (n as u64, true));
    }
    assert_eq!(j.append(&OpFrame::plain(&op(3)).unwrap()), (3, false));
    assert_eq!(j.head(), cursor(5));

    let (frames, end) = j.pull(&Pull {
        cursor: Some(cursor(1)),
        limit: 3,
    });
    let seqs: Vec<_> = frames.iter().map(|f| f.seq.unwrap()).collect();
    assert_eq!(seqs, [2, 3, 4]);
    assert_eq!(
        end,
        PullEnd {
            cursor: cursor(4),
            more: true
        }
    );

    let (frames, end) = j.pull(&Pull {
        cursor: Some(end.cursor),
        limit: 3,
    });
    assert_eq!(frames.len(), 1);
    assert_eq!(
        end,
        PullEnd {
            cursor: cursor(5),
            more: false
        }
    );

    // A cursor of another journal (new hub) starts from the beginning.
    let (frames, _) = j.pull(&Pull {
        cursor: Some(Cursor {
            log: "old-hub".into(),
            seq: 4,
        }),
        limit: 10,
    });
    assert_eq!(frames.len(), 5);
    assert!(Journal::new("").is_err());
}

/// SYNC-014: a player sends its position every second; the journal keeps
/// only the newest `progress.upsert` of a title, the older ones leave holes
/// that keep the cursors and pages right.
#[test]
fn journal_keeps_the_newest_position_of_a_title() {
    let position = |n: u128, wall: u64, key: &str| Op {
        aliases: Vec::new(),
        op_id: OpId(Uuid::from_u128(n)),
        hlc: Hlc {
            wall_ms: wall,
            counter: 0,
            device: "phone".into(),
        },
        body: OpBody::ProgressUpsert {
            profile: "default".into(),
            key: key.parse().unwrap(),
            kind: MediaKind::Movie,
            position_ms: wall,
            duration_ms: None,
        },
    };
    let mut j = Journal::new("j1").unwrap();
    // seq 1: watched; 2–4: positions of movie 603; 5: movie 604; 6: an
    // older position of 603 that arrived late.
    j.append(&OpFrame::plain(&op(1)).unwrap());
    for (n, wall) in [(2, 1_000), (3, 2_000), (4, 3_000)] {
        j.append(&OpFrame::plain(&position(n, wall, "tmdb:movie:603")).unwrap());
    }
    j.append(&OpFrame::plain(&position(5, 1_500, "tmdb:movie:604")).unwrap());
    assert_eq!(
        j.append(&OpFrame::plain(&position(6, 500, "tmdb:movie:603")).unwrap()),
        (6, true)
    );
    assert_eq!(j.head(), cursor(6));
    let kept: Vec<u64> = j.iter().map(|f| f.seq.unwrap()).collect();
    assert_eq!(kept, [1, 4, 5]);
    // A dropped op is still known: no second append.
    assert_eq!(
        j.append(&OpFrame::plain(&position(2, 1_000, "tmdb:movie:603")).unwrap()),
        (2, false)
    );

    // Pages skip the holes; the cursor ends after the last slot looked at.
    let (frames, end) = j.pull(&Pull {
        cursor: Some(cursor(0)),
        limit: 2,
    });
    let seqs: Vec<_> = frames.iter().map(|f| f.seq.unwrap()).collect();
    assert_eq!(seqs, [1, 4]);
    assert_eq!(
        end,
        PullEnd {
            cursor: cursor(4),
            more: true
        }
    );
    let (frames, end) = j.pull(&Pull {
        cursor: Some(end.cursor),
        limit: 2,
    });
    let seqs: Vec<_> = frames.iter().map(|f| f.seq.unwrap()).collect();
    assert_eq!(seqs, [5]);
    assert_eq!(
        end,
        PullEnd {
            cursor: cursor(6),
            more: false
        }
    );
    // Past the end, or behind only holes: nothing, and the end.
    let (frames, end) = j.pull(&Pull {
        cursor: Some(cursor(5)),
        limit: 10,
    });
    assert!(frames.is_empty());
    assert_eq!(end.cursor, cursor(6));
    assert!(!end.more);
}

/// Sends `msg` over a pretend wire: encode, then decode on the other side.
fn wire(msg: Message) -> Message {
    proto::decode(&proto::encode(&msg).unwrap()).unwrap()
}

#[test]
fn phone_hands_playback_over_to_tv_through_a_hub() {
    let phone_db = pa_library::open_in_memory().unwrap();
    let mut phone = Replica::new("phone").unwrap();
    let mut tv = Replica::new("tv").unwrap();
    let mut hub = Journal::new("j1").unwrap();

    // Offline on the phone: the user pauses at 41:11.
    let progress = OpBody::ProgressUpsert {
        profile: "default".into(),
        key: movie(),
        kind: MediaKind::Movie,
        position_ms: 2_471_200,
        duration_ms: Some(8_160_000),
    };
    let recorded = phone.record(&phone_db, progress, T0).unwrap();

    // Phone connects: both sides say hello and agree on a version.
    let Message::Hello(from_hub) = wire(Message::Hello(Hello {
        proto: ProtoRange::LOCAL,
        device_id: "server".into(),
        role: Role::Hub,
        cursor: None,
        head: Some(hub.head()),
        nonce: None,
        vv: None,
    })) else {
        panic!("not a hello");
    };
    assert_eq!(
        proto::negotiate(ProtoRange::LOCAL, from_hub.proto).unwrap(),
        proto::PROTOCOL_VERSION
    );

    // Outbox first: every due op goes out, the hub acks, the outbox drains.
    for pending in outbox::due(&phone_db, T0, 100).unwrap() {
        let Message::Op(frame) = wire(Message::Op(OpFrame::plain(&pending.op).unwrap())) else {
            panic!("not an op");
        };
        let (seq, _) = hub.append(&frame);
        let Message::Ack(ack) = wire(Message::Ack(Ack {
            op_id: frame.op_id,
            seq: Some(seq),
        })) else {
            panic!("not an ack");
        };
        assert!(outbox::ack(&phone_db, ack.op_id).unwrap());
    }
    assert_eq!(outbox::len(&phone_db).unwrap(), 0);

    // Phone resends after a lost ack: the hub keeps one copy, same seq.
    assert_eq!(hub.append(&OpFrame::plain(&recorded).unwrap()), (1, false));

    // The TV wakes up later with no cursor and pulls.
    let Message::Pull(pull) = wire(Message::Pull(Pull {
        cursor: None,
        limit: proto::MAX_PULL_LIMIT,
    })) else {
        panic!("not a pull");
    };
    let (frames, end) = hub.pull(&pull);
    for frame in frames {
        let Message::Op(frame) = wire(Message::Op(frame)) else {
            panic!("not an op");
        };
        assert!(tv.receive(&frame.open().unwrap(), T0 + 50).unwrap());
    }
    assert!(!end.more);
    let pos = &tv.state().progress[&("default".to_owned(), movie())];
    assert_eq!(pos.value.position_ms, 2_471_200);

    // The TV starts playing and tells the phone (same profile) to pause.
    let start = Takeover {
        aliases: Vec::new(),
        profile: "default".into(),
        key: movie(),
        hlc: tv.clock_mut().tick(u64::try_from(T0).unwrap() + 100),
    };
    let Message::PbTakeover(got) = wire(Message::PbTakeover(start.clone())) else {
        panic!("not a takeover");
    };
    assert_eq!(got, start);
    assert!(got.hlc > recorded.hlc);
}

/// SYNC-004: `aliases` of ops and `pb.*` are optional and read leniently.
/// Without aliases the frames are the v1 frames above; with them, a reader
/// that does not know the field (an older build) still gets the same op,
/// and keys of unknown kinds in the list are skipped, not fatal.
#[test]
fn aliases_are_optional_and_unknown_kinds_are_skipped() {
    let mut o = op(7);
    assert!(!serde_json::to_string(&o).unwrap().contains("aliases"));
    o.aliases = vec![
        "file:8e245d9679d31e12".into(),
        "anidb:episode:23:1".into(),
        "imdb:movie:tt0133093".into(),
    ];
    let json = serde_json::to_string(&o).unwrap();
    let back: Op = serde_json::from_str(&json).unwrap();
    assert_eq!(back, o);
    let known: Vec<String> = back.alias_keys().iter().map(ToString::to_string).collect();
    assert_eq!(known, ["file:8e245d9679d31e12", "anidb:episode:23:1"]);
    // An unknown top-level field next to the flattened body is ignored, as
    // `aliases` is by builds before SYNC-004.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["from_the_future"] = serde_json::json!([1, 2]);
    let future: Op = serde_json::from_value(value).unwrap();
    assert_eq!(future.body, o.body);

    let mut u = update(PlayState::Playing, 1_000);
    u.aliases = vec!["file:8e245d9679d31e12".into()];
    assert_eq!(wire(Message::PbUpdate(u.clone())), Message::PbUpdate(u));
}
