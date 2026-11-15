//! Thin `#[tauri::command]` wrappers over [`AppCore`]; names and arguments
//! follow docs/CONTRACT.md exactly (JS camelCase → Rust snake_case).

use std::sync::Arc;

use tauri::State;
use wa_core::AppError;
use wa_core::types::{
    AutomationEvent, ChatQuery, ChatSummary, ConnectionState, Issue, JobRecord, JobSpec, LogFilter,
    LogLine, MediaItem, MediaQuery, MediaRef, Message, MessagesQuery, Overview, Page, PreviewRef,
    ReactReq, Rule, RuleEval, SearchQuery, SearchResults, SendReq, SendResult, Settings,
};

use crate::core::AppCore;

type Core<'a> = State<'a, Arc<AppCore>>;
type R<T> = Result<T, AppError>;

/// Log failed commands at debug level (the UI shows the error itself).
fn logged<T>(cmd: &'static str, r: R<T>) -> R<T> {
    if let Err(e) = &r {
        tracing::debug!(cmd, kind = ?e.kind, error = %e.message, "command failed");
    }
    r
}

/// `overview` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn overview(core: Core<'_>) -> R<Overview> {
    logged("overview", core.overview().await)
}

/// `list_chats` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_chats(core: Core<'_>, query: ChatQuery) -> R<Page<ChatSummary>> {
    logged("list_chats", core.list_chats(query).await)
}

/// `get_chat` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn get_chat(core: Core<'_>, jid: String) -> R<ChatSummary> {
    logged("get_chat", core.get_chat(jid).await)
}

/// `get_messages` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn get_messages(core: Core<'_>, query: MessagesQuery) -> R<Page<Message>> {
    logged("get_messages", core.get_messages(query).await)
}

/// `get_message_context` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn get_message_context(
    core: Core<'_>,
    chat_jid: String,
    message_id: String,
    radius: u32,
) -> R<Vec<Message>> {
    logged(
        "get_message_context",
        core.get_message_context(chat_jid, message_id, radius).await,
    )
}

/// `search` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn search(core: Core<'_>, query: SearchQuery) -> R<SearchResults> {
    logged("search", core.search(query).await)
}

/// `list_media` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_media(core: Core<'_>, query: MediaQuery) -> R<Page<MediaItem>> {
    logged("list_media", core.list_media(query).await)
}

/// `ensure_media` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn ensure_media(core: Core<'_>, chat_jid: String, message_id: String) -> R<MediaRef> {
    logged(
        "ensure_media",
        core.ensure_media(chat_jid, message_id).await,
    )
}

/// `preview_doc` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn preview_doc(core: Core<'_>, chat_jid: String, message_id: String) -> R<PreviewRef> {
    logged("preview_doc", core.preview_doc(chat_jid, message_id).await)
}

/// `open_media_external` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn open_media_external(core: Core<'_>, chat_jid: String, message_id: String) -> R<()> {
    logged(
        "open_media_external",
        core.open_media_external(chat_jid, message_id).await,
    )
}

/// `save_media_as` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn save_media_as(
    core: Core<'_>,
    chat_jid: String,
    message_id: String,
    dest: String,
) -> R<String> {
    logged(
        "save_media_as",
        core.save_media_as(chat_jid, message_id, dest).await,
    )
}

/// `open_path` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn open_path(core: Core<'_>, path: String) -> R<()> {
    logged("open_path", core.open_path(path).await)
}

/// `reveal_path` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn reveal_path(core: Core<'_>, path: String) -> R<()> {
    logged("reveal_path", core.reveal_path(path).await)
}

/// `send_message` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn send_message(core: Core<'_>, req: SendReq) -> R<SendResult> {
    logged("send_message", core.send_message(req).await)
}

/// `react` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn react(core: Core<'_>, req: ReactReq) -> R<()> {
    logged("react", core.react(req).await)
}

/// `mark_read` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn mark_read(core: Core<'_>, chat_jid: String) -> R<()> {
    logged("mark_read", core.mark_read(chat_jid).await)
}

/// `connection_status` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn connection_status(core: Core<'_>) -> R<ConnectionState> {
    Ok(core.connection_status().await)
}

/// `reconnect` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn reconnect(core: Core<'_>) -> R<ConnectionState> {
    logged("reconnect", core.reconnect().await)
}

/// `start_job` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn start_job(core: Core<'_>, spec: JobSpec) -> R<JobRecord> {
    logged("start_job", core.start_job(spec).await)
}

/// `cancel_job` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn cancel_job(core: Core<'_>, id: i64) -> R<()> {
    logged("cancel_job", core.cancel_job(id).await)
}

/// `list_jobs` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_jobs(core: Core<'_>, limit: u32) -> R<Vec<JobRecord>> {
    logged("list_jobs", core.list_jobs(limit).await)
}

/// `list_rules` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_rules(core: Core<'_>) -> R<Vec<Rule>> {
    logged("list_rules", core.list_rules().await)
}

/// `save_rule` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn save_rule(core: Core<'_>, rule: Rule) -> R<Rule> {
    logged("save_rule", core.save_rule(rule).await)
}

/// `delete_rule` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn delete_rule(core: Core<'_>, id: i64) -> R<()> {
    logged("delete_rule", core.delete_rule(id).await)
}

/// `test_rule` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn test_rule(core: Core<'_>, rule: Rule, sample: Message) -> R<RuleEval> {
    logged("test_rule", core.test_rule(rule, sample).await)
}

/// `automation_log` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn automation_log(core: Core<'_>, limit: u32) -> R<Vec<AutomationEvent>> {
    logged("automation_log", core.automation_log(limit).await)
}

/// `set_automation_enabled` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn set_automation_enabled(core: Core<'_>, enabled: bool) -> R<Settings> {
    logged(
        "set_automation_enabled",
        core.set_automation_enabled(enabled).await,
    )
}

/// `query_logs` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn query_logs(core: Core<'_>, filter: LogFilter) -> R<Vec<LogLine>> {
    logged("query_logs", core.query_logs(filter).await)
}

/// `list_issues` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_issues(core: Core<'_>) -> R<Vec<Issue>> {
    logged("list_issues", core.list_issues().await)
}

/// `resolve_issue` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn resolve_issue(core: Core<'_>, id: i64) -> R<()> {
    logged("resolve_issue", core.resolve_issue(id).await)
}

/// `clear_resolved_issues` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn clear_resolved_issues(core: Core<'_>) -> R<u32> {
    logged("clear_resolved_issues", core.clear_resolved_issues().await)
}

/// `export_issue_bundle` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn export_issue_bundle(core: Core<'_>, ids: Option<Vec<i64>>) -> R<String> {
    logged("export_issue_bundle", core.export_issue_bundle(ids).await)
}

/// `get_settings` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn get_settings(core: Core<'_>) -> R<Settings> {
    logged("get_settings", core.get_settings().await)
}

/// `set_settings` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn set_settings(core: Core<'_>, settings: Settings) -> R<Settings> {
    logged("set_settings", core.set_settings(settings).await)
}
