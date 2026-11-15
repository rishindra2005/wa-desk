//! Shared harness: standard fixture + axum mock bridge (Host + token
//! enforced) + recording emitter/platform around a real `AppCore`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use parking_lot::Mutex;
use serde_json::{Value, json};
use wa_core::Result;
use wa_desk_lib::core::{AppCore, CoreDeps, Emitter, Platform};
use wa_obs::IssueStore;
use wa_testkit::{Fixture, TOKEN};

/// Queued downloads: message id → (path to write, bytes).
pub type Downloads = HashMap<String, (PathBuf, Vec<u8>)>;

#[derive(Clone, Default)]
pub struct Mock {
    pub host: Arc<Mutex<String>>,
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    pub connection: Arc<Mutex<Value>>,
    /// message id → (path to write, bytes) for /api/download.
    pub downloads: Arc<Mutex<Downloads>>,
}

impl Mock {
    pub fn calls_to(&self, path: &str) -> Vec<Value> {
        self.calls
            .lock()
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

async fn guard(State(m): State<Mock>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if host != *m.host.lock() {
        return (StatusCode::FORBIDDEN, "Forbidden: host not allowed\n").into_response();
    }
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if auth != format!("Bearer {TOKEN}") {
        return (StatusCode::UNAUTHORIZED, "Unauthorized\n").into_response();
    }
    next.run(req).await
}

async fn connection(State(m): State<Mock>) -> axum::Json<Value> {
    axum::Json(m.connection.lock().clone())
}

fn record(m: &Mock, path: &str, body: Value) {
    m.calls.lock().push((path.to_string(), body));
}

async fn ok(State(m): State<Mock>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let bytes = axum::body::to_bytes(req.into_body(), 1 << 20)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    record(&m, &path, body);
    axum::Json(json!({"success": true, "message": "Message sent"})).into_response()
}

async fn download(State(m): State<Mock>, axum::Json(body): axum::Json<Value>) -> Response {
    record(&m, "/api/download", body.clone());
    let id = body["message_id"].as_str().unwrap_or_default().to_string();
    let Some((path, data)) = m.downloads.lock().get(&id).cloned() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"success": false, "message": "media expired"})),
        )
            .into_response();
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, data).unwrap();
    axum::Json(json!({"success": true, "message": "ok", "path": path, "filename": "x"}))
        .into_response()
}

/// Start the mock on a random port; returns (base url, mock).
pub async fn start_mock() -> (String, Mock) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let m = Mock::default();
    *m.host.lock() = addr.to_string();
    *m.connection.lock() =
        json!({"state": "connected", "connected": true, "paired": true, "message": "Connected"});
    let app = Router::new()
        .route("/api/connection", get(connection))
        .route("/api/connection/reconnect", post(ok))
        .route("/api/send", post(ok))
        .route("/api/react", post(ok))
        .route("/api/mark-read", post(ok))
        .route("/api/download", post(download))
        .layer(middleware::from_fn_with_state(m.clone(), guard))
        .with_state(m.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), m)
}

#[derive(Default)]
pub struct Rec {
    pub events: Mutex<Vec<(String, Value)>>,
}

impl Rec {
    pub fn named(&self, name: &str) -> Vec<Value> {
        self.events
            .lock()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

impl Emitter for Rec {
    fn emit(&self, event: &str, payload: Value) {
        self.events.lock().push((event.to_string(), payload));
    }
}

#[derive(Default)]
pub struct TestPlatform {
    pub opened: Mutex<Vec<PathBuf>>,
    pub revealed: Mutex<Vec<PathBuf>>,
    pub autostart: Mutex<Vec<bool>>,
    pub notes: Mutex<Vec<(String, String)>>,
    pub focused: Mutex<bool>,
}

impl Platform for TestPlatform {
    fn open_path(&self, path: &Path) -> Result<()> {
        self.opened.lock().push(path.to_path_buf());
        Ok(())
    }
    fn reveal_path(&self, path: &Path) -> Result<()> {
        self.revealed.lock().push(path.to_path_buf());
        Ok(())
    }
    fn set_autostart(&self, enabled: bool) -> Result<()> {
        self.autostart.lock().push(enabled);
        Ok(())
    }
    fn notify(&self, title: &str, body: &str) {
        self.notes.lock().push((title.into(), body.into()));
    }
    fn main_window_focused(&self) -> bool {
        *self.focused.lock()
    }
}

pub struct H {
    pub f: Fixture,
    pub core: Arc<AppCore>,
    pub mock: Mock,
    pub rec: Arc<Rec>,
    pub platform: Arc<TestPlatform>,
}

async fn build(mut f: Fixture, url: String, mock: Mock) -> H {
    f.paths.bridge_url = url;
    let rec = Arc::new(Rec::default());
    let platform = Arc::new(TestPlatform::default());
    let bridge = Arc::new(wa_bridge::BridgeClient::new(&f.paths).unwrap());
    let core = AppCore::new(CoreDeps {
        paths: f.paths.clone(),
        bridge,
        issues: Arc::new(IssueStore::open(&f.paths).unwrap()),
        emitter: rec.clone(),
        platform: platform.clone(),
    })
    .await
    .unwrap();
    H {
        f,
        core,
        mock,
        rec,
        platform,
    }
}

/// Standard fixture against a live mock bridge.
pub async fn harness() -> H {
    let (url, mock) = start_mock().await;
    build(Fixture::standard(), url, mock).await
}

/// Standard fixture with the bridge down (closed port).
pub async fn harness_down() -> H {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    build(
        Fixture::standard(),
        format!("http://127.0.0.1:{port}"),
        Mock::default(),
    )
    .await
}

/// Raw timestamp text for "now" (+ offset seconds), in the bridge's format.
pub fn ts_now(offset: i64) -> String {
    let t = chrono::Utc::now() + chrono::Duration::seconds(offset);
    t.format("%Y-%m-%d %H:%M:%S+00:00").to_string()
}

/// Add a document message with its file in the bridge store.
pub fn add_doc(f: &Fixture, id: &str, name: &str, data: &[u8]) -> PathBuf {
    let ts = "2026-11-15 09:00:00+05:30";
    f.add_message(
        &wa_testkit::Msg::text(id, wa_testkit::ids::DM_ALICE, "919811111111", ts, "").media(
            "document",
            Some(name),
            data,
        ),
    );
    f.put_store_media(wa_testkit::ids::DM_ALICE, "document", id, ts, data)
}

/// Wait for a job to finish (via list_jobs).
pub async fn wait_job(core: &AppCore, id: i64) -> wa_core::types::JobRecord {
    for _ in 0..200 {
        let jobs = core.list_jobs(50).await.unwrap();
        if let Some(j) = jobs.into_iter().find(|j| j.id == id)
            && j.status.is_terminal()
        {
            return j;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("job {id} did not finish");
}
