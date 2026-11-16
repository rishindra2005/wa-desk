//! Tests for the call log and the watcher's call detection.

use wa_core::types::*;
use wa_testkit::{Fixture, ME, ids::*};

use crate::Db;

fn ts(secs_ago: i64) -> String {
    let t = chrono::Utc::now() - chrono::Duration::seconds(secs_ago);
    t.format("%Y-%m-%d %H:%M:%S+00:00").to_string()
}

fn seeded() -> (Fixture, Db) {
    let f = Fixture::standard();
    f.add_call(
        "C1",
        DM_ALICE,
        DM_ALICE,
        &ts(5000),
        false,
        "voice",
        "answered",
        Some(125),
    );
    f.add_call(
        "C2",
        DM_ALICE,
        DM_ALICE,
        &ts(4000),
        false,
        "voice",
        "missed",
        None,
    );
    f.add_call(
        "C3",
        GROUP_CLASS,
        DM_BOB,
        &ts(3000),
        false,
        "video",
        "ended",
        Some(1800),
    );
    f.add_call(
        "C4",
        DM_BOB,
        DM_BOB,
        &ts(2000),
        false,
        "video",
        "rejected",
        None,
    );
    f.add_call("C5", DM_ALICE, ME, &ts(1000), true, "voice", "missed", None);
    let db = Db::open(&f.paths).unwrap();
    (f, db)
}

fn ids(p: &Page<CallRecord>) -> Vec<&str> {
    p.items.iter().map(|c| c.call_id.as_str()).collect()
}

#[test]
fn list_newest_first_with_names_and_group() {
    let (_f, db) = seeded();
    let p = db.list_calls(&CallQuery::default()).unwrap();
    assert_eq!(ids(&p), vec!["C5", "C4", "C3", "C2", "C1"]);
    let g = p.items.iter().find(|c| c.call_id == "C3").unwrap();
    assert!(g.is_group);
    assert_eq!(g.media, CallMedia::Video);
    assert_eq!(g.chat_name, "ML CSE - 3C");
    assert_eq!(g.caller_jid, DM_BOB);
    assert_eq!(g.duration_secs, Some(1800));
    assert_eq!(g.result, CallResult::Ended);
    assert!(g.avatar_url.contains("120363000000000001%40g.us"));
    let a = p.items.iter().find(|c| c.call_id == "C1").unwrap();
    assert_eq!(a.chat_name, "Alice");
    assert_eq!(a.caller_name, "Alice");
    assert!(!a.is_group && a.media == CallMedia::Voice);
    let mine = p.items.iter().find(|c| c.call_id == "C5").unwrap();
    assert!(mine.is_from_me);
    assert_eq!(mine.caller_name, "Me");
    assert!(a.ts > 0);
}

#[test]
fn filters_chat_missed_media() {
    let (_f, db) = seeded();
    let q = |f: &dyn Fn(&mut CallQuery)| {
        let mut q = CallQuery::default();
        f(&mut q);
        db.list_calls(&q).unwrap()
    };
    let p = q(&|q| q.chat_jid = Some(DM_ALICE.into()));
    assert_eq!(ids(&p), vec!["C5", "C2", "C1"]);
    // Missed + rejected, never our own unanswered outgoing call.
    let p = q(&|q| q.missed_only = true);
    assert_eq!(ids(&p), vec!["C4", "C2"]);
    let p = q(&|q| q.media = Some(CallMedia::Video));
    assert_eq!(ids(&p), vec!["C4", "C3"]);
    let p = q(&|q| {
        q.media = Some(CallMedia::Video);
        q.missed_only = true;
    });
    assert_eq!(ids(&p), vec!["C4"]);
}

#[test]
fn paging_with_cursor() {
    let (_f, db) = seeded();
    let p1 = db
        .list_calls(&CallQuery {
            limit: Some(2),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ids(&p1), vec!["C5", "C4"]);
    let c = p1.next_cursor.clone().unwrap();
    assert!(c.contains(':'));
    let p2 = db
        .list_calls(&CallQuery {
            limit: Some(2),
            cursor: Some(c),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ids(&p2), vec!["C3", "C2"]);
    let p3 = db
        .list_calls(&CallQuery {
            limit: Some(2),
            cursor: p2.next_cursor.clone(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ids(&p3), vec!["C1"]);
    assert!(p3.next_cursor.is_none());
    let bad = db.list_calls(&CallQuery {
        cursor: Some("junk".into()),
        ..Default::default()
    });
    assert_eq!(bad.unwrap_err().kind, wa_core::ErrorKind::Invalid);
}

#[test]
fn missing_table_gives_empty_page() {
    let f = Fixture::standard();
    f.bridge_conn().execute_batch("DROP TABLE calls").unwrap();
    let db = Db::open(&f.paths).unwrap();
    let p = db.list_calls(&CallQuery::default()).unwrap();
    assert!(p.items.is_empty() && p.next_cursor.is_none());
    // The watcher tolerates it too.
    assert!(db.poll().unwrap().calls.is_empty());
    assert!(db.poll().unwrap().calls.is_empty());
}

#[test]
fn watcher_reports_new_and_changed_calls() {
    let (f, db) = seeded();
    let first = db.poll().unwrap();
    assert!(first.calls.is_empty(), "existing calls are not replayed");
    assert!(db.poll().unwrap().calls.is_empty());

    f.add_call(
        "R1",
        DM_ALICE,
        DM_ALICE,
        &ts(2),
        false,
        "voice",
        "in_progress",
        None,
    );
    let o = db.poll().unwrap();
    assert_eq!(o.calls.len(), 1);
    assert_eq!(o.calls[0].call_id, "R1");
    assert_eq!(o.calls[0].result, CallResult::InProgress);
    assert_eq!(o.calls[0].caller_name, "Alice");
    assert!(db.poll().unwrap().calls.is_empty(), "quiet afterwards");

    // in_progress -> ended with a duration (same row).
    f.add_call(
        "R1",
        DM_ALICE,
        DM_ALICE,
        &ts(2),
        false,
        "voice",
        "ended",
        Some(42),
    );
    let o = db.poll().unwrap();
    assert_eq!(o.calls.len(), 1);
    assert_eq!(o.calls[0].result, CallResult::Ended);
    assert_eq!(o.calls[0].duration_secs, Some(42));
    assert!(o.calls[0].ended_at.is_some());
    assert!(db.poll().unwrap().calls.is_empty());

    // A group video call ringing then missed.
    f.add_call(
        "R2",
        GROUP_CLASS,
        DM_BOB,
        &ts(1),
        false,
        "video",
        "in_progress",
        None,
    );
    f.add_call(
        "R2",
        GROUP_CLASS,
        DM_BOB,
        &ts(1),
        false,
        "video",
        "missed",
        None,
    );
    let o = db.poll().unwrap();
    assert_eq!(o.calls.len(), 1);
    assert_eq!(o.calls[0].result, CallResult::Missed);
    assert!(o.calls[0].is_group);
}
