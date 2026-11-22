//! Background ticks: watcher events, automations, notifications, bridge state.

mod common;

use common::*;
use serde_json::json;
use wa_core::types::*;
use wa_testkit::Msg;
use wa_testkit::ids::*;

async fn prime(h: &H) {
    // First poll initialises the watcher at the current max rowid.
    assert_eq!(h.core.watch_tick().await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn new_rows_emit_event_and_notify_dm() {
    let h = harness().await;
    prime(&h).await;
    h.f.add_message(&Msg::text(
        "N1",
        DM_ALICE,
        "919811111111",
        &ts_now(0),
        "are you there?",
    ));
    h.f.add_message(&Msg::text(
        "N2",
        DM_BOB,
        "919822222222",
        &ts_now(0),
        "build is green",
    ));
    assert_eq!(h.core.watch_tick().await.unwrap(), 2);
    let ev = h.rec.named(events::NEW_MESSAGES);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["count"], 2);
    assert_eq!(ev[0]["chat_jids"], json!([DM_ALICE, DM_BOB]));
    assert!(ev[0]["max_rowid"].as_i64().unwrap() > 0);
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes.len(), 2);
    assert_eq!(
        notes[0],
        ("Alice".to_string(), "are you there?".to_string())
    );
    // Nothing new → no event.
    h.core.watch_tick().await.unwrap();
    assert_eq!(h.rec.named(events::NEW_MESSAGES).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_notification_when_focused_or_disabled_or_own() {
    let h = harness().await;
    prime(&h).await;
    *h.platform.focused.lock() = true;
    h.f.add_message(&Msg::text("F1", DM_ALICE, "919811111111", &ts_now(0), "hi"));
    h.core.watch_tick().await.unwrap();
    *h.platform.focused.lock() = false;
    h.f.add_message(&Msg::text("F2", DM_ALICE, "919900000000", &ts_now(0), "mine").mine());
    h.core.watch_tick().await.unwrap();
    let mut s = h.core.get_settings().await.unwrap();
    s.desktop_notifications = false;
    h.core.set_settings(s).await.unwrap();
    h.f.add_message(&Msg::text(
        "F3",
        DM_ALICE,
        "919811111111",
        &ts_now(0),
        "hi again",
    ));
    h.core.watch_tick().await.unwrap();
    assert!(h.platform.notes.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn group_mention_of_own_lid_notifies() {
    let h = harness().await;
    prime(&h).await;
    h.f.add_message(&Msg::text(
        "GL1",
        GROUP_CLASS,
        "333333333333333",
        &ts_now(0),
        "@111111111111111 please review",
    ));
    h.core.watch_tick().await.unwrap();
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes.len(), 1, "{notes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn group_notifies_only_when_mentioned() {
    let h = harness().await;
    prime(&h).await;
    h.f.add_message(&Msg::text(
        "GN1",
        GROUP_CLASS,
        "919811111111",
        &ts_now(0),
        "general chatter",
    ));
    h.core.watch_tick().await.unwrap();
    assert!(h.platform.notes.lock().is_empty());
    h.f.add_message(&Msg::text(
        "GN2",
        GROUP_CLASS,
        "919811111111",
        &ts_now(0),
        "@919900000000 can you check?",
    ));
    h.core.watch_tick().await.unwrap();
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].0.ends_with("in ML CSE - 3C"), "{notes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn many_messages_of_one_chat_collapse_to_one_notification() {
    let h = harness().await;
    prime(&h).await;
    for i in 0..5 {
        h.f.add_message(&Msg::text(
            &format!("M{i}"),
            DM_ALICE,
            "919811111111",
            &ts_now(0),
            "x",
        ));
    }
    h.core.watch_tick().await.unwrap();
    // One notification per chat, carrying the count.
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes.len(), 1);
    assert_eq!(
        notes[0],
        ("Alice (5 messages)".to_string(), "x".to_string())
    );
}

fn ping_rule() -> Rule {
    Rule {
        id: None,
        name: "ping".into(),
        enabled: true,
        dry_run: true,
        trigger: Trigger {
            keywords: vec!["ping".into()],
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
async fn automation_fires_on_new_rows_only_after_enable() {
    let h = harness().await;
    h.core.save_rule(ping_rule()).await.unwrap();
    prime(&h).await;
    // Disabled: nothing fires.
    h.f.add_message(&Msg::text("P0", DM_BOB, "919822222222", &ts_now(0), "ping"));
    h.core.watch_tick().await.unwrap();
    assert!(h.rec.named(events::AUTOMATION_FIRED).is_empty());

    h.core.set_automation_enabled(true).await.unwrap();
    h.f.add_message(&Msg::text(
        "P1",
        DM_BOB,
        "919822222222",
        &ts_now(0),
        "ping please",
    ));
    h.core.watch_tick().await.unwrap();
    let fired = h.rec.named(events::AUTOMATION_FIRED);
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert_eq!(fired[0]["message_id"], "P1");
    assert_eq!(fired[0]["dry_run"], true);
    assert!(
        h.mock.calls_to("/api/send").is_empty(),
        "dry run never sends"
    );
    assert_eq!(h.core.automation_log(10).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn paused_automations_do_not_fire_and_resume_skips_backlog() {
    let h = harness().await;
    h.core.save_rule(ping_rule()).await.unwrap();
    h.core.set_automation_enabled(true).await.unwrap();
    prime(&h).await;
    h.core.set_automation_paused(true).await.unwrap();
    h.f.add_message(&Msg::text("Q1", DM_BOB, "919822222222", &ts_now(0), "ping"));
    h.core.watch_tick().await.unwrap();
    h.core.set_automation_paused(false).await.unwrap();
    h.core.watch_tick().await.unwrap();
    assert!(h.rec.named(events::AUTOMATION_FIRED).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn revoke_emits_db_changed() {
    let h = harness().await;
    h.f.add_message(&Msg::text(
        "R1",
        DM_BOB,
        "919822222222",
        &ts_now(-60),
        "typo",
    ));
    // Edit detection compares against the search index, so build it first.
    let j = h.core.ensure_index().await.unwrap().unwrap();
    wait_job(&h.core, j.id).await;
    prime(&h).await;
    h.f.add_message(
        &Msg::text("R1", DM_BOB, "919822222222", &ts_now(-60), "typo").deleted(&ts_now(0)),
    );
    h.core.watch_tick().await.unwrap();
    let ev = h.rec.named(events::DB_CHANGED);
    assert_eq!(ev.len(), 1, "{:?}", h.rec.events.lock());
    assert_eq!(ev[0]["chat_jids"], json!([DM_BOB]));
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_state_emitted_only_on_change() {
    let h = harness().await;
    h.core.bridge_tick().await;
    h.core.bridge_tick().await;
    assert_eq!(h.rec.named(events::BRIDGE_STATE).len(), 1);
    *h.mock.connection.lock() = json!({"state": "pairing", "connected": false, "paired": false,
        "message": "Scan", "qrCode": "2@abc"});
    let s = h.core.bridge_tick().await;
    assert_eq!(s.state, BridgeState::Pairing);
    assert!(
        s.qr_svg
            .as_deref()
            .unwrap()
            .starts_with("data:image/svg+xml")
    );
    let ev = h.rec.named(events::BRIDGE_STATE);
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[1]["state"], "pairing");
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_unreachable_is_a_state_not_an_error() {
    let h = harness_down().await;
    let s = h.core.bridge_tick().await;
    assert_eq!(s.state, BridgeState::Unreachable);
    assert_eq!(h.rec.named(events::BRIDGE_STATE).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn job_progress_and_reconcile_and_cleanup() {
    let h = harness().await;
    let j = h.core.ensure_index().await.unwrap().unwrap();
    let done = wait_job(&h.core, j.id).await;
    assert_eq!(done.status, JobStatus::Succeeded);
    // The finished event is emitted just after the status flips; allow for a busy machine.
    let mut fin = h.rec.named(events::JOB_FINISHED);
    for _ in 0..100 {
        if !fin.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        fin = h.rec.named(events::JOB_FINISHED);
    }
    assert_eq!(fin.last().unwrap()["id"], j.id);
    h.core.reconcile_tick().await.unwrap();
    assert_eq!(h.core.outbox_cleanup_tick().await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn background_loops_run_and_stop() {
    let h = harness().await;
    let token = tokio_util::sync::CancellationToken::new();
    wa_desk_lib::background::spawn_all(h.core.clone(), token.clone());
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    h.f.add_message(&Msg::text(
        "BG1",
        DM_ALICE,
        "919811111111",
        &ts_now(0),
        "loop",
    ));
    for _ in 0..40 {
        if !h.rec.named(events::NEW_MESSAGES).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(h.rec.named(events::NEW_MESSAGES).len(), 1);
    assert!(!h.rec.named(events::BRIDGE_STATE).is_empty());
    token.cancel();
}

// ---- transient failure reporting ------------------------------------------

mod health {
    use super::*;
    use std::time::{Duration, Instant};
    use wa_core::{AppError, ErrorKind};
    use wa_desk_lib::background::{TaskHealth, on_failure, on_success};

    const TITLE: &str = "Cannot read new messages from the bridge database";
    const SRC: &str = "app.watcher";

    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    /// Run `f` with a DEBUG-level capture and return everything logged.
    fn logs(f: impl FnOnce()) -> String {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(buf.clone())
            .finish();
        tracing::subscriber::with_default(sub, f);
        let bytes = buf.0.lock().unwrap().clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn timeout() -> AppError {
        AppError::timeout("database query exceeded its deadline").with_detail("interrupted")
    }

    #[tokio::test]
    async fn single_timeout_is_quiet_and_recovers_silently() {
        let h = harness().await;
        let mut th = TaskHealth::default();
        let out = logs(|| {
            on_failure(
                &h.core,
                "watcher",
                TITLE,
                &mut th,
                Instant::now(),
                &timeout(),
            );
            on_success(&h.core, "watcher", TITLE, &mut th);
        });
        assert!(!out.contains("WARN") && !out.contains("INFO"), "{out}");
        assert!(out.contains("DEBUG"), "{out}");
        assert!(h.core.issues().list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn persistent_timeouts_raise_one_issue_and_recovery_resolves_it() {
        let h = harness().await;
        let mut th = TaskHealth::default();
        let t0 = Instant::now();
        // Many attempts but under 60 s: still nothing.
        let out = logs(|| {
            for i in 0..8 {
                let at = t0 + Duration::from_secs(i);
                on_failure(&h.core, "watcher", TITLE, &mut th, at, &timeout());
            }
        });
        assert!(!out.contains("WARN"), "{out}");
        assert!(h.core.issues().list().unwrap().is_empty());
        // Crossing 60 s with >= 5 attempts escalates exactly once.
        let out = logs(|| {
            for i in 0..3 {
                let at = t0 + Duration::from_secs(60 + i);
                on_failure(&h.core, "watcher", TITLE, &mut th, at, &timeout());
            }
        });
        assert_eq!(out.matches("WARN").count(), 1, "{out}");
        let issues = h.core.issues().list().unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].source, SRC);
        assert_eq!(issues[0].kind, ErrorKind::Timeout);
        assert_eq!(issues[0].status, IssueStatus::Open);
        // Recovery: INFO (an issue was raised) and the issue resolves.
        let out = logs(|| on_success(&h.core, "watcher", TITLE, &mut th));
        assert!(out.contains("INFO") && out.contains("recovered"), "{out}");
        assert_eq!(
            h.core.issues().list().unwrap()[0].status,
            IssueStatus::Resolved
        );
        // A later blip starts a fresh streak.
        let at = t0 + Duration::from_secs(500);
        let out = logs(|| on_failure(&h.core, "watcher", TITLE, &mut th, at, &timeout()));
        assert!(!out.contains("WARN"), "{out}");
    }

    #[tokio::test]
    async fn non_transient_errors_warn_and_raise_immediately() {
        let h = harness().await;
        let mut th = TaskHealth::default();
        let t0 = Instant::now();
        let e = AppError::internal("boom").with_detail("detail");
        let out = logs(|| {
            on_failure(&h.core, "watcher", TITLE, &mut th, t0, &e);
            let at = t0 + Duration::from_secs(1);
            on_failure(&h.core, "watcher", TITLE, &mut th, at, &e);
        });
        assert_eq!(out.matches("WARN").count(), 1, "{out}");
        let issues = h.core.issues().list().unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, ErrorKind::Internal);
        assert_eq!(issues[0].count, 1);
        // Recovery resolves it too.
        on_success(&h.core, "watcher", TITLE, &mut th);
        assert_eq!(
            h.core.issues().list().unwrap()[0].status,
            IssueStatus::Resolved
        );
    }

    #[tokio::test]
    async fn busy_is_transient_too() {
        let h = harness().await;
        let mut th = TaskHealth::default();
        let t = "Search index could not be updated";
        let e = AppError::busy("locked");
        let out = logs(|| on_failure(&h.core, "index", t, &mut th, Instant::now(), &e));
        assert!(!out.contains("WARN"), "{out}");
        assert!(h.core.issues().list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn startup_cleanup_resolves_stale_timeout_issues() {
        let h = harness().await;
        let s = h.core.issues();
        let idx = "Search index could not be updated";
        let names = "Chat names could not be refreshed";
        let t = ErrorKind::Timeout;
        s.record_issue(t, "app.watcher", TITLE, Some("interrupted"))
            .unwrap();
        s.record_issue(t, "app.index", idx, None).unwrap();
        s.record_issue(ErrorKind::Internal, "app.watcher", "Different", None)
            .unwrap();
        s.record_issue(t, "app.chat_names", names, None).unwrap();
        let mut w = TaskHealth::default();
        let mut i = TaskHealth::default();
        on_success(&h.core, "watcher", TITLE, &mut w);
        on_success(&h.core, "index", idx, &mut i);
        let open: Vec<_> = s
            .list()
            .unwrap()
            .into_iter()
            .filter(|x| x.status == IssueStatus::Open)
            .map(|x| format!("{}/{:?}", x.source, x.kind))
            .collect();
        assert_eq!(open.len(), 2, "{open:?}");
        assert!(open.contains(&"app.watcher/Internal".to_string()));
        assert!(open.contains(&"app.chat_names/Timeout".to_string()));
    }
}
