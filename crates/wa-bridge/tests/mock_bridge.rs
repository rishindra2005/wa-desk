//! BridgeClient against an axum mock that enforces the real bridge's Host + token rules.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use parking_lot::Mutex;
use serde_json::{Value, json};
use wa_bridge::{BridgeClient, Timeouts};
use wa_core::ErrorKind;
use wa_core::traits::{BridgeApi, BridgeSend};
use wa_core::types::BridgeState;

const TOKEN_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOKEN_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Clone)]
struct Mock {
    allowed_host: Arc<Mutex<String>>,
    token: Arc<Mutex<String>>,
    unauthorized: Arc<AtomicUsize>,
    connection: Arc<Mutex<Value>>,
    last_body: Arc<Mutex<Value>>,
}

async fn guard(State(m): State<Mock>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host != *m.allowed_host.lock() {
        return (StatusCode::FORBIDDEN, "Forbidden: host not allowed\n").into_response();
    }
    let want = format!("Bearer {}", m.token.lock());
    let got = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if got != want {
        m.unauthorized.fetch_add(1, Ordering::SeqCst);
        let mut h = HeaderMap::new();
        h.insert(
            header::WWW_AUTHENTICATE,
            "Bearer realm=\"whatsapp-bridge\"".parse().unwrap(),
        );
        return (StatusCode::UNAUTHORIZED, h, "Unauthorized\n").into_response();
    }
    next.run(req).await
}

async fn connection(State(m): State<Mock>) -> axum::Json<Value> {
    axum::Json(m.connection.lock().clone())
}

async fn send(State(m): State<Mock>, axum::Json(body): axum::Json<Value>) -> Response {
    *m.last_body.lock() = body.clone();
    match body["message"].as_str().unwrap_or_default() {
        "plain500" => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database exploded near {TOKEN_A}\n"),
        )
            .into_response(),
        "json403" => (
            StatusCode::FORBIDDEN,
            axum::Json(
                json!({"success": false, "message": "media_path rejected: not under outbox"}),
            ),
        )
            .into_response(),
        "soft" => axum::Json(json!({"success": false, "message": "Not connected"})).into_response(),
        "huge" => (StatusCode::BAD_GATEWAY, "z".repeat(5000)).into_response(),
        _ => axum::Json(json!({"success": true, "message": "Message sent to x"})).into_response(),
    }
}

async fn download(axum::Json(body): axum::Json<Value>) -> Response {
    match body["message_id"].as_str().unwrap_or_default() {
        "gone" => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"success": false, "message": "media expired"})),
        )
            .into_response(),
        "down" => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"success": false, "message": "WhatsApp client is not connected."})),
        )
            .into_response(),
        "slow" => {
            tokio::time::sleep(Duration::from_secs(3)).await;
            axum::Json(json!({"success": true})).into_response()
        }
        id => axum::Json(json!({
            "success": true, "message": "ok", "filename": format!("{id}.jpg"),
            "path": format!("/store/x/{id}.jpg"), "no_retry_seen": body["no_retry"]
        }))
        .into_response(),
    }
}

async fn ok_json(State(m): State<Mock>, axum::Json(body): axum::Json<Value>) -> Response {
    *m.last_body.lock() = body;
    axum::Json(json!({"success": true, "message": "done", "ok": true})).into_response()
}

#[derive(serde::Deserialize)]
struct JidQ {
    jid: String,
    full: Option<String>,
}

async fn profile(axum::extract::Query(q): axum::extract::Query<JidQ>) -> Response {
    match q.jid.as_str() {
        "919811111111@s.whatsapp.net" => axum::Json(json!({
            "jid": q.jid, "push_name": "Ally", "verified_name": null, "about": "Busy",
            "about_set_at": null, "picture_id": "123", "devices": [0, 5],
            "business": {"description": "", "category": "Shop, Food", "email": "a@b.c",
                         "website": "", "address": "Main St"}
        }))
        .into_response(),
        "down@s.whatsapp.net" => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"success": false, "message": "not connected"})),
        )
            .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({"success": false, "message": "unknown user"})),
        )
            .into_response(),
    }
}

async fn group(axum::extract::Query(q): axum::extract::Query<JidQ>) -> Response {
    if !q.jid.ends_with("@g.us") {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"success": false, "message": "not a group"})),
        )
            .into_response();
    }
    axum::Json(json!({
        "jid": q.jid, "name": "Club", "topic": "", "created_at": "2026-09-28T05:40:20Z",
        "creator": "9190@s.whatsapp.net", "is_parent": true, "linked_parent_jid": null,
        "announce": true, "locked": false,
        "participants": [{"jid": "2235@lid", "phone_jid": "9190@s.whatsapp.net", "lid": "2235@lid",
                          "is_admin": true, "is_super_admin": true, "display_name": null}],
        "sub_groups": [{"jid": "1@g.us", "name": "A"}]
    }))
    .into_response()
}

async fn newsletter(axum::extract::Query(q): axum::extract::Query<JidQ>) -> Response {
    axum::Json(
        json!({"jid": q.jid, "name": "TFI", "description": "d", "subscribers": 23101,
                      "verified": false, "picture_id": null}),
    )
    .into_response()
}

async fn avatar(axum::extract::Query(q): axum::extract::Query<JidQ>) -> Response {
    match q.jid.as_str() {
        "none@g.us" => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({"success": false, "message": "no picture"})),
        )
            .into_response(),
        "slow@g.us" => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            StatusCode::OK.into_response()
        }
        j => axum::Json(json!({
            "path": format!("/store/avatars/{j}{}.jpg", if q.full.as_deref() == Some("1") {"-full"} else {""}),
            "picture_id": "9", "cached": true
        }))
        .into_response(),
    }
}

async fn refresh() -> Response {
    axum::Json(
        json!({"success": true, "groups": 162, "communities": 21, "channels": 2,
                      "renamed": 4, "cached": false}),
    )
    .into_response()
}

async fn health() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(json!({"status": "disconnected", "connected": false})),
    )
        .into_response()
}

struct Harness {
    mock: Mock,
    port: u16,
    dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mock = Mock {
            allowed_host: Arc::new(Mutex::new(format!("127.0.0.1:{port}"))),
            token: Arc::new(Mutex::new(TOKEN_A.into())),
            unauthorized: Arc::new(AtomicUsize::new(0)),
            connection: Arc::new(Mutex::new(json!({
                "state": "connected", "connected": true, "paired": true, "message": "Connected",
                "qrCode": null, "qrExpiresAt": null, "updatedAt": "2026-11-16T10:00:00Z", "lastError": null
            }))),
            last_body: Arc::new(Mutex::new(Value::Null)),
        };
        let app = Router::new()
            .route("/api/connection", get(connection))
            .route("/api/connection/reconnect", post(connection))
            .route("/api/send", post(send))
            .route("/api/download", post(download))
            .route("/api/react", post(ok_json))
            .route("/api/mark-read", post(ok_json))
            .route("/api/health", get(health))
            .route("/api/profile", get(profile))
            .route("/api/group", get(group))
            .route("/api/newsletter", get(newsletter))
            .route("/api/avatar", get(avatar))
            .route("/api/chats/refresh", post(refresh))
            .layer(middleware::from_fn_with_state(mock.clone(), guard))
            .with_state(mock.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".bridge-token"), format!("{TOKEN_A}\n")).unwrap();
        Self { mock, port, dir }
    }

    fn client_for(&self, host: &str, t: Timeouts) -> BridgeClient {
        BridgeClient::with_timeouts(
            &format!("http://{host}:{}", self.port),
            &self.dir.path().join(".bridge-token"),
            t,
        )
        .unwrap()
    }

    fn client(&self) -> BridgeClient {
        self.client_for("127.0.0.1", Timeouts::default())
    }

    fn write_token(&self, t: &str) {
        std::fs::write(self.dir.path().join(".bridge-token"), t).unwrap();
    }
}

#[tokio::test]
async fn connected_state_roundtrip() {
    let h = Harness::start().await;
    let s = h.client().connection().await;
    assert_eq!(s.state, BridgeState::Connected);
    assert!(s.connected && s.paired && s.qr_svg.is_none());
}

#[tokio::test]
async fn pairing_qr_becomes_svg_data_url() {
    let h = Harness::start().await;
    *h.mock.connection.lock() = json!({
        "state": "pairing", "connected": false, "paired": false, "message": "Scan the QR",
        "qrCode": "2@XYZ,abc,def==", "qrExpiresAt": "2026-11-16T10:00:20Z",
        "updatedAt": "2026-11-16T10:00:00Z", "lastError": null
    });
    let s = h.client().connection().await;
    assert_eq!(s.state, BridgeState::Pairing);
    let svg = s.qr_svg.expect("qr");
    assert!(svg.starts_with("data:image/svg+xml;base64,"));
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(svg.trim_start_matches("data:image/svg+xml;base64,"))
        .unwrap();
    assert!(String::from_utf8(raw).unwrap().contains("<svg"));
    assert!(s.qr_expires_at.is_some());
}

#[tokio::test]
async fn wrong_host_is_forbidden() {
    let h = Harness::start().await;
    *h.mock.allowed_host.lock() = format!("localhost:{}", h.port);
    let e = h.client().reconnect().await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Forbidden);
    assert!(e.message.contains("host not allowed"));
    // connection() never errors; reports Error state.
    let s = h.client().connection().await;
    assert_eq!(s.state, BridgeState::Error);
}

#[tokio::test]
async fn localhost_host_header_is_exact() {
    let h = Harness::start().await;
    *h.mock.allowed_host.lock() = format!("localhost:{}", h.port);
    let c = h.client_for("localhost", Timeouts::default());
    assert_eq!(c.connection().await.state, BridgeState::Connected);
}

#[tokio::test]
async fn token_rotation_rereads_file_once() {
    let h = Harness::start().await;
    let c = h.client();
    assert!(c.reconnect().await.is_ok());
    // Bridge regenerates its token and rewrites the file (CRLF this time).
    *h.mock.token.lock() = TOKEN_B.into();
    h.write_token(&format!("{TOKEN_B}\r\n"));
    let before = h.mock.unauthorized.load(Ordering::SeqCst);
    assert!(c.reconnect().await.is_ok(), "retried with fresh token");
    assert_eq!(h.mock.unauthorized.load(Ordering::SeqCst), before + 1);
    // Cached now: no more 401s.
    assert!(c.reconnect().await.is_ok());
    assert_eq!(h.mock.unauthorized.load(Ordering::SeqCst), before + 1);
}

#[tokio::test]
async fn wrong_token_is_auth_error_after_one_retry() {
    let h = Harness::start().await;
    h.write_token("cccccccccccccccccccccccccccccccc\n");
    let e = h.client().reconnect().await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    assert_eq!(h.mock.unauthorized.load(Ordering::SeqCst), 2);
    assert!(!format!("{e:?}").contains("cccccccc"));
}

#[tokio::test]
async fn plain_text_500_is_surfaced_and_redacted() {
    let h = Harness::start().await;
    let e = h
        .client()
        .send(BridgeSend {
            recipient: "x@s.whatsapp.net".into(),
            message: Some("plain500".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::External);
    assert!(e.message.contains("database exploded"), "{e:?}");
    let all = format!("{e:?}");
    assert!(!all.contains(TOKEN_A), "token leaked: {all}");
}

#[tokio::test]
async fn huge_error_body_is_truncated() {
    let h = Harness::start().await;
    let e = h
        .client()
        .send(BridgeSend {
            recipient: "x".into(),
            message: Some("huge".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(e.detail.unwrap().chars().count() <= 501);
    assert!(e.message.chars().count() < 260);
}

#[tokio::test]
async fn send_403_json_and_soft_failure() {
    let h = Harness::start().await;
    let c = h.client();
    let e = c
        .send(BridgeSend {
            recipient: "x".into(),
            message: Some("json403".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Forbidden);
    assert!(e.message.contains("media_path rejected"));
    let e = c
        .send(BridgeSend {
            recipient: "x".into(),
            message: Some("soft".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.message.as_str()),
        (ErrorKind::External, "Not connected")
    );
}

#[tokio::test]
async fn send_body_shape() {
    let h = Harness::start().await;
    let msg = h
        .client()
        .send(BridgeSend {
            recipient: "91999@s.whatsapp.net".into(),
            message: Some("hello".into()),
            media_path: Some("/outbox/u/a.pdf".into()),
            quoted_message_id: Some("Q1".into()),
            mentions: vec!["91888".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(msg, "Message sent to x");
    let b = h.mock.last_body.lock().clone();
    assert_eq!(b["recipient"], "91999@s.whatsapp.net");
    assert_eq!(b["media_path"], "/outbox/u/a.pdf");
    assert_eq!(b["quoted_message_id"], "Q1");
    assert_eq!(b["mentions"][0], "91888");
}

#[tokio::test]
async fn send_validates_locally() {
    let h = Harness::start().await;
    let e = h
        .client()
        .send(BridgeSend {
            recipient: "x".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
}

#[tokio::test]
async fn download_success_and_failures() {
    let h = Harness::start().await;
    let c = h.client();
    let d = c.download("c@s.whatsapp.net", "M1", true).await.unwrap();
    assert_eq!(d.path, std::path::PathBuf::from("/store/x/M1.jpg"));
    assert_eq!(d.filename, "M1.jpg");
    let e = c.download("c", "gone", true).await.unwrap_err();
    assert_eq!(
        (e.kind, e.message.as_str()),
        (ErrorKind::External, "media expired")
    );
    let e = c.download("c", "down", true).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::BridgeDown);
}

#[tokio::test]
async fn download_timeout() {
    let h = Harness::start().await;
    let c = h.client_for(
        "127.0.0.1",
        Timeouts {
            download: Duration::from_millis(300),
            ..Timeouts::default()
        },
    );
    let e = c.download("c", "slow", true).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout);
}

#[tokio::test]
async fn health_503_is_bridge_down() {
    let h = Harness::start().await;
    assert_eq!(
        h.client().health().await.unwrap_err().kind,
        ErrorKind::BridgeDown
    );
}

#[tokio::test]
async fn react_and_mark_read_bodies() {
    let h = Harness::start().await;
    let c = h.client();
    c.react("g@g.us", "M1", "👍", false, Some("91@s.whatsapp.net"))
        .await
        .unwrap();
    let b = h.mock.last_body.lock().clone();
    assert_eq!(b["emoji"], "👍");
    assert_eq!(b["sender_jid"], "91@s.whatsapp.net");
    assert_eq!(b["from_me"], false);
    c.mark_read(
        "g@g.us",
        &["M1".into(), "M2".into()],
        Some("91@s.whatsapp.net"),
    )
    .await
    .unwrap();
    let b = h.mock.last_body.lock().clone();
    assert_eq!(b["message_ids"][1], "M2");
    // Empty id list is a no-op, not a bridge 400.
    c.mark_read("g@g.us", &[], None).await.unwrap();
}

#[tokio::test]
async fn refused_port_is_bridge_down_and_unreachable() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let dir = tempfile::tempdir().unwrap();
    let tf = dir.path().join(".bridge-token");
    std::fs::write(&tf, TOKEN_A).unwrap();
    let c = BridgeClient::with_timeouts(
        &format!("http://127.0.0.1:{port}"),
        &tf,
        Timeouts::default(),
    )
    .unwrap();
    assert_eq!(c.reconnect().await.unwrap_err().kind, ErrorKind::BridgeDown);
    let s = c.connection().await;
    assert_eq!(s.state, BridgeState::Unreachable);
    assert!(!s.connected);
}

#[tokio::test]
async fn missing_token_file_is_auth() {
    let h = Harness::start().await;
    std::fs::remove_file(h.dir.path().join(".bridge-token")).unwrap();
    let e = h.client().reconnect().await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    let _ = Body::empty();
}

#[test]
fn rejects_non_loopback_paths_config() {
    let mut p = wa_core::config::Paths::under_root(
        std::path::Path::new("/tmp/x"),
        "http://10.0.0.1:8080".into(),
    );
    assert_eq!(
        BridgeClient::new(&p).unwrap_err().kind,
        ErrorKind::Forbidden
    );
    p.bridge_url = "http://127.0.0.1:8080".into();
    assert!(BridgeClient::new(&p).is_ok());
}

#[tokio::test]
async fn profile_group_newsletter_parse() {
    let h = Harness::start().await;
    let c = h.client();
    let p = c.profile("919811111111@s.whatsapp.net").await.unwrap();
    assert_eq!(p.push_name.as_deref(), Some("Ally"));
    assert_eq!(p.verified_name, None);
    let b = p.business.unwrap();
    assert_eq!(b.category.as_deref(), Some("Shop, Food"));
    assert_eq!(
        c.profile("x@s.whatsapp.net").await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.profile("down@s.whatsapp.net").await.unwrap_err().kind,
        ErrorKind::BridgeDown
    );
    assert_eq!(
        c.profile("nojid").await.unwrap_err().kind,
        ErrorKind::Invalid
    );
    let g = c.group("120@g.us").await.unwrap();
    assert!(g.is_parent && g.announce && !g.locked);
    assert_eq!(
        g.participants[0].phone_jid.as_deref(),
        Some("9190@s.whatsapp.net")
    );
    assert_eq!(g.participants[0].display_name, None);
    assert_eq!(g.sub_groups.len(), 1);
    assert_eq!(g.created_at.as_deref(), Some("2026-09-28T05:40:20Z"));
    assert_eq!(
        c.group("1@newsletter").await.unwrap_err().kind,
        ErrorKind::External
    );
    let n = c.newsletter("5@newsletter").await.unwrap();
    assert_eq!((n.subscribers, n.picture_id), (Some(23101), None));
}

#[tokio::test]
async fn avatar_and_refresh() {
    let h = Harness::start().await;
    let c = h.client();
    let a = c.avatar("7@g.us", false).await.unwrap();
    assert_eq!(
        a.path,
        std::path::PathBuf::from("/store/avatars/7@g.us.jpg")
    );
    assert_eq!(a.picture_id.as_deref(), Some("9"));
    let a = c.avatar("7@g.us", true).await.unwrap();
    assert!(a.path.to_string_lossy().ends_with("-full.jpg"));
    assert_eq!(
        c.avatar("none@g.us", false).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    let r = c.refresh_chats().await.unwrap();
    assert_eq!(
        (r.groups, r.communities, r.channels, r.renamed),
        (162, 21, 2, 4)
    );
    assert!(!r.cached);
}

#[tokio::test]
async fn missing_endpoints_are_not_found() {
    // An old bridge without the new routes answers 404 → NotFound (callers fall back).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, Router::new()).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let tf = dir.path().join(".bridge-token");
    std::fs::write(&tf, TOKEN_A).unwrap();
    let c = BridgeClient::with_timeouts(
        &format!("http://127.0.0.1:{port}"),
        &tf,
        Timeouts::default(),
    )
    .unwrap();
    assert_eq!(
        c.refresh_chats().await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.avatar("1@g.us", false).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}
