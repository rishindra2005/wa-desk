//! Seams between crates. `wa-db` implements `MessageStore` + `AppStore` +
//! `LinkStore`; `wa-bridge` implements `BridgeApi`; `wa-obs` implements
//! `IssueSink`; the Tauri app implements `Notifier` and `ProgressSink`.
//! `wa-jobs` and `wa-automation` depend only on these traits (mock them in tests).

use std::path::PathBuf;

use async_trait::async_trait;

use crate::error::{ErrorKind, Result};
use crate::types::{
    AutomationEvent, ConnectionState, JobRecord, MediaKind, MediaSource, Message, Rule, Settings,
};

// ---------- bridge ----------

/// Low-level send request (maps 1:1 to the bridge's POST /api/send).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BridgeSend {
    pub recipient: String,
    pub message: Option<String>,
    /// Must already be inside the outbox (use `wa_bridge::outbox::stage`).
    pub media_path: Option<PathBuf>,
    pub quoted_message_id: Option<String>,
    pub quoted_sender_jid: Option<String>,
    pub quoted_content: Option<String>,
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadOutcome {
    pub path: PathBuf,
    pub filename: String,
}

// ---------- bridge metadata (docs/BRIDGE_API.md) ----------

/// `business` of `GET /api/profile`.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(default)]
pub struct BridgeBusiness {
    pub description: Option<String>,
    pub category: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub address: Option<String>,
}

/// `GET /api/profile?jid=`.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(default)]
pub struct BridgeProfile {
    pub jid: String,
    pub push_name: Option<String>,
    pub verified_name: Option<String>,
    pub about: Option<String>,
    pub picture_id: Option<String>,
    pub business: Option<BridgeBusiness>,
}

/// One participant of `GET /api/group`.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(default)]
pub struct BridgeParticipant {
    pub jid: String,
    pub phone_jid: Option<String>,
    pub lid: Option<String>,
    pub is_admin: bool,
    pub is_super_admin: bool,
    pub display_name: Option<String>,
}

/// `{jid, name}` entry of `sub_groups`.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(default)]
pub struct BridgeSubGroup {
    pub jid: String,
    pub name: Option<String>,
}

/// `GET /api/group?jid=`.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(default)]
pub struct BridgeGroup {
    pub jid: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    /// RFC3339 creation time.
    pub created_at: Option<String>,
    pub creator: Option<String>,
    pub is_parent: bool,
    pub linked_parent_jid: Option<String>,
    pub announce: bool,
    pub locked: bool,
    pub participants: Vec<BridgeParticipant>,
    pub sub_groups: Vec<BridgeSubGroup>,
}

/// `GET /api/newsletter?jid=`.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(default)]
pub struct BridgeNewsletter {
    pub jid: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub subscribers: Option<u64>,
    pub verified: bool,
    pub picture_id: Option<String>,
}

/// `GET /api/avatar`: a cached picture file in the bridge store.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAvatar {
    pub path: PathBuf,
    pub picture_id: Option<String>,
}

/// `POST /api/chats/refresh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(default)]
pub struct RefreshStats {
    pub groups: u32,
    pub communities: u32,
    pub channels: u32,
    pub renamed: u32,
    pub cached: bool,
}

fn unsupported<T>() -> Result<T> {
    Err(crate::error::AppError::not_found(
        "This bridge does not support that request",
    ))
}

#[async_trait]
pub trait BridgeApi: Send + Sync {
    /// GET /api/connection (Unreachable state if the bridge is down — never Err for that).
    async fn connection(&self) -> ConnectionState;
    async fn reconnect(&self) -> Result<()>;
    /// Returns the bridge's status text on success.
    async fn send(&self, req: BridgeSend) -> Result<String>;
    async fn react(
        &self,
        chat_jid: &str,
        message_id: &str,
        emoji: &str,
        from_me: bool,
        sender_jid: Option<&str>,
    ) -> Result<()>;
    /// POST /api/download. `no_retry=true` never asks the phone to re-upload.
    async fn download(
        &self,
        chat_jid: &str,
        message_id: &str,
        no_retry: bool,
    ) -> Result<DownloadOutcome>;
    async fn mark_read(
        &self,
        chat_jid: &str,
        message_ids: &[String],
        sender_jid: Option<&str>,
    ) -> Result<()>;

    /// GET /api/profile (contacts). `NotFound` when the bridge lacks it.
    async fn profile(&self, _jid: &str) -> Result<BridgeProfile> {
        unsupported()
    }
    /// GET /api/group (groups and communities).
    async fn group(&self, _jid: &str) -> Result<BridgeGroup> {
        unsupported()
    }
    /// GET /api/newsletter (channels).
    async fn newsletter(&self, _jid: &str) -> Result<BridgeNewsletter> {
        unsupported()
    }
    /// GET /api/avatar: `NotFound` when the chat has no picture.
    async fn avatar(&self, _jid: &str, _full: bool) -> Result<BridgeAvatar> {
        unsupported()
    }
    /// POST /api/chats/refresh: names/types of groups, communities, channels.
    async fn refresh_chats(&self) -> Result<RefreshStats> {
        unsupported()
    }
}

// ---------- message data (read side of messages.db) ----------

/// A new row seen by the watcher, with the raw fields automation needs.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingMessage {
    pub rowid: i64,
    pub message: Message,
    pub is_group: bool,
    pub chat_name: String,
    /// Raw `media_type` column ("" text, "reaction", "image", ...).
    pub raw_media_type: String,
    /// True for status@broadcast and similar non-chat sources.
    pub is_status: bool,
}

/// Everything needed to locate/download/link one media message.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub chat_jid: String,
    pub message_id: String,
    pub kind: MediaKind,
    pub filename: Option<String>,
    pub file_length: Option<u64>,
    pub file_sha256: Option<Vec<u8>>,
    /// Raw timestamp text (needed to rebuild the bridge store path).
    pub raw_ts: String,
    pub ts: i64,
    /// Existing local file, if any.
    pub local_path: Option<PathBuf>,
    pub source: MediaSource,
}

pub trait MessageStore: Send + Sync {
    fn max_rowid(&self) -> Result<i64>;
    /// Rows with rowid > `after_rowid`, oldest first, at most `limit`.
    fn messages_after(&self, after_rowid: i64, limit: u32) -> Result<Vec<IncomingMessage>>;
    fn get_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Message>>;
    fn media_info(&self, chat_jid: &str, message_id: &str) -> Result<Option<MediaInfo>>;
    /// Media rows newer than `since` (epoch secs; None = all) of the given kinds (empty = all).
    /// Revoked (deleted) messages are excluded.
    fn media_rows(&self, since: Option<i64>, kinds: &[MediaKind]) -> Result<Vec<MediaInfo>>;
}

// ---------- phone-media links ----------

#[derive(Debug, Clone, PartialEq)]
pub struct LinkRecord {
    pub chat_jid: String,
    pub message_id: String,
    pub kind: MediaKind,
    pub source_path: PathBuf,
    /// Where the file now sits in the bridge store (hard link or copy).
    pub store_path: Option<PathBuf>,
    /// "hash" | "name+size"
    pub matched_by: String,
}

pub trait LinkStore: Send + Sync {
    /// Upsert links (app.db) and mirror them to messages.db `phone_media_links` in short transactions.
    fn upsert_links(&self, links: &[LinkRecord]) -> Result<()>;
}

// ---------- app state (app.db) ----------

pub trait AppStore: Send + Sync {
    fn get_settings(&self) -> Result<Settings>;
    fn put_settings(&self, s: &Settings) -> Result<()>;

    fn list_rules(&self) -> Result<Vec<Rule>>;
    /// Insert (id None) or update; returns the stored rule with id.
    fn save_rule(&self, rule: &Rule) -> Result<Rule>;
    fn delete_rule(&self, id: i64) -> Result<()>;
    fn record_rule_fired(&self, id: i64, at: i64) -> Result<()>;
    fn log_automation(&self, ev: &AutomationEvent) -> Result<()>;
    fn recent_automation(&self, limit: u32) -> Result<Vec<AutomationEvent>>;

    /// Small key/value store (watermarks, versions).
    fn get_kv(&self, key: &str) -> Result<Option<String>>;
    fn set_kv(&self, key: &str, value: &str) -> Result<()>;

    /// Insert a new job (id ignored); returns the id.
    fn insert_job(&self, job: &JobRecord) -> Result<i64>;
    fn update_job(&self, job: &JobRecord) -> Result<()>;
    fn list_jobs(&self, limit: u32) -> Result<Vec<JobRecord>>;
    /// On startup: mark Running/Queued jobs from a previous run as Failed("interrupted").
    fn fail_interrupted_jobs(&self) -> Result<u32>;
}

// ---------- side channels ----------

pub trait ProgressSink: Send + Sync {
    fn progress(&self, done: u64, total: u64, phase: &str, message: Option<&str>);
    fn is_cancelled(&self) -> bool;
    /// Record an output file produced by the job.
    fn output(&self, _path: &std::path::Path) {}
}

pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

pub trait IssueSink: Send + Sync {
    /// Record (deduplicated) a problem worth showing in the Issues view.
    fn record(&self, kind: ErrorKind, source: &str, title: &str, detail: Option<&str>);
}

/// No-op implementations for tests and headless use.
pub mod noop {
    use super::*;

    pub struct NoProgress;
    impl ProgressSink for NoProgress {
        fn progress(&self, _: u64, _: u64, _: &str, _: Option<&str>) {}
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    pub struct NoNotify;
    impl Notifier for NoNotify {
        fn notify(&self, _: &str, _: &str) {}
    }

    pub struct NoIssues;
    impl IssueSink for NoIssues {
        fn record(&self, _: ErrorKind, _: &str, _: &str, _: Option<&str>) {}
    }
}
