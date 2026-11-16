//! Tests for archive / pin / mute state (`chat_state`).

use rusqlite::params;
use wa_core::time::now_epoch;
use wa_core::types::*;
use wa_testkit::{Fixture, ids::*};

use crate::Db;

fn add_state(f: &Fixture, jid: &str, archived: i64, pinned: i64, muted: Option<i64>) {
    let c = f.bridge_conn();
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS chat_state(jid TEXT PRIMARY KEY, archived INTEGER NOT NULL DEFAULT 0, \
         pinned INTEGER NOT NULL DEFAULT 0, muted_until INTEGER, updated_at TIMESTAMP)",
    )
    .unwrap();
    c.execute(
        "INSERT OR REPLACE INTO chat_state (jid, archived, pinned, muted_until, updated_at) VALUES (?1,?2,?3,?4,datetime('now'))",
        params![jid, archived, pinned, muted],
    )
    .unwrap();
}

fn list(db: &Db, archived: bool) -> Vec<ChatSummary> {
    db.list_chats(&ChatQuery {
        archived,
        ..Default::default()
    })
    .unwrap()
    .items
}

fn jids(v: &[ChatSummary]) -> Vec<&str> {
    v.iter().map(|c| c.jid.as_str()).collect()
}

#[test]
fn missing_table_means_plain_chats() {
    let f = Fixture::standard();
    let db = Db::open(&f.paths).unwrap();
    let v = list(&db, false);
    assert_eq!(v.len(), 5);
    assert!(
        v.iter()
            .all(|c| !c.archived && !c.pinned && c.muted_until.is_none())
    );
    assert!(list(&db, true).is_empty());
    assert_eq!(db.archive_count().unwrap(), 0);
    assert!(db.poll().unwrap().changed_chats.is_empty());
}

#[test]
fn archived_hidden_and_listed_separately() {
    let f = Fixture::standard();
    add_state(&f, DM_BOB, 1, 0, None);
    add_state(&f, DM_ALICE, 0, 0, None);
    let db = Db::open(&f.paths).unwrap();
    let main = list(&db, false);
    assert_eq!(jids(&main), vec![GROUP_CLASS, DM_ALICE, DM_LID, DM_UNKNOWN]);
    let arch = list(&db, true);
    assert_eq!(jids(&arch), vec![DM_BOB]);
    assert!(arch[0].archived);
    assert_eq!(db.archive_count().unwrap(), 1);
    assert!(db.get_chat(DM_BOB).unwrap().archived);
    // unread_only still applies inside the archive (Bob is read).
    let unread = db
        .list_chats(&ChatQuery {
            archived: true,
            unread_only: true,
            ..Default::default()
        })
        .unwrap();
    assert!(unread.items.is_empty());
    // The kind filter applies too.
    let groups = db
        .list_chats(&ChatQuery {
            kind: ChatKind::Group,
            archived: true,
            ..Default::default()
        })
        .unwrap();
    assert!(groups.items.is_empty());
}

#[test]
fn lid_keyed_rows_map_to_chats() {
    let f = Fixture::standard();
    // DM_LID is stored under its LID in `chats`, state under the phone JID.
    add_state(&f, CAROL_PN, 1, 0, None);
    let db = Db::open(&f.paths).unwrap();
    assert_eq!(jids(&list(&db, true)), vec![DM_LID]);
    assert!(!jids(&list(&db, false)).contains(&DM_LID));
    // The LID row wins over the phone row when both exist.
    add_state(&f, DM_LID, 0, 1, None);
    let db = Db::open(&f.paths).unwrap();
    let v = list(&db, false);
    assert_eq!(v[0].jid, DM_LID);
    assert!(v[0].pinned && !v[0].archived);
    // An unmapped LID keyed row stays under its @lid JID and matches an LID chat.
    let f = Fixture::standard();
    f.add_chat(
        "999999999999999@lid",
        Some("Ghost"),
        Some("2026-11-16 08:00:00Z"),
    );
    f.add_message(&wa_testkit::Msg::text(
        "GH1",
        "999999999999999@lid",
        "999999999999999",
        "2026-11-16 08:00:00Z",
        "boo",
    ));
    add_state(&f, "999999999999999@lid", 1, 0, None);
    let db = Db::open(&f.paths).unwrap();
    assert!(jids(&list(&db, true)).contains(&"999999999999999@lid"));
}

#[test]
fn pinned_sort_first_then_by_time() {
    let f = Fixture::standard();
    add_state(&f, DM_UNKNOWN, 0, 1, None);
    add_state(&f, DM_BOB, 0, 1, None);
    let db = Db::open(&f.paths).unwrap();
    let v = list(&db, false);
    assert_eq!(
        jids(&v),
        vec![DM_BOB, DM_UNKNOWN, GROUP_CLASS, DM_ALICE, DM_LID]
    );
    assert!(v[0].pinned && v[1].pinned && !v[2].pinned);
    assert_eq!(db.pinned_count().unwrap(), 2);
    // Pinned chats still honour unread_only.
    let u = db
        .list_chats(&ChatQuery {
            unread_only: true,
            ..Default::default()
        })
        .unwrap();
    assert!(!jids(&u.items).contains(&DM_BOB));
}

#[test]
fn mute_values_and_expiry() {
    let f = Fixture::standard();
    let now = now_epoch();
    add_state(&f, DM_ALICE, 0, 0, Some(-1));
    add_state(&f, DM_BOB, 0, 0, Some(now + 3600));
    add_state(&f, DM_LID, 0, 0, Some(now - 3600)); // expired
    add_state(&f, DM_UNKNOWN, 0, 0, None);
    let db = Db::open(&f.paths).unwrap();
    let v = list(&db, false);
    let m = |j: &str| v.iter().find(|c| c.jid == j).unwrap().muted_until;
    assert_eq!(m(DM_ALICE), Some(-1));
    assert_eq!(m(DM_BOB), Some(now + 3600));
    assert_eq!(m(DM_LID), None, "expired mute is not muted");
    assert_eq!(m(DM_UNKNOWN), None);
}

#[test]
fn new_message_keeps_chat_archived() {
    let f = Fixture::standard();
    add_state(&f, DM_ALICE, 1, 0, None);
    let db = Db::open(&f.paths).unwrap();
    f.add_message(&wa_testkit::Msg::text(
        "NEW1",
        DM_ALICE,
        "919811111111",
        "2026-11-16 10:00:00+05:30",
        "ping",
    ));
    assert!(jids(&list(&db, true)).contains(&DM_ALICE));
    assert!(!jids(&list(&db, false)).contains(&DM_ALICE));
}

#[test]
fn watcher_reports_state_changes() {
    let f = Fixture::standard();
    add_state(&f, DM_BOB, 1, 0, None);
    let db = Db::open(&f.paths).unwrap();
    assert!(db.poll().unwrap().changed_chats.is_empty());
    assert!(db.poll().unwrap().changed_chats.is_empty());
    add_state(&f, DM_BOB, 0, 0, None);
    let o = db.poll().unwrap();
    assert!(
        o.changed_chats.contains(&DM_BOB.to_string()),
        "{:?}",
        o.changed_chats
    );
    assert!(
        db.poll().unwrap().changed_chats.is_empty(),
        "quiet afterwards"
    );
    add_state(&f, DM_ALICE, 0, 1, None);
    assert!(
        db.poll()
            .unwrap()
            .changed_chats
            .contains(&DM_ALICE.to_string())
    );
    // A LID-keyed change is reported under the normalised JID too.
    add_state(&f, DM_LID, 0, 0, Some(-1));
    let o = db.poll().unwrap();
    assert!(o.changed_chats.contains(&CAROL_PN.to_string()));
    assert!(o.changed_chats.contains(&DM_LID.to_string()));
}
