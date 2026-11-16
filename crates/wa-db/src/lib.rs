//! Read-only access to the bridge DBs, name resolution, queries, app.db,
//! search index and change watcher.
//!
//! # Threading model
//! Every API here is **synchronous and blocking**. The Tauri app calls it
//! through `tokio::task::spawn_blocking`; nothing here ever awaits, so no
//! SQLite statement or transaction can be held across an `.await`.
//!
//! `Db` is `Send + Sync` and meant to live in an `Arc`. Internally:
//! - messages.db / whatsapp.db are read through small read-only connection
//!   pools (`bridge_ro`): `mode=ro`, `query_only`, busy_timeout 1 s and a
//!   300 ms per-statement deadline. Each helper drains and resets its
//!   statement before returning, so no lock outlives a call.
//! - app.db has one writer connection (mutex) and a pool of readers (WAL).
//! - The index (`Indexer`) serialises its own runs; the watcher only runs an
//!   incremental pass when no other index operation is in progress.
//! - The only write to messages.db is `LinkStore::upsert_links` (mirroring
//!   `phone_media_links`) on a separate short-lived connection in short
//!   immediate transactions of ≤ 200 rows.

pub mod app_db;
pub mod bridge_ro;
pub mod calls;
pub mod chats;
pub mod indexer;
pub mod insights;
pub mod media;
pub mod messages;
pub mod names;
pub mod raw;
mod receipts;
pub mod search;
mod util;
pub mod watcher;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_calls;
#[cfg(test)]
mod tests_v2;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use wa_core::config::Paths;
use wa_core::time::{epoch_to_rfc3339, now_epoch};
use wa_core::traits::{AppStore, IncomingMessage, LinkRecord, LinkStore, MediaInfo, MessageStore};
use wa_core::types::{
    AutomationEvent, CallQuery, CallRecord, ChatQuery, ChatRef, ChatStats, ChatSummary, JobRecord,
    LinkItem, MediaItem, MediaKind, MediaQuery, Message, MessagesQuery, Overview, Page, Rule,
    SearchHit, SearchQuery, SearchResults, Settings,
};
use wa_core::{AppError, Result};

pub use app_db::AppDb;
pub use bridge_ro::{RoConn, RoPool};
pub use indexer::{Indexer, ReconcileStats};
pub use names::{ChatMeta, NameCache, NameResolver};
pub use watcher::{WatchOutcome, Watcher};

use crate::bridge_ro::DEFAULT_DEADLINE;
use crate::names::{is_group_jid, is_status_jid};
use crate::raw::RawMsg;
use crate::util::sql_err;

/// Rows per transaction when mirroring links into messages.db.
pub const LINK_MIRROR_BATCH: usize = 200;

/// Shared state behind `Db`, `Indexer` and `Watcher`.
pub(crate) struct Core {
    pub paths: Paths,
    pub bridge: Arc<RoPool>,
    pub names: Arc<NameResolver>,
    pub app: Arc<AppDb>,
    pub building: AtomicBool,
    pub index_lock: Mutex<()>,
}

impl Core {
    /// Index complete and no full build running.
    pub(crate) fn index_ready(&self) -> Result<bool> {
        Ok(!self.building.load(Ordering::SeqCst) && self.app.index_complete()?)
    }
}

/// Build `IncomingMessage`s for raw rows (any kind).
pub(crate) fn incoming(
    core: &Core,
    conn: &RoConn,
    rows: Vec<RawMsg>,
) -> Result<Vec<IncomingMessage>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let names = core.names.snapshot()?;
    let msgs = messages::assemble(core, &names, conn, &rows)?;
    Ok(rows
        .into_iter()
        .zip(msgs)
        .map(|(r, m)| IncomingMessage {
            rowid: r.rowid,
            is_group: is_group_jid(&r.chat_jid),
            chat_name: names.chat_name(&r.chat_jid),
            is_status: is_status_jid(&r.chat_jid),
            raw_media_type: r.media_type,
            message: m,
        })
        .collect())
}

/// Facade over everything in this crate.
pub struct Db {
    core: Arc<Core>,
    indexer: Indexer,
    watcher: Watcher,
}

impl Db {
    /// Open with the given paths. Never touches the bridge files at open time;
    /// creates/migrates app.db. The watcher starts at the current max rowid.
    pub fn open(paths: &Paths) -> Result<Self> {
        Self::open_with(paths, None, DEFAULT_DEADLINE)
    }

    /// Like `open`, with an explicit watcher start rowid and read deadline.
    pub fn open_with(
        paths: &Paths,
        watch_after: Option<i64>,
        deadline: std::time::Duration,
    ) -> Result<Self> {
        let app = Arc::new(AppDb::open(&paths.app_db)?);
        let bridge = Arc::new(RoPool::new(&paths.messages_db, deadline));
        let wa = Arc::new(RoPool::new(&paths.whatsapp_db, deadline));
        let names = Arc::new(NameResolver::new(Arc::clone(&bridge), wa));
        let core = Arc::new(Core {
            paths: paths.clone(),
            bridge,
            names,
            app,
            building: AtomicBool::new(false),
            index_lock: Mutex::new(()),
        });
        Ok(Self {
            indexer: Indexer::new(Arc::clone(&core)),
            watcher: Watcher::new(Arc::clone(&core), watch_after),
            core,
        })
    }

    /// Paths in use.
    pub fn paths(&self) -> &Paths {
        &self.core.paths
    }

    /// app.db handle (also implements `AppStore`).
    pub fn app(&self) -> &AppDb {
        &self.core.app
    }

    /// Name resolver (call `invalidate()` after contact changes).
    pub fn names(&self) -> &NameResolver {
        &self.core.names
    }

    /// Search index maintenance.
    pub fn indexer(&self) -> &Indexer {
        &self.indexer
    }

    /// Change watcher.
    pub fn watcher(&self) -> &Watcher {
        &self.watcher
    }

    /// Shortcut for `watcher().poll()`.
    pub fn poll(&self) -> Result<WatchOutcome> {
        self.watcher.poll()
    }

    /// Chat list (see `chats`).
    pub fn list_chats(&self, q: &ChatQuery) -> Result<Page<ChatSummary>> {
        chats::list_chats(&self.core, q)
    }

    /// Call log, newest first; cursor `"ts:rowid"`. Empty without a `calls` table.
    pub fn list_calls(&self, q: &CallQuery) -> Result<Page<CallRecord>> {
        calls::list_calls(&self.core, q)
    }

    /// One chat summary; `NotFound` if there is no such chat row.
    pub fn get_chat(&self, jid: &str) -> Result<ChatSummary> {
        chats::get_chat(&self.core, jid)
    }

    /// Thread page: newest first, items ascending, cursor `"ts:rowid"` pages older.
    pub fn get_messages(&self, q: &MessagesQuery) -> Result<Page<Message>> {
        messages::get_messages(&self.core, q)
    }

    /// One message (any row kind) by chat + id.
    pub fn get_message(&self, chat_jid: &str, id: &str) -> Result<Option<Message>> {
        messages::get_message(&self.core, chat_jid, id)
    }

    /// Up to `radius` messages before and after (same chat), ascending, including the target.
    pub fn get_message_context(
        &self,
        chat_jid: &str,
        id: &str,
        radius: u32,
    ) -> Result<Vec<Message>> {
        messages::get_message_context(&self.core, chat_jid, id, radius)
    }

    /// Full-text / filter search.
    pub fn search(&self, q: &SearchQuery) -> Result<SearchResults> {
        search::search(&self.core, q)
    }

    /// Media library page, newest first.
    pub fn list_media(&self, q: &MediaQuery) -> Result<Page<MediaItem>> {
        media::list_media(&self.core, q)
    }

    /// `MediaRef` for a message (None if not a media message or deleted).
    pub fn media_ref(&self, chat_jid: &str, id: &str) -> Result<Option<wa_core::types::MediaRef>> {
        Ok(self.get_message(chat_jid, id)?.and_then(|m| m.media))
    }

    /// Canonical local path for the `wa-media` handler (scope-checked against
    /// `Paths::media_roots`). `Invalid` for malformed ids, `Ok(None)` if not local.
    pub fn resolve_media_path(&self, chat_jid: &str, id: &str) -> Result<Option<PathBuf>> {
        media::resolve_media_path(&self.core, chat_jid, id)
    }

    /// Dashboard numbers.
    pub fn overview(&self) -> Result<Overview> {
        overview(&self.core)
    }

    /// Messages of one chat by id, ascending (unknown ids skipped).
    pub fn messages_by_ids(&self, chat_jid: &str, ids: &[String]) -> Result<Vec<Message>> {
        messages::messages_by_ids(&self.core, chat_jid, ids)
    }

    /// Links in one chat (or all), newest first; cursor `"ts:rowid:skip"`.
    pub fn list_links(
        &self,
        chat_jid: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<Page<LinkItem>> {
        insights::list_links(&self.core, chat_jid, cursor, limit)
    }

    /// Info-panel counts for a chat.
    pub fn chat_stats(&self, chat_jid: &str) -> Result<ChatStats> {
        insights::chat_stats(&self.core, chat_jid)
    }

    /// Star or unstar messages (stored in app.db).
    pub fn set_starred(&self, chat_jid: &str, ids: &[String], starred: bool) -> Result<()> {
        insights::set_starred(&self.core, chat_jid, ids, starred)
    }

    /// Starred messages newest first; offset cursor.
    pub fn list_starred(
        &self,
        chat_jid: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<Page<SearchHit>> {
        insights::list_starred(&self.core, chat_jid, cursor, limit)
    }

    /// Groups in which `jid` has written (from the index).
    pub fn common_groups(&self, jid: &str) -> Result<Vec<ChatRef>> {
        insights::common_groups(&self.core, jid)
    }
}

fn overview(core: &Core) -> Result<Overview> {
    let conn = core.bridge.get()?;
    let chats: i64 = conn
        .row(
            "SELECT count(*) FROM chats WHERE jid != 'status@broadcast' AND jid NOT LIKE '%@broadcast' \
             AND (last_message_time IS NOT NULL OR EXISTS (SELECT 1 FROM messages m WHERE m.chat_jid = chats.jid))",
            [],
            |r| r.get(0),
        )?
        .unwrap_or(0);
    let messages = match conn.row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0)) {
        Ok(n) => n.unwrap_or(0),
        Err(e) if e.kind == wa_core::ErrorKind::Timeout => raw::max_rowid(&conn)?,
        Err(e) => return Err(e),
    };
    drop(conn);
    let (media_total, oldest, newest): (i64, Option<i64>, Option<i64>) = core.app.read(|c| {
        Ok((
            c.query_row(
                "SELECT count(*) FROM msg_index WHERE media_kind IS NOT NULL AND deleted = 0",
                [],
                |r| r.get(0),
            )?,
            c.query_row("SELECT min(ts) FROM msg_index WHERE ts > 0", [], |r| {
                r.get(0)
            })?,
            c.query_row("SELECT max(ts) FROM msg_index", [], |r| r.get(0))?,
        ))
    })?;
    Ok(Overview {
        chats: chats.max(0) as u64,
        messages: messages.max(0) as u64,
        media_total: media_total.max(0) as u64,
        media_local: count_local_media(core)?,
        oldest_ts: oldest,
        newest_ts: newest,
        indexed: core.app.indexed_count()?,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Approximate local media count: bridge-store files named like
/// `<type>_<YYYYMMDD>_<HHMMSS>_<id>` plus phone links (not already in the store) whose file exists.
fn count_local_media(core: &Core) -> Result<u64> {
    let mut seen = std::collections::HashSet::new();
    if let Ok(dirs) = std::fs::read_dir(&core.paths.bridge_store) {
        for d in dirs.flatten() {
            if !d.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let chat = d.file_name().to_string_lossy().into_owned();
            let Ok(files) = std::fs::read_dir(d.path()) else {
                continue;
            };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let mut parts = name.splitn(4, '_');
                let (Some(t), Some(_), Some(_), Some(rest)) =
                    (parts.next(), parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                if MediaKind::from_db(t).is_none() {
                    continue;
                }
                let id = rest.split('.').next().unwrap_or(rest).to_string();
                seen.insert((chat.clone(), id));
            }
        }
    }
    let links: Vec<(String, String, String, Option<String>)> = core.app.read(|c| {
        let mut s =
            c.prepare("SELECT chat_jid, message_id, source_path, store_path FROM phone_links")?;
        let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect()
    })?;
    let mut n = seen.len() as u64;
    for (chat, id, src, store) in links {
        if seen.contains(&(chat.replace(':', "_"), id)) {
            continue;
        }
        if store
            .as_deref()
            .is_some_and(|p| std::path::Path::new(p).is_file())
            || std::path::Path::new(&src).is_file()
        {
            n += 1;
        }
    }
    Ok(n)
}

impl MessageStore for Db {
    fn max_rowid(&self) -> Result<i64> {
        let conn = self.core.bridge.get()?;
        raw::max_rowid(&conn)
    }

    fn messages_after(&self, after_rowid: i64, limit: u32) -> Result<Vec<IncomingMessage>> {
        let conn = self.core.bridge.get()?;
        let limit = (limit as usize).clamp(1, 5000);
        let mut rows = Vec::new();
        let mut last = after_rowid;
        while rows.len() < limit {
            let page =
                raw::after_rowid(&conn, last, (limit - rows.len()).min(bridge_ro::PAGE_ROWS))?;
            let n = page.len();
            if let Some(r) = page.last() {
                last = r.rowid;
            }
            rows.extend(page);
            if n == 0 {
                break;
            }
        }
        incoming(&self.core, &conn, rows)
    }

    fn get_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Message>> {
        Db::get_message(self, chat_jid, message_id)
    }

    fn media_info(&self, chat_jid: &str, message_id: &str) -> Result<Option<MediaInfo>> {
        media::media_info(&self.core, chat_jid, message_id)
    }

    fn media_rows(&self, since: Option<i64>, kinds: &[MediaKind]) -> Result<Vec<MediaInfo>> {
        media::media_rows(&self.core, since, kinds)
    }
}

impl LinkStore for Db {
    fn upsert_links(&self, links: &[LinkRecord]) -> Result<()> {
        if links.is_empty() {
            return Ok(());
        }
        self.core.app.upsert_phone_links(links)?;
        mirror_links(&self.core.paths.messages_db, links)
    }
}

/// Mirror links into messages.db `phone_media_links` (short immediate txns, ≤ 200 rows).
fn mirror_links(db: &std::path::Path, links: &[LinkRecord]) -> Result<()> {
    if !db.exists() {
        return Err(AppError::not_found("messages.db not found"));
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut c = Connection::open_with_flags(db, flags).map_err(sql_err)?;
    c.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(sql_err)?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS phone_media_links (
            message_id TEXT NOT NULL, chat_jid TEXT NOT NULL, media_type TEXT,
            phone_path TEXT NOT NULL, store_path TEXT, matched_by TEXT NOT NULL,
            linked_at TEXT NOT NULL, PRIMARY KEY (message_id, chat_jid));",
    )
    .map_err(sql_err)?;
    let now = epoch_to_rfc3339(now_epoch());
    for chunk in links.chunks(LINK_MIRROR_BATCH) {
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_err)?;
        {
            let mut st = tx
                .prepare_cached(
                    "INSERT INTO phone_media_links (message_id, chat_jid, media_type, phone_path, store_path, matched_by, linked_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)
                     ON CONFLICT(message_id, chat_jid) DO UPDATE SET media_type=excluded.media_type,
                       phone_path=excluded.phone_path, store_path=excluded.store_path,
                       matched_by=excluded.matched_by, linked_at=excluded.linked_at",
                )
                .map_err(sql_err)?;
            for l in chunk {
                st.execute(params![
                    l.message_id,
                    l.chat_jid,
                    l.kind.as_db(),
                    l.source_path.to_string_lossy(),
                    l.store_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned()),
                    l.matched_by,
                    now
                ])
                .map_err(sql_err)?;
            }
        }
        tx.commit().map_err(sql_err)?;
    }
    Ok(())
}

impl AppStore for Db {
    fn get_settings(&self) -> Result<Settings> {
        self.core.app.get_settings()
    }
    fn put_settings(&self, s: &Settings) -> Result<()> {
        self.core.app.put_settings(s)
    }
    fn list_rules(&self) -> Result<Vec<Rule>> {
        self.core.app.list_rules()
    }
    fn save_rule(&self, rule: &Rule) -> Result<Rule> {
        self.core.app.save_rule(rule)
    }
    fn delete_rule(&self, id: i64) -> Result<()> {
        self.core.app.delete_rule(id)
    }
    fn record_rule_fired(&self, id: i64, at: i64) -> Result<()> {
        self.core.app.record_rule_fired(id, at)
    }
    fn log_automation(&self, ev: &AutomationEvent) -> Result<()> {
        self.core.app.log_automation(ev)
    }
    fn recent_automation(&self, limit: u32) -> Result<Vec<AutomationEvent>> {
        self.core.app.recent_automation(limit)
    }
    fn get_kv(&self, key: &str) -> Result<Option<String>> {
        self.core.app.get_kv(key)
    }
    fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.core.app.set_kv(key, value)
    }
    fn insert_job(&self, job: &JobRecord) -> Result<i64> {
        self.core.app.insert_job(job)
    }
    fn update_job(&self, job: &JobRecord) -> Result<()> {
        self.core.app.update_job(job)
    }
    fn list_jobs(&self, limit: u32) -> Result<Vec<JobRecord>> {
        self.core.app.list_jobs(limit)
    }
    fn fail_interrupted_jobs(&self) -> Result<u32> {
        self.core.app.fail_interrupted_jobs()
    }
}

#[cfg(test)]
mod static_checks {
    fn assert_send_sync<T: Send + Sync>() {}
    #[test]
    fn db_is_send_sync() {
        assert_send_sync::<super::Db>();
        assert_send_sync::<super::AppDb>();
    }
}
