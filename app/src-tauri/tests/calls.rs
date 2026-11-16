//! Calls: list_calls, reject_call, `call:update` events and notifications.

mod common;

use common::*;
use wa_core::ErrorKind;
use wa_core::types::*;
use wa_testkit::ME;
use wa_testkit::ids::*;

async fn prime(h: &H) {
    assert_eq!(h.core.watch_tick().await.unwrap(), 0);
}

fn ring(h: &H, id: &str, chat: &str, from: &str, kind: &str, result: &str) {
    h.f.add_call(id, chat, from, &ts_now(0), false, kind, result, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_calls_returns_log() {
    let h = harness().await;
    ring(&h, "C1", DM_ALICE, DM_ALICE, "voice", "missed");
    ring(&h, "C2", GROUP_CLASS, DM_BOB, "video", "ended");
    let p = h.core.list_calls(CallQuery::default()).await.unwrap();
    assert_eq!(p.items.len(), 2);
    let m = h
        .core
        .list_calls(CallQuery {
            missed_only: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(m.items.len(), 1);
    assert_eq!(m.items[0].chat_name, "Alice");
}

#[tokio::test(flavor = "multi_thread")]
async fn ringing_call_emits_and_notifies_then_updates() {
    let h = harness().await;
    prime(&h).await;
    ring(&h, "R1", DM_ALICE, DM_ALICE, "voice", "in_progress");
    h.core.watch_tick().await.unwrap();
    let ev = h.rec.named(events::CALL_UPDATE);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["result"], "in_progress");
    assert_eq!(ev[0]["caller_name"], "Alice");
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].0, "📞 Voice call from Alice");
    assert!(notes[0].1.contains("phone"));
    // Becomes missed: a second update and a missed-call notification.
    ring(&h, "R1", DM_ALICE, DM_ALICE, "voice", "missed");
    h.core.watch_tick().await.unwrap();
    assert_eq!(h.rec.named(events::CALL_UPDATE).len(), 2);
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[1].0, "📞 Missed voice call");
    assert_eq!(notes[1].1, "from Alice");
    // Quiet afterwards.
    h.core.watch_tick().await.unwrap();
    assert_eq!(h.rec.named(events::CALL_UPDATE).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn video_and_group_notification_text() {
    let h = harness().await;
    prime(&h).await;
    ring(&h, "V1", DM_BOB, DM_BOB, "video", "in_progress");
    ring(&h, "G1", GROUP_CLASS, DM_BOB, "video", "in_progress");
    h.core.watch_tick().await.unwrap();
    let notes = h.platform.notes.lock().clone();
    assert!(notes[0].0.starts_with("📹 Video call from "), "{notes:?}");
    assert_eq!(notes[1].0, "👥 Group call in ML CSE - 3C");
    assert_eq!(notes[1].1, "Answer on your phone");
}

#[tokio::test(flavor = "multi_thread")]
async fn call_notifications_respect_settings_and_own_calls() {
    let h = harness().await;
    prime(&h).await;
    let mut s = h.core.get_settings().await.unwrap();
    s.desktop_notifications = false;
    h.core.set_settings(s).await.unwrap();
    ring(&h, "Q1", DM_ALICE, DM_ALICE, "voice", "in_progress");
    h.core.watch_tick().await.unwrap();
    assert!(h.platform.notes.lock().is_empty());
    assert_eq!(h.rec.named(events::CALL_UPDATE).len(), 1);
    // Own outgoing calls never notify.
    let mut s = h.core.get_settings().await.unwrap();
    s.desktop_notifications = true;
    h.core.set_settings(s).await.unwrap();
    h.f.add_call(
        "O1",
        DM_ALICE,
        ME,
        &ts_now(0),
        true,
        "voice",
        "missed",
        None,
    );
    h.core.watch_tick().await.unwrap();
    assert!(h.platform.notes.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn reject_validates_in_progress_and_calls_bridge() {
    let h = harness().await;
    ring(&h, "R1", DM_ALICE, DM_ALICE, "voice", "in_progress");
    ring(&h, "R2", DM_ALICE, DM_ALICE, "voice", "ended");
    h.core
        .reject_call("R1".into(), DM_ALICE.into())
        .await
        .unwrap();
    let c = h.mock.calls_to("/api/call/reject");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["call_id"], "R1");
    assert_eq!(c[0]["caller_jid"], DM_ALICE);
    for id in ["R2", "NOPE"] {
        let e = h
            .core
            .reject_call(id.into(), DM_ALICE.into())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Invalid);
    }
    assert_eq!(h.mock.calls_to("/api/call/reject").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn reject_blocked_in_read_only() {
    let h = harness().await;
    ring(&h, "R1", DM_ALICE, DM_ALICE, "voice", "in_progress");
    h.core.set_read_only(true);
    let e = h
        .core
        .reject_call("R1".into(), DM_ALICE.into())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Forbidden);
    assert!(h.mock.calls_to("/api/call/reject").is_empty());
    // Reading the log still works.
    assert_eq!(
        h.core
            .list_calls(CallQuery::default())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
}
