//! In-memory implementations of the wa-core traits for job tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use wa_core::config::Paths;
use wa_core::traits::*;
use wa_core::types::*;
use wa_core::{AppError, ErrorKind, Result};
use wa_jobs::{JobDeps, JobEvent, JobManager};

#[derive(Default)]
pub struct MemApp {
    pub jobs: Mutex<Vec<JobRecord>>,
    pub updates: Mutex<u32>,
    pub kv: Mutex<HashMap<String, String>>,
}

impl AppStore for MemApp {
    fn get_settings(&self) -> Result<Settings> {
        Ok(Settings::default())
    }
    fn put_settings(&self, _: &Settings) -> Result<()> {
        Ok(())
    }
    fn list_rules(&self) -> Result<Vec<Rule>> {
        Ok(vec![])
    }
    fn save_rule(&self, r: &Rule) -> Result<Rule> {
        Ok(r.clone())
    }
    fn delete_rule(&self, _: i64) -> Result<()> {
        Ok(())
    }
    fn record_rule_fired(&self, _: i64, _: i64) -> Result<()> {
        Ok(())
    }
    fn log_automation(&self, _: &AutomationEvent) -> Result<()> {
        Ok(())
    }
    fn recent_automation(&self, _: u32) -> Result<Vec<AutomationEvent>> {
        Ok(vec![])
    }
    fn get_kv(&self, k: &str) -> Result<Option<String>> {
        Ok(self.kv.lock().get(k).cloned())
    }
    fn set_kv(&self, k: &str, v: &str) -> Result<()> {
        self.kv.lock().insert(k.into(), v.into());
        Ok(())
    }
    fn insert_job(&self, job: &JobRecord) -> Result<i64> {
        let mut j = self.jobs.lock();
        let id = j.len() as i64 + 1;
        let mut r = job.clone();
        r.id = id;
        j.push(r);
        Ok(id)
    }
    fn update_job(&self, job: &JobRecord) -> Result<()> {
        *self.updates.lock() += 1;
        let mut j = self.jobs.lock();
        let slot = j
            .iter_mut()
            .find(|r| r.id == job.id)
            .ok_or_else(|| AppError::not_found("job"))?;
        *slot = job.clone();
        Ok(())
    }
    fn list_jobs(&self, limit: u32) -> Result<Vec<JobRecord>> {
        let mut v = self.jobs.lock().clone();
        v.reverse();
        v.truncate(limit as usize);
        Ok(v)
    }
    fn fail_interrupted_jobs(&self) -> Result<u32> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct MemStore {
    pub rows: Mutex<Vec<MediaInfo>>,
}

impl MessageStore for MemStore {
    fn max_rowid(&self) -> Result<i64> {
        Ok(0)
    }
    fn messages_after(&self, _: i64, _: u32) -> Result<Vec<IncomingMessage>> {
        Ok(vec![])
    }
    fn get_message(&self, _: &str, _: &str) -> Result<Option<Message>> {
        Ok(None)
    }
    fn media_info(&self, chat: &str, id: &str) -> Result<Option<MediaInfo>> {
        Ok(self
            .rows
            .lock()
            .iter()
            .find(|r| r.chat_jid == chat && r.message_id == id)
            .cloned())
    }
    fn media_rows(&self, since: Option<i64>, kinds: &[MediaKind]) -> Result<Vec<MediaInfo>> {
        Ok(self
            .rows
            .lock()
            .iter()
            .filter(|r| since.is_none_or(|s| r.ts >= s))
            .filter(|r| kinds.is_empty() || kinds.contains(&r.kind))
            .cloned()
            .collect())
    }
}

#[derive(Default)]
pub struct MemLinks {
    pub links: Mutex<Vec<LinkRecord>>,
    pub batches: Mutex<u32>,
}

impl LinkStore for MemLinks {
    fn upsert_links(&self, links: &[LinkRecord]) -> Result<()> {
        *self.batches.lock() += 1;
        self.links.lock().extend_from_slice(links);
        Ok(())
    }
}

/// Bridge mock: `download` writes `content` (or a fixed file) into `dir`.
pub struct MockBridge {
    pub dir: PathBuf,
    pub fail: Mutex<HashMap<String, AppError>>,
    pub all_fail: Mutex<Option<AppError>>,
    pub calls: Mutex<Vec<(String, String, bool)>>,
    pub content: Mutex<HashMap<String, Vec<u8>>>,
}

impl MockBridge {
    pub fn new(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        Self {
            dir: dir.to_path_buf(),
            fail: Mutex::new(HashMap::new()),
            all_fail: Mutex::new(None),
            calls: Mutex::new(vec![]),
            content: Mutex::new(HashMap::new()),
        }
    }
    pub fn ids(&self) -> Vec<String> {
        self.calls.lock().iter().map(|c| c.1.clone()).collect()
    }
}

#[async_trait]
impl BridgeApi for MockBridge {
    async fn connection(&self) -> ConnectionState {
        ConnectionState {
            state: BridgeState::Connected,
            connected: true,
            paired: true,
            message: String::new(),
            qr_svg: None,
            qr_expires_at: None,
            last_error: None,
            checked_at: 0,
        }
    }
    async fn reconnect(&self) -> Result<()> {
        Ok(())
    }
    async fn send(&self, _: BridgeSend) -> Result<String> {
        Ok("sent".into())
    }
    async fn react(&self, _: &str, _: &str, _: &str, _: bool, _: Option<&str>) -> Result<()> {
        Ok(())
    }
    async fn download(&self, chat: &str, id: &str, no_retry: bool) -> Result<DownloadOutcome> {
        self.calls.lock().push((chat.into(), id.into(), no_retry));
        if let Some(e) = self.all_fail.lock().clone() {
            return Err(e);
        }
        if let Some(e) = self.fail.lock().get(id).cloned() {
            return Err(e);
        }
        let data = self
            .content
            .lock()
            .get(id)
            .cloned()
            .unwrap_or_else(|| b"media".to_vec());
        let path = self.dir.join(format!("{id}.bin"));
        std::fs::write(&path, data).unwrap();
        Ok(DownloadOutcome {
            path,
            filename: format!("{id}.bin"),
        })
    }
    async fn mark_read(&self, _: &str, _: &[String], _: Option<&str>) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct MemIssues(pub Mutex<Vec<(ErrorKind, String, String)>>);

impl IssueSink for MemIssues {
    fn record(&self, kind: ErrorKind, source: &str, title: &str, _: Option<&str>) {
        self.0.lock().push((kind, source.into(), title.into()));
    }
}

pub struct Env {
    pub dir: tempfile::TempDir,
    pub paths: Paths,
    pub app: Arc<MemApp>,
    pub store: Arc<MemStore>,
    pub links: Arc<MemLinks>,
    pub bridge: Arc<MockBridge>,
    pub issues: Arc<MemIssues>,
}

impl Env {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under_root(dir.path(), "http://127.0.0.1:8080".into());
        paths.ensure_own_dirs().unwrap();
        std::fs::create_dir_all(&paths.bridge_store).unwrap();
        std::fs::create_dir_all(&paths.phone_media).unwrap();
        let bridge = Arc::new(MockBridge::new(&dir.path().join("dl")));
        Self {
            paths,
            app: Arc::default(),
            store: Arc::default(),
            links: Arc::default(),
            bridge,
            issues: Arc::default(),
            dir,
        }
    }

    pub fn deps(&self) -> JobDeps {
        let mut d = JobDeps::new(
            self.paths.clone(),
            self.store.clone(),
            self.links.clone(),
            self.app.clone(),
            self.bridge.clone(),
        );
        d.phone_root = self.dir.path().join("gvfs");
        d.issues = Some(self.issues.clone());
        d.backfill_retry_delay = Duration::from_millis(10);
        d
    }

    pub fn manager(&self) -> (JobManager, Events) {
        self.manager_with(self.deps())
    }

    pub fn manager_with(&self, deps: JobDeps) -> (JobManager, Events) {
        let (tx, rx) = unbounded_channel();
        let m = JobManager::new(
            deps,
            Arc::new(move |e| {
                let _ = tx.send(e);
            }),
        )
        .unwrap();
        (m, Events { rx, seen: vec![] })
    }
}

pub struct Events {
    pub rx: UnboundedReceiver<JobEvent>,
    pub seen: Vec<JobEvent>,
}

impl Events {
    /// Wait for job `id` to finish (60 s cap).
    pub async fn finished(&mut self, id: i64) -> JobRecord {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            let ev = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .expect("job did not finish in time")
                .expect("channel closed");
            self.seen.push(ev.clone());
            if let JobEvent::Finished(r) = ev
                && r.id == id
            {
                return r;
            }
        }
    }

    pub fn progress_count(&self, id: i64) -> usize {
        self.seen
            .iter()
            .filter(|e| matches!(e, JobEvent::Progress(p) if p.id == id))
            .count()
    }
}

pub fn media(chat: &str, id: &str, kind: MediaKind, ts: i64) -> MediaInfo {
    MediaInfo {
        chat_jid: chat.into(),
        message_id: id.into(),
        kind,
        filename: None,
        file_length: None,
        file_sha256: None,
        raw_ts: "2026-11-15 10:00:00+05:30".into(),
        ts,
        local_path: None,
        source: MediaSource::Missing,
    }
}

/// Path of an output URL produced by a job.
pub fn url_path(url: &str) -> PathBuf {
    let enc = url.strip_prefix("wa-media://localhost/x/").unwrap();
    PathBuf::from(
        percent_encoding::percent_decode_str(enc)
            .decode_utf8()
            .unwrap()
            .into_owned(),
    )
}
