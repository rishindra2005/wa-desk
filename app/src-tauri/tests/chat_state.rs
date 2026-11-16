//! Archive / pin / mute commands, the pin limit, events, read-only and resync.

mod common;

use common::*;
use rusqlite::params;
use wa_core::ErrorKind;
use wa_core::time::now_epoch;
use wa_core::types::*;
use wa_testkit::ids::*;

fn put_state(h: &H, jid: &str, archived: i64, pinned: i64, muted: Option<i64>) {
    let c = h.f.bridge_conn();
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS chat_state(jid TEXT PRIMARY KEY, archived INTEGER NOT NULL DEFAULT 0, \
         pinned INTEGER NOT NULL DEFAULT 0, muted_until INTEGER, updated_at TIMESTAMP)",
    )
    .unwrap();
    c.execute(
        "INSERT OR REPLACE INTO chat_state (jid, archived, pinned, muted_until) VALUES (?1,?2,?3,?4)",
        params![jid, archived, pinned, muted],
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_pin_mute_call_bridge_and_emit() {
    let h = harness().await;
    h.core.set_archived(DM_ALICE.into(), true).await.unwrap();
    let c = h.mock.calls_to("/api/chat/archive");
    assert_eq!(c.len(), 1);
    assert_eq!(
        (c[0]["jid"].as_str(), c[0]["archived"].as_bool()),
        (Some(DM_ALICE), Some(true))
    );
    h.core.set_pinned(DM_BOB.into(), true).await.unwrap();
    assert_eq!(h.mock.calls_to("/api/chat/pin")[0]["pinned"], true);
    let until = now_epoch() + 8 * 3600;
    h.core.set_muted(DM_LID.into(), Some(until)).await.unwrap();
    assert_eq!(h.mock.calls_to("/api/chat/mute")[0]["until"], until);
    h.core.set_muted(DM_LID.into(), None).await.unwrap();
    assert!(h.mock.calls_to("/api/chat/mute")[1]["until"].is_null());
    let ev = h.rec.named("db:changed");
    assert_eq!(ev.len(), 4);
    assert_eq!(ev[0]["chat_jids"][0], DM_ALICE);
    assert_eq!(ev[1]["chat_jids"][0], DM_BOB);
    // Invalid input never reaches the bridge.
    assert_eq!(
        h.core
            .set_muted(DM_ALICE.into(), Some(5))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Invalid
    );
    assert_eq!(
        h.core
            .set_archived(" ".into(), true)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Invalid
    );
    assert_eq!(h.mock.calls_to("/api/chat/mute").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn fourth_pin_is_rejected() {
    let h = harness().await;
    for j in [DM_ALICE, DM_BOB, DM_LID] {
        put_state(&h, j, 0, 1, None);
    }
    let e = h
        .core
        .set_pinned(GROUP_CLASS.into(), true)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert_eq!(e.message, "You can only pin up to 3 chats");
    assert!(h.mock.calls_to("/api/chat/pin").is_empty());
    // Re-pinning an already pinned chat and unpinning are fine.
    h.core.set_pinned(DM_ALICE.into(), true).await.unwrap();
    h.core.set_pinned(DM_BOB.into(), false).await.unwrap();
    assert_eq!(h.mock.calls_to("/api/chat/pin").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_count_and_list() {
    let h = harness().await;
    assert_eq!(h.core.archive_count().await.unwrap(), 0);
    put_state(&h, DM_BOB, 1, 0, None);
    put_state(&h, DM_UNKNOWN, 1, 0, Some(-1));
    assert_eq!(h.core.archive_count().await.unwrap(), 2);
    let p = h
        .core
        .list_chats(ChatQuery {
            archived: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(p.items.len(), 2);
    assert!(
        p.items
            .iter()
            .any(|c| c.jid == DM_UNKNOWN && c.muted_until == Some(-1))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_blocks_chat_state_writes() {
    let h = harness().await;
    h.core.set_read_only(true);
    for e in [
        h.core.set_archived(DM_ALICE.into(), true).await,
        h.core.set_pinned(DM_ALICE.into(), true).await,
        h.core.set_muted(DM_ALICE.into(), Some(-1)).await,
    ] {
        assert_eq!(e.unwrap_err().kind, ErrorKind::Forbidden);
    }
    assert_eq!(
        h.core.resync_chat_state().await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    // The startup tick treats it as skipped; reads still work.
    h.core.resync_state_tick().await.unwrap();
    assert!(h.core.archive_count().await.is_ok());
    assert!(h.mock.calls.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn resync_emits_only_when_changed_and_tolerates_down_bridge() {
    let h = harness().await;
    assert_eq!(h.core.resync_chat_state().await.unwrap(), 0);
    assert!(h.rec.named("db:changed").is_empty());
    *h.mock.resync_changed.lock() = 5;
    assert_eq!(h.core.resync_chat_state().await.unwrap(), 5);
    let ev = h.rec.named("db:changed");
    assert_eq!(ev.len(), 1);
    assert!(!ev[0]["chat_jids"].as_array().unwrap().is_empty());
    *h.mock.meta_down.lock() = true;
    assert_eq!(
        h.core.resync_chat_state().await.unwrap_err().kind,
        ErrorKind::BridgeDown
    );
    h.core.resync_state_tick().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn watcher_tick_emits_db_changed_for_state_changes() {
    let h = harness().await;
    put_state(&h, DM_BOB, 1, 0, None);
    h.core.watch_tick().await.unwrap();
    let before = h.rec.named("db:changed").len();
    put_state(&h, DM_BOB, 0, 0, None);
    h.core.watch_tick().await.unwrap();
    let ev = h.rec.named("db:changed");
    assert_eq!(ev.len(), before + 1);
    let jids = ev.last().unwrap()["chat_jids"].as_array().unwrap().clone();
    assert!(jids.iter().any(|j| j == DM_BOB));
}
