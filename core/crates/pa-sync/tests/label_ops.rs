// SPDX-License-Identifier: MPL-2.0
//! SYNC-013: the ops of user categories, their items and tags — wire form,
//! checks, registers, last writer wins — and what a node of an older build
//! does with them (it relays them unread and keeps syncing the rest).

use pa_sync::proto::{self, Journal, Message, OpFrame, Pull};
use pa_sync::{Hlc, Op, OpBody, OpId, Replica, SyncState, outbox};
use uuid::Uuid;

const T0: i64 = 1_763_000_000_000;

fn op(n: u128, wall_ms: u64, device: &str, body: OpBody) -> Op {
    Op {
        op_id: OpId(Uuid::from_u128(n)),
        hlc: Hlc {
            wall_ms,
            counter: 0,
            device: device.into(),
        },
        aliases: Vec::new(),
        body,
    }
}

fn upsert(category: &str, name: &str) -> OpBody {
    OpBody::CategoryUpsert {
        profile: "default".into(),
        category: category.into(),
        name: name.into(),
    }
}

fn item(category: &str, target: &str, member: bool) -> OpBody {
    OpBody::CategoryItem {
        profile: "default".into(),
        category: category.into(),
        target: target.into(),
        member,
    }
}

fn tag(name: &str, target: &str, tagged: bool) -> OpBody {
    OpBody::TagSet {
        profile: "default".into(),
        tag: name.into(),
        target: target.into(),
        tagged,
    }
}

fn rating(target: &str, rating: Option<u8>) -> OpBody {
    OpBody::Rating {
        profile: "default".into(),
        target: target.into(),
        rating,
    }
}

/// Every new op kind, as this build sends it.
fn label_ops() -> Vec<Op> {
    vec![
        op(1, 1, "phone", upsert("c1", "To watch")),
        op(
            2,
            2,
            "phone",
            OpBody::CategoryDelete {
                profile: "default".into(),
                category: "c1".into(),
            },
        ),
        op(3, 3, "phone", item("c1", "tmdb:movie:603", true)),
        op(4, 4, "phone", tag("Sci-Fi", "tmdb:tv:1399", true)),
        op(5, 5, "phone", rating("tmdb:movie:603", Some(8))),
    ]
}

#[test]
fn wire_form_of_the_label_ops() {
    let mut item = op(3, 3, "phone", item("c1", "tmdb:movie:603", true));
    item.aliases = vec!["file:8e245d9679d31e12".into()];
    let v: serde_json::Value = serde_json::from_str(&outbox::encode(&item).unwrap()).unwrap();
    assert_eq!(v["data"]["op"], "category.item");
    assert_eq!(v["data"]["category"], "c1");
    assert_eq!(v["data"]["target"], "tmdb:movie:603");
    assert_eq!(v["data"]["member"], true);
    assert_eq!(v["data"]["aliases"][0], "file:8e245d9679d31e12");
    let kinds: Vec<String> = label_ops()
        .iter()
        .map(|o| {
            let v: serde_json::Value = serde_json::from_str(&outbox::encode(o).unwrap()).unwrap();
            v["data"]["op"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "category.upsert",
            "category.delete",
            "category.item",
            "tag.set",
            "rating.set"
        ]
    );
    for o in label_ops() {
        let payload = outbox::encode(&o).unwrap();
        assert_eq!(outbox::decode(&payload).unwrap(), o);
    }
}

#[test]
fn label_ops_are_checked() {
    for body in label_ops().into_iter().map(|o| o.body) {
        body.validate().unwrap();
    }
    let bad = [
        upsert("C1", "x"),
        upsert("c 1", "x"),
        upsert(&"c".repeat(65), "x"),
        upsert("c1", "  "),
        upsert("c1", &"x".repeat(101)),
        upsert("c1", "a\u{7}b"),
        item("c1", "", true),
        item("c1", &"x".repeat(300), true),
        tag(" ", "tmdb:movie:1", true),
        tag(&"t".repeat(65), "tmdb:movie:1", true),
        rating("tmdb:movie:1", Some(11)),
    ];
    for body in bad {
        assert!(body.validate().is_err(), "{body:?}");
    }
}

#[test]
fn registers_key_tags_by_name_key_and_categories_by_id() {
    assert_eq!(
        tag("Sci-Fi", "t", true).register(),
        tag("  sci-FI ", "t", false).register()
    );
    assert_ne!(
        tag("Sci-Fi", "t", true).register(),
        tag("Sci-Fi", "u", true).register()
    );
    assert_eq!(
        upsert("c1", "A").register(),
        OpBody::CategoryDelete {
            profile: "default".into(),
            category: "c1".into()
        }
        .register()
    );
    assert_ne!(
        item("c1", "t", true).register(),
        item("c2", "t", true).register()
    );
    // A newer op of one register makes the older one redundant in the
    // outbox.
    let conn = pa_library::open_in_memory().unwrap();
    let mut phone = Replica::new("phone").unwrap();
    phone.record(&conn, tag("Sci-Fi", "t", true), T0).unwrap();
    phone
        .record(&conn, tag("sci-fi", "t", false), T0 + 1)
        .unwrap();
    phone.record(&conn, upsert("c1", "A"), T0 + 2).unwrap();
    outbox::compact(&conn).unwrap();
    assert_eq!(outbox::len(&conn).unwrap(), 2);
}

#[test]
fn a_delete_is_a_tombstone_and_the_newest_write_wins() {
    let created = op(1, 10, "phone", upsert("c1", "To watch"));
    let deleted = op(
        2,
        20,
        "tv",
        OpBody::CategoryDelete {
            profile: "default".into(),
            category: "c1".into(),
        },
    );
    let renamed_late = op(3, 15, "phone", upsert("c1", "Later"));
    let mut a = SyncState::default();
    for o in [&created, &deleted, &renamed_late] {
        a.apply(o);
    }
    let mut b = SyncState::default();
    for o in [&renamed_late, &deleted, &created] {
        b.apply(o);
    }
    assert_eq!(a, b);
    assert_eq!(a.categories[&("default".into(), "c1".into())].value, None);

    let on = op(4, 30, "phone", tag("Sci-Fi", "t", true));
    let off = op(5, 31, "tv", tag("sci-fi", "t", false));
    let mut s = SyncState::default();
    s.apply(&off);
    assert!(!s.apply(&on), "older than the tombstone");
    assert!(!s.tags[&("default".into(), "sci-fi".into(), "t".into())].value);
}

/// What a node of a build before SYNC-013 does with the new ops: the frame
/// decodes (op payloads are opaque), the journal keeps and serves it, only
/// opening it fails — with an error, not a panic — so the node relays it
/// unread, as it does with ops of a newer schema. Here an unknown kind
/// stands in for the kinds such a build does not know.
#[test]
fn a_node_that_does_not_know_a_kind_relays_it_and_keeps_syncing() {
    let mut hub = Journal::new("j1").unwrap();
    for (i, o) in label_ops().iter().enumerate() {
        let mut frame = OpFrame::plain(o).unwrap();
        let kind = serde_json::from_str::<serde_json::Value>(&outbox::encode(o).unwrap()).unwrap()
            ["data"]["op"]
            .as_str()
            .unwrap()
            .to_owned();
        frame.payload.body = frame.payload.body.replace(
            &format!("\"op\":\"{kind}\""),
            &format!("\"op\":\"{kind}.v9\""),
        );
        assert!(frame.open().is_err(), "{kind}");
        assert!(outbox::decode(&frame.payload.body).is_err());
        let wire = proto::encode(&Message::Op(frame.clone())).unwrap();
        let Message::Op(got) = proto::decode(&wire).unwrap() else {
            panic!("not an op");
        };
        assert_eq!(hub.append(&got), (i as u64 + 1, true));
    }
    // A known op after them still opens and applies.
    let watched = op(
        9,
        9,
        "phone",
        OpBody::Watched {
            profile: "default".into(),
            key: "tmdb:movie:603".parse().unwrap(),
        },
    );
    let wire = proto::encode(&Message::Op(OpFrame::plain(&watched).unwrap())).unwrap();
    let Message::Op(got) = proto::decode(&wire).unwrap() else {
        panic!("not an op");
    };
    assert_eq!(hub.append(&got).0, 6);
    let (page, _) = hub.pull(&Pull {
        cursor: None,
        limit: 100,
    });
    assert_eq!(page.len(), 6);
    let mut tv = Replica::new("tv").unwrap();
    let readable: Vec<Op> = page.iter().filter_map(|f| f.open().ok()).collect();
    assert_eq!(readable, std::slice::from_ref(&watched));
    assert!(tv.receive(&readable[0], T0).unwrap());
    assert!(
        tv.state()
            .is_watched("default", &"tmdb:movie:603".parse().unwrap())
    );
}
