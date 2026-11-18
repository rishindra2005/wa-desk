//! Clipboard (copy image out, paste files in) and app zoom commands.

mod common;

use common::*;
use wa_core::ErrorKind;
use wa_core::types::*;
use wa_testkit::ids::*;

fn img_bytes() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([200, 30, 30]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
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

fn b64(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[tokio::test(flavor = "multi_thread")]
async fn copy_image_decodes_and_hands_pixels_to_the_platform() {
    let h = harness().await;
    queue_group_image(&h, &img_bytes());
    h.core
        .copy_image(GROUP_CLASS.into(), M_GROUP_IMG.into())
        .await
        .unwrap();
    assert_eq!(*h.platform.copied.lock(), vec![(64, 48)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn copy_image_refuses_non_images_and_bad_ids() {
    let h = harness().await;
    let pdf = h
        .core
        .copy_image(DM_ALICE.into(), M_ALICE_PDF.into())
        .await
        .unwrap_err();
    assert_eq!(pdf.kind, ErrorKind::Invalid);
    let text = h
        .core
        .copy_image(DM_ALICE.into(), M_ALICE_1.into())
        .await
        .unwrap_err();
    assert_eq!(text.kind, ErrorKind::NotFound);
    let evil = h
        .core
        .copy_image(DM_ALICE.into(), "../../etc/passwd".into())
        .await
        .unwrap_err();
    assert!(matches!(
        evil.kind,
        ErrorKind::Invalid | ErrorKind::NotFound
    ));
    assert!(h.platform.copied.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn copy_image_of_corrupt_file_is_invalid() {
    let h = harness().await;
    queue_group_image(&h, b"definitely not an image");
    let e = h
        .core
        .copy_image(GROUP_CLASS.into(), M_GROUP_IMG.into())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert!(h.platform.copied.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn pasted_image_is_staged_in_outbox_and_can_be_sent() {
    let h = harness().await;
    let data = img_bytes();
    let path = h
        .core
        .stage_pasted_file(Some("image.png".into()), None, b64(&data))
        .await
        .unwrap();
    let staged = std::path::PathBuf::from(&path);
    assert!(staged.starts_with(&h.f.paths.outbox), "{staged:?}");
    assert_eq!(std::fs::read(&staged).unwrap(), data);
    // A nameless screenshot gets a name from its MIME type.
    let p2 = h
        .core
        .stage_pasted_file(None, Some("image/png".into()), b64(&data))
        .await
        .unwrap();
    let name = std::path::Path::new(&p2)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with("pasted-") && name.ends_with(".png"),
        "{name}"
    );
    // And it goes through the normal send path.
    h.core
        .send_message(SendReq {
            recipient: DM_ALICE.into(),
            attachment: Some(path),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(h.mock.calls_to("/api/send").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn pasted_file_refusals() {
    let h = harness().await;
    let k = |name: &str, data: String| {
        let core = h.core.clone();
        let name = name.to_string();
        async move {
            core.stage_pasted_file(Some(name), None, data)
                .await
                .unwrap_err()
                .kind
        }
    };
    assert_eq!(k("messages.db", b64(b"x")).await, ErrorKind::Forbidden);
    assert_eq!(k(".bridge-token", b64(b"x")).await, ErrorKind::Forbidden);
    assert_eq!(
        k("a.png", "***not base64***".into()).await,
        ErrorKind::Invalid
    );
    assert_eq!(k("a.png", String::new()).await, ErrorKind::Invalid);
    let huge = "A".repeat(wa_bridge::outbox::MAX_PASTE_BYTES / 3 * 4 + 100);
    assert_eq!(k("a.png", huge).await, ErrorKind::Invalid);
    let left = std::fs::read_dir(&h.f.paths.outbox)
        .map(|r| r.count())
        .unwrap_or(0);
    assert_eq!(left, 0, "nothing left behind");
}

#[tokio::test(flavor = "multi_thread")]
async fn pasting_is_blocked_in_read_only_mode() {
    let h = harness().await;
    h.core.set_read_only(true);
    let e = h
        .core
        .stage_pasted_file(Some("a.png".into()), None, b64(b"x"))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Forbidden);
    let left = std::fs::read_dir(&h.f.paths.outbox)
        .map(|r| r.count())
        .unwrap_or(0);
    assert_eq!(left, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_zoom_clamps_persists_and_applies() {
    let h = harness().await;
    assert_eq!(h.core.set_zoom(120).await.unwrap(), 120);
    assert_eq!(h.core.get_settings().await.unwrap().zoom_percent, 120);
    assert_eq!(h.core.set_zoom(10).await.unwrap(), 50);
    assert_eq!(h.core.set_zoom(900).await.unwrap(), 200);
    assert_eq!(*h.platform.zooms.lock(), vec![120, 50, 200]);
    // A stale settings form can't undo (or corrupt) the zoom.
    let mut s = h.core.get_settings().await.unwrap();
    s.zoom_percent = 5000;
    s.confirm_sends = false;
    let saved = h.core.set_settings(s).await.unwrap();
    assert_eq!(saved.zoom_percent, 200);
    assert!(!h.core.get_settings().await.unwrap().confirm_sends);
    // Startup re-applies the saved level.
    h.platform.zooms.lock().clear();
    h.core.apply_saved_zoom().await.unwrap();
    assert_eq!(*h.platform.zooms.lock(), vec![200]);
}
