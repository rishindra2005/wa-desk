//! AppCore command paths against the standard fixture and a mock bridge.

mod common;

use std::io::Read;

use common::*;
use wa_core::ErrorKind;
use wa_core::types::*;
use wa_testkit::ids::*;
use wa_testkit::{ME, TOKEN};

fn img_bytes() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([200, 30, 30]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn overview_reports_counts_and_version() {
    let h = harness().await;
    let o = h.core.overview().await.unwrap();
    assert!(o.chats >= 5);
    assert_eq!(o.messages, 13);
    assert_eq!(o.app_version, env!("CARGO_PKG_VERSION"));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_chats_newest_first() {
    let h = harness().await;
    let p = h.core.list_chats(ChatQuery::default()).await.unwrap();
    let ts: Vec<i64> = p.items.iter().filter_map(|c| c.last_message_ts).collect();
    assert!(ts.windows(2).all(|w| w[0] >= w[1]), "{ts:?}");
    assert_eq!(p.items[0].jid, GROUP_CLASS);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_chat_and_missing_chat() {
    let h = harness().await;
    assert_eq!(
        h.core.get_chat(DM_ALICE.into()).await.unwrap().name,
        "Alice"
    );
    let e = h
        .core
        .get_chat("nobody@s.whatsapp.net".into())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let e = h.core.get_chat(" ".into()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_ascending_and_context() {
    let h = harness().await;
    let p = h
        .core
        .get_messages(MessagesQuery {
            chat_jid: DM_ALICE.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(p.items.windows(2).all(|w| w[0].ts <= w[1].ts));
    let ctx = h
        .core
        .get_message_context(DM_ALICE.into(), M_ALICE_PDF.into(), 1)
        .await
        .unwrap();
    assert!(ctx.iter().any(|m| m.id == M_ALICE_PDF));
    assert!(ctx.len() <= 3);
    let e = h
        .core
        .get_message_context(DM_ALICE.into(), "NOPE".into(), 3)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn search_after_index_build_with_byte_highlights() {
    let h = harness().await;
    let job = h
        .core
        .ensure_index()
        .await
        .unwrap()
        .expect("full build job");
    assert_eq!(job.spec, JobSpec::Reindex);
    let done = wait_job(&h.core, job.id).await;
    assert_eq!(done.status, JobStatus::Succeeded, "{done:?}");
    assert!(!h.rec.named(events::JOB_FINISHED).is_empty());
    let r = h
        .core
        .search(SearchQuery {
            text: "exam".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!r.index_building);
    let hit = r
        .hits
        .iter()
        .find(|h| h.message.id == M_GROUP_DEVANAGARI)
        .unwrap();
    let (s, e) = hit.highlights[0];
    assert_eq!(&hit.message.content[s as usize..e as usize], "exam");
    // Already complete: silent incremental pass, no job.
    assert!(h.core.ensure_index().await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn list_media_has_urls_for_local_files() {
    let h = harness().await;
    let j = h.core.ensure_index().await.unwrap().unwrap();
    wait_job(&h.core, j.id).await;
    let p = h.core.list_media(MediaQuery::default()).await.unwrap();
    let pdf = p
        .items
        .iter()
        .find(|i| i.message_id == M_ALICE_PDF)
        .unwrap();
    assert!(
        pdf.media
            .url
            .as_deref()
            .unwrap()
            .starts_with("wa-media://localhost/m/")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ensure_media_local_needs_no_bridge() {
    let h = harness().await;
    let r = h
        .core
        .ensure_media(DM_ALICE.into(), M_ALICE_PDF.into())
        .await
        .unwrap();
    assert_eq!(r.source, MediaSource::Store);
    assert!(h.mock.calls_to("/api/download").is_empty());
}

fn queue_group_image(h: &H, data: &[u8]) {
    let p = wa_core::paths::bridge_media_path(
        &h.f.paths.bridge_store,
        GROUP_CLASS,
        "image",
        M_GROUP_IMG,
        "2026-11-15 11:58:00+05:30",
    )
    .unwrap();
    h.mock
        .downloads
        .lock()
        .insert(M_GROUP_IMG.into(), (p, data.to_vec()));
}

#[tokio::test(flavor = "multi_thread")]
async fn ensure_media_missing_triggers_download() {
    let h = harness().await;
    queue_group_image(&h, &img_bytes());
    let r = h
        .core
        .ensure_media(GROUP_CLASS.into(), M_GROUP_IMG.into())
        .await
        .unwrap();
    assert_ne!(r.source, MediaSource::Missing);
    let calls = h.mock.calls_to("/api/download");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["no_retry"], false);
    assert_eq!(calls[0]["chat_jid"], GROUP_CLASS);
}

#[tokio::test(flavor = "multi_thread")]
async fn ensure_media_download_failure_is_external() {
    let h = harness().await;
    let e = h
        .core
        .ensure_media(GROUP_CLASS.into(), M_GROUP_IMG.into())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::External);
}

#[tokio::test(flavor = "multi_thread")]
async fn ensure_media_bridge_down_is_friendly() {
    let h = harness_down().await;
    let e = h
        .core
        .ensure_media(GROUP_CLASS.into(), M_GROUP_IMG.into())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::BridgeDown);
    assert!(e.message.contains("offline"), "{}", e.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn ensure_media_on_text_is_not_found() {
    let h = harness().await;
    let e = h
        .core
        .ensure_media(DM_ALICE.into(), M_ALICE_1.into())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_pdf_and_image() {
    let h = harness().await;
    let p = h
        .core
        .preview_doc(DM_ALICE.into(), M_ALICE_PDF.into())
        .await
        .unwrap();
    assert_eq!(p.kind, PreviewKind::Pdf);
    assert!(p.url.as_deref().unwrap().contains("/m/"));
    assert_eq!(p.url, p.original_url);
    queue_group_image(&h, &img_bytes());
    let p = h
        .core
        .preview_doc(GROUP_CLASS.into(), M_GROUP_IMG.into())
        .await
        .unwrap();
    assert_eq!(p.kind, PreviewKind::Image);
    assert_eq!(
        h.mock.calls_to("/api/download").len(),
        1,
        "missing file was ensured"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_text_small_and_large() {
    let h = harness().await;
    add_doc(&h.f, "TXT1", "notes.md", b"# hello");
    let p = h
        .core
        .preview_doc(DM_ALICE.into(), "TXT1".into())
        .await
        .unwrap();
    assert_eq!(p.kind, PreviewKind::Text);
    add_doc(&h.f, "TXT2", "huge.log", &vec![b'a'; 1_100_000]);
    let p = h
        .core
        .preview_doc(DM_ALICE.into(), "TXT2".into())
        .await
        .unwrap();
    assert_eq!(p.kind, PreviewKind::Unsupported);
    assert!(p.note.unwrap().contains("too large"));
    assert!(p.original_url.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_unsupported_type() {
    let h = harness().await;
    add_doc(&h.f, "ZIP1", "archive.zip", b"PK\x03\x04junk");
    let p = h
        .core
        .preview_doc(DM_ALICE.into(), "ZIP1".into())
        .await
        .unwrap();
    assert_eq!(p.kind, PreviewKind::Unsupported);
    assert!(p.url.is_none() && p.note.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_office_converts_or_explains() {
    let h = harness().await;
    add_doc(&h.f, "DOC1", "report.rtf", b"{\\rtf1\\ansi hello world}");
    let p = h
        .core
        .preview_doc(DM_ALICE.into(), "DOC1".into())
        .await
        .unwrap();
    match p.kind {
        PreviewKind::Pdf => {
            let url = p.url.unwrap();
            assert!(url.starts_with("wa-media://localhost/p/"));
            let req = tauri::http::Request::builder()
                .uri(&url)
                .body(Vec::new())
                .unwrap();
            let r = wa_desk_lib::protocol::handle(&h.core, &req).await;
            assert_eq!(r.status(), 200);
            assert!(r.body().starts_with(b"%PDF"));
        }
        PreviewKind::Unsupported => assert!(p.note.is_some()),
        k => panic!("unexpected {k:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn open_media_external_opens_named_copy() {
    let h = harness().await;
    h.core
        .open_media_external(DM_ALICE.into(), M_ALICE_PDF.into())
        .await
        .unwrap();
    let opened = h.platform.opened.lock().clone();
    assert_eq!(opened.len(), 1);
    // Opened under its original name (with extension) so the desktop picks the
    // right app, and the content is the stored file.
    assert!(
        opened[0].ends_with("A3PDF/ML-MODULE 1 Notes.pdf"),
        "{:?}",
        opened[0]
    );
    assert_eq!(
        std::fs::read(&opened[0]).unwrap(),
        wa_testkit::ids::PDF_BYTES
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn save_media_as_copies_and_overwrites() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("notes.pdf");
    std::fs::write(&dest, b"old").unwrap();
    let out = h
        .core
        .save_media_as(
            DM_ALICE.into(),
            M_ALICE_PDF.into(),
            dest.to_string_lossy().into(),
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(out).unwrap(), PDF_BYTES);
}

#[tokio::test(flavor = "multi_thread")]
async fn save_media_as_refusals() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let store = h.f.paths.bridge_store.join("copy.pdf");
    let cases = [
        ("relative.pdf".to_string(), ErrorKind::Invalid),
        (
            dir.path().join("missing/x.pdf").to_string_lossy().into(),
            ErrorKind::Invalid,
        ),
        (store.to_string_lossy().into(), ErrorKind::Forbidden),
        (
            dir.path().join("evil.db").to_string_lossy().into(),
            ErrorKind::Forbidden,
        ),
        (
            dir.path().join(".bridge-token").to_string_lossy().into(),
            ErrorKind::Forbidden,
        ),
        (
            dir.path().join("sub").to_string_lossy().into(),
            ErrorKind::Invalid,
        ),
    ];
    for (dest, kind) in cases {
        let e = h
            .core
            .save_media_as(DM_ALICE.into(), M_ALICE_PDF.into(), dest.clone())
            .await
            .unwrap_err();
        assert_eq!(e.kind, kind, "{dest}");
    }
    assert!(!store.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn send_text() {
    let h = harness().await;
    let r = h
        .core
        .send_message(SendReq {
            recipient: DM_ALICE.into(),
            text: Some("hello".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(r.ok);
    let c = h.mock.calls_to("/api/send");
    assert_eq!(c[0]["recipient"], DM_ALICE);
    assert_eq!(c[0]["message"], "hello");
    assert_eq!(c[0]["media_path"], "");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_validation() {
    let h = harness().await;
    let bad = [
        SendReq {
            recipient: " ".into(),
            text: Some("x".into()),
            ..Default::default()
        },
        SendReq {
            recipient: DM_ALICE.into(),
            text: Some("   ".into()),
            ..Default::default()
        },
        SendReq {
            recipient: DM_ALICE.into(),
            text: Some("x".repeat(65_537)),
            ..Default::default()
        },
        SendReq {
            recipient: "not a number".into(),
            text: Some("x".into()),
            ..Default::default()
        },
    ];
    for r in bad {
        let e = h.core.send_message(r).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Invalid);
    }
    assert!(h.mock.calls_to("/api/send").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn send_with_attachment_is_staged_in_outbox() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("photo.jpg");
    std::fs::write(&src, b"jpegdata").unwrap();
    h.core
        .send_message(SendReq {
            recipient: "+91 98111 11111".into(),
            attachment: Some(src.to_string_lossy().into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let c = h.mock.calls_to("/api/send");
    let staged = std::path::PathBuf::from(c[0]["media_path"].as_str().unwrap());
    assert!(staged.starts_with(&h.f.paths.outbox), "{staged:?}");
    assert_eq!(std::fs::read(&staged).unwrap(), b"jpegdata");
    assert!(src.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn send_refuses_database_attachment() {
    let h = harness().await;
    let e = h
        .core
        .send_message(SendReq {
            recipient: DM_ALICE.into(),
            attachment: Some(h.f.paths.messages_db.to_string_lossy().into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Forbidden);
}

#[tokio::test(flavor = "multi_thread")]
async fn reply_quotes_own_and_inbound_messages() {
    let h = harness().await;
    for (rid, sender, content) in [
        (M_ALICE_2_ME, ME, "Yes, module 4 too"),
        (M_ALICE_1, DM_ALICE, "Hi! Did you get the ML notes?"),
    ] {
        h.core
            .send_message(SendReq {
                recipient: DM_ALICE.into(),
                text: Some("re".into()),
                reply_to: Some(rid.into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let c = h.mock.calls_to("/api/send").pop().unwrap();
        assert_eq!(c["quoted_message_id"], rid);
        assert_eq!(c["quoted_sender_jid"], sender);
        assert_eq!(c["quoted_content"], content);
    }
    let e = h
        .core
        .send_message(SendReq {
            recipient: DM_ALICE.into(),
            text: Some("re".into()),
            reply_to: Some(M_BOB_1.into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Invalid,
        "reply to another chat's message"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_bridge_down_cleans_staged_file() {
    let h = harness_down().await;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("a.txt");
    std::fs::write(&src, b"x").unwrap();
    let e = h
        .core
        .send_message(SendReq {
            recipient: DM_ALICE.into(),
            text: Some("hi".into()),
            attachment: Some(src.to_string_lossy().into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::BridgeDown);
    let left = std::fs::read_dir(&h.f.paths.outbox).unwrap().count();
    assert_eq!(left, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn react_in_group_sets_sender() {
    let h = harness().await;
    h.core
        .react(ReactReq {
            chat_jid: GROUP_CLASS.into(),
            message_id: M_GROUP_DEVANAGARI.into(),
            emoji: "🙏".into(),
        })
        .await
        .unwrap();
    h.core
        .react(ReactReq {
            chat_jid: DM_ALICE.into(),
            message_id: M_ALICE_2_ME.into(),
            emoji: String::new(),
        })
        .await
        .unwrap();
    let c = h.mock.calls_to("/api/react");
    assert_eq!(c[0]["from_me"], false);
    assert_eq!(c[0]["sender_jid"], "919822222222@s.whatsapp.net");
    assert_eq!(c[1]["from_me"], true);
    assert_eq!(c[1]["sender_jid"], "");
    assert_eq!(c[1]["emoji"], "");
    let e = h
        .core
        .react(ReactReq {
            chat_jid: DM_ALICE.into(),
            message_id: "NOPE".into(),
            emoji: "👍".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_read_dm_only_after_marker() {
    let h = harness().await;
    h.core.mark_read(DM_ALICE.into()).await.unwrap();
    let c = h.mock.calls_to("/api/mark-read");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["message_ids"], serde_json::json!([M_ALICE_REPLY]));
    assert!(c[0].get("sender_jid").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_read_group_one_call_per_sender() {
    let h = harness().await;
    h.core.mark_read(GROUP_CLASS.into()).await.unwrap();
    let c = h.mock.calls_to("/api/mark-read");
    assert_eq!(c.len(), 3, "{c:?}");
    for call in &c {
        assert_eq!(call["message_ids"].as_array().unwrap().len(), 1);
        assert!(
            call["sender_jid"]
                .as_str()
                .unwrap()
                .ends_with("@s.whatsapp.net")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_read_read_chat_is_noop() {
    let h = harness().await;
    h.core.mark_read(DM_BOB.into()).await.unwrap();
    assert!(h.mock.calls_to("/api/mark-read").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_and_reconnect() {
    let h = harness().await;
    let s = h.core.connection_status().await;
    assert_eq!(s.state, BridgeState::Connected);
    let s = h.core.reconnect().await.unwrap();
    assert!(s.connected);
    assert_eq!(h.mock.calls_to("/api/connection/reconnect").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_down_states() {
    let h = harness_down().await;
    assert_eq!(
        h.core.connection_status().await.state,
        BridgeState::Unreachable
    );
    assert_eq!(
        h.core.reconnect().await.unwrap_err().kind,
        ErrorKind::BridgeDown
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn jobs_reindex_link_defaults_and_cancel() {
    let h = harness().await;
    let j = h.core.start_job(JobSpec::Reindex).await.unwrap();
    assert_eq!(wait_job(&h.core, j.id).await.status, JobStatus::Succeeded);
    let mut s = h.core.get_settings().await.unwrap();
    let extra = tempfile::tempdir().unwrap();
    s.link_extra_dirs = vec![extra.path().to_string_lossy().into()];
    h.core.set_settings(s.clone()).await.unwrap();
    let j = h
        .core
        .start_job(JobSpec::LinkPhoneMedia { extra_dirs: vec![] })
        .await
        .unwrap();
    assert_eq!(
        j.spec,
        JobSpec::LinkPhoneMedia {
            extra_dirs: s.link_extra_dirs.clone()
        }
    );
    wait_job(&h.core, j.id).await;
    let all = h.core.list_jobs(10).await.unwrap();
    assert!(all.len() >= 2 && all[0].id > all[1].id);
    assert_eq!(
        h.core.cancel_job(9999).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

fn rule(regex: Option<&str>) -> Rule {
    Rule {
        id: None,
        name: "ping".into(),
        enabled: true,
        dry_run: true,
        trigger: Trigger {
            keywords: vec!["ping".into()],
            regex: regex.map(Into::into),
            ..Default::default()
        },
        actions: vec![Action::Reply {
            text: "pong".into(),
        }],
        cooldown_secs: 600,
        fired_count: 0,
        last_fired_at: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rules_crud_and_test() {
    let h = harness().await;
    let e = h.core.save_rule(rule(Some("(unclosed"))).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    let saved = h.core.save_rule(rule(None)).await.unwrap();
    let id = saved.id.unwrap();
    assert_eq!(h.core.list_rules().await.unwrap().len(), 1);
    let mut sample = h
        .core
        .get_messages(MessagesQuery {
            chat_jid: DM_BOB.into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .remove(0);
    sample.content = "ping?".into();
    let ev = h.core.test_rule(saved.clone(), sample).await.unwrap();
    assert!(ev.matched, "{ev:?}");
    assert!(!ev.planned.is_empty());
    h.core.delete_rule(id).await.unwrap();
    assert!(h.core.list_rules().await.unwrap().is_empty());
    assert!(h.core.automation_log(10).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn enabling_automation_sets_watermark() {
    let h = harness().await;
    let s = h.core.set_automation_enabled(true).await.unwrap();
    assert!(s.automation_enabled);
    use wa_core::traits::{AppStore, MessageStore};
    let db = h.core.db();
    let wm = db.get_kv(wa_automation::WATERMARK_KEY).unwrap().unwrap();
    assert_eq!(wm, db.max_rowid().unwrap().to_string());
    assert!(h.core.get_settings().await.unwrap().automation_enabled);
    let s = h.core.set_automation_enabled(false).await.unwrap();
    assert!(!s.automation_enabled);
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_roundtrip_and_autostart() {
    let h = harness().await;
    let mut s = h.core.get_settings().await.unwrap();
    s.theme = Theme::Dark;
    s.autostart = true;
    s.automation_max_per_hour = 0;
    let out = h.core.set_settings(s.clone()).await.unwrap();
    assert_eq!(out.automation_max_per_hour, 1, "clamped");
    let back = h.core.get_settings().await.unwrap();
    assert_eq!(back, out);
    assert_eq!(*h.platform.autostart.lock(), vec![true]);
    h.core.set_settings(back.clone()).await.unwrap();
    assert_eq!(
        h.platform.autostart.lock().len(),
        1,
        "unchanged → not reapplied"
    );
    let mut bad = back;
    bad.link_extra_dirs = vec!["relative/dir".into()];
    assert_eq!(
        h.core.set_settings(bad).await.unwrap_err().kind,
        ErrorKind::Invalid
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn issues_lifecycle_emits_event() {
    use wa_core::traits::IssueSink;
    let h = harness().await;
    h.core.issues().record(
        ErrorKind::External,
        "test",
        "Something broke",
        Some("detail"),
    );
    assert_eq!(h.rec.named(events::ISSUE_NEW).len(), 1);
    let list = h.core.list_issues().await.unwrap();
    assert_eq!(list.len(), 1);
    h.core.resolve_issue(list[0].id).await.unwrap();
    assert_eq!(h.core.clear_resolved_issues().await.unwrap(), 1);
    assert!(h.core.list_issues().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn bundle_contains_no_token() {
    use wa_core::traits::IssueSink;
    let h = harness().await;
    h.core.issues().record(
        ErrorKind::Auth,
        "test",
        "token problem",
        Some(&format!("Bearer {TOKEN}")),
    );
    let p = h.core.export_issue_bundle(None).await.unwrap();
    let p = std::path::PathBuf::from(p);
    assert!(p.starts_with(&h.f.paths.exports));
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&p).unwrap()).unwrap();
    let mut seen_system = false;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert!(!s.contains(TOKEN), "{} leaks the token", f.name());
        if f.name().contains("system") {
            seen_system = true;
            assert!(s.contains("app_version"));
        }
    }
    assert!(seen_system);
}

#[tokio::test(flavor = "multi_thread")]
async fn query_logs_empty_ok() {
    let h = harness().await;
    assert!(
        h.core
            .query_logs(LogFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_and_reveal_only_outputs() {
    let h = harness().await;
    let out = h.f.paths.exports.join("report.zip");
    std::fs::write(&out, b"z").unwrap();
    h.core
        .open_path(out.to_string_lossy().into())
        .await
        .unwrap();
    h.core
        .reveal_path(out.to_string_lossy().into())
        .await
        .unwrap();
    assert_eq!(h.platform.opened.lock().len(), 1);
    assert_eq!(h.platform.revealed.lock().len(), 1);
    for bad in [
        h.f.paths.token_file.clone(),
        h.f.paths.messages_db.clone(),
        std::path::PathBuf::from("/etc/passwd"),
    ] {
        let e = h
            .core
            .open_path(bad.to_string_lossy().into())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Forbidden, "{bad:?}");
    }
    let e = h.core.reveal_path("relative".into()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Forbidden);
}
