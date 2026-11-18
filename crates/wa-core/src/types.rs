//! Domain types shared by every crate and exported to TypeScript (ts-rs).
//!
//! Conventions:
//! - Times are Unix **seconds** (UTC) as `i64`, exported to TS as `number`.
//! - Optional values serialise as `null`.
//! - Enums serialise snake_case; tagged unions use `{"type": "...", ...}`.
//! - Paths never leave Rust except as `wa-media://` URLs (see `MediaRef::url`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------- paging ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Opaque cursor for the next page; `null` when exhausted.
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

// ---------- chats ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChatKind {
    #[default]
    All,
    Direct,
    /// Ordinary groups (excludes communities' announcement parents).
    Group,
    Community,
    Channel,
    Broadcast,
}

/// What a chat actually is (for icons, labels and filtering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChatType {
    #[default]
    Direct,
    Group,
    /// Community parent (the community itself).
    Community,
    /// WhatsApp channel (`@newsletter`).
    Channel,
    /// Broadcast list (`@broadcast`, not status).
    Broadcast,
    /// `status@broadcast`.
    Status,
}

impl ChatType {
    /// Best effort from the JID alone (communities need bridge metadata).
    pub fn from_jid(jid: &str) -> Self {
        if jid == "status@broadcast" {
            Self::Status
        } else if jid.ends_with("@g.us") {
            Self::Group
        } else if jid.ends_with("@newsletter") {
            Self::Channel
        } else if jid.ends_with("@broadcast") {
            Self::Broadcast
        } else {
            Self::Direct
        }
    }
}

/// Lightweight reference to another chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChatRef {
    pub jid: String,
    pub name: String,
    pub chat_type: ChatType,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChatQuery {
    /// Case-insensitive substring on resolved name or JID.
    pub filter: Option<String>,
    #[serde(default)]
    pub kind: ChatKind,
    #[serde(default)]
    pub unread_only: bool,
    /// false (default) = hide archived chats; true = only archived chats.
    #[serde(default)]
    pub archived: bool,
    pub cursor: Option<String>,
    /// 1..=500, default 100.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChatSummary {
    pub jid: String,
    /// Resolved display name (never empty; falls back to the phone/JID).
    pub name: String,
    pub is_group: bool,
    pub chat_type: ChatType,
    /// For groups inside a community: the community.
    pub parent: Option<ChatRef>,
    /// `wa-media://localhost/a/<jid>` (404 when the chat has no picture).
    pub avatar_url: String,
    #[ts(type = "number | null")]
    pub last_message_ts: Option<i64>,
    /// First ~120 chars of the last message, or a media label like "📄 file.pdf".
    pub last_preview: Option<String>,
    pub last_sender_name: Option<String>,
    pub last_is_from_me: bool,
    /// Delivery status of the last message when it is ours.
    pub last_status: Option<MessageStatus>,
    pub unread: bool,
    /// Archived on WhatsApp (synced app state).
    pub archived: bool,
    /// Pinned on WhatsApp; pinned chats sort first.
    pub pinned: bool,
    /// Muted until (epoch secs); `-1` = muted forever; null = not muted.
    #[ts(type = "number | null")]
    pub muted_until: Option<i64>,
}

// ---------- messages ----------

/// WhatsApp delivery ticks for our own messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MessageStatus {
    /// Accepted by the server (single grey tick). Default when no receipt is known.
    Sent,
    /// Delivered to the recipient's device (double grey tick).
    Delivered,
    /// Read (double blue tick).
    Read,
    /// Voice note / video played (blue mic/tick).
    Played,
}

impl MessageStatus {
    /// Map a stored bridge receipt status.
    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "sent" | "server" => Some(Self::Sent),
            "delivered" => Some(Self::Delivered),
            "read" => Some(Self::Read),
            "played" => Some(Self::Played),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Document,
    Sticker,
}

impl MediaKind {
    /// Map the bridge's `media_type` column. Reactions/empty → None.
    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "document" => Some(Self::Document),
            "sticker" => Some(Self::Sticker),
            _ => None,
        }
    }
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::Sticker => "sticker",
        }
    }
}

/// Where a locally available media file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MediaSource {
    /// Downloaded by the bridge into its store.
    Store,
    /// Linked from the phone copy (phone_media_links).
    Phone,
    /// Not on disk yet; `ensure_media` can try to fetch it.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaRef {
    pub kind: MediaKind,
    /// Original file name for documents; generated label otherwise.
    pub filename: Option<String>,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    /// Best-guess MIME type (from name/kind).
    pub mime: Option<String>,
    pub source: MediaSource,
    /// `wa-media://m/<chat>/<id>` when available locally, else null.
    pub url: Option<String>,
    /// `wa-media://t/<chat>/<id>` thumbnail (images/videos/pdfs) when available.
    pub thumb_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QuotedRef {
    pub id: String,
    pub sender_name: Option<String>,
    /// Short preview of the quoted message if we have it locally.
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Reaction {
    pub emoji: String,
    pub sender_name: String,
    pub is_from_me: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Message {
    pub id: String,
    pub chat_jid: String,
    /// Normalised sender JID (phone@s.whatsapp.net when resolvable), null for own messages without sender.
    pub sender_jid: Option<String>,
    /// "Me" for own messages; resolved contact name or phone otherwise.
    pub sender_name: String,
    pub is_from_me: bool,
    #[ts(type = "number")]
    pub ts: i64,
    /// Text content or caption ("" for pure media).
    pub content: String,
    pub media: Option<MediaRef>,
    pub quoted: Option<QuotedRef>,
    pub reactions: Vec<Reaction>,
    /// Revoked ("This message was deleted").
    pub deleted: bool,
    /// Delivery status (own messages only; null for inbound).
    pub status: Option<MessageStatus>,
    /// Bookmarked locally (wa-desk "starred").
    pub starred: bool,
}

/// Opaque cursor for paging a thread backwards: encoded "<ts>:<rowid>".
pub type MsgCursor = String;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessagesQuery {
    pub chat_jid: String,
    /// Return messages strictly older than this cursor (null = newest page).
    pub before: Option<MsgCursor>,
    /// Return messages strictly newer than this cursor, ascending (forward paging
    /// after jumping to an old message). Mutually exclusive with `before`.
    pub after: Option<MsgCursor>,
    /// 1..=500, default 80.
    pub limit: Option<u32>,
}

// ---------- search ----------

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchQuery {
    /// Free text. 1–2 char queries fall back to substring scan.
    pub text: String,
    #[serde(default)]
    pub chat_jids: Vec<String>,
    /// Sender JIDs or bare phone numbers.
    #[serde(default)]
    pub senders: Vec<String>,
    #[serde(default)]
    pub media_kinds: Vec<MediaKind>,
    pub from_me: Option<bool>,
    #[ts(type = "number | null")]
    pub after: Option<i64>,
    #[ts(type = "number | null")]
    pub before: Option<i64>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchHit {
    pub message: Message,
    pub chat_name: String,
    /// Byte ranges into `message.content` (or filename) that matched, for highlighting.
    pub highlights: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Total matches (may be capped at 10_000).
    pub total: u32,
    pub took_ms: u32,
    /// True while the FTS index is still being built (results may be incomplete).
    pub index_building: bool,
}

// ---------- media library ----------

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaQuery {
    pub chat_jid: Option<String>,
    #[serde(default)]
    pub kinds: Vec<MediaKind>,
    /// Substring on filename / caption.
    pub text: Option<String>,
    #[ts(type = "number | null")]
    pub after: Option<i64>,
    #[ts(type = "number | null")]
    pub before: Option<i64>,
    /// true = only local files, false = only missing, null = both.
    pub available: Option<bool>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaItem {
    pub message_id: String,
    pub chat_jid: String,
    pub chat_name: String,
    /// Normalised sender JID (null for own messages).
    pub sender_jid: Option<String>,
    pub sender_name: String,
    pub is_from_me: bool,
    #[ts(type = "number")]
    pub ts: i64,
    pub caption: String,
    pub media: MediaRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PreviewKind {
    Pdf,
    Image,
    Video,
    Audio,
    Text,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PreviewRef {
    pub kind: PreviewKind,
    /// wa-media URL of the previewable file (a cached PDF for office docs).
    pub url: Option<String>,
    /// Original document URL (for "open externally"/save).
    pub original_url: Option<String>,
    pub note: Option<String>,
}

// ---------- sending ----------

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SendReq {
    /// Chat JID or bare phone number.
    pub recipient: String,
    pub text: Option<String>,
    /// Absolute local path chosen by the user (copied into the outbox before sending).
    pub attachment: Option<String>,
    /// Message id to reply to (must be in the same chat).
    pub reply_to: Option<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SendResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReactReq {
    pub chat_jid: String,
    pub message_id: String,
    /// "" removes our reaction.
    pub emoji: String,
}

// ---------- bridge connection ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BridgeState {
    Connected,
    Connecting,
    Pairing,
    Disconnected,
    LoggedOut,
    Outdated,
    Error,
    /// Bridge process not reachable at all.
    Unreachable,
}

impl BridgeState {
    pub fn from_bridge(s: &str) -> Self {
        match s {
            "connected" => Self::Connected,
            "connecting" => Self::Connecting,
            "pairing" => Self::Pairing,
            "disconnected" => Self::Disconnected,
            "logged_out" => Self::LoggedOut,
            "outdated" => Self::Outdated,
            _ => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConnectionState {
    pub state: BridgeState,
    pub connected: bool,
    pub paired: bool,
    pub message: String,
    /// SVG data URL of the pairing QR (only while pairing).
    pub qr_svg: Option<String>,
    #[ts(type = "number | null")]
    pub qr_expires_at: Option<i64>,
    pub last_error: Option<String>,
    #[ts(type = "number")]
    pub checked_at: i64,
}

// ---------- jobs ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum JobSpec {
    /// Download media still on WhatsApp servers (never asks the phone).
    Backfill {
        days: u32,
        #[serde(default)]
        kinds: Vec<MediaKind>,
        workers: u8,
        #[serde(default)]
        retry_failed: bool,
    },
    /// Copy WhatsApp/Media from a USB (MTP) phone into phone_media.
    PhoneSync,
    /// Hash phone_media + extra folders and link files to messages.
    LinkPhoneMedia {
        #[serde(default)]
        extra_dirs: Vec<String>,
    },
    Convert(ConvertSpec),
    /// Rebuild the search index from scratch.
    Reindex,
    /// Generate thumbnails for local media.
    Thumbnails,
}

impl JobSpec {
    pub fn label(&self) -> String {
        match self {
            Self::Backfill { days, .. } => format!("Media backfill (last {days} days)"),
            Self::PhoneSync => "Phone media sync (USB)".into(),
            Self::LinkPhoneMedia { .. } => "Link phone media to messages".into(),
            Self::Convert(c) => format!("Convert: {}", c.kind.label()),
            Self::Reindex => "Rebuild search index".into(),
            Self::Thumbnails => "Generate thumbnails".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobRecord {
    #[ts(type = "number")]
    pub id: i64,
    pub spec: JobSpec,
    pub label: String,
    pub status: JobStatus,
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
    pub phase: String,
    pub message: Option<String>,
    pub error: Option<String>,
    /// Output files produced (conversions), as wa-media URLs.
    pub outputs: Vec<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobProgress {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
    pub phase: String,
    pub message: Option<String>,
}

// ---------- combined media export ----------

/// One message's media, addressed by chat and message id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaTarget {
    pub chat_jid: String,
    pub message_id: String,
}

/// How `export_media_files` writes its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum MediaExportMode {
    /// One `.zip` file; `dest` is the zip path chosen in a save dialog.
    Zip,
    /// Individual files in an existing folder; `dest` is that folder.
    Folder,
}

/// Arguments of `export_media_files`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaExportReq {
    pub items: Vec<MediaTarget>,
    pub mode: MediaExportMode,
    /// Zip file path (`zip`) or folder path (`folder`), from the dialog.
    pub dest: String,
    /// Echoed in progress events so the UI can match them.
    pub request_id: String,
}

/// Payload of the `media-export:progress` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaExportProgress {
    pub request_id: String,
    #[ts(type = "number")]
    pub done: u32,
    #[ts(type = "number")]
    pub total: u32,
    /// File being processed.
    pub current: Option<String>,
}

/// An item that could not be exported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaExportFailure {
    pub chat_jid: String,
    pub message_id: String,
    pub reason: String,
}

/// Final result of `export_media_files`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaExportSummary {
    pub request_id: String,
    #[ts(type = "number")]
    pub saved: u32,
    #[ts(type = "number")]
    pub failed: u32,
    pub failures: Vec<MediaExportFailure>,
    /// The zip file or folder written; `null` when nothing was saved.
    pub dest: Option<String>,
}

// ---------- conversions ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ImageFormat {
    Jpeg,
    Webp,
    Png,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ConvertKind {
    /// docx/pptx/xlsx/odt/... → PDF via LibreOffice.
    OfficeToPdf,
    /// Resize (longest side ≤ max_dim) and re-encode.
    ImageCompress {
        quality: u8,
        max_dim: u32,
        format: ImageFormat,
    },
    /// H.264/AAC MP4 at CRF, height ≤ max_height.
    VideoCompress { crf: u8, max_height: u32 },
    /// Ghostscript: "screen" | "ebook" | "printer".
    PdfCompress { level: String },
    /// Any audio → Ogg Opus (WhatsApp voice-note compatible).
    AudioToOpus { bitrate_kbps: u32 },
    /// Render PDF pages to PNG images.
    PdfToImages { dpi: u32 },
}

impl ConvertKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::OfficeToPdf => "Office → PDF",
            Self::ImageCompress { .. } => "Compress image",
            Self::VideoCompress { .. } => "Compress video",
            Self::PdfCompress { .. } => "Compress PDF",
            Self::AudioToOpus { .. } => "Audio → Opus",
            Self::PdfToImages { .. } => "PDF → images",
        }
    }
}

/// Where a conversion reads from: a local file the user picked, or a message's media.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ConvertInput {
    File {
        path: String,
    },
    Message {
        chat_jid: String,
        message_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConvertSpec {
    pub input: ConvertInput,
    pub kind: ConvertKind,
    /// Defaults to the exports dir.
    pub output_dir: Option<String>,
}

// ---------- automation ----------

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Trigger {
    /// Empty = any direct chat (groups need `include_groups` + listing them here).
    #[serde(default)]
    pub chat_jids: Vec<String>,
    #[serde(default)]
    pub include_groups: bool,
    #[serde(default)]
    pub senders: Vec<String>,
    /// Any of these (case-insensitive substring) must appear; empty = no keyword filter.
    #[serde(default)]
    pub keywords: Vec<String>,
    pub regex: Option<String>,
    #[serde(default)]
    pub media_kinds: Vec<MediaKind>,
    /// Local time window "HH:MM"; both null = always.
    pub active_from: Option<String>,
    pub active_to: Option<String>,
    /// In groups, only fire when we are @mentioned (always true for replies in groups).
    #[serde(default)]
    pub mention_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Action {
    Reply { text: String },
    DownloadMedia,
    Notify { title: Option<String> },
    Forward { to_jid: String },
    Tag { label: String },
    Webhook { url: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Rule {
    #[ts(type = "number | null")]
    pub id: Option<i64>,
    pub name: String,
    pub enabled: bool,
    /// Log what would happen instead of doing it (default true).
    pub dry_run: bool,
    pub trigger: Trigger,
    pub actions: Vec<Action>,
    /// Minimum seconds between firings in the same chat (default 600).
    pub cooldown_secs: u32,
    #[ts(type = "number")]
    pub fired_count: u64,
    #[ts(type = "number | null")]
    pub last_fired_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RuleEval {
    pub matched: bool,
    /// Human-readable reasons for match / non-match (each check).
    pub reasons: Vec<String>,
    /// What would be executed.
    pub planned: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AutomationEvent {
    #[ts(type = "number")]
    pub rule_id: i64,
    pub rule_name: String,
    pub chat_jid: String,
    pub message_id: String,
    pub dry_run: bool,
    pub outcomes: Vec<String>,
    #[ts(type = "number")]
    pub at: i64,
}

// ---------- observability ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogFilter {
    /// Minimum level (default info).
    pub level: Option<LogLevel>,
    pub contains: Option<String>,
    pub target: Option<String>,
    #[ts(type = "number | null")]
    pub since: Option<i64>,
    /// Newest first, default 500, max 5000.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogLine {
    #[ts(type = "number")]
    pub ts: i64,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    /// Extra structured fields as compact JSON text ("" if none).
    pub fields: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IssueStatus {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Issue {
    #[ts(type = "number")]
    pub id: i64,
    /// Stable hash of (kind, source, normalised message) used for dedup.
    pub fingerprint: String,
    pub title: String,
    pub kind: crate::error::ErrorKind,
    /// Component that raised it, e.g. "bridge", "jobs.phone_sync".
    pub source: String,
    #[ts(type = "number")]
    pub count: u64,
    #[ts(type = "number")]
    pub first_seen: i64,
    #[ts(type = "number")]
    pub last_seen: i64,
    pub last_detail: Option<String>,
    pub status: IssueStatus,
}

// ---------- profiles ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BusinessInfo {
    pub description: Option<String>,
    pub category: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContactProfile {
    pub jid: String,
    pub name: String,
    /// Name the person set for themselves.
    pub push_name: Option<String>,
    /// "+91 98765 43210" style, when known.
    pub phone: Option<String>,
    /// "About" text.
    pub about: Option<String>,
    pub avatar_url: String,
    pub is_me: bool,
    pub business: Option<BusinessInfo>,
    pub common_groups: Vec<ChatRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Participant {
    pub jid: String,
    pub name: String,
    pub phone: Option<String>,
    pub is_admin: bool,
    pub is_super_admin: bool,
    pub is_me: bool,
    pub avatar_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GroupProfile {
    pub jid: String,
    pub name: String,
    pub chat_type: ChatType,
    pub description: Option<String>,
    #[ts(type = "number | null")]
    pub created_at: Option<i64>,
    pub created_by: Option<String>,
    pub participants: Vec<Participant>,
    pub participant_count: u32,
    /// Community this group belongs to.
    pub parent: Option<ChatRef>,
    /// Groups of a community (only for communities).
    pub sub_groups: Vec<ChatRef>,
    /// Only admins can send.
    pub announce_only: bool,
    /// Only admins can edit info.
    pub locked: bool,
    pub avatar_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChannelProfile {
    pub jid: String,
    pub name: String,
    pub description: Option<String>,
    #[ts(type = "number | null")]
    pub subscribers: Option<u64>,
    pub verified: bool,
    pub avatar_url: String,
}

/// Result of `get_profile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Profile {
    Contact(ContactProfile),
    Group(GroupProfile),
    Channel(ChannelProfile),
}

/// Per-chat counts for the info panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChatStats {
    #[ts(type = "number")]
    pub messages: u64,
    #[ts(type = "number")]
    pub media: u64,
    #[ts(type = "number")]
    pub docs: u64,
    #[ts(type = "number")]
    pub links: u64,
    #[ts(type = "number")]
    pub starred: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LinkItem {
    pub url: String,
    pub message_id: String,
    pub chat_jid: String,
    pub sender_name: String,
    #[ts(type = "number")]
    pub ts: i64,
    /// Message text around the link (≤ 200 chars).
    pub context: String,
}

// ---------- export / forward ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExportFormat {
    Html,
    Txt,
    Json,
    Pdf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportReq {
    pub chat_jid: String,
    pub format: ExportFormat,
    /// Only these messages (selection export); empty = whole chat / range.
    #[serde(default)]
    pub message_ids: Vec<String>,
    #[ts(type = "number | null")]
    pub after: Option<i64>,
    #[ts(type = "number | null")]
    pub before: Option<i64>,
    /// Copy locally available media next to the export (HTML) and link it.
    pub include_media: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForwardReq {
    pub from_chat: String,
    pub message_ids: Vec<String>,
    /// Target chat JIDs (≤ 5 per call, like WhatsApp).
    pub to: Vec<String>,
}

// ---------- calls ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CallMedia {
    Voice,
    Video,
}

/// Outcome as recorded by the bridge (`calls.result`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CallResult {
    /// Ringing / ongoing right now.
    InProgress,
    Answered,
    Ended,
    Missed,
    Rejected,
}

impl CallResult {
    pub fn from_db(s: &str) -> Self {
        match s {
            "answered" => Self::Answered,
            "ended" => Self::Ended,
            "missed" => Self::Missed,
            "rejected" => Self::Rejected,
            _ => Self::InProgress,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallRecord {
    pub call_id: String,
    /// Group JID for group calls, the caller for 1:1.
    pub chat_jid: String,
    pub chat_name: String,
    pub caller_jid: String,
    pub caller_name: String,
    pub avatar_url: String,
    pub is_from_me: bool,
    pub is_group: bool,
    pub media: CallMedia,
    pub result: CallResult,
    #[ts(type = "number")]
    pub ts: i64,
    pub duration_secs: Option<u32>,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallQuery {
    pub chat_jid: Option<String>,
    /// Only missed (and rejected) calls.
    #[serde(default)]
    pub missed_only: bool,
    pub media: Option<CallMedia>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

// ---------- settings ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Who runs the WhatsApp bridge (`Settings.bridge_mode`). Read at app start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum BridgeMode {
    /// Use a bridge that already answers; otherwise start the bundled one.
    #[default]
    Auto,
    /// Never start a bridge; the user runs it.
    External,
    /// Always start the bundled bridge (error if the port is taken).
    Bundled,
}

/// What the app's bridge supervisor is doing (System view, bridge card).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorState {
    /// Still deciding (app just started).
    Starting,
    /// A bridge not started by wa-desk answers on the configured URL.
    External,
    /// `bridge_mode = external`: wa-desk never starts a bridge.
    Disabled,
    /// No bridge answers and this build has no bundled bridge binary.
    NotInstalled,
    /// The bundled bridge is running (`pid`).
    Running,
    /// The bundled bridge exited; waiting to restart (`detail` = last error).
    Crashed,
    /// Too many fast crashes; wa-desk stopped restarting it.
    GaveUp,
    /// It cannot start (for example the port is taken); see `detail`.
    Error,
    /// Stopped because the app is quitting.
    Stopped,
}

/// Snapshot returned by `bridge_supervisor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SupervisorStatus {
    pub state: SupervisorState,
    /// Process id while `state` is `running`.
    #[ts(type = "number | null")]
    pub pid: Option<u32>,
    /// Last error or explanation, if any.
    pub detail: Option<String>,
    /// Restarts since the app started.
    pub restarts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Settings {
    pub theme: Theme,
    /// Master switch for the automation engine.
    pub automation_enabled: bool,
    /// Global cap on automated sends per hour.
    pub automation_max_per_hour: u32,
    pub desktop_notifications: bool,
    /// Ask before sending attachments or messages to groups.
    pub confirm_sends: bool,
    pub backfill_days: u32,
    pub start_hidden: bool,
    pub autostart: bool,
    /// Extra folders searched by the phone-media linker.
    pub link_extra_dirs: Vec<String>,
    /// App zoom in percent (50 to 200), applied to the whole window.
    pub zoom_percent: u32,
    /// Who runs the WhatsApp bridge. Applied at the next app start.
    pub bridge_mode: BridgeMode,
}

impl Settings {
    /// Lowest allowed app zoom, in percent.
    pub const ZOOM_MIN: u32 = 50;
    /// Highest allowed app zoom, in percent.
    pub const ZOOM_MAX: u32 = 200;

    /// Clamp a zoom request into the allowed range.
    pub fn clamp_zoom(percent: u32) -> u32 {
        percent.clamp(Self::ZOOM_MIN, Self::ZOOM_MAX)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            automation_enabled: false,
            automation_max_per_hour: 20,
            desktop_notifications: true,
            confirm_sends: true,
            backfill_days: 30,
            start_hidden: false,
            autostart: false,
            link_extra_dirs: Vec::new(),
            zoom_percent: 100,
            bridge_mode: BridgeMode::Auto,
        }
    }
}

// ---------- app overview / events ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Overview {
    #[ts(type = "number")]
    pub chats: u64,
    #[ts(type = "number")]
    pub messages: u64,
    #[ts(type = "number")]
    pub media_total: u64,
    #[ts(type = "number")]
    pub media_local: u64,
    #[ts(type = "number | null")]
    pub oldest_ts: Option<i64>,
    #[ts(type = "number | null")]
    pub newest_ts: Option<i64>,
    /// Indexed messages / total (search readiness).
    #[ts(type = "number")]
    pub indexed: u64,
    pub app_version: String,
}

/// Payload of the `db:new-messages` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NewMessagesEvent {
    pub chat_jids: Vec<String>,
    #[ts(type = "number")]
    pub max_rowid: i64,
    pub count: u32,
}

/// Payload of the `db:changed` event (edits/revokes/name changes; refresh views).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbChangedEvent {
    pub chat_jids: Vec<String>,
}

/// Payload of the `notification:open` event: the user clicked a desktop
/// notification of this chat (its notifications are already withdrawn).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NotificationOpenEvent {
    pub chat_jid: String,
}

/// Event names (keep in sync with app/ui/src/lib/api.ts).
pub mod events {
    pub const NEW_MESSAGES: &str = "db:new-messages";
    pub const DB_CHANGED: &str = "db:changed";
    pub const BRIDGE_STATE: &str = "bridge:state";
    pub const JOB_PROGRESS: &str = "job:progress";
    pub const JOB_FINISHED: &str = "job:finished";
    pub const AUTOMATION_FIRED: &str = "automation:fired";
    pub const ISSUE_NEW: &str = "issue:new";
    /// Payload `CallRecord`: a call started ringing (result in_progress) or changed state.
    pub const CALL_UPDATE: &str = "call:update";
    /// Payload `MediaExportProgress`: progress of `export_media_files`.
    pub const MEDIA_EXPORT_PROGRESS: &str = "media-export:progress";
    /// Payload `NotificationOpenEvent`: a desktop notification was clicked; open its chat.
    pub const NOTIFICATION_OPEN: &str = "notification:open";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_enums_roundtrip() {
        let j = serde_json::to_value(JobSpec::Backfill {
            days: 30,
            kinds: vec![MediaKind::Document],
            workers: 4,
            retry_failed: false,
        })
        .unwrap();
        assert_eq!(j["type"], "backfill");
        assert_eq!(j["kinds"][0], "document");
        let a: Action =
            serde_json::from_value(serde_json::json!({"type":"reply","text":"hi"})).unwrap();
        assert_eq!(a, Action::Reply { text: "hi".into() });
        let c: ConvertSpec = serde_json::from_value(serde_json::json!({
            "input": {"type":"message","chat_jid":"a@g.us","message_id":"X"},
            "kind": {"type":"image_compress","quality":80,"max_dim":1600,"format":"webp"},
            "output_dir": null
        }))
        .unwrap();
        assert!(matches!(
            c.kind,
            ConvertKind::ImageCompress {
                format: ImageFormat::Webp,
                ..
            }
        ));
    }

    #[test]
    fn query_defaults_are_lenient() {
        let q: SearchQuery = serde_json::from_value(serde_json::json!({"text":"hello"})).unwrap();
        assert!(q.chat_jids.is_empty() && q.limit.is_none());
        let q: ChatQuery = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(q.kind, ChatKind::All);
    }

    #[test]
    fn media_kind_mapping() {
        for k in [
            MediaKind::Image,
            MediaKind::Video,
            MediaKind::Audio,
            MediaKind::Document,
            MediaKind::Sticker,
        ] {
            assert_eq!(MediaKind::from_db(k.as_db()), Some(k));
        }
        assert_eq!(MediaKind::from_db("reaction"), None);
        assert_eq!(MediaKind::from_db(""), None);
    }

    #[test]
    fn bridge_state_mapping() {
        assert_eq!(
            BridgeState::from_bridge("logged_out"),
            BridgeState::LoggedOut
        );
        assert_eq!(BridgeState::from_bridge("weird"), BridgeState::Error);
    }

    #[test]
    fn chat_type_from_jid() {
        assert_eq!(ChatType::from_jid("status@broadcast"), ChatType::Status);
        assert_eq!(ChatType::from_jid("1234@broadcast"), ChatType::Broadcast);
        assert_eq!(ChatType::from_jid("1203@g.us"), ChatType::Group);
        assert_eq!(ChatType::from_jid("1203@newsletter"), ChatType::Channel);
        assert_eq!(ChatType::from_jid("9198@s.whatsapp.net"), ChatType::Direct);
        assert_eq!(ChatType::from_jid("2222@lid"), ChatType::Direct);
    }

    #[test]
    fn message_status_order_and_mapping() {
        assert!(MessageStatus::Sent < MessageStatus::Delivered);
        assert!(MessageStatus::Delivered < MessageStatus::Read);
        assert!(MessageStatus::Read < MessageStatus::Played);
        assert_eq!(MessageStatus::from_db("read"), Some(MessageStatus::Read));
        assert_eq!(MessageStatus::from_db("bogus"), None);
    }

    #[test]
    fn profile_is_tagged() {
        let p = Profile::Channel(ChannelProfile {
            jid: "1@newsletter".into(),
            name: "News".into(),
            description: None,
            subscribers: Some(5),
            verified: true,
            avatar_url: String::new(),
        });
        assert_eq!(serde_json::to_value(&p).unwrap()["type"], "channel");
    }

    #[test]
    fn settings_default_is_safe() {
        let s = Settings::default();
        assert!(!s.automation_enabled);
        assert!(s.confirm_sends);
    }
}
