//! Tests for contract v2: receipts, chat types/meta, avatars, after-cursor,
//! starred, links, stats, common groups and cancellable indexing.

use std::cell::Cell;
use std::collections::HashSet;

use wa_core::ErrorKind;
use wa_core::types::*;
use wa_testkit::{Fixture, Msg, ids::*};

use crate::Db;

const COMMUNITY: &str = "120363000000000099@g.us";
const SUB: &str = "120363000000000098@g.us";
const CHANNEL: &str = "120363999999999999@newsletter";
const BCAST: &str = "1731000000@broadcast";

fn indexed(f: &Fixture) -> Db {
    let db = Db::open(&f.paths).unwrap();
    db.indexer().build_full(&|_, _| {}).unwrap();
    db
}

fn receipts_table(f: &Fixture) {
    f.bridge_conn()
        .execute_batch(
            "CREATE TABLE message_receipts(message_id TEXT, chat_jid TEXT, status TEXT, updated_at TIMESTAMP, \
             PRIMARY KEY(message_id, chat_jid));",
        )
        .unwrap();
}

fn meta_table(f: &Fixture) {
    f.bridge_conn()
        .execute_batch(
            "CREATE TABLE chat_meta(jid TEXT PRIMARY KEY, chat_type TEXT NOT NULL, parent_jid TEXT, \
             name TEXT, description TEXT, updated_at TIMESTAMP);",
        )
        .unwrap();
}

fn add_meta(f: &Fixture, jid: &str, ty: &str, parent: Option<&str>, name: Option<&str>) {
    f.bridge_conn()
        .execute(
            "INSERT INTO chat_meta (jid, chat_type, parent_jid, name) VALUES (?1,?2,?3,?4)",
            rusqlite::params![jid, ty, parent, name],
        )
        .unwrap();
}

fn kinds(db: &Db, kind: ChatKind) -> Vec<String> {
    let mut v: Vec<String> = db
        .list_chats(&ChatQuery {
            kind,
            ..Default::default()
        })
        .unwrap()
        .items
        .into_iter()
        .map(|c| c.jid)
        .collect();
    v.sort();
    v
}

fn q(chat: &str, before: Option<String>, after: Option<String>, limit: u32) -> MessagesQuery {
    MessagesQuery {
        chat_jid: chat.into(),
        before,
        after,
        limit: Some(limit),
    }
}

// ---------- receipts ----------

#[test]
fn status_defaults_without_receipts_table() {
    let f = Fixture::standard();
    let db = Db::open(&f.paths).unwrap();
    let m = db.get_message(DM_ALICE, M_ALICE_2_ME).unwrap().unwrap();
    assert_eq!(m.status, Some(MessageStatus::Sent));
    let m = db.get_message(DM_ALICE, M_ALICE_1).unwrap().unwrap();
    assert_eq!(m.status, None);
    assert!(!m.starred);
}

#[test]
fn status_from_receipts_and_last_status() {
    let f = Fixture::standard();
    receipts_table(&f);
    f.bridge_conn()
        .execute(
            "INSERT INTO message_receipts VALUES (?1, ?2, 'read', '2026-11-15T10:02:00Z')",
            [M_ALICE_2_ME, DM_ALICE],
        )
        .unwrap();
    // A row for an inbound message is ignored; unknown status strings are ignored.
    f.bridge_conn()
        .execute(
            "INSERT INTO message_receipts VALUES (?1, ?2, 'read', '2026-11-15T10:02:00Z')",
            [M_ALICE_1, DM_ALICE],
        )
        .unwrap();
    f.add_message(
        &Msg::text(
            "ME2",
            DM_BOB,
            "919900000000",
            "2026-11-14 10:00:00+05:30",
            "ok",
        )
        .mine(),
    );
    f.bridge_conn()
        .execute(
            "INSERT INTO message_receipts VALUES ('ME2', ?1, 'weird', NULL)",
            [DM_BOB],
        )
        .unwrap();
    for db in [Db::open(&f.paths).unwrap(), indexed(&f)] {
        let m = db.get_message(DM_ALICE, M_ALICE_2_ME).unwrap().unwrap();
        assert_eq!(m.status, Some(MessageStatus::Read));
        assert_eq!(
            db.get_message(DM_ALICE, M_ALICE_1).unwrap().unwrap().status,
            None
        );
        let bob = db.get_chat(DM_BOB).unwrap();
        assert!(bob.last_is_from_me);
        assert_eq!(bob.last_status, Some(MessageStatus::Sent));
        // Alice's last message is inbound.
        assert_eq!(db.get_chat(DM_ALICE).unwrap().last_status, None);
    }
}

// ---------- chat types ----------

#[test]
fn chat_types_from_jid_without_meta() {
    let f = Fixture::standard();
    f.add_chat(
        CHANNEL,
        Some("120363999999999999"),
        Some("2026-11-10 08:00:00Z"),
    );
    f.add_message(&Msg::text(
        "N1",
        CHANNEL,
        "",
        "2026-11-10 08:00:00Z",
        "channel post https://news.example/a",
    ));
    let db = indexed(&f);
    let all = db.list_chats(&ChatQuery::default()).unwrap().items;
    let ch = all.iter().find(|c| c.jid == CHANNEL).unwrap();
    assert_eq!(ch.chat_type, ChatType::Channel);
    assert_eq!(ch.name, "WhatsApp channel", "never raw digits");
    assert_eq!(
        ch.last_preview.as_deref(),
        Some("channel post https://news.example/a")
    );
    assert!(all.iter().all(|c| c.jid != STATUS));
    assert!(
        all.iter()
            .all(|c| c.avatar_url.starts_with("wa-media://localhost/a/"))
    );
    assert_eq!(
        chat_of(&all, GROUP_CLASS).avatar_url,
        "wa-media://localhost/a/120363000000000001%40g.us"
    );
    assert_eq!(kinds(&db, ChatKind::Channel), vec![CHANNEL.to_string()]);
    assert!(!kinds(&db, ChatKind::Direct).contains(&CHANNEL.to_string()));
    // Channel thread works although channels are not indexed.
    let t = db.get_messages(&q(CHANNEL, None, None, 50)).unwrap();
    assert_eq!(t.items.len(), 1);
}

fn chat_of<'a>(v: &'a [ChatSummary], jid: &str) -> &'a ChatSummary {
    v.iter().find(|c| c.jid == jid).unwrap()
}

#[test]
fn chat_meta_types_parents_names_and_filters() {
    let f = Fixture::standard();
    meta_table(&f);
    f.add_chat(COMMUNITY, None, Some("2026-11-01 08:00:00Z"));
    f.add_chat(
        SUB,
        Some("Group 120363000000000098"),
        Some("2026-11-02 08:00:00Z"),
    );
    f.add_chat(BCAST, None, Some("2026-11-03 08:00:00Z"));
    f.add_chat(
        CHANNEL,
        Some("120363999999999999"),
        Some("2026-11-04 08:00:00Z"),
    );
    add_meta(&f, COMMUNITY, "community", None, Some("CSE Batch 2027"));
    add_meta(&f, SUB, "group", Some(COMMUNITY), Some("CSE Announcements"));
    add_meta(&f, GROUP_PLACEHOLDER, "group", None, Some("Hostel Wing B"));
    add_meta(&f, CHANNEL, "channel", None, Some("Tech News"));
    add_meta(&f, BCAST, "broadcast", None, None);
    let db = Db::open(&f.paths).unwrap();
    let all = db.list_chats(&ChatQuery::default()).unwrap().items;
    let sub = chat_of(&all, SUB);
    assert_eq!(sub.name, "CSE Announcements");
    assert_eq!(sub.chat_type, ChatType::Group);
    let p = sub.parent.clone().unwrap();
    assert_eq!(
        (p.jid.as_str(), p.name.as_str(), p.chat_type),
        (COMMUNITY, "CSE Batch 2027", ChatType::Community)
    );
    assert_eq!(chat_of(&all, CHANNEL).name, "Tech News");
    assert_eq!(chat_of(&all, BCAST).name, "Broadcast list");
    assert_eq!(chat_of(&all, BCAST).chat_type, ChatType::Broadcast);
    // Real chats.name always wins over meta.
    assert_eq!(chat_of(&all, GROUP_CLASS).name, "ML CSE - 3C");
    assert_eq!(
        db.get_chat(GROUP_PLACEHOLDER).unwrap().name,
        "Hostel Wing B"
    );

    assert_eq!(kinds(&db, ChatKind::Community), vec![COMMUNITY.to_string()]);
    let groups = kinds(&db, ChatKind::Group);
    assert!(groups.contains(&SUB.to_string()) && groups.contains(&GROUP_CLASS.to_string()));
    assert!(
        !groups.contains(&COMMUNITY.to_string()),
        "Group excludes communities"
    );
    assert_eq!(kinds(&db, ChatKind::Broadcast), vec![BCAST.to_string()]);
    assert_eq!(kinds(&db, ChatKind::Channel), vec![CHANNEL.to_string()]);
    let names = db.names().snapshot().unwrap();
    assert_eq!(names.children(COMMUNITY)[0].jid, SUB);
    assert!(names.parent(GROUP_CLASS).is_none());
}

#[test]
fn own_user_parts_include_lid() {
    let f = Fixture::standard();
    let db = Db::open(&f.paths).unwrap();
    let own = db.names().snapshot().unwrap().own_user_parts();
    assert!(own.contains(&"919900000000".to_string()), "{own:?}");
    assert!(own.contains(&"111111111111111".to_string()), "{own:?}");
}

// ---------- after cursor ----------

#[test]
fn after_cursor_pages_forward_to_newest() {
    let f = Fixture::standard();
    for i in 0..25 {
        f.add_message(&Msg::text(
            &format!("F{i:02}"),
            DM_UNKNOWN,
            "919844444444",
            &format!("2026-11-12 09:{i:02}:00Z"),
            &format!("forward {i}"),
        ));
    }
    for db in [Db::open(&f.paths).unwrap(), indexed(&f)] {
        let newest = db
            .get_messages(&q(DM_UNKNOWN, None, None, 500))
            .unwrap()
            .items;
        assert_eq!(newest.len(), 26);
        // Jump to the oldest message: context, then page forward.
        let first = db.get_messages(&q(DM_UNKNOWN, None, None, 500)).unwrap();
        assert!(first.next_cursor.is_none());
        let start = crate::util::encode_msg_cursor(i64::MIN, i64::MIN);
        let mut cursor = Some(start);
        let mut got: Vec<String> = Vec::new();
        let mut pages = 0;
        while let Some(c) = cursor {
            let p = db.get_messages(&q(DM_UNKNOWN, None, Some(c), 7)).unwrap();
            assert!(p.items.windows(2).all(|w| w[0].ts <= w[1].ts));
            got.extend(p.items.iter().map(|m| m.id.clone()));
            cursor = p.next_cursor;
            pages += 1;
            assert!(pages < 10);
        }
        let want: Vec<String> = newest.iter().map(|m| m.id.clone()).collect();
        assert_eq!(
            got, want,
            "forward paging yields every message once, ascending"
        );
        assert_eq!(pages, 4);
        // After the newest message: empty page, null cursor.
        let last = newest.last().unwrap();
        let p = db
            .get_messages(&q(
                DM_UNKNOWN,
                None,
                Some(format!("{}:{}", last.ts, i64::MAX)),
                10,
            ))
            .unwrap();
        assert!(p.items.is_empty() && p.next_cursor.is_none());
        // before + after together is invalid.
        let e = db
            .get_messages(&q(DM_UNKNOWN, Some("1:1".into()), Some("1:1".into()), 10))
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Invalid);
        assert_eq!(
            db.get_messages(&q(DM_UNKNOWN, None, Some("x".into()), 10))
                .unwrap_err()
                .kind,
            ErrorKind::Invalid
        );
    }
}

// ---------- media sender ----------

#[test]
fn media_items_carry_sender_jid() {
    let f = Fixture::standard();
    f.add_message(
        &Msg::text(
            "MEIMG",
            DM_ALICE,
            "919900000000",
            "2026-11-15 10:07:00+05:30",
            "",
        )
        .mine()
        .media("image", None, IMG_BYTES),
    );
    let db = indexed(&f);
    let items = db.list_media(&MediaQuery::default()).unwrap().items;
    let g = items.iter().find(|m| m.message_id == M_GROUP_IMG).unwrap();
    assert_eq!(g.sender_jid.as_deref(), Some(DM_ALICE));
    let mine = items.iter().find(|m| m.message_id == "MEIMG").unwrap();
    assert_eq!(mine.sender_jid, None);
}

// ---------- starred ----------

#[test]
fn starred_roundtrip_and_paging() {
    let f = Fixture::standard();
    let db = indexed(&f);
    db.set_starred(
        DM_ALICE,
        &[M_ALICE_1.into(), M_ALICE_PDF.into(), "missing".into()],
        true,
    )
    .unwrap();
    db.set_starred(GROUP_CLASS, &[M_GROUP_DEVANAGARI.into()], true)
        .unwrap();
    // Idempotent.
    db.set_starred(DM_ALICE, &[M_ALICE_1.into()], true).unwrap();
    assert!(
        db.get_message(DM_ALICE, M_ALICE_1)
            .unwrap()
            .unwrap()
            .starred
    );
    let t = db.get_messages(&q(DM_ALICE, None, None, 50)).unwrap().items;
    let starred: HashSet<&str> = t
        .iter()
        .filter(|m| m.starred)
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(starred, HashSet::from([M_ALICE_1, M_ALICE_PDF]));

    let all = db.list_starred(None, None, 2).unwrap();
    let ids: Vec<&str> = all.items.iter().map(|h| h.message.id.as_str()).collect();
    assert_eq!(ids, vec![M_GROUP_DEVANAGARI, M_ALICE_PDF], "newest first");
    assert!(all.items.iter().all(|h| h.highlights.is_empty()));
    assert_eq!(all.items[0].chat_name, "ML CSE - 3C");
    let next = db
        .list_starred(None, all.next_cursor.as_deref(), 2)
        .unwrap();
    assert_eq!(next.items.len(), 1);
    assert!(next.next_cursor.is_none());
    let alice = db.list_starred(Some(DM_ALICE), None, 50).unwrap();
    assert_eq!(alice.items.len(), 2);
    assert_eq!(db.chat_stats(DM_ALICE).unwrap().starred, 2);

    db.set_starred(DM_ALICE, &[M_ALICE_1.into(), "never".into()], false)
        .unwrap();
    assert!(
        !db.get_message(DM_ALICE, M_ALICE_1)
            .unwrap()
            .unwrap()
            .starred
    );
    assert_eq!(
        db.list_starred(Some(DM_ALICE), None, 50)
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        db.set_starred(DM_ALICE, &["nope".into()], true)
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        db.list_starred(None, Some("bad"), 5).unwrap_err().kind,
        ErrorKind::Invalid
    );
    db.set_starred(DM_ALICE, &[], true).unwrap();
}

// ---------- links + stats ----------

#[test]
fn links_newest_first_dedupe_and_cursor() {
    let f = Fixture::standard();
    f.add_message(&Msg::text(
        "L-A",
        DM_ALICE,
        "919811111111",
        "2026-11-15 10:10:00+05:30",
        "two: https://a.example/x, and www.b.example/y. again https://a.example/x!",
    ));
    f.add_message(
        &Msg::text(
            "L-B",
            DM_ALICE,
            "919900000000",
            "2026-11-15 10:11:00+05:30",
            "देखो (https://c.example/पथ)। ok",
        )
        .mine(),
    );
    f.add_message(
        &Msg::text(
            "L-DEL",
            DM_ALICE,
            "919811111111",
            "2026-11-15 10:12:00+05:30",
            "https://gone.example",
        )
        .deleted("2026-11-15 10:13:00+05:30"),
    );
    let db = indexed(&f);
    let p = db.list_links(Some(DM_ALICE), None, 50).unwrap();
    let urls: Vec<&str> = p.items.iter().map(|l| l.url.as_str()).collect();
    assert_eq!(
        urls,
        vec![
            "https://c.example/पथ",
            "https://a.example/x",
            "www.b.example/y"
        ]
    );
    assert_eq!(p.items[0].sender_name, "Me");
    assert_eq!(p.items[1].sender_name, "Alice");
    assert!(p.items[1].context.contains("two:"));
    assert!(p.next_cursor.is_none());
    // Cursor splits inside a message.
    let a = db.list_links(Some(DM_ALICE), None, 2).unwrap();
    assert_eq!(a.items.len(), 2);
    let b = db
        .list_links(Some(DM_ALICE), a.next_cursor.as_deref(), 2)
        .unwrap();
    assert_eq!(b.items.len(), 1);
    assert_eq!(b.items[0].url, "www.b.example/y");
    assert!(b.next_cursor.is_none());
    // All chats include the group link.
    let all = db.list_links(None, None, 50).unwrap();
    assert!(
        all.items
            .iter()
            .any(|l| l.url == "https://example.org/course")
    );
    assert!(all.items.windows(2).all(|w| w[0].ts >= w[1].ts));
    // Empty chat.
    let e = db.list_links(Some(GROUP_PLACEHOLDER), None, 50).unwrap();
    assert!(e.items.is_empty() && e.next_cursor.is_none());
    assert_eq!(
        db.list_links(None, Some("1:2"), 5).unwrap_err().kind,
        ErrorKind::Invalid
    );

    let s = db.chat_stats(DM_ALICE).unwrap();
    assert_eq!(s.links, 3);
    assert_eq!(s.docs, 1);
    assert_eq!(s.media, 0);
    assert_eq!(s.starred, 0);
    assert_eq!(s.messages, 6, "revoked messages are not counted");
    let g = db.chat_stats(GROUP_CLASS).unwrap();
    assert_eq!((g.media, g.links, g.messages), (1, 1, 3));
    let empty = db.chat_stats(GROUP_PLACEHOLDER).unwrap();
    assert_eq!(
        empty,
        ChatStats {
            messages: 0,
            media: 0,
            docs: 0,
            links: 0,
            starred: 0
        }
    );
}

#[test]
fn links_scan_past_link_free_pages() {
    let f = Fixture::standard();
    f.add_message(&Msg::text(
        "OLD-LINK",
        DM_UNKNOWN,
        "919844444444",
        "2026-01-01 00:00:00Z",
        "https://old.example",
    ));
    for i in 0..450 {
        f.add_message(&Msg::text(
            &format!("H{i}"),
            DM_UNKNOWN,
            "919844444444",
            &format!("2026-02-01 00:{:02}:{:02}Z", i / 60, i % 60),
            "http but not a link: http://",
        ));
    }
    let db = indexed(&f);
    let p = db.list_links(Some(DM_UNKNOWN), None, 10).unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].url, "https://old.example");
}

// ---------- common groups ----------

#[test]
fn common_groups_from_index() {
    let f = Fixture::standard();
    let db = indexed(&f);
    let alice = db.common_groups(DM_ALICE).unwrap();
    assert_eq!(alice.len(), 1);
    assert_eq!(alice[0].jid, GROUP_CLASS);
    assert_eq!(alice[0].name, "ML CSE - 3C");
    // LID sender resolves through the LID map.
    let sharma = db.common_groups("919855555555@s.whatsapp.net").unwrap();
    assert_eq!(sharma.len(), 1);
    assert!(db.common_groups(DM_UNKNOWN).unwrap().is_empty());
}

// ---------- cancellable indexing ----------

#[test]
fn build_full_and_reconcile_stop_when_asked() {
    let f = Fixture::standard();
    let db = Db::open(&f.paths).unwrap();
    let calls = Cell::new(0);
    let e = db
        .indexer()
        .build_full_with(&|_, _| {}, &|| {
            calls.set(calls.get() + 1);
            true
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert_eq!(calls.get(), 1);
    assert!(!db.app().index_complete().unwrap());
    assert!(!db.indexer().is_building());
    // Resumable afterwards.
    db.indexer().ensure(&|_, _| {}).unwrap();
    assert!(db.app().index_complete().unwrap());
    let e = db
        .indexer()
        .reconcile_with(&|_, _| {}, &|| true)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert_eq!(db.app().indexed_count().unwrap(), 11);
}

#[test]
fn messages_by_ids_ascending() {
    let f = Fixture::standard();
    let db = Db::open(&f.paths).unwrap();
    let v = db
        .messages_by_ids(
            DM_ALICE,
            &[
                M_ALICE_REPLY.into(),
                M_ALICE_1.into(),
                "zz".into(),
                M_ALICE_REACT.into(),
            ],
        )
        .unwrap();
    let ids: Vec<&str> = v.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec![M_ALICE_1, M_ALICE_REPLY]);
}
