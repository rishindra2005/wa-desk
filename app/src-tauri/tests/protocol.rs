//! `wa-media://` handler: status codes, Range, scope checks, content types.

mod common;

use common::*;
use tauri::http::{Request, Response, StatusCode, header};
use wa_desk_lib::protocol::{FULL_LIMIT, handle};
use wa_desk_lib::range::MAX_CHUNK;
use wa_testkit::ids::*;

fn m_url(chat: &str, id: &str) -> String {
    wa_db::media::media_url("m", chat, id)
}

async fn get(h: &H, url: &str, range: Option<&str>) -> Response<Vec<u8>> {
    let mut b = Request::builder().method("GET").uri(url);
    if let Some(r) = range {
        b = b.header(header::RANGE, r);
    }
    handle(&h.core, &b.body(Vec::new()).unwrap()).await
}

fn hdr(r: &Response<Vec<u8>>, h: header::HeaderName) -> &str {
    r.headers()
        .get(h)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

#[tokio::test(flavor = "multi_thread")]
async fn serves_original_with_type_and_ranges() {
    let h = harness().await;
    let r = get(&h, &m_url(DM_ALICE, M_ALICE_PDF), None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.body(), PDF_BYTES);
    assert_eq!(hdr(&r, header::CONTENT_TYPE), "application/pdf");
    assert_eq!(hdr(&r, header::ACCEPT_RANGES), "bytes");
    assert!(hdr(&r, header::CACHE_CONTROL).contains("max-age=3600"));
    assert_eq!(hdr(&r, header::CONTENT_LENGTH), PDF_BYTES.len().to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn range_206_and_416() {
    let h = harness().await;
    let url = m_url(DM_ALICE, M_ALICE_PDF);
    let r = get(&h, &url, Some("bytes=1-4")).await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.body(), &PDF_BYTES[1..5]);
    assert_eq!(
        hdr(&r, header::CONTENT_RANGE),
        format!("bytes 1-4/{}", PDF_BYTES.len())
    );
    let r = get(&h, &url, Some("bytes=9999-")).await;
    assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        hdr(&r, header::CONTENT_RANGE),
        format!("bytes */{}", PDF_BYTES.len())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn head_has_length_but_no_body() {
    let h = harness().await;
    let req = Request::builder()
        .method("HEAD")
        .uri(m_url(DM_ALICE, M_ALICE_PDF))
        .body(Vec::new())
        .unwrap();
    let r = handle(&h.core, &req).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.body().is_empty());
    assert_eq!(hdr(&r, header::CONTENT_LENGTH), PDF_BYTES.len().to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_400_404_405() {
    let h = harness().await;
    assert_eq!(
        get(&h, "wa-media://localhost/zz/a/b", None).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&h, "wa-media://localhost/m/a%40b/..", None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    // Not downloaded yet.
    assert_eq!(
        get(&h, &m_url(GROUP_CLASS, M_GROUP_IMG), None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&h, &m_url(DM_ALICE, "UNKNOWN"), None).await.status(),
        StatusCode::NOT_FOUND
    );
    let req = Request::builder()
        .method("POST")
        .uri(m_url(DM_ALICE, M_ALICE_PDF))
        .body(Vec::new())
        .unwrap();
    assert_eq!(
        handle(&h.core, &req).await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

fn x_url(p: &std::path::Path) -> String {
    wa_desk_lib::media_url::export_url(p)
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(unix)]
async fn export_urls_are_scoped() {
    let h = harness().await;
    let ok = h.f.paths.exports.join("out 50%.txt");
    std::fs::write(&ok, b"hello").unwrap();
    let r = get(&h, &x_url(&ok), None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.body(), b"hello");
    assert_eq!(hdr(&r, header::CONTENT_TYPE), "text/plain; charset=utf-8");
    for (p, want) in [
        (h.f.paths.token_file.clone(), StatusCode::FORBIDDEN),
        (h.f.paths.messages_db.clone(), StatusCode::FORBIDDEN),
        (
            h.f.paths
                .bridge_store
                .join(DM_ALICE)
                .join("document_20261115_100200_A3PDF"),
            StatusCode::FORBIDDEN,
        ),
        (h.f.paths.exports.join("missing.pdf"), StatusCode::NOT_FOUND),
    ] {
        let r = get(&h, &x_url(&p), None).await;
        assert_eq!(r.status(), want, "{p:?}");
        assert!(!String::from_utf8_lossy(r.body()).contains(wa_testkit::TOKEN));
    }
    // A secret symlinked into exports is still refused.
    let link = h.f.paths.exports.join("innocent.txt");
    std::os::unix::fs::symlink(&h.f.paths.token_file, &link).unwrap();
    assert_eq!(
        get(&h, &x_url(&link), None).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn active_content_is_served_as_text() {
    let h = harness().await;
    let p = h.f.paths.exports.join("page.html");
    std::fs::write(&p, b"<script>alert(1)</script>").unwrap();
    let r = get(&h, &x_url(&p), None).await;
    assert_eq!(hdr(&r, header::CONTENT_TYPE), "text/plain; charset=utf-8");
    assert!(hdr(&r, header::CONTENT_SECURITY_POLICY).contains("sandbox"));
    assert_eq!(hdr(&r, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
}

#[tokio::test(flavor = "multi_thread")]
async fn large_files_are_chunked() {
    let h = harness().await;
    let p = h.f.paths.exports.join("big.mp4");
    let f = std::fs::File::create(&p).unwrap();
    f.set_len(FULL_LIMIT + 1024).unwrap();
    let r = get(&h, &x_url(&p), None).await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.body().len() as u64, MAX_CHUNK);
    assert_eq!(
        hdr(&r, header::CONTENT_RANGE),
        format!("bytes 0-{}/{}", MAX_CHUNK - 1, FULL_LIMIT + 1024)
    );
    assert_eq!(hdr(&r, header::CONTENT_TYPE), "video/mp4");
    // An explicit huge range is capped too.
    let r = get(&h, &x_url(&p), Some("bytes=100-999999999")).await;
    assert_eq!(r.body().len() as u64, MAX_CHUNK);
    assert_eq!(
        hdr(&r, header::CONTENT_RANGE),
        format!("bytes 100-{}/{}", 100 + MAX_CHUNK - 1, FULL_LIMIT + 1024)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn thumbnail_served_for_image_404_for_audio() {
    let h = harness().await;
    let img = image::RgbImage::from_pixel(600, 400, image::Rgb([10, 200, 10]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
    let data = buf.into_inner();
    let ts = "2026-11-15 09:30:00+05:30";
    h.f.add_message(
        &wa_testkit::Msg::text("IMG9", DM_ALICE, "919811111111", ts, "")
            .media("image", None, &data),
    );
    h.f.put_store_media(DM_ALICE, "image", "IMG9", ts, &data);
    let r = get(&h, &wa_db::media::media_url("t", DM_ALICE, "IMG9"), None).await;
    assert_eq!(
        r.status(),
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(r.body())
    );
    assert!(hdr(&r, header::CONTENT_TYPE).starts_with("image/"));
    let thumb = image::load_from_memory(r.body()).unwrap();
    assert!(thumb.width() <= 256 && thumb.height() <= 256);
    assert!(hdr(&r, header::CACHE_CONTROL).contains("max-age=604800"));

    h.f.add_message(
        &wa_testkit::Msg::text("AUD9", DM_ALICE, "919811111111", ts, "")
            .media("audio", None, b"OggS"),
    );
    h.f.put_store_media(DM_ALICE, "audio", "AUD9", ts, b"OggS");
    let r = get(&h, &wa_db::media::media_url("t", DM_ALICE, "AUD9"), None).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    // Original audio is served with its type.
    let r = get(&h, &m_url(DM_ALICE, "AUD9"), None).await;
    assert!(hdr(&r, header::CONTENT_TYPE).starts_with("audio/"));
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_url_404_for_non_office() {
    let h = harness().await;
    let r = get(
        &h,
        &wa_db::media::media_url("p", DM_ALICE, M_ALICE_PDF),
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn cors_for_app_origins_only() {
    let h = harness().await;
    let url = m_url(DM_ALICE, M_ALICE_PDF);
    for (origin, allowed) in [
        ("tauri://localhost", true),
        ("http://127.0.0.1:1420", true),
        ("https://evil.example", false),
    ] {
        let req = Request::builder()
            .uri(&url)
            .header(header::ORIGIN, origin)
            .body(Vec::new())
            .unwrap();
        let r = handle(&h.core, &req).await;
        let got = hdr(&r, header::ACCESS_CONTROL_ALLOW_ORIGIN);
        assert_eq!(got == origin, allowed, "{origin}");
    }
    let req = Request::builder()
        .method("OPTIONS")
        .uri(&url)
        .header(header::ORIGIN, "tauri://localhost")
        .body(Vec::new())
        .unwrap();
    let r = handle(&h.core, &req).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert!(hdr(&r, header::ACCESS_CONTROL_ALLOW_HEADERS).contains("Range"));
}
