//! Tauri-independent application core.
//!
//! `AppCore` owns every service (DB, bridge client, job manager, automation
//! engine, issue store) and implements each IPC command as an async method.
//! The Tauri layer (`commands.rs`, `protocol.rs`, `tray.rs`) is a thin shell
//! over it, so everything here is testable without a webview: side effects
//! that need Tauri go through the [`Emitter`] and [`Platform`] traits.

mod export;
mod forward;
mod profile;
mod warm;

pub use export::{body_text, html_escape, render_html, safe_name, wa_time, whatsapp_lines};
pub use forward::{FORWARD_MAX_MESSAGES, FORWARD_MAX_TARGETS};
pub use profile::{AVATAR_WAIT, PROFILE_TTL, format_phone};
pub use warm::{UserBusy, WarmState, WarmStats};

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde_json::{Value, json};
use wa_automation::Engine;
use wa_core::config::Paths;
use wa_core::traits::{
    AppStore, BridgeApi, BridgeSend, IncomingMessage, IssueSink, MessageStore, Notifier,
    ProgressSink,
};
use wa_core::types::{
    AutomationEvent, ChatQuery, ChatStats, ChatSummary, ConnectionState, DbChangedEvent, Issue,
    JobRecord, JobSpec, LinkItem, LogFilter, LogLine, MediaItem, MediaKind, MediaQuery, MediaRef,
    MediaSource, Message, MessagesQuery, NewMessagesEvent, Overview, Page, PreviewKind, PreviewRef,
    ReactReq, Rule, RuleEval, SearchHit, SearchQuery, SearchResults, SendReq, SendResult, Settings,
    events,
};
use wa_core::{AppError, ErrorKind, Result};
use wa_db::Db;
use wa_jobs::{JobDeps, JobEvent, JobEventFn, JobManager};
use wa_obs::IssueStore;

/// Longest text message accepted by `send_message` (characters).
pub const MAX_TEXT_CHARS: usize = 65_536;
/// Most message ids sent in one `mark_read`.
pub const MARK_READ_CAP: usize = 200;
/// Text previews are offered for files up to this size.
pub const TEXT_PREVIEW_MAX: u64 = 1024 * 1024;
/// Staged outbox files older than this are removed by the daily cleanup.
pub const OUTBOX_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// Office formats converted to PDF for preview.
pub const OFFICE_EXTS: &[&str] = &[
    "doc", "docx", "odt", "rtf", "ppt", "pptx", "odp", "xls", "xlsx", "ods", "csv",
];
/// Plain-text formats previewed as text (when small).
pub const TEXT_EXTS: &[&str] = &["txt", "md", "json", "log", "csv"];

/// Sends named events to the UI (Tauri `emit` in the app, a recorder in tests).
pub trait Emitter: Send + Sync {
    fn emit(&self, event: &str, payload: Value);
}

/// Desktop-shell side effects the core needs.
pub trait Platform: Send + Sync {
    /// Open a local file with the system default application.
    fn open_path(&self, path: &Path) -> Result<()>;
    /// Show a local file in the system file manager.
    fn reveal_path(&self, path: &Path) -> Result<()>;
    /// Enable/disable launching at login.
    fn set_autostart(&self, enabled: bool) -> Result<()>;
    /// Show a desktop notification (callers check the user setting).
    fn notify(&self, title: &str, body: &str);
    /// Whether the main window is visible and focused.
    fn main_window_focused(&self) -> bool;
}

/// Serialize `payload` and emit it; serialization failures are logged.
pub fn emit_typed<T: Serialize>(em: &dyn Emitter, event: &str, payload: &T) {
    match serde_json::to_value(payload) {
        Ok(v) => em.emit(event, v),
        Err(e) => tracing::warn!(event, error = %e, "could not serialize event payload"),
    }
}

/// Run blocking work (SQLite, file copies) off the async runtime.
pub async fn blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal("a background task failed").with_detail(e.to_string()))?
}

/// Rewrites a `BridgeDown` error into a friendly, action-specific message.
fn friendly_bridge(e: AppError, what: &str) -> AppError {
    if e.kind != ErrorKind::BridgeDown {
        return e;
    }
    let detail = e.detail.clone().unwrap_or_else(|| e.message.clone());
    AppError::bridge_down(format!(
        "The WhatsApp bridge is offline, so {what}. Try again once it is connected."
    ))
    .with_detail(detail)
}

/// Notifier used by the automation engine: honours `desktop_notifications`.
struct DesktopNotifier {
    platform: Arc<dyn Platform>,
    db: Arc<Db>,
}

impl Notifier for DesktopNotifier {
    fn notify(&self, title: &str, body: &str) {
        let on = self
            .db
            .get_settings()
            .map(|s| s.desktop_notifications)
            .unwrap_or(true);
        if on {
            self.platform.notify(title, body);
        }
    }
}

/// Inputs for [`AppCore::new`].
pub struct CoreDeps {
    pub paths: Paths,
    pub bridge: Arc<dyn BridgeApi>,
    pub issues: Arc<IssueStore>,
    pub emitter: Arc<dyn Emitter>,
    pub platform: Arc<dyn Platform>,
}

/// Everything the app does, independent of Tauri.
pub struct AppCore {
    paths: Paths,
    db: Arc<Db>,
    bridge: Arc<dyn BridgeApi>,
    jobs: JobManager,
    engine: RwLock<Arc<Engine>>,
    engine_has_me: AtomicBool,
    issues: Arc<IssueStore>,
    emitter: Arc<dyn Emitter>,
    platform: Arc<dyn Platform>,
    automation_paused: AtomicBool,
    read_only: AtomicBool,
    last_bridge: Mutex<Option<ConnectionState>>,
    caches: profile::ProfileCaches,
    warm: Arc<warm::WarmState>,
}

/// Our own user parts (phone and LID) from the name snapshot; empty if unknown yet.
fn own_users(db: &Db) -> Vec<String> {
    match db.names().snapshot() {
        Ok(n) => n.own_user_parts(),
        Err(e) => {
            tracing::debug!(error = %e.message, "own JID not resolvable yet");
            Vec::new()
        }
    }
}

impl AppCore {
    /// Open the DB, mark interrupted jobs, build the job manager and engine.
    /// Must run inside a Tokio runtime.
    pub async fn new(deps: CoreDeps) -> Result<Arc<Self>> {
        let p = deps.paths.clone();
        let db = Arc::new(blocking(move || Db::open(&p)).await?);
        {
            let db = db.clone();
            match blocking(move || db.fail_interrupted_jobs()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(count = n, "marked interrupted jobs as failed"),
                Err(e) => tracing::warn!(error = %e, "could not mark interrupted jobs"),
            }
        }
        let em = deps.emitter.clone();
        deps.issues.set_listener(Arc::new(move |i: &Issue| {
            emit_typed(&*em, events::ISSUE_NEW, i);
        }));
        let em = deps.emitter.clone();
        let warm = Arc::new(warm::WarmState::default());
        let warm_ev = warm.clone();
        let on_event: JobEventFn = Arc::new(move |ev| match ev {
            JobEvent::Progress(p) => emit_typed(&*em, events::JOB_PROGRESS, &p),
            JobEvent::Finished(r) => {
                tracing::info!(
                    job = r.id,
                    label = %r.label,
                    status = ?r.status,
                    secs = r.finished_at.unwrap_or(r.created_at) - r.created_at,
                    "job finished"
                );
                if warm::kicks_warm(&r.spec, r.status) {
                    warm_ev.kick();
                }
                emit_typed(&*em, events::JOB_FINISHED, &r);
            }
        });
        let mut jd = JobDeps::new(
            deps.paths.clone(),
            db.clone(),
            db.clone(),
            db.clone(),
            deps.bridge.clone(),
        );
        jd.issues = Some(deps.issues.clone());
        let jobs = JobManager::new(jd, on_event)?;
        let me = {
            let db = db.clone();
            blocking(move || Ok(own_users(&db))).await?
        };
        let has_me = !me.is_empty();
        let paused = {
            let db = db.clone();
            blocking(move || db.get_settings())
                .await
                .map(|s| !s.automation_enabled)
                .unwrap_or(false)
        };
        let core = Arc::new(Self {
            engine: RwLock::new(Arc::new(Self::build_engine(
                &db,
                &deps.bridge,
                &deps.platform,
                &deps.issues,
                &deps.paths,
                me,
            ))),
            engine_has_me: AtomicBool::new(has_me),
            paths: deps.paths,
            db,
            bridge: deps.bridge,
            jobs,
            issues: deps.issues,
            emitter: deps.emitter,
            platform: deps.platform,
            automation_paused: AtomicBool::new(paused),
            read_only: AtomicBool::new(false),
            last_bridge: Mutex::new(None),
            caches: profile::ProfileCaches::default(),
            warm,
        });
        tracing::info!(own_known = has_me, "app core ready");
        Ok(core)
    }

    fn build_engine(
        db: &Arc<Db>,
        bridge: &Arc<dyn BridgeApi>,
        platform: &Arc<dyn Platform>,
        issues: &Arc<IssueStore>,
        paths: &Paths,
        me: Vec<String>,
    ) -> Engine {
        let notifier = Arc::new(DesktopNotifier {
            platform: platform.clone(),
            db: db.clone(),
        });
        Engine::new(db.clone(), bridge.clone(), notifier, me)
            .with_issues(issues.clone())
            .with_messages(db.clone())
            .with_paths(paths.clone())
    }

    /// Rebuild the engine once our own JID becomes known (fresh installs).
    async fn refresh_engine_identity(&self) {
        if self.engine_has_me.load(Ordering::SeqCst) {
            return;
        }
        let db = self.db.clone();
        let me = blocking(move || Ok(own_users(&db)))
            .await
            .unwrap_or_default();
        if me.is_empty() {
            return;
        }
        let e = Self::build_engine(
            &self.db,
            &self.bridge,
            &self.platform,
            &self.issues,
            &self.paths,
            me,
        );
        *self.engine.write() = Arc::new(e);
        self.engine_has_me.store(true, Ordering::SeqCst);
        tracing::info!("automation engine now knows our own account");
    }

    fn engine(&self) -> Arc<Engine> {
        self.engine.read().clone()
    }

    /// Paths in use.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The database facade.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// The job manager.
    pub fn jobs(&self) -> &JobManager {
        &self.jobs
    }

    /// The issue store (also an `IssueSink`).
    pub fn issues(&self) -> &Arc<IssueStore> {
        &self.issues
    }

    /// Record a problem for the Issues view (deduplicated by the store).
    pub(crate) fn record_issue(&self, kind: ErrorKind, source: &str, title: &str, detail: &str) {
        self.issues.record(kind, source, title, Some(detail));
    }

    /// Tray "Pause automations": persisted as `settings.automation_enabled`
    /// through [`Self::set_automation_enabled`], so resuming moves the
    /// watermark and messages that arrived meanwhile never fire.
    pub async fn set_automation_paused(&self, paused: bool) -> Result<()> {
        self.set_automation_enabled(!paused).await?;
        tracing::info!(paused, "automation pause toggled");
        Ok(())
    }

    /// Safety switch (`WA_DESK_READ_ONLY=1`) for running against a copy of the
    /// data while the live bridge is up: refuses sends, reactions, read
    /// receipts, downloads and reconnects, and never runs automations.
    pub fn set_read_only(&self, on: bool) {
        self.read_only.store(on, Ordering::SeqCst);
    }

    fn writable(&self) -> Result<()> {
        if self.read_only.load(Ordering::SeqCst) {
            Err(AppError::forbidden("wa-desk is running in read-only mode"))
        } else {
            Ok(())
        }
    }

    /// Whether automations are paused from the tray.
    pub fn automation_paused(&self) -> bool {
        self.automation_paused.load(Ordering::SeqCst)
    }

    // ---------------- browsing ----------------

    /// `overview`.
    pub async fn overview(&self) -> Result<Overview> {
        let db = self.db.clone();
        let mut o = blocking(move || db.overview()).await?;
        o.app_version = env!("CARGO_PKG_VERSION").to_string();
        Ok(o)
    }

    /// `list_chats`.
    pub async fn list_chats(&self, query: ChatQuery) -> Result<Page<ChatSummary>> {
        let db = self.db.clone();
        blocking(move || db.list_chats(&query)).await
    }

    /// `get_chat`.
    pub async fn get_chat(&self, jid: String) -> Result<ChatSummary> {
        require("chat", &jid)?;
        let db = self.db.clone();
        blocking(move || db.get_chat(&jid)).await
    }

    /// `get_messages`.
    pub async fn get_messages(&self, query: MessagesQuery) -> Result<Page<Message>> {
        require("chat", &query.chat_jid)?;
        let db = self.db.clone();
        blocking(move || db.get_messages(&query)).await
    }

    /// `get_message_context` (radius capped at 200).
    pub async fn get_message_context(
        &self,
        chat_jid: String,
        message_id: String,
        radius: u32,
    ) -> Result<Vec<Message>> {
        require("chat", &chat_jid)?;
        require("message", &message_id)?;
        let db = self.db.clone();
        let v = blocking(move || db.get_message_context(&chat_jid, &message_id, radius.min(200)))
            .await?;
        if v.is_empty() {
            return Err(AppError::not_found("message not found"));
        }
        Ok(v)
    }

    /// `search`.
    pub async fn search(&self, query: SearchQuery) -> Result<SearchResults> {
        let db = self.db.clone();
        blocking(move || db.search(&query)).await
    }

    /// `list_media`.
    pub async fn list_media(&self, query: MediaQuery) -> Result<Page<MediaItem>> {
        let db = self.db.clone();
        blocking(move || db.list_media(&query)).await
    }

    /// `chat_stats`.
    pub async fn chat_stats(&self, chat_jid: String) -> Result<ChatStats> {
        require("chat", &chat_jid)?;
        let db = self.db.clone();
        blocking(move || db.chat_stats(&chat_jid)).await
    }

    /// `list_links` (newest first).
    pub async fn list_links(
        &self,
        chat_jid: Option<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<Page<LinkItem>> {
        let db = self.db.clone();
        let chat = chat_jid.filter(|c| !c.trim().is_empty());
        blocking(move || db.list_links(chat.as_deref(), cursor.as_deref(), limit)).await
    }

    /// `set_starred` (emits `db:changed` for the chat).
    pub async fn set_starred(
        &self,
        chat_jid: String,
        message_ids: Vec<String>,
        starred: bool,
    ) -> Result<()> {
        require("chat", &chat_jid)?;
        if message_ids.len() > 1000 {
            return Err(AppError::invalid("Too many messages selected (max 1000)"));
        }
        let (db, c) = (self.db.clone(), chat_jid.clone());
        blocking(move || db.set_starred(&c, &message_ids, starred)).await?;
        emit_typed(
            &*self.emitter,
            events::DB_CHANGED,
            &DbChangedEvent {
                chat_jids: vec![chat_jid],
            },
        );
        Ok(())
    }

    /// `list_starred` (newest first).
    pub async fn list_starred(
        &self,
        chat_jid: Option<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<Page<SearchHit>> {
        let db = self.db.clone();
        let chat = chat_jid.filter(|c| !c.trim().is_empty());
        blocking(move || db.list_starred(chat.as_deref(), cursor.as_deref(), limit)).await
    }

    async fn message(&self, chat_jid: &str, id: &str) -> Result<Message> {
        let (db, c, i) = (self.db.clone(), chat_jid.to_string(), id.to_string());
        blocking(move || db.get_message(&c, &i))
            .await?
            .ok_or_else(|| AppError::not_found("Message not found"))
    }

    async fn media_ref(&self, chat_jid: &str, id: &str) -> Result<MediaRef> {
        require("chat", chat_jid)?;
        require("message", id)?;
        let (db, c, i) = (self.db.clone(), chat_jid.to_string(), id.to_string());
        blocking(move || db.media_ref(&c, &i))
            .await?
            .ok_or_else(|| AppError::not_found("This message has no media"))
    }

    /// Scope-checked local path of a message's media, or `None` if not on disk.
    pub async fn resolve_media(&self, chat_jid: &str, id: &str) -> Result<Option<PathBuf>> {
        let (db, c, i) = (self.db.clone(), chat_jid.to_string(), id.to_string());
        blocking(move || db.resolve_media_path(&c, &i)).await
    }

    /// Like `resolve_media`, downloading first when the file is missing.
    async fn local_media(&self, chat_jid: &str, id: &str) -> Result<(MediaRef, PathBuf)> {
        let r = self
            .ensure_media(chat_jid.to_string(), id.to_string())
            .await?;
        match self.resolve_media(chat_jid, id).await? {
            Some(p) => Ok((r, p)),
            None => Err(AppError::not_found(
                "The file is not available on this computer",
            )),
        }
    }

    /// Media kind + MIME for the protocol handler.
    pub async fn media_meta(&self, chat_jid: &str, id: &str) -> Result<Option<MediaRef>> {
        let (db, c, i) = (self.db.clone(), chat_jid.to_string(), id.to_string());
        blocking(move || db.media_ref(&c, &i)).await
    }

    // ---------------- media ----------------

    /// `ensure_media`: download via the bridge (asking the phone if needed) when missing.
    pub async fn ensure_media(&self, chat_jid: String, message_id: String) -> Result<MediaRef> {
        let r = self.media_ref(&chat_jid, &message_id).await?;
        let on_disk = r.source != MediaSource::Missing
            && self.resolve_media(&chat_jid, &message_id).await?.is_some();
        if on_disk {
            return Ok(r);
        }
        self.writable()?;
        tracing::info!(chat = %chat_jid, id = %message_id, "downloading missing media");
        self.bridge
            .download(&chat_jid, &message_id, false)
            .await
            .map_err(|e| friendly_bridge(e, "this file can't be downloaded right now"))?;
        let r = self.media_ref(&chat_jid, &message_id).await?;
        if r.source == MediaSource::Missing {
            return Err(AppError::external(
                "The download finished, but the file was not found where expected",
            ));
        }
        Ok(r)
    }

    /// `preview_doc`.
    pub async fn preview_doc(&self, chat_jid: String, message_id: String) -> Result<PreviewRef> {
        let (r, path) = self.local_media(&chat_jid, &message_id).await?;
        let m_url = r
            .url
            .clone()
            .unwrap_or_else(|| wa_db::media::media_url("m", &chat_jid, &message_id));
        let mk = |kind, url: Option<String>, note: Option<String>| PreviewRef {
            kind,
            url,
            original_url: Some(m_url.clone()),
            note,
        };
        let ext = r
            .filename
            .as_deref()
            .and_then(|f| Path::new(f).extension())
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let mime = r.mime.clone().unwrap_or_default();
        let size = match r.size {
            Some(s) => s,
            None => std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
        };
        let kind = match r.kind {
            MediaKind::Image | MediaKind::Sticker => Some(PreviewKind::Image),
            MediaKind::Video => Some(PreviewKind::Video),
            MediaKind::Audio => Some(PreviewKind::Audio),
            MediaKind::Document if ext == "pdf" || mime == "application/pdf" => {
                Some(PreviewKind::Pdf)
            }
            MediaKind::Document if mime.starts_with("image/") => Some(PreviewKind::Image),
            MediaKind::Document if mime.starts_with("video/") => Some(PreviewKind::Video),
            MediaKind::Document if mime.starts_with("audio/") => Some(PreviewKind::Audio),
            MediaKind::Document => None,
        };
        if let Some(k) = kind {
            return Ok(mk(k, Some(m_url.clone()), None));
        }
        if TEXT_EXTS.contains(&ext.as_str()) && size < TEXT_PREVIEW_MAX {
            return Ok(mk(PreviewKind::Text, Some(m_url.clone()), None));
        }
        if OFFICE_EXTS.contains(&ext.as_str()) {
            let _busy = self.warm.busy();
            return Ok(match wa_jobs::office_preview(&self.paths, &path).await {
                Ok(_) => mk(
                    PreviewKind::Pdf,
                    Some(wa_db::media::media_url("p", &chat_jid, &message_id)),
                    None,
                ),
                Err(e) => {
                    tracing::warn!(error = %e, ext, "office preview failed");
                    mk(
                        PreviewKind::Unsupported,
                        None,
                        Some(format!("Could not convert this document: {}", e.message)),
                    )
                }
            });
        }
        let note = if TEXT_EXTS.contains(&ext.as_str()) {
            "This file is too large to preview; open it externally instead.".to_string()
        } else {
            "No preview for this file type; open it externally instead.".to_string()
        };
        Ok(mk(PreviewKind::Unsupported, None, Some(note)))
    }

    /// `open_media_external`.
    pub async fn open_media_external(&self, chat_jid: String, message_id: String) -> Result<()> {
        let (_, path) = self.local_media(&chat_jid, &message_id).await?;
        self.platform.open_path(&path)
    }

    /// `save_media_as`: copy the file to a user-chosen absolute destination.
    pub async fn save_media_as(
        &self,
        chat_jid: String,
        message_id: String,
        dest: String,
    ) -> Result<String> {
        let dest = check_save_dest(&self.paths, Path::new(dest.trim()))?;
        let (_, src) = self.local_media(&chat_jid, &message_id).await?;
        if std::fs::canonicalize(&dest).is_ok_and(|d| d == src) {
            return Err(AppError::invalid("The destination is the original file"));
        }
        let d2 = dest.clone();
        blocking(move || {
            std::fs::copy(&src, &d2)?;
            Ok(())
        })
        .await?;
        Ok(dest.to_string_lossy().into_owned())
    }

    /// Scope-check an output path (job output, exported report): only files
    /// inside the exports or cache folders.
    pub fn output_path(&self, path: &str) -> Result<PathBuf> {
        let roots = [self.paths.exports.clone(), self.paths.cache.clone()];
        let p = wa_core::paths::resolve_within(Path::new(path.trim()), &roots)?;
        if !p.is_file() {
            return Err(AppError::invalid("Only files can be opened"));
        }
        Ok(p)
    }

    /// `open_path`: open an output file with the default application.
    pub async fn open_path(&self, path: String) -> Result<()> {
        let p = self.output_path(&path)?;
        self.platform.open_path(&p)
    }

    /// `reveal_path`: show an output file in the file manager.
    pub async fn reveal_path(&self, path: String) -> Result<()> {
        let p = self.output_path(&path)?;
        self.platform.reveal_path(&p)
    }

    // ---------------- sending ----------------

    /// `send_message`.
    pub async fn send_message(&self, req: SendReq) -> Result<SendResult> {
        self.writable()?;
        let recipient = req.recipient.trim().to_string();
        if recipient.is_empty() {
            return Err(AppError::invalid("Choose who to send the message to"));
        }
        let text = req.text.filter(|t| !t.trim().is_empty());
        if text
            .as_ref()
            .is_some_and(|t| t.chars().count() > MAX_TEXT_CHARS)
        {
            return Err(AppError::invalid(
                "The message is too long (max 65,536 characters)",
            ));
        }
        let attachment = req.attachment.filter(|a| !a.trim().is_empty());
        if text.is_none() && attachment.is_none() {
            return Err(AppError::invalid("Type a message or attach a file"));
        }
        let chat_jid = recipient_jid(&recipient)?;
        let mut send = BridgeSend {
            recipient: recipient.clone(),
            message: text,
            mentions: req.mentions,
            ..Default::default()
        };
        if let Some(rid) = req.reply_to.filter(|r| !r.trim().is_empty()) {
            let q = self
                .message(&chat_jid, &rid)
                .await
                .map_err(|e| match e.kind {
                    ErrorKind::NotFound | ErrorKind::Invalid => {
                        AppError::invalid("The message you are replying to is not in this chat")
                    }
                    _ => e,
                })?;
            send.quoted_sender_jid = if q.is_from_me {
                let db = self.db.clone();
                blocking(move || Ok(db.names().snapshot()?.own_jid())).await?
            } else {
                q.sender_jid.clone()
            };
            send.quoted_content = Some(q.content.chars().take(1024).collect());
            send.quoted_message_id = Some(q.id);
        }
        if let Some(a) = attachment {
            let paths = self.paths.clone();
            let staged =
                blocking(move || wa_bridge::outbox::stage(&paths, Path::new(a.trim()))).await?;
            send.media_path = Some(staged);
        }
        let staged = send.media_path.clone();
        match self.bridge.send(send).await {
            Ok(message) => {
                tracing::info!(attachment = staged.is_some(), "message sent");
                Ok(SendResult { ok: true, message })
            }
            Err(e) => {
                if let Some(dir) = staged.as_deref().and_then(Path::parent)
                    && dir.starts_with(&self.paths.outbox)
                    && dir != self.paths.outbox
                {
                    let _ = std::fs::remove_dir_all(dir);
                }
                Err(friendly_bridge(e, "the message was not sent"))
            }
        }
    }

    /// `react` ("" removes our reaction).
    pub async fn react(&self, req: ReactReq) -> Result<()> {
        self.writable()?;
        require("chat", &req.chat_jid)?;
        require("message", &req.message_id)?;
        if req.emoji.len() > 64 {
            return Err(AppError::invalid("That reaction is not a single emoji"));
        }
        let m = self.message(&req.chat_jid, &req.message_id).await?;
        let sender = if req.chat_jid.ends_with("@g.us") && !m.is_from_me {
            m.sender_jid.clone()
        } else {
            None
        };
        self.bridge
            .react(
                &req.chat_jid,
                &req.message_id,
                &req.emoji,
                m.is_from_me,
                sender.as_deref(),
            )
            .await
            .map_err(|e| friendly_bridge(e, "the reaction was not sent"))
    }

    /// `mark_read`: inbound messages newer than the last read marker (≤ 200).
    pub async fn mark_read(&self, chat_jid: String) -> Result<()> {
        self.writable()?;
        let chat = self.get_chat(chat_jid.clone()).await?;
        if !chat.unread {
            return Ok(());
        }
        let last_read = {
            let (p, c) = (self.paths.messages_db.clone(), chat_jid.clone());
            blocking(move || Ok(last_read_ts(&p, &c))).await?
        };
        let page = self
            .get_messages(MessagesQuery {
                chat_jid: chat_jid.clone(),
                before: None,
                after: None,
                limit: Some(MARK_READ_CAP as u32),
            })
            .await?;
        let unread: Vec<&Message> = page
            .items
            .iter()
            .filter(|m| !m.is_from_me && last_read.is_none_or(|r| m.ts > r))
            .collect();
        let unread = &unread[unread.len().saturating_sub(MARK_READ_CAP)..];
        if unread.is_empty() {
            return Ok(());
        }
        let calls: Vec<(Option<String>, Vec<String>)> = if chat.is_group {
            let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for m in unread {
                if let Some(s) = &m.sender_jid {
                    by.entry(s.clone()).or_default().push(m.id.clone());
                }
            }
            by.into_iter().map(|(s, ids)| (Some(s), ids)).collect()
        } else {
            vec![(None, unread.iter().map(|m| m.id.clone()).collect())]
        };
        for (sender, ids) in calls {
            self.bridge
                .mark_read(&chat_jid, &ids, sender.as_deref())
                .await
                .map_err(|e| friendly_bridge(e, "the chat could not be marked as read"))?;
        }
        Ok(())
    }

    // ---------------- bridge ----------------

    /// `connection_status`.
    pub async fn connection_status(&self) -> ConnectionState {
        self.bridge.connection().await
    }

    /// `reconnect`: ask the bridge to reconnect, then return the fresh state.
    pub async fn reconnect(&self) -> Result<ConnectionState> {
        self.writable()?;
        self.bridge
            .reconnect()
            .await
            .map_err(|e| friendly_bridge(e, "it cannot reconnect to WhatsApp"))?;
        Ok(self.bridge.connection().await)
    }

    // ---------------- jobs ----------------

    /// `start_job`.
    pub async fn start_job(&self, spec: JobSpec) -> Result<JobRecord> {
        match spec {
            JobSpec::Reindex => self.start_reindex(true),
            JobSpec::LinkPhoneMedia { extra_dirs } if extra_dirs.is_empty() => {
                let db = self.db.clone();
                let s = blocking(move || db.get_settings()).await?;
                self.jobs.start(JobSpec::LinkPhoneMedia {
                    extra_dirs: s.link_extra_dirs,
                })
            }
            other => self.jobs.start(other),
        }
    }

    /// Start the search-index job: `full` rebuilds from scratch, otherwise
    /// finishes/resumes an incomplete build.
    pub fn start_reindex(&self, full: bool) -> Result<JobRecord> {
        let db = self.db.clone();
        self.jobs.start_custom(JobSpec::Reindex, move |ctl| {
            Box::pin(async move {
                ctl.check()?;
                let c2 = ctl.clone();
                let n = blocking(move || {
                    let progress = |d: u64, t: u64| c2.progress(d, t, "indexing", None);
                    let stop = || c2.cancel_token().is_cancelled();
                    if full {
                        db.indexer().build_full_with(&progress, &stop)
                    } else {
                        db.indexer().ensure_with(&progress, &stop)
                    }
                })
                .await?;
                Ok(Some(format!("Indexed {n} messages")))
            })
        })
    }

    /// `cancel_job`.
    pub async fn cancel_job(&self, id: i64) -> Result<()> {
        self.jobs.cancel(id)
    }

    /// `list_jobs` (newest first, 1..=500).
    pub async fn list_jobs(&self, limit: u32) -> Result<Vec<JobRecord>> {
        let jobs = self.jobs.clone();
        let limit = limit.clamp(1, 500);
        blocking(move || jobs.list(limit)).await
    }

    // ---------------- automation ----------------

    /// `list_rules`.
    pub async fn list_rules(&self) -> Result<Vec<Rule>> {
        let db = self.db.clone();
        blocking(move || db.list_rules()).await
    }

    /// `save_rule`: validated first.
    pub async fn save_rule(&self, rule: Rule) -> Result<Rule> {
        self.engine().validate_rule(&rule)?;
        let db = self.db.clone();
        blocking(move || db.save_rule(&rule)).await
    }

    /// `delete_rule`.
    pub async fn delete_rule(&self, id: i64) -> Result<()> {
        let db = self.db.clone();
        blocking(move || db.delete_rule(id)).await
    }

    /// `test_rule`.
    pub async fn test_rule(&self, rule: Rule, sample: Message) -> Result<RuleEval> {
        Ok(self.engine().test_rule(&rule, &sample))
    }

    /// `automation_log`.
    pub async fn automation_log(&self, limit: u32) -> Result<Vec<AutomationEvent>> {
        let db = self.db.clone();
        let limit = limit.clamp(1, 1000);
        blocking(move || db.recent_automation(limit)).await
    }

    /// `set_automation_enabled`: enabling moves the watermark to the newest row.
    pub async fn set_automation_enabled(&self, enabled: bool) -> Result<Settings> {
        let db = self.db.clone();
        let engine = self.engine();
        blocking(move || {
            if enabled {
                engine.enable(db.max_rowid()?)?;
            }
            let mut s = db.get_settings()?;
            s.automation_enabled = enabled;
            db.put_settings(&s)?;
            Ok(s)
        })
        .await
        .inspect(|_| self.automation_paused.store(!enabled, Ordering::SeqCst))
    }

    // ---------------- observability ----------------

    /// `query_logs`.
    pub async fn query_logs(&self, filter: LogFilter) -> Result<Vec<LogLine>> {
        let paths = self.paths.clone();
        blocking(move || wa_obs::query_logs(&paths, &filter)).await
    }

    /// `list_issues`.
    pub async fn list_issues(&self) -> Result<Vec<Issue>> {
        let s = self.issues.clone();
        blocking(move || s.list()).await
    }

    /// `resolve_issue`.
    pub async fn resolve_issue(&self, id: i64) -> Result<()> {
        let s = self.issues.clone();
        blocking(move || s.resolve(id)).await
    }

    /// `clear_resolved_issues`: number removed.
    pub async fn clear_resolved_issues(&self) -> Result<u32> {
        let s = self.issues.clone();
        blocking(move || s.clear_resolved()).await
    }

    /// `export_issue_bundle`: path of the zip in the exports folder.
    pub async fn export_issue_bundle(&self, ids: Option<Vec<i64>>) -> Result<String> {
        let bridge = self.bridge.connection().await;
        let overview = self.overview().await.ok();
        let settings = {
            let db = self.db.clone();
            blocking(move || db.get_settings()).await.ok()
        };
        let extra = json!({
            "app_version": env!("CARGO_PKG_VERSION"),
            "bridge": bridge,
            "overview": overview,
            "settings": settings,
            "automation_paused": self.automation_paused(),
        });
        let (paths, store) = (self.paths.clone(), self.issues.clone());
        let p =
            blocking(move || wa_obs::export_bundle(&paths, &store, ids.as_deref(), extra)).await?;
        tracing::info!(file = %p.display(), "issue bundle exported");
        Ok(p.to_string_lossy().into_owned())
    }

    // ---------------- settings ----------------

    /// `get_settings`.
    pub async fn get_settings(&self) -> Result<Settings> {
        let db = self.db.clone();
        blocking(move || db.get_settings()).await
    }

    /// `set_settings`: persist, set the watermark when automation gets enabled,
    /// and apply autostart when it changed.
    pub async fn set_settings(&self, settings: Settings) -> Result<Settings> {
        let mut s = settings;
        s.automation_max_per_hour = s.automation_max_per_hour.clamp(1, 1000);
        s.backfill_days = s.backfill_days.clamp(1, 3650);
        s.link_extra_dirs = s
            .link_extra_dirs
            .into_iter()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .collect();
        if let Some(bad) = s
            .link_extra_dirs
            .iter()
            .find(|d| !Path::new(d).is_absolute())
        {
            return Err(AppError::invalid(format!(
                "Folder must be an absolute path: {bad}"
            )));
        }
        let db = self.db.clone();
        let engine = self.engine();
        let s2 = s.clone();
        let old = blocking(move || {
            let old = db.get_settings()?;
            if s2.automation_enabled && !old.automation_enabled {
                engine.enable(db.max_rowid()?)?;
            }
            db.put_settings(&s2)?;
            Ok(old)
        })
        .await?;
        self.automation_paused
            .store(!s.automation_enabled, Ordering::SeqCst);
        if old.autostart != s.autostart
            && let Err(e) = self.platform.set_autostart(s.autostart)
        {
            tracing::warn!(error = %e, "could not change autostart");
            self.record_issue(
                e.kind,
                "app.autostart",
                "Could not change start-at-login",
                &e.message,
            );
        }
        Ok(s)
    }

    // ---------------- background ticks (driven by `background.rs`) ----------------

    /// One watcher poll: emit events, run automations, notify. Returns new rows.
    pub async fn watch_tick(&self) -> Result<usize> {
        let db = self.db.clone();
        let out = blocking(move || db.poll()).await?;
        if !out.changed_chats.is_empty() {
            emit_typed(
                &*self.emitter,
                events::DB_CHANGED,
                &DbChangedEvent {
                    chat_jids: out.changed_chats.clone(),
                },
            );
        }
        if out.new.is_empty() {
            return Ok(0);
        }
        let mut seen = HashSet::new();
        let chat_jids: Vec<String> = out
            .new
            .iter()
            .filter(|m| seen.insert(m.message.chat_jid.clone()))
            .map(|m| m.message.chat_jid.clone())
            .collect();
        emit_typed(
            &*self.emitter,
            events::NEW_MESSAGES,
            &NewMessagesEvent {
                chat_jids,
                max_rowid: out.max_rowid,
                count: out.new.len() as u32,
            },
        );
        let settings = self.get_settings().await.unwrap_or_default();
        if settings.automation_enabled
            && !self.automation_paused()
            && !self.read_only.load(Ordering::SeqCst)
        {
            self.refresh_engine_identity().await;
            for ev in self.engine().process(&out.new).await {
                tracing::info!(rule = ev.rule_id, dry_run = ev.dry_run, "automation fired");
                emit_typed(&*self.emitter, events::AUTOMATION_FIRED, &ev);
            }
        }
        if settings.desktop_notifications && !self.platform.main_window_focused() {
            self.notify_new(&out.new).await;
        }
        Ok(out.new.len())
    }

    async fn notify_new(&self, new: &[IncomingMessage]) {
        let me = {
            let db = self.db.clone();
            blocking(move || Ok(own_users(&db)))
                .await
                .unwrap_or_default()
        };
        let picked: Vec<&IncomingMessage> = new.iter().filter(|m| should_notify(m, &me)).collect();
        match picked.len() {
            0 => {}
            1..=3 => {
                for m in picked {
                    let (title, body) = notification_text(m);
                    self.platform.notify(&title, &body);
                }
            }
            n => {
                let chats: HashSet<&str> =
                    picked.iter().map(|m| m.message.chat_jid.as_str()).collect();
                self.platform.notify(
                    "wa-desk",
                    &format!("{n} new messages in {} chats", chats.len()),
                );
            }
        }
    }

    /// Poll the bridge state; emits `bridge:state` only when it changed.
    pub async fn bridge_tick(&self) -> ConnectionState {
        let s = self.bridge.connection().await;
        let changed = {
            let mut last = self.last_bridge.lock();
            let changed = last.as_ref().is_none_or(|l| !same_bridge_state(l, &s));
            if changed {
                *last = Some(s.clone());
            }
            changed
        };
        if changed {
            tracing::info!(state = ?s.state, paired = s.paired, "bridge state changed");
            emit_typed(&*self.emitter, events::BRIDGE_STATE, &s);
        }
        s
    }

    /// Bring the search index up to date: a visible job when a (resumed)
    /// full build is needed, otherwise a silent incremental pass.
    pub async fn ensure_index(&self) -> Result<Option<JobRecord>> {
        let db = self.db.clone();
        let complete = blocking(move || db.app().index_complete()).await?;
        if !complete {
            return match self.start_reindex(false) {
                Ok(job) => {
                    tracing::info!(job = job.id, "search index build started");
                    Ok(Some(job))
                }
                Err(e) if e.kind == ErrorKind::Busy => Ok(None),
                Err(e) => Err(e),
            };
        }
        let db = self.db.clone();
        let n = blocking(move || db.indexer().ensure(&|_, _| {})).await?;
        tracing::debug!(rows = n, "incremental index pass");
        Ok(None)
    }

    /// Reconcile the index with the bridge DB (edits/revokes older than 48 h).
    pub async fn reconcile_tick(&self) -> Result<()> {
        let db = self.db.clone();
        if db.indexer().is_building() {
            return Ok(());
        }
        let st = blocking(move || db.indexer().reconcile(&|_, _| {})).await?;
        tracing::info!(
            scanned = st.scanned,
            updated = st.updated,
            removed = st.removed,
            "index reconciled"
        );
        Ok(())
    }

    /// Background chat-name refresh: a down or old bridge is not an error.
    pub async fn refresh_names_tick(&self) -> Result<()> {
        match self.refresh_chat_names().await {
            Ok(_) => Ok(()),
            Err(e)
                if matches!(
                    e.kind,
                    ErrorKind::BridgeDown | ErrorKind::NotFound | ErrorKind::Timeout
                ) =>
            {
                tracing::info!(kind = ?e.kind, "chat name refresh skipped");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Remove staged outbox files older than 7 days.
    pub async fn outbox_cleanup_tick(&self) -> Result<usize> {
        let paths = self.paths.clone();
        let n =
            blocking(move || wa_bridge::outbox::cleanup_older_than(&paths, OUTBOX_MAX_AGE)).await?;
        if n > 0 {
            tracing::info!(removed = n, "outbox cleaned");
        }
        Ok(n)
    }

    /// Cancel running jobs (app exit).
    pub fn shutdown(&self) {
        self.jobs.cancel_all();
    }
}

fn require(what: &str, v: &str) -> Result<()> {
    if v.trim().is_empty() {
        Err(AppError::invalid(format!("missing {what} id")))
    } else {
        Ok(())
    }
}

/// Chat JID for a recipient given as a JID or a phone number.
pub fn recipient_jid(recipient: &str) -> Result<String> {
    let r = recipient.trim();
    if r.contains('@') {
        return Ok(r.to_string());
    }
    let digits: String = r
        .chars()
        .filter(|c| !matches!(c, '+' | ' ' | '-' | '(' | ')'))
        .collect();
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        Ok(format!("{digits}@s.whatsapp.net"))
    } else {
        Err(AppError::invalid(
            "The recipient must be a chat or a phone number",
        ))
    }
}

/// The chat's `last_read_time` (Unix seconds), if the bridge records it.
fn last_read_ts(messages_db: &Path, chat_jid: &str) -> Option<i64> {
    let conn = wa_db::RoConn::open(messages_db, Duration::from_millis(300)).ok()?;
    if !conn
        .columns("chats")
        .ok()?
        .iter()
        .any(|c| c == "last_read_time")
    {
        return None;
    }
    let raw: Option<String> = conn
        .row(
            "SELECT CAST(last_read_time AS TEXT) FROM chats WHERE jid = ?1",
            [chat_jid],
            |r| r.get(0),
        )
        .ok()??;
    wa_core::time::parse_ts(raw.as_deref()?)
}

/// Validate a `save_media_as` destination.
pub fn check_save_dest(paths: &Paths, dest: &Path) -> Result<PathBuf> {
    if !dest.is_absolute() {
        return Err(AppError::invalid("Choose an absolute destination path"));
    }
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| AppError::invalid("The destination needs a file name"))?;
    let lower = name.to_ascii_lowercase();
    if wa_core::paths::SECRET_NAMES.contains(&name.as_str())
        || lower.ends_with(".db")
        || lower.contains(".db-")
        || lower.ends_with(".sqlite")
        || lower.ends_with(".sqlite3")
        || lower.ends_with(".bridge-token")
    {
        return Err(AppError::forbidden(
            "Refusing to write a database or secret file",
        ));
    }
    let parent = dest
        .parent()
        .ok_or_else(|| AppError::invalid("The destination needs a folder"))?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|_| AppError::invalid("The destination folder does not exist"))?;
    if !parent.is_dir() {
        return Err(AppError::invalid("The destination folder does not exist"));
    }
    let store =
        std::fs::canonicalize(&paths.bridge_store).unwrap_or_else(|_| paths.bridge_store.clone());
    if parent.starts_with(&store) {
        return Err(AppError::forbidden(
            "Refusing to write into the WhatsApp bridge's store",
        ));
    }
    let target = parent.join(&name);
    if target.is_dir() {
        return Err(AppError::invalid("The destination is a folder"));
    }
    if let Ok(real) = std::fs::canonicalize(&target)
        && real.starts_with(&store)
    {
        return Err(AppError::forbidden(
            "Refusing to write into the WhatsApp bridge's store",
        ));
    }
    Ok(target)
}

fn same_bridge_state(a: &ConnectionState, b: &ConnectionState) -> bool {
    a.state == b.state
        && a.connected == b.connected
        && a.paired == b.paired
        && a.qr_svg == b.qr_svg
        && a.last_error == b.last_error
}

/// Inbound, live, non-status messages: DMs always, groups only when @mentioned.
fn should_notify(m: &IncomingMessage, me: &[String]) -> bool {
    let msg = &m.message;
    if msg.is_from_me || msg.deleted || m.is_status || m.raw_media_type == "reaction" {
        return false;
    }
    if m.is_group {
        return wa_automation::rules::mentions_me(&msg.content, me);
    }
    true
}

fn notification_text(m: &IncomingMessage) -> (String, String) {
    let msg = &m.message;
    let title = if m.is_group {
        format!("{} in {}", msg.sender_name, m.chat_name)
    } else {
        m.chat_name.clone()
    };
    let body = if !msg.content.trim().is_empty() {
        let t: String = msg.content.chars().take(140).collect();
        if msg.content.chars().count() > 140 {
            format!("{t}…")
        } else {
            t
        }
    } else {
        match msg.media.as_ref().map(|r| r.kind) {
            Some(MediaKind::Image) => "📷 Photo".into(),
            Some(MediaKind::Video) => "🎥 Video".into(),
            Some(MediaKind::Audio) => "🎵 Audio".into(),
            Some(MediaKind::Sticker) => "Sticker".into(),
            Some(MediaKind::Document) => format!(
                "📄 {}",
                msg.media
                    .as_ref()
                    .and_then(|r| r.filename.clone())
                    .unwrap_or_else(|| "Document".into())
            ),
            None => "New message".into(),
        }
    };
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients_normalise() {
        assert_eq!(
            recipient_jid("+91 98111-11111").unwrap(),
            "919811111111@s.whatsapp.net"
        );
        assert_eq!(recipient_jid("1203@g.us").unwrap(), "1203@g.us");
        assert!(recipient_jid("alice").is_err());
    }

    #[test]
    fn bridge_state_comparison_ignores_timestamps() {
        let a = ConnectionState {
            state: wa_core::types::BridgeState::Connected,
            connected: true,
            paired: true,
            message: "ok".into(),
            qr_svg: None,
            qr_expires_at: None,
            last_error: None,
            checked_at: 1,
        };
        let mut b = a.clone();
        b.checked_at = 99;
        b.message = "still ok".into();
        assert!(same_bridge_state(&a, &b));
        b.paired = false;
        assert!(!same_bridge_state(&a, &b));
    }
}
