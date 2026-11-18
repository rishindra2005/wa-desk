//! Thin `#[tauri::command]` wrappers over [`AppCore`]; names and arguments
//! follow docs/CONTRACT.md exactly (JS camelCase → Rust snake_case).

use std::sync::Arc;

use tauri::State;
use wa_core::AppError;
use wa_core::types::{
    AutomationEvent, CallQuery, CallRecord, ChatQuery, ChatStats, ChatSummary, ConnectionState,
    ExportReq, ForwardReq, Issue, JobRecord, JobSpec, LinkItem, LogFilter, LogLine, MediaItem,
    MediaQuery, MediaRef, Message, MessagesQuery, Overview, Page, PreviewRef, Profile, ReactReq,
    Rule, RuleEval, SearchHit, SearchQuery, SearchResults, SendReq, SendResult, Settings,
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

/// `stage_pasted_file` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn stage_pasted_file(
    core: Core<'_>,
    name: Option<String>,
    mime: Option<String>,
    data_base64: String,
) -> R<String> {
    logged(
        "stage_pasted_file",
        core.stage_pasted_file(name, mime, data_base64).await,
    )
}

/// `copy_image` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn copy_image(core: Core<'_>, chat_jid: String, message_id: String) -> R<()> {
    logged("copy_image", core.copy_image(chat_jid, message_id).await)
}

/// `set_zoom` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn set_zoom(core: Core<'_>, percent: u32) -> R<u32> {
    logged("set_zoom", core.set_zoom(percent).await)
}

/// `react` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn react(core: Core<'_>, req: ReactReq) -> R<()> {
    logged("react", core.react(req).await)
}

/// `list_calls` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_calls(core: Core<'_>, query: CallQuery) -> R<Page<CallRecord>> {
    logged("list_calls", core.list_calls(query).await)
}

/// `reject_call` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn reject_call(core: Core<'_>, call_id: String, caller_jid: String) -> R<()> {
    logged("reject_call", core.reject_call(call_id, caller_jid).await)
}

/// `archive_count` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn archive_count(core: Core<'_>) -> R<u32> {
    logged("archive_count", core.archive_count().await)
}

/// `set_archived` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn set_archived(core: Core<'_>, jid: String, archived: bool) -> R<()> {
    logged("set_archived", core.set_archived(jid, archived).await)
}

/// `set_pinned` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn set_pinned(core: Core<'_>, jid: String, pinned: bool) -> R<()> {
    logged("set_pinned", core.set_pinned(jid, pinned).await)
}

/// `set_muted` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn set_muted(core: Core<'_>, jid: String, until: Option<i64>) -> R<()> {
    logged("set_muted", core.set_muted(jid, until).await)
}

/// `resync_chat_state` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn resync_chat_state(core: Core<'_>) -> R<u32> {
    logged("resync_chat_state", core.resync_chat_state().await)
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

/// `get_profile` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn get_profile(core: Core<'_>, jid: String) -> R<Profile> {
    logged("get_profile", core.get_profile(jid).await)
}

/// `chat_stats` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn chat_stats(core: Core<'_>, chat_jid: String) -> R<ChatStats> {
    logged("chat_stats", core.chat_stats(chat_jid).await)
}

/// `list_links` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_links(
    core: Core<'_>,
    chat_jid: Option<String>,
    cursor: Option<String>,
    limit: u32,
) -> R<Page<LinkItem>> {
    logged("list_links", core.list_links(chat_jid, cursor, limit).await)
}

/// `set_starred` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn set_starred(
    core: Core<'_>,
    chat_jid: String,
    message_ids: Vec<String>,
    starred: bool,
) -> R<()> {
    logged(
        "set_starred",
        core.set_starred(chat_jid, message_ids, starred).await,
    )
}

/// `list_starred` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn list_starred(
    core: Core<'_>,
    chat_jid: Option<String>,
    cursor: Option<String>,
    limit: u32,
) -> R<Page<SearchHit>> {
    logged(
        "list_starred",
        core.list_starred(chat_jid, cursor, limit).await,
    )
}

/// `export_chat` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn export_chat(core: Core<'_>, req: ExportReq) -> R<String> {
    logged("export_chat", core.export_chat(req).await)
}

/// `forward_messages` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn forward_messages(core: Core<'_>, req: ForwardReq) -> R<Vec<SendResult>> {
    logged("forward_messages", core.forward_messages(req).await)
}

/// `copy_messages_text` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn copy_messages_text(
    core: Core<'_>,
    chat_jid: String,
    message_ids: Vec<String>,
) -> R<String> {
    logged(
        "copy_messages_text",
        core.copy_messages_text(chat_jid, message_ids).await,
    )
}

/// `refresh_chat_names` (see docs/CONTRACT.md).
#[tauri::command]
pub async fn refresh_chat_names(core: Core<'_>) -> R<u32> {
    logged("refresh_chat_names", core.refresh_chat_names().await)
}
