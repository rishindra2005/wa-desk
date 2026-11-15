//! `WA_DESK_READ_ONLY` safety switch.

mod common;

use common::*;
use wa_core::ErrorKind;
use wa_core::types::*;
use wa_testkit::ids::*;

#[tokio::test(flavor = "multi_thread")]
async fn read_only_mode_refuses_bridge_writes() {
    let h = harness().await;
    h.core.set_read_only(true);
    let send = h
        .core
        .send_message(SendReq {
            recipient: DM_ALICE.into(),
            text: Some("x".into()),
            ..Default::default()
        })
        .await;
    assert_eq!(send.unwrap_err().kind, ErrorKind::Forbidden);
    let react = h
        .core
        .react(ReactReq {
            chat_jid: DM_ALICE.into(),
            message_id: M_ALICE_1.into(),
            emoji: "👍".into(),
        })
        .await;
    assert_eq!(react.unwrap_err().kind, ErrorKind::Forbidden);
    assert_eq!(
        h.core.mark_read(DM_ALICE.into()).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        h.core
            .ensure_media(GROUP_CLASS.into(), M_GROUP_IMG.into())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        h.core.reconnect().await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    // Reads still work.
    assert!(h.core.overview().await.is_ok());
    assert!(h.mock.calls.lock().is_empty());
}
