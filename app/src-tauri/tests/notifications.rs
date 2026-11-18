//! Desktop notifications are replaced per chat and withdrawn once the chat is
//! opened, read here or on the phone, or one of its notifications is clicked
//! (fake notification server from `common`).

mod common;

use common::*;
use serde_json::json;
use wa_core::types::*;
use wa_desk_lib::core::ServerEvent;
use wa_testkit::Msg;
use wa_testkit::ids::*;

async fn prime(h: &H) {
    assert_eq!(h.core.watch_tick().await.unwrap(), 0);
}

async fn alice_says(h: &H, id: &str, text: &str) {
    h.f.add_message(&Msg::text(id, DM_ALICE, "919811111111", &ts_now(0), text));
    h.core.watch_tick().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn second_message_replaces_the_chat_notification() {
    let h = harness().await;
    prime(&h).await;
    alice_says(&h, "R1", "one").await;
    alice_says(&h, "R2", "two").await;
    assert_eq!(*h.platform.shown.lock(), vec![(1, 0), (1, 1)]);
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes[1], ("Alice (2 messages)".into(), "two".into()));
    // Several in one poll count at once.
    h.f.add_message(&Msg::text("R3", DM_ALICE, "919811111111", &ts_now(0), "3"));
    h.f.add_message(&Msg::text("R4", DM_ALICE, "919811111111", &ts_now(0), "4"));
    h.core.watch_tick().await.unwrap();
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes[2], ("Alice (4 messages)".into(), "4".into()));
    assert!(h.platform.closed.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_the_chat_withdraws_its_notifications() {
    let h = harness().await;
    prime(&h).await;
    alice_says(&h, "O1", "hello").await;
    h.f.add_message(&Msg::text("O2", DM_BOB, "919822222222", &ts_now(0), "yo"));
    h.core.watch_tick().await.unwrap();
    // The UI calls mark_read when a chat is opened.
    h.core.mark_read(DM_ALICE.into()).await.unwrap();
    assert_eq!(*h.platform.closed.lock(), vec![1], "only Alice's");
    assert_eq!(h.mock.calls_to("/api/mark-read").len(), 1);
    // A later message starts a new notification rather than replacing a closed one.
    alice_says(&h, "O3", "again").await;
    assert_eq!(h.platform.shown.lock().last(), Some(&(3, 0)));
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_mode_still_withdraws() {
    let h = harness().await;
    prime(&h).await;
    alice_says(&h, "W1", "hello").await;
    h.core.set_read_only(true);
    assert_eq!(
        h.core.mark_read(DM_ALICE.into()).await.unwrap_err().kind,
        wa_core::ErrorKind::Forbidden
    );
    assert_eq!(*h.platform.closed.lock(), vec![1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn read_on_the_phone_withdraws() {
    let h = harness().await;
    prime(&h).await;
    alice_says(&h, "P1", "hello").await;
    h.f.add_message(&Msg::text("P2", DM_BOB, "919822222222", &ts_now(0), "yo"));
    h.core.watch_tick().await.unwrap();
    // The phone moves Alice's read marker past her last message.
    h.f.set_last_read(DM_ALICE, Some(&ts_now(5)));
    h.core.watch_tick().await.unwrap();
    assert_eq!(*h.platform.closed.lock(), vec![1]);
    // Nothing more to close on later polls.
    h.core.watch_tick().await.unwrap();
    assert_eq!(h.platform.closed.lock().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn marker_moving_but_still_unread_keeps_the_notification() {
    let h = harness().await;
    prime(&h).await;
    alice_says(&h, "K1", "hello").await;
    h.f.set_last_read(DM_ALICE, Some(&ts_now(-600)));
    h.core.watch_tick().await.unwrap();
    assert!(h.platform.closed.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn click_opens_the_chat_and_clears_its_other_notifications() {
    let h = harness().await;
    prime(&h).await;
    alice_says(&h, "C1", "hello").await;
    h.f.add_call(
        "CALL1",
        DM_ALICE,
        DM_ALICE,
        &ts_now(0),
        false,
        "voice",
        "missed",
        None,
    );
    h.core.watch_tick().await.unwrap();
    assert_eq!(h.platform.shown.lock().len(), 2);
    h.core
        .on_notification_event(ServerEvent::Activated(1))
        .await;
    assert_eq!(*h.platform.shown_main.lock(), 1);
    let mut closed = h.platform.closed.lock().clone();
    closed.sort();
    assert_eq!(closed, vec![1, 2]);
    assert_eq!(
        h.rec.named(events::NOTIFICATION_OPEN),
        vec![json!({ "chat_jid": DM_ALICE })]
    );
    // Another application's notification: ignored.
    h.core
        .on_notification_event(ServerEvent::Activated(99))
        .await;
    assert_eq!(*h.platform.shown_main.lock(), 1);
    assert_eq!(h.rec.named(events::NOTIFICATION_OPEN).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn dismissed_by_the_user_is_forgotten() {
    let h = harness().await;
    prime(&h).await;
    alice_says(&h, "D1", "hello").await;
    h.core.on_notification_event(ServerEvent::Closed(1)).await;
    h.core.mark_read(DM_ALICE.into()).await.unwrap();
    assert!(h.platform.closed.lock().is_empty(), "nothing left to close");
    assert_eq!(h.rec.named(events::NOTIFICATION_OPEN).len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn ringing_call_notification_closes_when_the_call_ends() {
    let h = harness().await;
    prime(&h).await;
    let call = |result: &str| {
        h.f.add_call(
            "RING1",
            DM_BOB,
            DM_BOB,
            &ts_now(0),
            false,
            "voice",
            result,
            None,
        )
    };
    call("in_progress");
    h.core.watch_tick().await.unwrap();
    assert_eq!(h.platform.shown.lock().len(), 1);
    call("ended");
    h.core.watch_tick().await.unwrap();
    assert_eq!(*h.platform.closed.lock(), vec![1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn burst_across_many_chats_is_one_summary_closed_with_its_last_chat() {
    let h = harness().await;
    prime(&h).await;
    for (i, (chat, sender)) in [
        (DM_ALICE, "919811111111"),
        (DM_BOB, "919822222222"),
        (DM_LID, "222222222222222"),
        (GROUP_CLASS, "919811111111"),
    ]
    .into_iter()
    .enumerate()
    {
        let text = if chat == GROUP_CLASS {
            "@919900000000 look".to_string()
        } else {
            format!("m{i}")
        };
        h.f.add_message(&Msg::text(
            &format!("S{i}"),
            chat,
            sender,
            &ts_now(0),
            &text,
        ));
    }
    h.core.watch_tick().await.unwrap();
    let notes = h.platform.notes.lock().clone();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].1, "4 new messages in 4 chats");
    for chat in [DM_ALICE, DM_BOB, DM_LID] {
        h.core.mark_read(chat.into()).await.ok();
    }
    assert!(h.platform.closed.lock().is_empty());
    h.core.mark_read(GROUP_CLASS.into()).await.ok();
    assert_eq!(*h.platform.closed.lock(), vec![1]);
}
