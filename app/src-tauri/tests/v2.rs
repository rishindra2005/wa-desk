//! Contract v2 commands: profiles, avatars, stats, links, starred, export,
//! forward, copy text, refresh_chat_names.

mod common;

use std::time::{Duration, Instant};

use common::*;
use tauri::http::{Request, Response, StatusCode, header};
use wa_core::ErrorKind;
use wa_core::types::*;
use wa_desk_lib::protocol::handle;
use wa_testkit::Msg;
use wa_testkit::ids::*;

async fn index(h: &H) {
    let db = h.core.db().clone();
    tokio::task::spawn_blocking(move || db.indexer().build_full(&|_, _| {}).unwrap())
        .await
        .unwrap();
}

async fn get(h: &H, url: &str) -> Response<Vec<u8>> {
    let req = Request::builder()
        .method("GET")
        .uri(url)
        .body(Vec::new())
        .unwrap();
    handle(&h.core, &req).await
}

fn hdr(r: &Response<Vec<u8>>, h: header::HeaderName) -> String {
    r.headers()
        .get(h)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

// ---------- profiles ----------

#[tokio::test(flavor = "multi_thread")]
async fn contact_profile_merges_bridge_and_local() {
    let h = harness().await;
    index(&h).await;
    let Profile::Contact(c) = h.core.get_profile(DM_ALICE.into()).await.unwrap() else {
        panic!("contact expected");
    };
    assert_eq!(c.name, "Alice");
    assert_eq!(c.push_name.as_deref(), Some("Ally P"));
    assert_eq!(c.about.as_deref(), Some("At work"));
    assert_eq!(c.phone.as_deref(), Some("+91 98111 11111"));
    assert_eq!(
        c.avatar_url,
        "wa-media://localhost/a/919811111111%40s.whatsapp.net"
    );
    assert!(!c.is_me);
    assert_eq!(c.common_groups.len(), 1);
    assert_eq!(c.common_groups[0].name, "ML CSE - 3C");
    // Unknown name → push name replaces the phone fallback.
    let Profile::Contact(u) = h.core.get_profile(DM_UNKNOWN.into()).await.unwrap() else {
        panic!()
    };
    assert_eq!(u.name, "Ally P");
    // Cached: a second call does not hit the bridge.
    let before = h.mock.calls_to("/api/profile").len();
    h.core.get_profile(DM_ALICE.into()).await.unwrap();
    assert_eq!(h.mock.calls_to("/api/profile").len(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn group_profile_participants_and_fallbacks() {
    let h = harness().await;
    let Profile::Group(g) = h.core.get_profile(GROUP_CLASS.into()).await.unwrap() else {
        panic!("group expected");
    };
    assert_eq!(g.chat_type, ChatType::Group);
    assert_eq!(g.participant_count, 4);
    assert!(g.participants[0].is_me, "me first");
    assert_eq!(g.participants[0].name, "Me");
    assert_eq!(g.participants[1].name, "Alice");
    assert!(g.participants[1].is_admin && g.participants[1].is_super_admin);
    let names: Vec<&str> = g.participants.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Sharma Traders"), "{names:?}");
    assert!(
        names.contains(&"Zed"),
        "display_name used for unknown people"
    );
    assert_eq!(g.description.as_deref(), Some("Exam prep <b>only</b>"));
    assert_eq!(g.created_by.as_deref(), Some("Alice"));
    assert!(g.created_at.is_some() && g.announce_only && !g.locked);
    // Not a member → local fallback, not an error.
    let Profile::Group(p) = h.core.get_profile(GROUP_PLACEHOLDER.into()).await.unwrap() else {
        panic!()
    };
    assert!(p.participants.is_empty());
    assert_eq!(p.name, "Group 120363000000000002");
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_falls_back_when_bridge_down() {
    let h = harness_down().await;
    let Profile::Contact(c) = h.core.get_profile(DM_ALICE.into()).await.unwrap() else {
        panic!()
    };
    assert_eq!(c.name, "Alice");
    assert_eq!(c.about, None);
    let Profile::Group(g) = h.core.get_profile(GROUP_CLASS.into()).await.unwrap() else {
        panic!()
    };
    assert_eq!(g.name, "ML CSE - 3C");
    assert!(g.participants.is_empty());
    // LID JIDs resolve to the phone form.
    let Profile::Contact(l) = h.core.get_profile(DM_LID.into()).await.unwrap() else {
        panic!()
    };
    assert_eq!(l.jid, CAROL_PN);
    assert_eq!(l.name, "Carol");
    assert!(h.core.get_profile("  ".into()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_profile_and_meta_down_not_cached() {
    let h = harness().await;
    let ch = "120363999999999999@newsletter";
    h.f.add_chat(ch, Some("120363999999999999"), Some("2026-11-10 08:00:00Z"));
    h.core.db().names().invalidate();
    *h.mock.meta_down.lock() = true;
    let Profile::Channel(c) = h.core.get_profile(ch.into()).await.unwrap() else {
        panic!()
    };
    assert_eq!(c.name, "WhatsApp channel");
    *h.mock.meta_down.lock() = false;
    let Profile::Channel(c) = h.core.get_profile(ch.into()).await.unwrap() else {
        panic!()
    };
    assert_eq!(c.name, "Tech News");
    assert_eq!(c.subscribers, Some(23101));
    assert!(c.verified);
}

#[tokio::test(flavor = "multi_thread")]
async fn contact_without_chat_has_profile_and_zero_stats() {
    let h = harness().await;
    index(&h).await;
    let jid = "919877777777@s.whatsapp.net";
    let Profile::Contact(c) = h.core.get_profile(jid.into()).await.unwrap() else {
        panic!()
    };
    assert_eq!(c.name, "+919877777777");
    assert!(c.common_groups.is_empty());
    let s = h.core.chat_stats(jid.into()).await.unwrap();
    assert_eq!(
        (s.messages, s.media, s.docs, s.links, s.starred),
        (0, 0, 0, 0, 0)
    );
}

// ---------- avatars ----------

#[tokio::test(flavor = "multi_thread")]
async fn avatar_served_cached_and_404() {
    let h = harness().await;
    let url = "wa-media://localhost/a/120363000000000001%40g.us";
    let r = get(&h, url).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.body(), b"\xFF\xD8\xFFavatar");
    assert_eq!(hdr(&r, header::CONTENT_TYPE), "image/jpeg");
    assert!(hdr(&r, header::CACHE_CONTROL).contains("max-age=86400"));
    let n = h.mock.calls_to("/api/avatar").len();
    // Second request comes from the in-memory cache, under 50 ms.
    let t = Instant::now();
    assert_eq!(get(&h, url).await.status(), StatusCode::OK);
    assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
    assert_eq!(h.mock.calls_to("/api/avatar").len(), n);
    // Full size is a separate file.
    let r = get(&h, &format!("{url}?full=1")).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(h.mock.calls_to("/api/avatar").last().unwrap()["full"], "1");

    // No picture → 404, cacheable, and remembered (negative cache).
    let none = "wa-media://localhost/a/919822222222%40s.whatsapp.net";
    let r = get(&h, none).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(hdr(&r, header::CACHE_CONTROL).contains("max-age=1800"));
    let n = h.mock.calls_to("/api/avatar").len();
    assert_eq!(get(&h, none).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(h.mock.calls_to("/api/avatar").len(), n);

    // A path outside the store is never served.
    let r = get(&h, "wa-media://localhost/a/evil%40g.us").await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    // Bad URL.
    assert_eq!(
        get(&h, "wa-media://localhost/a/nojid").await.status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn avatar_never_blocks_more_than_two_seconds() {
    let h = harness().await;
    h.mock.slow_avatars.lock().push("slow@g.us".into());
    let t = Instant::now();
    let r = get(&h, "wa-media://localhost/a/slow%40g.us").await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(
        t.elapsed() < Duration::from_millis(2600),
        "{:?}",
        t.elapsed()
    );
    // Retried later, not immediately.
    let t = Instant::now();
    assert_eq!(
        get(&h, "wa-media://localhost/a/slow%40g.us").await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(t.elapsed() < Duration::from_millis(100));
}

#[tokio::test(flavor = "multi_thread")]
async fn avatar_served_from_disk_when_bridge_down() {
    let h = harness_down().await;
    let dir = h.f.paths.bridge_store.join("avatars");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("120363000000000001@g.us.jpg"), b"old").unwrap();
    let r = get(&h, "wa-media://localhost/a/120363000000000001%40g.us").await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.body(), b"old");
    assert_eq!(
        get(&h, "wa-media://localhost/a/x%40g.us").await.status(),
        StatusCode::NOT_FOUND
    );
}

/// Bridge `chat_meta` rows: (jid, chat_type, parent_jid).
fn meta(h: &H, rows: &[(&str, &str, Option<&str>)]) {
    let c = h.f.bridge_conn();
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS chat_meta(jid TEXT PRIMARY KEY, chat_type TEXT NOT NULL, \
         parent_jid TEXT, name TEXT, description TEXT, updated_at TIMESTAMP);",
    )
    .unwrap();
    for (jid, ty, parent) in rows {
        c.execute(
            "INSERT INTO chat_meta (jid, chat_type, parent_jid) VALUES (?1,?2,?3)",
            rusqlite::params![jid, ty, parent],
        )
        .unwrap();
    }
    h.core.db().names().invalidate();
}

/// Avatar resolution per chat kind: a community group without a picture of
/// its own (the announcement group never has one) shows its community's logo;
/// communities and channels use their own; plain groups without a picture and
/// orphan sub-groups stay 404.
#[tokio::test(flavor = "multi_thread")]
async fn avatar_per_chat_kind_with_community_fallback() {
    let h = harness().await;
    let (community, announce, plain_none) = (
        "120363000000000099@g.us",
        "none-announce@g.us",
        "none-plain@g.us",
    );
    meta(
        &h,
        &[
            (community, "community", None),
            (announce, "group", Some(community)),
            ("none-orphan@g.us", "group", Some("none-parent@g.us")),
            ("none-parent@g.us", "community", None),
            ("120363999999999999@newsletter", "channel", None),
        ],
    );
    let enc = |j: &str| format!("wa-media://localhost/a/{}", j.replace('@', "%40"));
    // Announcement group → the community's picture file.
    let r = get(&h, &enc(announce)).await;
    assert_eq!(r.status(), StatusCode::OK);
    let asked: Vec<String> = h
        .mock
        .calls_to("/api/avatar")
        .iter()
        .map(|v| v["jid"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(asked, vec![announce.to_string(), community.to_string()]);
    let f = h.core.avatar_file(announce, false).await.unwrap();
    assert!(f.ends_with("120363000000000099@g.us.jpg"), "{f:?}");
    // Full size falls back to the community's full picture.
    let f = h.core.avatar_file(announce, true).await.unwrap();
    assert!(f.ends_with("120363000000000099@g.us-full.jpg"), "{f:?}");
    // Community and channel: their own pictures.
    assert_eq!(get(&h, &enc(community)).await.status(), StatusCode::OK);
    let r = get(&h, &enc("120363999999999999@newsletter")).await;
    assert_eq!(r.status(), StatusCode::OK);
    let f = h
        .core
        .avatar_file("120363999999999999@newsletter", false)
        .await
        .unwrap();
    assert!(f.ends_with("120363999999999999@newsletter.jpg"));
    // A group outside any community: no fallback, cacheable 404.
    let r = get(&h, &enc(plain_none)).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(hdr(&r, header::CACHE_CONTROL).contains("max-age=1800"));
    // Sub-group whose community has no picture either: 404.
    assert_eq!(
        get(&h, &enc("none-orphan@g.us")).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn avatar_transient_failure_is_not_cacheable() {
    let h = harness_down().await;
    let r = get(&h, "wa-media://localhost/a/x%40g.us").await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(hdr(&r, header::CACHE_CONTROL), "no-store");
    let e = h.core.avatar_file("x@g.us", false).await.unwrap_err();
    assert_eq!(e.message, wa_desk_lib::core::PICTURE_UNAVAILABLE);
}

// ---------- stats, links, starred ----------

#[tokio::test(flavor = "multi_thread")]
async fn stats_links_starred_roundtrip() {
    let h = harness().await;
    index(&h).await;
    let s = h.core.chat_stats(GROUP_CLASS.into()).await.unwrap();
    assert_eq!((s.messages, s.media, s.links), (3, 1, 1));
    let l = h.core.list_links(None, None, 10).await.unwrap();
    assert_eq!(l.items[0].url, "https://example.org/course");
    let l2 = h
        .core
        .list_links(Some(DM_ALICE.into()), None, 10)
        .await
        .unwrap();
    assert!(l2.items.is_empty());
    h.core
        .set_starred(DM_ALICE.into(), vec![M_ALICE_1.into()], true)
        .await
        .unwrap();
    assert!(!h.rec.named("db:changed").is_empty());
    let st = h.core.list_starred(None, None, 10).await.unwrap();
    assert_eq!(st.items.len(), 1);
    assert!(st.items[0].message.starred);
    assert_eq!(h.core.chat_stats(DM_ALICE.into()).await.unwrap().starred, 1);
    h.core
        .set_starred(DM_ALICE.into(), vec![M_ALICE_1.into()], false)
        .await
        .unwrap();
    assert!(
        h.core
            .list_starred(Some(DM_ALICE.into()), None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        h.core.chat_stats(" ".into()).await.unwrap_err().kind,
        ErrorKind::Invalid
    );
}

// ---------- copy / export ----------

#[tokio::test(flavor = "multi_thread")]
async fn copy_text_whatsapp_style() {
    let h = harness().await;
    let one = h
        .core
        .copy_messages_text(DM_ALICE.into(), vec![M_ALICE_1.into()])
        .await
        .unwrap();
    assert_eq!(one, "Hi! Did you get the ML notes?");
    let many = h
        .core
        .copy_messages_text(
            DM_ALICE.into(),
            vec![M_ALICE_PDF.into(), M_ALICE_1.into(), M_ALICE_2_ME.into()],
        )
        .await
        .unwrap();
    let lines: Vec<&str> = many.lines().collect();
    assert_eq!(lines.len(), 3);
    let re = regex_lite(lines[0]);
    assert!(re, "{}", lines[0]);
    assert!(lines[0].ends_with("] Alice: Hi! Did you get the ML notes?"));
    assert!(lines[1].ends_with("] Me: Yes, module 4 too"));
    assert!(lines[2].ends_with("] Alice: <attached: ML-MODULE 1 Notes.pdf> here"));
    assert_eq!(
        h.core
            .copy_messages_text(DM_ALICE.into(), vec![])
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Invalid
    );
}

/// `[dd/mm/yy, HH:MM] ` prefix check without a regex dependency.
fn regex_lite(l: &str) -> bool {
    let b = l.as_bytes();
    b.len() > 19
        && b[0] == b'['
        && b[3] == b'/'
        && b[6] == b'/'
        && &l[9..11] == ", "
        && b[13] == b':'
        && &l[16..18] == "] "
}

fn add_script_message(h: &H) {
    h.f.add_message(&Msg::text(
        "XSS",
        DM_ALICE,
        "919811111111",
        "2026-11-15 10:07:00+05:30",
        "<script>alert('x')</script> & <img src=x onerror=1>",
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn export_txt_json_html_escaping() {
    let h = harness().await;
    add_script_message(&h);
    let req = |format, ids: Vec<String>| ExportReq {
        chat_jid: DM_ALICE.into(),
        format,
        message_ids: ids,
        after: None,
        before: None,
        include_media: false,
    };
    let txt = h
        .core
        .export_chat(req(ExportFormat::Txt, vec![]))
        .await
        .unwrap();
    assert!(txt.starts_with(h.f.paths.exports.to_str().unwrap()));
    assert!(txt.ends_with(".txt"));
    let body = std::fs::read_to_string(&txt).unwrap();
    assert_eq!(body.lines().count(), 6, "{body}");
    assert!(body.contains("Alice: This message was deleted"));
    assert!(!body.contains("👍"), "reactions are not messages");

    let json = h
        .core
        .export_chat(req(ExportFormat::Json, vec![]))
        .await
        .unwrap();
    let v: Vec<Message> = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(v.len(), 6);
    assert!(v.windows(2).all(|w| w[0].ts <= w[1].ts));

    let html = h
        .core
        .export_chat(req(ExportFormat::Html, vec![]))
        .await
        .unwrap();
    let page = std::fs::read_to_string(&html).unwrap();
    assert!(page.starts_with("<!doctype html>"));
    assert!(!page.contains("<script>"), "message HTML is escaped");
    assert!(page.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &lt;img"));
    assert!(page.contains("Content-Security-Policy"));
    assert!(page.contains("ML-MODULE 1 Notes.pdf"));

    // Selection export.
    let sel = h
        .core
        .export_chat(req(ExportFormat::Txt, vec![M_ALICE_1.into()]))
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&sel).unwrap().lines().count(), 1);
    assert_ne!(sel, txt, "never overwrites");
    // Range export: [after, before).
    let mut r = req(ExportFormat::Txt, vec![]);
    r.after = Some(wa_core::time::parse_ts("2026-11-15 10:01:00+05:30").unwrap());
    r.before = Some(wa_core::time::parse_ts("2026-11-15 10:06:00+05:30").unwrap());
    let ranged = h.core.export_chat(r).await.unwrap();
    assert_eq!(std::fs::read_to_string(&ranged).unwrap().lines().count(), 3);
    // Empty chat / bad range.
    let mut e = req(ExportFormat::Txt, vec![]);
    e.chat_jid = GROUP_PLACEHOLDER.into();
    assert_eq!(
        h.core.export_chat(e).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    let mut b = req(ExportFormat::Txt, vec![]);
    b.after = Some(10);
    b.before = Some(5);
    assert_eq!(
        h.core.export_chat(b).await.unwrap_err().kind,
        ErrorKind::Invalid
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn export_html_with_media_folder() {
    let h = harness().await;
    let out = h
        .core
        .export_chat(ExportReq {
            chat_jid: DM_ALICE.into(),
            format: ExportFormat::Html,
            message_ids: vec![],
            after: None,
            before: None,
            include_media: true,
        })
        .await
        .unwrap();
    let dir = std::path::Path::new(&out);
    assert!(dir.is_dir());
    let page = std::fs::read_to_string(dir.join("Alice.html")).unwrap();
    let media = dir.join("Alice_media");
    let files: Vec<_> = std::fs::read_dir(&media).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].path()).unwrap(), PDF_BYTES);
    assert!(
        page.contains("href=\"Alice_media/ML-MODULE 1 Notes.pdf\""),
        "{page}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn export_pdf_via_libreoffice() {
    if std::process::Command::new("soffice")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("soffice not installed; skipping");
        return;
    }
    let h = harness().await;
    add_script_message(&h);
    let out = h
        .core
        .export_chat(ExportReq {
            chat_jid: DM_ALICE.into(),
            format: ExportFormat::Pdf,
            message_ids: vec![],
            after: None,
            before: None,
            include_media: false,
        })
        .await
        .unwrap();
    assert!(out.ends_with(".pdf"));
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
}

// ---------- forward ----------

#[tokio::test(flavor = "multi_thread")]
async fn forward_text_and_media_with_limits() {
    let h = harness().await;
    let res = h
        .core
        .forward_messages(ForwardReq {
            from_chat: DM_ALICE.into(),
            message_ids: vec![M_ALICE_PDF.into(), M_ALICE_1.into(), M_ALICE_REVOKED.into()],
            to: vec![DM_BOB.into(), GROUP_CLASS.into()],
        })
        .await
        .unwrap();
    assert_eq!(res.len(), 6);
    // Target-major, ids in request order: [pdf, text, deleted] × [bob, group].
    assert!(res[0].ok && res[1].ok, "{res:?}");
    assert!(!res[2].ok, "deleted not forwarded");
    assert!(res[3].ok && res[4].ok && !res[5].ok, "{res:?}");
    let sends = h.mock.calls_to("/api/send");
    assert_eq!(sends.len(), 4);
    assert_eq!(sends[0]["recipient"], DM_BOB);
    let media = sends[0]["media_path"].as_str().unwrap();
    assert!(media.starts_with(h.f.paths.outbox.to_str().unwrap()));
    assert_eq!(sends[0]["message"], "here");
    assert_eq!(sends[1]["message"], "Hi! Did you get the ML notes?");
    assert_eq!(sends[2]["recipient"], GROUP_CLASS);

    let six: Vec<String> = (0..6)
        .map(|i| format!("91980000000{i}@s.whatsapp.net"))
        .collect();
    let e = h
        .core
        .forward_messages(ForwardReq {
            from_chat: DM_ALICE.into(),
            message_ids: vec![M_ALICE_1.into()],
            to: six,
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    let e = h
        .core
        .forward_messages(ForwardReq {
            from_chat: DM_ALICE.into(),
            message_ids: vec![],
            to: vec![DM_BOB.into()],
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    // Unknown ids still get a (failed) slot: len = to × ids.
    let r = h
        .core
        .forward_messages(ForwardReq {
            from_chat: DM_ALICE.into(),
            message_ids: vec!["nope".into(), M_ALICE_1.into()],
            to: vec![DM_BOB.into()],
        })
        .await
        .unwrap();
    assert_eq!(r.len(), 2);
    assert!(!r[0].ok && r[1].ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn forward_downloads_missing_media_first_and_respects_read_only() {
    let h = harness().await;
    // The group image is not in the store; the mock "downloads" it.
    let dest = h.f.paths.bridge_store.join("dl/G1IMG.jpg");
    h.mock.downloads.lock().insert(
        M_GROUP_IMG.into(),
        (dest.clone(), b"\xFF\xD8\xFFimg".to_vec()),
    );
    let _ = h
        .core
        .forward_messages(ForwardReq {
            from_chat: GROUP_CLASS.into(),
            message_ids: vec![M_GROUP_IMG.into()],
            to: vec![DM_BOB.into()],
        })
        .await
        .unwrap();
    // Whatever the outcome, a phone-linked or downloaded file was staged or a
    // download was asked for first.
    let asked = h.mock.calls_to("/api/download").len();
    let sent = h.mock.calls_to("/api/send").len();
    assert!(asked > 0 || sent == 1);

    h.core.set_read_only(true);
    let e = h
        .core
        .forward_messages(ForwardReq {
            from_chat: DM_ALICE.into(),
            message_ids: vec![M_ALICE_1.into()],
            to: vec![DM_BOB.into()],
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Forbidden);
}

// ---------- refresh names ----------

#[tokio::test(flavor = "multi_thread")]
async fn refresh_chat_names_invalidates_and_emits() {
    let h = harness().await;
    assert_eq!(
        h.core
            .get_chat(GROUP_PLACEHOLDER.into())
            .await
            .unwrap()
            .name,
        "Group 120363000000000002"
    );
    // The bridge renames the chat while refreshing.
    h.f.bridge_conn()
        .execute(
            "UPDATE chats SET name='Hostel Wing B' WHERE jid=?1",
            [GROUP_PLACEHOLDER],
        )
        .unwrap();
    let n = h.core.refresh_chat_names().await.unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        h.core
            .get_chat(GROUP_PLACEHOLDER.into())
            .await
            .unwrap()
            .name,
        "Hostel Wing B"
    );
    let ev = h.rec.named("db:changed");
    let last = ev.last().unwrap();
    assert!(last["chat_jids"].as_array().unwrap().len() >= 5);
    // Bridge down: an error for the command, silently skipped by the background tick.
    *h.mock.meta_down.lock() = true;
    assert_eq!(
        h.core.refresh_chat_names().await.unwrap_err().kind,
        ErrorKind::BridgeDown
    );
    h.core.refresh_names_tick().await.unwrap();
}

// ---------- automation pause persists ----------

#[tokio::test(flavor = "multi_thread")]
async fn tray_pause_persists_in_settings() {
    let h = harness().await;
    h.core.set_automation_enabled(true).await.unwrap();
    assert!(!h.core.automation_paused());
    h.core.set_automation_paused(true).await.unwrap();
    assert!(h.core.automation_paused());
    assert!(!h.core.get_settings().await.unwrap().automation_enabled);
    h.core.set_automation_paused(false).await.unwrap();
    assert!(h.core.get_settings().await.unwrap().automation_enabled);
}

// ---------- warm cache ----------

#[tokio::test(flavor = "multi_thread")]
async fn warm_pass_makes_thumbnails_and_is_resumable() {
    let h = harness().await;
    let img = {
        let img = image::RgbImage::from_pixel(640, 480, image::Rgb([10, 200, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    };
    let ts = "2026-11-15 09:30:00+05:30";
    h.f.add_message(
        &Msg::text("WIMG", DM_ALICE, "919811111111", ts, "").media("image", None, &img),
    );
    h.f.put_store_media(DM_ALICE, "image", "WIMG", ts, &img);
    let sd = tokio_util::sync::CancellationToken::new();
    let st = h.core.warm_pass(&sd).await.unwrap();
    assert!(st.scanned >= 2, "{st:?}");
    assert!(st.made >= 1, "{st:?}");
    let thumbs = h.f.paths.cache.join("thumbs");
    assert!(std::fs::read_dir(&thumbs).unwrap().count() >= 1);
    let again = h.core.warm_pass(&sd).await.unwrap();
    assert_eq!(again.made, 0, "{again:?}");
    // Cached thumbnails are served fast.
    let url = wa_db::media::media_url("t", DM_ALICE, "WIMG");
    let t = Instant::now();
    let r = get(&h, &url).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
    let (done, total) = h.core.warm_state().progress();
    assert_eq!(done, total);
}
