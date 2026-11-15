//! Integration-style tests over `wa_testkit` fixtures.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use rusqlite::params;
use wa_core::ErrorKind;
use wa_core::traits::{AppStore, LinkRecord, LinkStore, MessageStore};
use wa_core::types::*;
use wa_testkit::{Fixture, Msg, ids::*};

use crate::Db;

fn open(f: &Fixture) -> Db {
    Db::open(&f.paths).unwrap()
}

fn indexed(f: &Fixture) -> Db {
    let db = open(f);
    db.indexer().build_full(&|_, _| {}).unwrap();
    db
}

fn both(f: &Fixture) -> [Db; 2] {
    let a = Db::open(&f.paths).unwrap();
    let mut p = f.paths.clone();
    p.app_db = f.root.join("other-app.db");
    let b = Db::open(&p).unwrap();
    b.indexer().build_full(&|_, _| {}).unwrap();
    [a, b]
}

fn ts_ago(secs: i64) -> String {
    let t = chrono::Utc::now() - chrono::Duration::seconds(secs);
    t.format("%Y-%m-%d %H:%M:%S+00:00").to_string()
}

fn all_chats(db: &Db) -> Vec<ChatSummary> {
    db.list_chats(&ChatQuery::default()).unwrap().items
}

fn chat<'a>(v: &'a [ChatSummary], jid: &str) -> &'a ChatSummary {
    v.iter().find(|c| c.jid == jid).unwrap()
}

fn thread(db: &Db, jid: &str) -> Vec<Message> {
    db.get_messages(&MessagesQuery {
        chat_jid: jid.into(),
        before: None,
        limit: Some(500),
    })
    .unwrap()
    .items
}

fn search(db: &Db, text: &str) -> SearchResults {
    db.search(&SearchQuery {
        text: text.into(),
        ..Default::default()
    })
    .unwrap()
}

fn hit_ids(r: &SearchResults) -> Vec<String> {
    r.hits.iter().map(|h| h.message.id.clone()).collect()
}

fn bulk(f: &Fixture, chat: &str, n: usize, base_epoch: i64, prefix: &str) {
    let mut c = f.bridge_conn();
    let tx = c.transaction().unwrap();
    for i in 0..n {
        // Shuffle offsets/forms; several messages share a timestamp.
        let epoch = base_epoch + (i as i64 / 3) * 60;
        let dt = chrono::DateTime::from_timestamp(epoch, 0).unwrap();
        let ts = match i % 4 {
            0 => dt
                .with_timezone(&chrono::FixedOffset::east_opt(19800).unwrap())
                .format("%Y-%m-%d %H:%M:%S%:z")
                .to_string(),
            1 => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            2 => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            _ => dt
                .with_timezone(&chrono::FixedOffset::west_opt(3600 * 5).unwrap())
                .format("%Y-%m-%dT%H:%M:%S%:z")
                .to_string(),
        };
        tx.execute(
            "INSERT INTO messages (id, chat_jid, sender, content, timestamp, is_from_me, media_type) VALUES (?1,?2,?3,?4,?5,0,'')",
            params![format!("{prefix}{i:05}"), chat, "919811111111", format!("bulk message number {i}"), ts],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

// ---------- chats ----------

#[test]
fn chat_list_names_order_and_exclusions() {
    let f = Fixture::standard();
    for db in both(&f) {
        let v = all_chats(&db);
        let jids: Vec<&str> = v.iter().map(|c| c.jid.as_str()).collect();
        assert_eq!(
            jids,
            vec![GROUP_CLASS, DM_ALICE, DM_BOB, DM_LID, DM_UNKNOWN],
            "sorted by epoch desc; status and empty placeholder group excluded"
        );
        assert_eq!(chat(&v, DM_ALICE).name, "Alice");
        assert_eq!(chat(&v, DM_BOB).name, "Bob Builder");
        assert_eq!(chat(&v, DM_LID).name, "Carol");
        assert_eq!(chat(&v, DM_UNKNOWN).name, "+919844444444");
        assert!(chat(&v, GROUP_CLASS).is_group);
        assert_eq!(
            chat(&v, GROUP_CLASS).last_sender_name.as_deref(),
            Some("Bob Builder")
        );
    }
}

#[test]
fn chat_unread_rules_on_epochs() {
    let f = Fixture::standard();
    for db in both(&f) {
        let v = all_chats(&db);
        assert!(chat(&v, DM_ALICE).unread, "last inbound after read marker");
        assert!(!chat(&v, DM_BOB).unread, "read after last message");
        assert!(chat(&v, DM_LID).unread, "no read marker → unread");
        let u = db
            .list_chats(&ChatQuery {
                unread_only: true,
                ..Default::default()
            })
            .unwrap();
        assert!(u.items.iter().all(|c| c.unread));
        assert!(!u.items.iter().any(|c| c.jid == DM_BOB));
    }
    // Read marker in a different offset than the message: 10:06+05:30 == 04:36Z.
    f.set_last_read(DM_ALICE, Some("2026-11-15T04:36:00Z"));
    let db = open(&f);
    assert!(!chat(&all_chats(&db), DM_ALICE).unread);
    f.set_last_read(DM_ALICE, Some("2026-11-15 04:35:59"));
    assert!(chat(&all_chats(&db), DM_ALICE).unread);
}

#[test]
fn chat_own_last_message_is_not_unread() {
    let f = Fixture::standard();
    f.add_message(
        &Msg::text(
            "A7",
            DM_ALICE,
            "919900000000",
            "2026-11-15 10:07:00+05:30",
            "ok bye",
        )
        .mine(),
    );
    let db = open(&f);
    let c = db.get_chat(DM_ALICE).unwrap();
    assert!(!c.unread);
    assert!(c.last_is_from_me);
    assert_eq!(c.last_sender_name.as_deref(), Some("Me"));
    assert_eq!(c.last_preview.as_deref(), Some("ok bye"));
}

#[test]
fn chat_preview_skips_reactions_and_labels_media() {
    let f = Fixture::standard();
    f.add_message(&Msg::reaction(
        "R9",
        DM_ALICE,
        "919811111111",
        "2026-11-15 10:09:00+05:30",
        M_ALICE_REPLY,
        "❤️",
    ));
    for db in both(&f) {
        let v = all_chats(&db);
        assert_eq!(
            chat(&v, DM_ALICE).last_preview.as_deref(),
            Some("Thanks for module four")
        );
        assert_eq!(
            chat(&v, GROUP_CLASS).last_preview.as_deref(),
            Some("नमस्ते सभी 🙏 exam कल है")
        );
    }
    f.add_message(
        &Msg::text(
            "D9",
            DM_BOB,
            "919822222222",
            "2026-11-14 10:00:00+05:30",
            "",
        )
        .media("document", Some("ML Notes.pdf"), b"x"),
    );
    assert_eq!(
        open(&f).get_chat(DM_BOB).unwrap().last_preview.as_deref(),
        Some("📄 ML Notes.pdf")
    );
    f.add_message(
        &Msg::text(
            "D10",
            DM_BOB,
            "919822222222",
            "2026-11-14 10:01:00+05:30",
            "x",
        )
        .deleted("2026-11-14 10:02:00+05:30"),
    );
    assert_eq!(
        open(&f).get_chat(DM_BOB).unwrap().last_preview.as_deref(),
        Some("🚫 Deleted message")
    );
}

#[test]
fn chat_filters_and_paging() {
    let f = Fixture::standard();
    let db = open(&f);
    let g = db
        .list_chats(&ChatQuery {
            kind: ChatKind::Group,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(g.items.len(), 1);
    let d = db
        .list_chats(&ChatQuery {
            kind: ChatKind::Direct,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.items.len(), 4);
    let byname = db
        .list_chats(&ChatQuery {
            filter: Some("bob".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(byname.items[0].jid, DM_BOB);
    let byjid = db
        .list_chats(&ChatQuery {
            filter: Some("9844444".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(byjid.items[0].jid, DM_UNKNOWN);
    // Page through 2 at a time.
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let p = db
            .list_chats(&ChatQuery {
                cursor: cursor.clone(),
                limit: Some(2),
                ..Default::default()
            })
            .unwrap();
        seen.extend(p.items.into_iter().map(|c| c.jid));
        cursor = p.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 5);
    assert_eq!(
        db.list_chats(&ChatQuery {
            cursor: Some("garbage".into()),
            ..Default::default()
        })
        .unwrap_err()
        .kind,
        ErrorKind::Invalid
    );
}

#[test]
fn get_chat_found_and_missing() {
    let f = Fixture::standard();
    let db = open(&f);
    assert_eq!(db.get_chat(DM_LID).unwrap().name, "Carol");
    assert_eq!(
        db.get_chat("nope@s.whatsapp.net").unwrap_err().kind,
        ErrorKind::NotFound
    );
    let p = db.get_chat(GROUP_PLACEHOLDER).unwrap();
    assert_eq!(p.name, "Group 120363000000000002");
    assert_eq!(p.last_message_ts, None);
    assert!(!p.unread);
}

// ---------- messages ----------

#[test]
fn thread_alice_content_and_flags() {
    let f = Fixture::standard();
    for db in both(&f) {
        let t = thread(&db, DM_ALICE);
        let ids: Vec<&str> = t.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                M_ALICE_1,
                M_ALICE_2_ME,
                M_ALICE_PDF,
                M_ALICE_REVOKED,
                M_ALICE_REPLY
            ],
            "ascending; reaction rows excluded"
        );
        assert_eq!(t[0].sender_name, "Alice");
        assert_eq!(t[0].sender_jid.as_deref(), Some(DM_ALICE));
        assert_eq!(t[1].sender_name, "Me");
        assert!(t[1].is_from_me);
        // revoked
        assert!(t[3].deleted);
        assert_eq!(t[3].content, "");
        // reaction on the PDF from me
        assert_eq!(
            t[2].reactions,
            vec![Reaction {
                emoji: "👍".into(),
                sender_name: "Me".into(),
                is_from_me: true
            }]
        );
        // quoted
        let q = t[4].quoted.as_ref().unwrap();
        assert_eq!(q.id, M_ALICE_2_ME);
        assert_eq!(q.sender_name.as_deref(), Some("Me"));
        assert_eq!(q.preview.as_deref(), Some("Yes, module 4 too"));
    }
}

#[test]
fn quoted_missing_target_has_no_preview() {
    let f = Fixture::standard();
    f.add_message(
        &Msg::text(
            "Q1",
            DM_BOB,
            "919822222222",
            "2026-11-14 09:05:00+05:30",
            "re",
        )
        .quoting("GONE"),
    );
    let db = open(&f);
    let m = db.get_message(DM_BOB, "Q1").unwrap().unwrap();
    let q = m.quoted.unwrap();
    assert_eq!(q.id, "GONE");
    assert_eq!(q.preview, None);
    assert_eq!(q.sender_name, None);
}

#[test]
fn reactions_latest_wins_removal_and_missing_target() {
    let f = Fixture::standard();
    let t = |m: u32| format!("2026-11-15 10:{m:02}:00+05:30");
    // Alice reacts 😂 then changes to 🔥 (latest wins).
    f.add_message(&Msg::reaction(
        "RX1",
        DM_ALICE,
        "919811111111",
        &t(10),
        M_ALICE_1,
        "😂",
    ));
    f.add_message(&Msg::reaction(
        "RX2",
        DM_ALICE,
        "919811111111@s.whatsapp.net",
        &t(11),
        M_ALICE_1,
        "🔥",
    ));
    // I react ❤️ then remove it.
    f.add_message(&Msg::reaction("RX3", DM_ALICE, "919900000000", &t(12), M_ALICE_1, "❤️").mine());
    f.add_message(&Msg::reaction("RX4", DM_ALICE, "919900000000", &t(13), M_ALICE_1, "").mine());
    // Reaction to a message we don't have.
    f.add_message(&Msg::reaction(
        "RX5",
        DM_ALICE,
        "919811111111",
        &t(14),
        "NOPE",
        "👀",
    ));
    // Offset trap: text-later but epoch-earlier reaction must not win.
    f.add_message(&Msg::reaction(
        "RX6",
        DM_ALICE,
        "919811111111",
        "2026-11-15 04:30:00+00:00",
        M_ALICE_1,
        "🙈",
    ));
    for db in both(&f) {
        let m = db.get_message(DM_ALICE, M_ALICE_1).unwrap().unwrap();
        assert_eq!(
            m.reactions,
            vec![Reaction {
                emoji: "🔥".into(),
                sender_name: "Alice".into(),
                is_from_me: false
            }]
        );
        assert_eq!(thread(&db, DM_ALICE).len(), 5);
    }
}

#[test]
fn reaction_row_itself_is_retrievable() {
    let f = Fixture::standard();
    let db = open(&f);
    let m = db.get_message(DM_ALICE, M_ALICE_REACT).unwrap().unwrap();
    assert_eq!(m.content, "👍");
    assert!(m.media.is_none());
    assert!(db.get_message(DM_ALICE, "missing").unwrap().is_none());
}

#[test]
fn mixed_offset_ordering() {
    let f = Fixture::new();
    let c = "1@s.whatsapp.net";
    f.add_chat(c, Some("X"), Some("2026-11-15 10:00:00+00:00"));
    // Real (UTC) order: m1 07:30, m2 08:00, m3 09:00, m4 09:30, m5 10:00
    f.add_message(&Msg::text("m3", c, "1", "2026-11-15 09:00:00", "third"));
    f.add_message(&Msg::text(
        "m5",
        c,
        "1",
        "2026-11-15 10:00:00+00:00",
        "fifth",
    ));
    f.add_message(&Msg::text(
        "m1",
        c,
        "1",
        "2026-11-15 13:00:00+05:30",
        "first",
    ));
    f.add_message(&Msg::text("m4", c, "1", "2026-11-15T09:30:00Z", "fourth"));
    f.add_message(&Msg::text(
        "m2",
        c,
        "1",
        "2026-11-15T03:00:00-05:00",
        "second",
    ));
    for db in both(&f) {
        let ids: Vec<String> = thread(&db, c).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["m1", "m2", "m3", "m4", "m5"]);
        let ctx = db.get_message_context(c, "m3", 1).unwrap();
        assert_eq!(
            ctx.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["m2", "m3", "m4"]
        );
    }
}

#[test]
fn thread_paging_1200_stable_no_dupes_or_gaps() {
    let f = Fixture::standard();
    bulk(&f, GROUP_CLASS, 1200, 1_700_000_000, "P");
    for db in both(&f) {
        let mut cursor = None;
        let mut all: Vec<Message> = Vec::new();
        let mut pages = 0;
        loop {
            let p = db
                .get_messages(&MessagesQuery {
                    chat_jid: GROUP_CLASS.into(),
                    before: cursor.clone(),
                    limit: Some(77),
                })
                .unwrap();
            pages += 1;
            assert!(
                p.items.windows(2).all(|w| (w[0].ts) <= (w[1].ts)),
                "ascending in page"
            );
            if let Some(first_prev) = all.first() {
                assert!(
                    p.items.last().unwrap().ts <= first_prev.ts,
                    "older pages are older"
                );
            }
            let mut items = p.items;
            items.append(&mut all);
            all = items;
            cursor = p.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(all.len(), 1203);
        assert_eq!(pages, 1203usize.div_ceil(77));
        let uniq: HashSet<&str> = all.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(uniq.len(), 1203);
        assert!(all.windows(2).all(|w| w[0].ts <= w[1].ts));
        // Same cursor → same page (stable).
        let c = Some(crate::util::encode_msg_cursor(all[600].ts, 0));
        let q = MessagesQuery {
            chat_jid: GROUP_CLASS.into(),
            before: c,
            limit: Some(10),
        };
        assert_eq!(db.get_messages(&q).unwrap(), db.get_messages(&q).unwrap());
    }
}

#[test]
fn bad_cursor_and_empty_chat_jid() {
    let f = Fixture::standard();
    let db = open(&f);
    let e = db
        .get_messages(&MessagesQuery {
            chat_jid: DM_ALICE.into(),
            before: Some("nope".into()),
            limit: None,
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert_eq!(
        db.get_messages(&MessagesQuery::default()).unwrap_err().kind,
        ErrorKind::Invalid
    );
    assert!(thread(&db, "unknown@s.whatsapp.net").is_empty());
}

#[test]
fn context_respects_chat_jid() {
    let f = Fixture::standard();
    // Same message id in another chat (Python ignored chat_jid).
    f.add_message(&Msg::text(
        M_ALICE_1,
        DM_BOB,
        "919822222222",
        "2026-11-14 08:00:00+05:30",
        "dup id in bob",
    ));
    for db in both(&f) {
        let ctx = db.get_message_context(DM_BOB, M_ALICE_1, 5).unwrap();
        assert!(ctx.iter().all(|m| m.chat_jid == DM_BOB));
        assert_eq!(ctx.len(), 2);
        let ctx = db.get_message_context(DM_ALICE, M_ALICE_PDF, 1).unwrap();
        assert_eq!(
            ctx.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec![M_ALICE_2_ME, M_ALICE_PDF, M_ALICE_REVOKED]
        );
        assert_eq!(
            db.get_message_context(DM_ALICE, "nope", 3)
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
        assert_eq!(
            db.get_message_context(DM_ALICE, M_ALICE_1, 0)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn lid_sender_in_group_resolves() {
    let f = Fixture::standard();
    let db = open(&f);
    let t = thread(&db, GROUP_CLASS);
    let g2 = t.iter().find(|m| m.id == M_GROUP_TXT_LID).unwrap();
    assert_eq!(g2.sender_name, "Sharma Traders");
    assert_eq!(
        g2.sender_jid.as_deref(),
        Some("919855555555@s.whatsapp.net")
    );
    let l1 = db.get_message(DM_LID, M_LID_1).unwrap().unwrap();
    assert_eq!(l1.sender_name, "Carol");
    assert_eq!(l1.sender_jid.as_deref(), Some(CAROL_PN));
}

// ---------- media ----------

#[test]
fn media_store_phone_missing_order() {
    let f = Fixture::standard();
    let db = open(&f);
    let pdf = db
        .get_message(DM_ALICE, M_ALICE_PDF)
        .unwrap()
        .unwrap()
        .media
        .unwrap();
    assert_eq!(pdf.source, MediaSource::Store);
    assert_eq!(pdf.kind, MediaKind::Document);
    assert_eq!(pdf.filename.as_deref(), Some("ML-MODULE 1 Notes.pdf"));
    assert_eq!(pdf.mime.as_deref(), Some("application/pdf"));
    assert_eq!(pdf.size, Some(PDF_BYTES.len() as u64));
    assert_eq!(
        pdf.url.as_deref(),
        Some("wa-media://localhost/m/919811111111%40s.whatsapp.net/A3PDF")
    );
    assert_eq!(
        pdf.thumb_url.as_deref(),
        Some("wa-media://localhost/t/919811111111%40s.whatsapp.net/A3PDF")
    );

    // Image only on the phone, not linked yet → Missing.
    let img = db.media_ref(GROUP_CLASS, M_GROUP_IMG).unwrap().unwrap();
    assert_eq!(img.source, MediaSource::Missing);
    assert_eq!(img.url, None);
    assert_eq!(img.thumb_url, None);
    assert_eq!(img.mime.as_deref(), Some("image/jpeg"));
    assert_eq!(
        img.filename.as_deref(),
        Some("image_20261115_115800_G1IMG.jpg")
    );

    // Link exists only in messages.db (written by an older linker) → Phone.
    let phone = f
        .paths
        .phone_media
        .join("WhatsApp Images/IMG-20261115-WA0001.jpg");
    f.bridge_conn()
        .execute(
            "INSERT INTO phone_media_links VALUES (?1, ?2, 'image', ?3, NULL, 'hash', 'now')",
            params![M_GROUP_IMG, GROUP_CLASS, phone.to_string_lossy()],
        )
        .unwrap();
    let img = db.media_ref(GROUP_CLASS, M_GROUP_IMG).unwrap().unwrap();
    assert_eq!(img.source, MediaSource::Phone);
    assert!(img.url.is_some() && img.thumb_url.is_some());

    // Dangling link → Missing again.
    std::fs::remove_file(&phone).unwrap();
    assert_eq!(
        db.media_ref(GROUP_CLASS, M_GROUP_IMG)
            .unwrap()
            .unwrap()
            .source,
        MediaSource::Missing
    );

    // Store wins over a phone link.
    let other = f.put_phone_file("WhatsApp Documents/x.pdf", PDF_BYTES);
    db.app()
        .upsert_phone_links(&[LinkRecord {
            chat_jid: DM_ALICE.into(),
            message_id: M_ALICE_PDF.into(),
            kind: MediaKind::Document,
            source_path: other.clone(),
            store_path: None,
            matched_by: "hash".into(),
        }])
        .unwrap();
    let info = db.media_info(DM_ALICE, M_ALICE_PDF).unwrap().unwrap();
    assert_eq!(info.source, MediaSource::Store);
    assert_eq!(info.raw_ts, "2026-11-15 10:02:00+05:30");
    assert_eq!(info.file_sha256.as_ref().map(Vec::len), Some(32));
    // Remove the store copy → the app.db phone link is used.
    std::fs::remove_file(info.local_path.unwrap()).unwrap();
    let info = db.media_info(DM_ALICE, M_ALICE_PDF).unwrap().unwrap();
    assert_eq!(info.source, MediaSource::Phone);
    assert_eq!(info.local_path.as_deref(), Some(other.as_path()));
}

#[test]
fn resolve_media_path_validates_and_scopes() {
    let f = Fixture::standard();
    let db = open(&f);
    let p = db
        .resolve_media_path(DM_ALICE, M_ALICE_PDF)
        .unwrap()
        .unwrap();
    assert!(p.is_absolute() && p.ends_with("document_20261115_100200_A3PDF"));
    for (c, i) in [
        (DM_ALICE, "../../etc/passwd"),
        (DM_ALICE, "a/b"),
        (DM_ALICE, ".."),
        ("../x", M_ALICE_PDF),
        (DM_ALICE, ""),
        (DM_ALICE, "a\0b"),
    ] {
        assert_eq!(
            db.resolve_media_path(c, i).unwrap_err().kind,
            ErrorKind::Invalid,
            "{c} {i:?}"
        );
    }
    assert_eq!(
        db.resolve_media_path(GROUP_CLASS, M_GROUP_IMG).unwrap(),
        None
    );
    assert_eq!(db.resolve_media_path(DM_ALICE, "UNKNOWN").unwrap(), None);
    assert_eq!(db.resolve_media_path(DM_ALICE, M_ALICE_1).unwrap(), None);
    // A link pointing outside the media roots is refused.
    let outside = tempfile::NamedTempFile::new().unwrap();
    db.app()
        .upsert_phone_links(&[LinkRecord {
            chat_jid: GROUP_CLASS.into(),
            message_id: M_GROUP_IMG.into(),
            kind: MediaKind::Image,
            source_path: outside.path().to_path_buf(),
            store_path: None,
            matched_by: "hash".into(),
        }])
        .unwrap();
    assert_eq!(
        db.resolve_media_path(GROUP_CLASS, M_GROUP_IMG)
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
}

#[test]
fn ids_with_slashes_in_rows_do_not_panic() {
    let f = Fixture::standard();
    f.add_message(
        &Msg::text(
            "../evil",
            DM_BOB,
            "919822222222",
            "2026-11-14 09:10:00+05:30",
            "",
        )
        .media("image", None, b"x"),
    );
    let db = indexed(&f);
    let m = db.get_message(DM_BOB, "../evil").unwrap().unwrap();
    let media = m.media.unwrap();
    assert_eq!(media.source, MediaSource::Missing);
    assert_eq!(
        db.resolve_media_path(DM_BOB, "../evil").unwrap_err().kind,
        ErrorKind::Invalid
    );
    assert!(db.list_media(&MediaQuery::default()).is_ok());
}

#[test]
fn media_rows_and_list_media() {
    let f = Fixture::standard();
    let db = indexed(&f);
    let rows = db.media_rows(None, &[]).unwrap();
    let ids: HashSet<&str> = rows.iter().map(|r| r.message_id.as_str()).collect();
    assert_eq!(ids, HashSet::from([M_ALICE_PDF, M_GROUP_IMG]));
    assert_eq!(db.media_rows(None, &[MediaKind::Image]).unwrap().len(), 1);
    assert_eq!(db.media_rows(Some(i64::MAX), &[]).unwrap().len(), 0);

    let all = db.list_media(&MediaQuery::default()).unwrap();
    assert_eq!(
        all.items
            .iter()
            .map(|m| m.message_id.as_str())
            .collect::<Vec<_>>(),
        vec![M_GROUP_IMG, M_ALICE_PDF],
        "newest first"
    );
    assert_eq!(all.items[0].chat_name, "ML CSE - 3C");
    assert_eq!(all.items[0].caption, "Module 4 diagram");
    let local = db
        .list_media(&MediaQuery {
            available: Some(true),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(local.items.len(), 1);
    assert_eq!(local.items[0].message_id, M_ALICE_PDF);
    let missing = db
        .list_media(&MediaQuery {
            available: Some(false),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(missing.items[0].message_id, M_GROUP_IMG);
    let txt = db
        .list_media(&MediaQuery {
            text: Some("notes".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(txt.items.len(), 1);
    let one = db
        .list_media(&MediaQuery {
            limit: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(one.items.len(), 1);
    let two = db
        .list_media(&MediaQuery {
            limit: Some(1),
            cursor: one.next_cursor,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(two.items[0].message_id, M_ALICE_PDF);
    assert!(two.next_cursor.is_none());
}

// ---------- search ----------

#[test]
fn search_basic_and_highlights() {
    let f = Fixture::standard();
    let db = indexed(&f);
    let r = search(&db, "module");
    assert!(!r.index_building);
    let ids = hit_ids(&r);
    assert!(ids.contains(&M_ALICE_2_ME.to_string()));
    assert!(ids.contains(&M_ALICE_REPLY.to_string()));
    assert!(ids.contains(&M_GROUP_IMG.to_string()));
    assert!(ids.contains(&M_ALICE_PDF.to_string()), "filename match");
    assert_eq!(r.total as usize, ids.len());
    let reply = r
        .hits
        .iter()
        .find(|h| h.message.id == M_ALICE_REPLY)
        .unwrap();
    assert_eq!(reply.chat_name, "Alice");
    let (a, b) = reply.highlights[0];
    assert_eq!(&reply.message.content[a as usize..b as usize], "module");
    let pdf = r.hits.iter().find(|h| h.message.id == M_ALICE_PDF).unwrap();
    let (a, b) = pdf.highlights[0];
    assert_eq!(
        &pdf.message
            .media
            .as_ref()
            .unwrap()
            .filename
            .as_ref()
            .unwrap()[a as usize..b as usize],
        "MODULE"
    );
    // Newest first.
    assert!(
        r.hits
            .windows(2)
            .all(|w| w[0].message.ts >= w[1].message.ts)
    );
    // Revoked content is not searchable; status isn't indexed.
    assert!(search(&db, "oops").hits.is_empty());
    assert!(search(&db, "status update").hits.is_empty());
}

#[test]
fn search_unicode_and_special_chars() {
    let f = Fixture::standard();
    f.add_message(&Msg::text(
        "SP1",
        DM_BOB,
        "919822222222",
        "2026-11-14 09:20:00+05:30",
        "he said \"quote\" * -x ^y col:on (paren) 50%_off",
    ));
    let db = indexed(&f);
    assert_eq!(hit_ids(&search(&db, "नमस्ते")), vec![M_GROUP_DEVANAGARI]);
    assert_eq!(
        hit_ids(&search(&db, "कल")),
        vec![M_GROUP_DEVANAGARI],
        "2-char Devanagari"
    );
    let r = search(&db, "🙏");
    assert_eq!(hit_ids(&r), vec![M_GROUP_DEVANAGARI]);
    let h = &r.hits[0];
    assert_eq!(
        &h.message.content[h.highlights[0].0 as usize..h.highlights[0].1 as usize],
        "🙏"
    );
    for q in [
        "\"quote\"",
        "* -x",
        "^y",
        "col:on",
        "(paren)",
        "\"",
        "50%_",
        "%_",
        "AND",
        "NEAR(",
    ] {
        let r = db
            .search(&SearchQuery {
                text: q.into(),
                ..Default::default()
            })
            .unwrap_or_else(|e| panic!("{q}: {e}"));
        if q != "AND" && q != "NEAR(" {
            assert_eq!(hit_ids(&r), vec!["SP1"], "{q}");
        }
    }
    // LIKE wildcards are literal for short queries.
    assert!(search(&db, "%").hits.iter().all(|h| h.message.id == "SP1"));
    assert!(search(&db, "_").hits.iter().all(|h| h.message.id == "SP1"));
    // Case-insensitive, 1–2 chars.
    assert!(hit_ids(&search(&db, "ML")).contains(&M_ALICE_1.to_string()));
    assert!(hit_ids(&search(&db, "EXAM")).contains(&M_GROUP_DEVANAGARI.to_string()));
    assert!(search(&db, "zzzzqqq").hits.is_empty());
}

#[test]
fn search_filters() {
    let f = Fixture::standard();
    let db = indexed(&f);
    let q = |q: SearchQuery| hit_ids(&db.search(&q).unwrap());
    let base = SearchQuery {
        text: "module".into(),
        ..Default::default()
    };
    assert_eq!(
        q(SearchQuery {
            chat_jids: vec![GROUP_CLASS.into()],
            ..base.clone()
        }),
        vec![M_GROUP_IMG]
    );
    assert_eq!(
        q(SearchQuery {
            from_me: Some(true),
            ..base.clone()
        }),
        vec![M_ALICE_2_ME]
    );
    assert_eq!(
        q(SearchQuery {
            media_kinds: vec![MediaKind::Document],
            ..base.clone()
        }),
        vec![M_ALICE_PDF]
    );
    let t = wa_core::time::parse_ts("2026-11-15 10:02:00+05:30").unwrap();
    let after = q(SearchQuery {
        after: Some(t),
        ..base.clone()
    });
    assert!(!after.contains(&M_ALICE_2_ME.to_string()) && after.contains(&M_ALICE_PDF.to_string()));
    let before = q(SearchQuery {
        before: Some(t),
        ..base.clone()
    });
    assert_eq!(before, vec![M_ALICE_2_ME]);
    // Sender filter via every alias form (Sharma Traders posts as a bare LID).
    for s in [
        "919855555555",
        "919855555555@s.whatsapp.net",
        "333333333333333",
        "333333333333333@lid",
    ] {
        assert_eq!(
            q(SearchQuery {
                text: "".into(),
                senders: vec![s.into()],
                ..Default::default()
            }),
            vec![M_GROUP_TXT_LID],
            "{s}"
        );
    }
    // Filter-only browse (no text), paging with offset.
    let all = q(SearchQuery {
        chat_jids: vec![DM_ALICE.into()],
        ..Default::default()
    });
    assert_eq!(all.len(), 5);
    assert_eq!(
        q(SearchQuery {
            chat_jids: vec![DM_ALICE.into()],
            limit: Some(2),
            offset: Some(1),
            ..Default::default()
        }),
        all[1..3].to_vec()
    );
}

#[test]
fn search_reports_building_before_index() {
    let f = Fixture::standard();
    let db = open(&f);
    let r = search(&db, "module");
    assert!(r.index_building);
    assert!(r.hits.is_empty());
}

// ---------- indexer ----------

#[test]
fn indexer_incremental_and_progress() {
    let f = Fixture::standard();
    let db = open(&f);
    let calls = std::cell::RefCell::new(Vec::new());
    let n = db
        .indexer()
        .ensure(&|d, t| calls.borrow_mut().push((d, t)))
        .unwrap();
    assert_eq!(n, 13);
    assert_eq!(calls.borrow().last(), Some(&(13, 13)));
    // Indexed: 13 minus reaction minus status.
    assert_eq!(db.app().indexed_count().unwrap(), 11);
    assert_eq!(db.app().watermark().unwrap(), 13);
    f.add_message(&Msg::text(
        "N1",
        DM_BOB,
        "919822222222",
        "2026-11-14 11:00:00+05:30",
        "brand new words",
    ));
    assert!(search(&db, "brand new").hits.is_empty());
    assert_eq!(db.indexer().incremental().unwrap(), 1);
    assert_eq!(hit_ids(&search(&db, "brand new")), vec!["N1"]);
    assert_eq!(db.indexer().incremental().unwrap(), 0);
    assert!(!db.indexer().is_building());
    // Thread sees unindexed tail rows too.
    f.add_message(&Msg::text(
        "N2",
        DM_BOB,
        "919822222222",
        "2026-11-14 12:00:00+05:30",
        "tail",
    ));
    assert_eq!(thread(&db, DM_BOB).last().unwrap().id, "N2");
}

#[test]
fn indexer_reconcile_updates_and_removes() {
    let f = Fixture::standard();
    let db = indexed(&f);
    // Old edit (outside the watcher's 48h window) and a deleted row.
    f.bridge_conn()
        .execute(
            "UPDATE messages SET content='fixed the build already' WHERE id=?1",
            [M_BOB_1],
        )
        .unwrap();
    f.bridge_conn()
        .execute("DELETE FROM messages WHERE id='U1'", [])
        .unwrap();
    let st = db.indexer().reconcile(&|_, _| {}).unwrap();
    assert_eq!(st.updated, 1);
    assert_eq!(st.removed, 1);
    assert_eq!(st.scanned, 12);
    assert_eq!(hit_ids(&search(&db, "already")), vec![M_BOB_1]);
    assert!(search(&db, "who is this").hits.is_empty());
    assert_eq!(db.indexer().reconcile(&|_, _| {}).unwrap().updated, 0);
}

#[test]
fn indexer_reconcile_handles_lid_migration() {
    let f = Fixture::standard();
    let db = indexed(&f);
    // Bridge migrates the LID chat to the phone chat: insert copies, delete originals.
    let c = f.bridge_conn();
    c.execute("INSERT INTO chats (jid, name, last_message_time) VALUES (?1, NULL, '2026-11-13 08:00:00Z')", [CAROL_PN]).unwrap();
    c.execute(
        "INSERT INTO messages (id, chat_jid, sender, content, timestamp, is_from_me, media_type)
         SELECT id, ?1, sender, content, timestamp, is_from_me, media_type FROM messages WHERE chat_jid=?2",
        [CAROL_PN, DM_LID],
    )
    .unwrap();
    c.execute("DELETE FROM messages WHERE chat_jid=?1", [DM_LID])
        .unwrap();
    c.execute("DELETE FROM chats WHERE jid=?1", [DM_LID])
        .unwrap();
    db.indexer().incremental().unwrap();
    let st = db.indexer().reconcile(&|_, _| {}).unwrap();
    assert_eq!(st.removed, 1);
    let r = search(&db, "lid chat");
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].message.chat_jid, CAROL_PN);
    db.names().invalidate();
    let v = all_chats(&db);
    assert!(!v.iter().any(|c| c.jid == DM_LID));
    assert_eq!(chat(&v, CAROL_PN).name, "Carol");
}

#[test]
fn full_rebuild_resets() {
    let f = Fixture::standard();
    let db = indexed(&f);
    f.bridge_conn()
        .execute("DELETE FROM messages WHERE id=?1", [M_BOB_1])
        .unwrap();
    db.indexer().build_full(&|_, _| {}).unwrap();
    assert_eq!(db.app().indexed_count().unwrap(), 10);
    assert!(db.app().index_complete().unwrap());
}

#[test]
fn perf_index_20k_rows() {
    let f = Fixture::standard();
    bulk(&f, GROUP_CLASS, 20_000, 1_700_000_000, "Z");
    let db = open(&f);
    let t = Instant::now();
    db.indexer().build_full(&|_, _| {}).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    let t = Instant::now();
    let r = search(&db, "number 1999");
    assert!(r.total >= 1);
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn index_5k_rows_reasonably_fast() {
    let f = Fixture::standard();
    bulk(&f, GROUP_CLASS, 5_000, 1_700_000_000, "Y");
    let db = open(&f);
    let t = Instant::now();
    db.indexer().build_full(&|_, _| {}).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    assert_eq!(db.app().indexed_count().unwrap(), 5011);
}

// ---------- watcher ----------

#[test]
fn watcher_idle_is_quiet_and_cheap() {
    let f = Fixture::standard();
    let db = indexed(&f);
    let first = db.poll().unwrap();
    assert!(first.new.is_empty() && first.changed_chats.is_empty());
    assert_eq!(first.max_rowid, 13);
    let mut worst = Duration::ZERO;
    for _ in 0..20 {
        let t = Instant::now();
        let o = db.poll().unwrap();
        worst = worst.max(t.elapsed());
        assert!(o.new.is_empty() && o.changed_chats.is_empty());
    }
    assert!(worst < Duration::from_millis(20), "{worst:?}");
}

#[test]
fn watcher_detects_insert_with_fields() {
    let f = Fixture::standard();
    let db = indexed(&f);
    db.poll().unwrap();
    let rowid = f.add_message(&Msg::text(
        "W1",
        GROUP_CLASS,
        "222222222222222@lid",
        &ts_ago(5),
        "new in group",
    ));
    f.add_message(&Msg::text(
        "W2",
        STATUS,
        "919811111111",
        &ts_ago(4),
        "status",
    ));
    let o = db.poll().unwrap();
    assert_eq!(o.new.len(), 2);
    let m = &o.new[0];
    assert_eq!(m.rowid, rowid);
    assert!(m.is_group && !m.is_status);
    assert_eq!(m.chat_name, "ML CSE - 3C");
    assert_eq!(m.raw_media_type, "");
    assert_eq!(m.message.sender_name, "Carol");
    assert!(o.new[1].is_status);
    assert_eq!(o.max_rowid, rowid + 1);
    // Indexed by the poll; next poll quiet.
    assert_eq!(hit_ids(&search(&db, "new in group")), vec!["W1"]);
    let o = db.poll().unwrap();
    assert!(o.new.is_empty() && o.changed_chats.is_empty());
}

#[test]
fn watcher_detects_same_rowid_edit_and_revoke() {
    let f = Fixture::standard();
    let rowid = f.add_message(&Msg::text(
        "E1",
        DM_BOB,
        "919822222222",
        &ts_ago(60),
        "first draft",
    ));
    let db = indexed(&f);
    db.poll().unwrap();
    let again = f.add_message(&Msg::text(
        "E1",
        DM_BOB,
        "919822222222",
        &ts_ago(60),
        "second draft",
    ));
    assert_eq!(again, rowid);
    let o = db.poll().unwrap();
    assert!(o.new.is_empty());
    assert_eq!(o.changed_chats, vec![DM_BOB.to_string()]);
    assert_eq!(hit_ids(&search(&db, "second draft")), vec!["E1"]);
    assert!(
        db.poll().unwrap().changed_chats.is_empty(),
        "quiet after reporting"
    );
    f.bridge_conn()
        .execute(
            "UPDATE messages SET deleted_at=?1 WHERE id='E1'",
            [ts_ago(1)],
        )
        .unwrap();
    let o = db.poll().unwrap();
    assert_eq!(o.changed_chats, vec![DM_BOB.to_string()]);
    assert!(search(&db, "second draft").hits.is_empty());
    assert!(db.get_message(DM_BOB, "E1").unwrap().unwrap().deleted);
}

#[test]
fn watcher_detects_lid_delete_reinsert() {
    let f = Fixture::standard();
    let lid_chat = "444444444444444@lid";
    f.add_chat(lid_chat, None, Some(&ts_ago(30)));
    f.add_message(&Msg::text(
        "LM1",
        lid_chat,
        "444444444444444@lid",
        &ts_ago(30),
        "moving soon",
    ));
    let db = indexed(&f);
    db.poll().unwrap();
    let c = f.bridge_conn();
    c.execute(
        "INSERT INTO chats (jid, name) VALUES ('919866666666@s.whatsapp.net', NULL)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO messages (id, chat_jid, sender, content, timestamp, is_from_me, media_type)
         SELECT id, '919866666666@s.whatsapp.net', sender, content, timestamp, is_from_me, media_type FROM messages WHERE chat_jid=?1",
        [lid_chat],
    )
    .unwrap();
    c.execute("DELETE FROM messages WHERE chat_jid=?1", [lid_chat])
        .unwrap();
    let o = db.poll().unwrap();
    assert_eq!(o.new.len(), 1);
    assert_eq!(o.new[0].message.chat_jid, "919866666666@s.whatsapp.net");
    assert!(o.changed_chats.contains(&lid_chat.to_string()));
    let r = search(&db, "moving soon");
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].message.chat_jid, "919866666666@s.whatsapp.net");
    let o = db.poll().unwrap();
    assert!(o.new.is_empty() && o.changed_chats.is_empty());
}

#[test]
fn watcher_detects_rename_and_read_marker() {
    let f = Fixture::standard();
    let db = indexed(&f);
    db.poll().unwrap();
    f.bridge_conn()
        .execute("UPDATE chats SET name='Alice W' WHERE jid=?1", [DM_ALICE])
        .unwrap();
    let o = db.poll().unwrap();
    assert_eq!(o.changed_chats, vec![DM_ALICE.to_string()]);
    assert_eq!(
        db.get_chat(DM_ALICE).unwrap().name,
        "Alice W",
        "names invalidated"
    );
    f.set_last_read(DM_BOB, Some("2026-11-16 00:00:00Z"));
    assert_eq!(db.poll().unwrap().changed_chats, vec![DM_BOB.to_string()]);
}

#[test]
fn watcher_start_after_delivers_backlog_in_order() {
    let f = Fixture::standard();
    let db = Db::open_with(&f.paths, Some(10), crate::bridge_ro::DEFAULT_DEADLINE).unwrap();
    let o = db.poll().unwrap();
    assert_eq!(
        o.new.iter().map(|m| m.rowid).collect::<Vec<_>>(),
        vec![11, 12, 13]
    );
    assert_eq!(db.watcher().last_rowid(), 13);
}

// ---------- MessageStore / LinkStore / misc ----------

#[test]
fn message_store_trait() {
    let f = Fixture::standard();
    let db = open(&f);
    let s: &dyn MessageStore = &db;
    assert_eq!(s.max_rowid().unwrap(), 13);
    let rows = s.messages_after(0, 100).unwrap();
    assert_eq!(rows.len(), 13);
    assert!(rows.windows(2).all(|w| w[0].rowid < w[1].rowid));
    let react = rows.iter().find(|r| r.message.id == M_ALICE_REACT).unwrap();
    assert_eq!(react.raw_media_type, "reaction");
    let st = rows.iter().find(|r| r.message.id == M_STATUS).unwrap();
    assert!(st.is_status);
    assert_eq!(s.messages_after(10, 2).unwrap().len(), 2);
    assert!(s.messages_after(13, 10).unwrap().is_empty());
    assert!(s.media_info(DM_ALICE, M_ALICE_1).unwrap().is_none());
    assert!(s.media_info(DM_ALICE, "nope").unwrap().is_none());
}

#[test]
fn link_store_mirrors_to_messages_db() {
    let f = Fixture::standard();
    let db = open(&f);
    let phone = f
        .paths
        .phone_media
        .join("WhatsApp Images/IMG-20261115-WA0001.jpg");
    let links: Vec<LinkRecord> = (0..450)
        .map(|i| LinkRecord {
            chat_jid: GROUP_CLASS.into(),
            message_id: if i == 0 {
                M_GROUP_IMG.into()
            } else {
                format!("X{i}")
            },
            kind: MediaKind::Image,
            source_path: phone.clone(),
            store_path: None,
            matched_by: "hash".into(),
        })
        .collect();
    db.upsert_links(&links).unwrap();
    db.upsert_links(&links[..1]).unwrap();
    let (n, mt, path): (i64, String, String) = f
        .bridge_conn()
        .query_row(
            "SELECT (SELECT count(*) FROM phone_media_links), media_type, phone_path FROM phone_media_links WHERE message_id=?1",
            [M_GROUP_IMG],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(n, 450);
    assert_eq!(mt, "image");
    assert_eq!(path, phone.to_string_lossy());
    assert_eq!(db.app().phone_link_count().unwrap(), 450);
    assert_eq!(
        db.media_ref(GROUP_CLASS, M_GROUP_IMG)
            .unwrap()
            .unwrap()
            .source,
        MediaSource::Phone
    );
    // messages untouched
    let m: i64 = f
        .bridge_conn()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(m, 13);
    db.upsert_links(&[]).unwrap();
}

#[test]
fn empty_install_works() {
    let f = Fixture::new();
    let db = indexed(&f);
    assert!(all_chats(&db).is_empty());
    assert!(thread(&db, DM_ALICE).is_empty());
    assert!(search(&db, "anything").hits.is_empty());
    assert!(search(&db, "").hits.is_empty());
    assert!(
        db.list_media(&MediaQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
    let o = db.overview().unwrap();
    assert_eq!(
        (o.chats, o.messages, o.indexed, o.oldest_ts),
        (0, 0, 0, None)
    );
    assert!(db.poll().unwrap().new.is_empty());
    assert!(db.media_rows(None, &[]).unwrap().is_empty());
}

#[test]
fn missing_whatsapp_db_degrades_everywhere() {
    let f = Fixture::standard();
    std::fs::remove_file(&f.paths.whatsapp_db).unwrap();
    let db = indexed(&f);
    let v = all_chats(&db);
    assert_eq!(chat(&v, DM_ALICE).name, "Alice");
    assert_eq!(chat(&v, DM_BOB).name, "+919822222222");
    assert_eq!(chat(&v, DM_LID).name, DM_LID);
    let t = thread(&db, DM_ALICE);
    assert_eq!(t[1].sender_name, "Me", "is_from_me still works");
    assert!(!search(&db, "module").hits.is_empty());
    assert!(!f.paths.whatsapp_db.exists());
}

#[test]
fn missing_messages_db_is_not_found() {
    let f = Fixture::new();
    std::fs::remove_file(&f.paths.messages_db).unwrap();
    let db = open(&f);
    assert_eq!(
        db.list_chats(&ChatQuery::default()).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(db.poll().unwrap_err().kind, ErrorKind::NotFound);
    assert!(!f.paths.messages_db.exists(), "never created");
}

#[test]
fn overview_counts() {
    let f = Fixture::standard();
    let db = indexed(&f);
    let o = db.overview().unwrap();
    assert_eq!(o.chats, 5);
    assert_eq!(o.messages, 13);
    assert_eq!(o.media_total, 2);
    assert_eq!(o.media_local, 1);
    assert_eq!(o.indexed, 11);
    assert_eq!(
        o.newest_ts,
        wa_core::time::parse_ts("2026-11-15 12:00:00+05:30")
    );
    assert_eq!(o.oldest_ts, wa_core::time::parse_ts("2026-11-12 08:00:00"));
}

#[test]
fn app_store_via_db() {
    let f = Fixture::new();
    let db = open(&f);
    let s: &dyn AppStore = &db;
    s.set_kv("k", "v").unwrap();
    assert_eq!(s.get_kv("k").unwrap().as_deref(), Some("v"));
    assert_eq!(s.get_settings().unwrap(), Settings::default());
    assert!(f.paths.app_db.exists());
}

#[test]
fn reads_never_write_bridge_db() {
    let f = Fixture::standard();
    let before = std::fs::read(&f.paths.messages_db).unwrap();
    let db = indexed(&f);
    all_chats(&db);
    thread(&db, GROUP_CLASS);
    search(&db, "module");
    db.list_media(&MediaQuery::default()).unwrap();
    db.overview().unwrap();
    db.poll().unwrap();
    db.indexer().reconcile(&|_, _| {}).unwrap();
    assert_eq!(std::fs::read(&f.paths.messages_db).unwrap(), before);
}

#[test]
fn long_read_does_not_block_bridge_writer() {
    // A big fallback read (index not built) in another thread while the "bridge" writes.
    let f = Fixture::standard();
    bulk(&f, GROUP_CLASS, 3000, 1_700_000_000, "K");
    let db = std::sync::Arc::new(open(&f));
    let d2 = std::sync::Arc::clone(&db);
    let reader = std::thread::spawn(move || {
        for _ in 0..10 {
            d2.get_messages(&MessagesQuery {
                chat_jid: GROUP_CLASS.into(),
                before: None,
                limit: Some(50),
            })
            .unwrap();
        }
    });
    let w = f.bridge_conn();
    w.busy_timeout(Duration::from_millis(5000)).unwrap();
    for i in 0..20 {
        let t = Instant::now();
        w.execute(
            "INSERT INTO messages (id, chat_jid, sender, content, timestamp, is_from_me, media_type) VALUES (?1, ?2, 'x', 'w', '2026-11-16 00:00:00Z', 0, '')",
            params![format!("WR{i}"), DM_BOB],
        )
        .unwrap();
        assert!(t.elapsed() < Duration::from_millis(1000));
    }
    reader.join().unwrap();
}
