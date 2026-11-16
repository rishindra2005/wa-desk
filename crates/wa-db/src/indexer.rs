//! Builds and maintains the search index (`msg_index` + `msg_fts` in app.db).
//!
//! Batches of up to `BATCH` rows are read from messages.db by rowid (as
//! `PAGE_ROWS`-sized statements), the read connection is released, and the
//! batch is written to app.db in one transaction together with the new
//! watermark, so an interrupted build resumes where it stopped.
//!
//! Not indexed: reaction rows (read live, see `messages`) and status /
//! broadcast / newsletter chats. Revoked messages are indexed with empty text.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rusqlite::params;
use wa_core::traits::AppStore;
use wa_core::{AppError, Result};

use crate::Core;
use crate::app_db::{AppDb, IndexRow, KV_COMPLETE, KV_WATERMARK};
use crate::bridge_ro::PAGE_ROWS;
use crate::messages::sender_jid_of;
use crate::names::{NameCache, is_status_jid};
use crate::raw::{self, RawMsg};

/// Rows per app.db write transaction.
pub const BATCH: usize = 5000;

/// Outcome of a reconcile pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileStats {
    /// Bridge rows examined.
    pub scanned: u64,
    /// Index rows inserted or updated.
    pub updated: u64,
    /// Index rows removed because their (chat, id) vanished.
    pub removed: u64,
}

/// The index row for a bridge row, or None if the row is not indexed.
pub(crate) fn index_row(r: &RawMsg, names: &NameCache) -> Option<IndexRow> {
    if r.is_reaction() || r.id.is_empty() || r.chat_jid.is_empty() || is_status_jid(&r.chat_jid) {
        return None;
    }
    let deleted = r.is_deleted();
    Some(IndexRow {
        chat_jid: r.chat_jid.clone(),
        id: r.id.clone(),
        src_rowid: r.rowid,
        ts: r.ts,
        sender_jid: sender_jid_of(r, names),
        is_from_me: r.is_from_me,
        media_kind: r.kind().map(|k| k.as_db()),
        filename: r.filename.clone(),
        file_length: r.file_length,
        deleted,
        row_hash: r.hash(),
        content: if deleted {
            String::new()
        } else {
            r.content.clone()
        },
    })
}

/// Resets the building flag when a build ends (even on error/panic).
struct BuildGuard<'a>(&'a Core);
impl Drop for BuildGuard<'_> {
    fn drop(&mut self) {
        self.0.building.store(false, Ordering::SeqCst);
    }
}

/// Search-index maintenance. Cheap to clone the handle via `Arc`.
pub struct Indexer {
    core: Arc<Core>,
}

impl Indexer {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    /// True while a full build is running.
    pub fn is_building(&self) -> bool {
        self.core.building.load(Ordering::SeqCst)
    }

    /// Drop the index and rebuild it from scratch. `progress(done, total)`.
    pub fn build_full(&self, progress: &dyn Fn(u64, u64)) -> Result<u64> {
        self.build_full_with(progress, &|| false)
    }

    /// `build_full` that stops between batches (with `Cancelled`) once
    /// `should_stop()` returns true. The index stays incomplete and resumable.
    pub fn build_full_with(
        &self,
        progress: &dyn Fn(u64, u64),
        should_stop: &dyn Fn() -> bool,
    ) -> Result<u64> {
        let _run = self.core.index_lock.lock();
        self.core.building.store(true, Ordering::SeqCst);
        let _g = BuildGuard(&self.core);
        self.core.app.clear_index()?;
        let n = self.run_from(0, progress, should_stop)?;
        self.core.app.set_kv(KV_COMPLETE, "1")?;
        Ok(n)
    }

    /// Finish an interrupted/never-run build (resuming at the watermark), or
    /// run an incremental pass if the index is already complete.
    pub fn ensure(&self, progress: &dyn Fn(u64, u64)) -> Result<u64> {
        self.ensure_with(progress, &|| false)
    }

    /// `ensure` with a cancel check between batches.
    pub fn ensure_with(
        &self,
        progress: &dyn Fn(u64, u64),
        should_stop: &dyn Fn() -> bool,
    ) -> Result<u64> {
        if self.core.app.index_complete()? {
            return self.incremental();
        }
        let _run = self.core.index_lock.lock();
        self.core.building.store(true, Ordering::SeqCst);
        let _g = BuildGuard(&self.core);
        let n = self.run_from(self.core.app.watermark()?, progress, should_stop)?;
        self.core.app.set_kv(KV_COMPLETE, "1")?;
        Ok(n)
    }

    /// Index rows above the watermark. Returns rows processed.
    pub fn incremental(&self) -> Result<u64> {
        let _run = self.core.index_lock.lock();
        self.run_from(self.core.app.watermark()?, &|_, _| {}, &|| false)
    }

    /// Like `incremental` but returns `None` immediately if another index operation runs.
    pub fn try_incremental(&self) -> Result<Option<u64>> {
        let Some(_run) = self.core.index_lock.try_lock() else {
            return Ok(None);
        };
        if self.is_building() {
            return Ok(None);
        }
        self.run_from(self.core.app.watermark()?, &|_, _| {}, &|| false)
            .map(Some)
    }

    fn read_batch(&self, after: i64) -> Result<Vec<RawMsg>> {
        let conn = self.core.bridge.get()?;
        let mut out = Vec::new();
        let mut last = after;
        while out.len() < BATCH {
            let page = raw::after_rowid(&conn, last, PAGE_ROWS)?;
            let n = page.len();
            if let Some(r) = page.last() {
                last = r.rowid;
            }
            out.extend(page);
            if n < PAGE_ROWS {
                break;
            }
        }
        Ok(out)
    }

    fn run_from(
        &self,
        start: i64,
        progress: &dyn Fn(u64, u64),
        should_stop: &dyn Fn() -> bool,
    ) -> Result<u64> {
        let total = {
            let conn = self.core.bridge.get()?;
            raw::max_rowid(&conn)?
        };
        let names = self.core.names.snapshot()?;
        let mut wm = start;
        let mut done = 0u64;
        progress(0, total.saturating_sub(start).max(0) as u64);
        loop {
            if should_stop() {
                return Err(AppError::cancelled());
            }
            let batch = self.read_batch(wm)?;
            let Some(last) = batch.last().map(|r| r.rowid) else {
                break;
            };
            let rows: Vec<IndexRow> = batch.iter().filter_map(|r| index_row(r, &names)).collect();
            self.core.app.write(|tx| {
                AppDb::upsert_index(tx, &rows)?;
                tx.execute(
                    "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![KV_WATERMARK, last.to_string()],
                )?;
                Ok(())
            })?;
            done += batch.len() as u64;
            wm = last;
            progress(done, total.max(wm).saturating_sub(start).max(0) as u64);
            if batch.len() < BATCH {
                break;
            }
        }
        Ok(done)
    }

    /// Re-index specific rows (watcher edits) and drop vanished keys.
    pub(crate) fn apply_changes(
        &self,
        rows: &[RawMsg],
        vanished: &[(String, String)],
    ) -> Result<()> {
        if rows.is_empty() && vanished.is_empty() {
            return Ok(());
        }
        let names = self.core.names.snapshot()?;
        let idx: Vec<IndexRow> = rows.iter().filter_map(|r| index_row(r, &names)).collect();
        self.core.app.write(|tx| {
            AppDb::delete_index(tx, vanished)?;
            AppDb::upsert_index(tx, &idx)?;
            Ok(())
        })
    }

    /// Compare every bridge row with the index: update changed rows, add
    /// missing ones and delete index rows whose (chat, id) no longer exists
    /// (e.g. after the bridge's LID→phone chat migration).
    pub fn reconcile(&self, progress: &dyn Fn(u64, u64)) -> Result<ReconcileStats> {
        self.reconcile_with(progress, &|| false)
    }

    /// `reconcile` with a cancel check between batches (nothing is deleted
    /// when it stops early).
    pub fn reconcile_with(
        &self,
        progress: &dyn Fn(u64, u64),
        should_stop: &dyn Fn() -> bool,
    ) -> Result<ReconcileStats> {
        let _run = self.core.index_lock.lock();
        let names = self.core.names.snapshot()?;
        let total = {
            let conn = self.core.bridge.get()?;
            raw::max_rowid(&conn)?
        };
        let mut st = ReconcileStats::default();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut wm = 0i64;
        loop {
            if should_stop() {
                return Err(AppError::cancelled());
            }
            let batch = self.read_batch(wm)?;
            let Some(last) = batch.last().map(|r| r.rowid) else {
                break;
            };
            let rows: Vec<IndexRow> = batch.iter().filter_map(|r| index_row(r, &names)).collect();
            seen.extend(rows.iter().map(|r| (r.chat_jid.clone(), r.id.clone())));
            st.updated += self.core.app.write(|tx| AppDb::upsert_index(tx, &rows))? as u64;
            st.scanned += batch.len() as u64;
            wm = last;
            progress(st.scanned, total.max(wm) as u64);
            if batch.len() < BATCH {
                break;
            }
        }
        let keys: Vec<(String, String)> = self.core.app.read(|c| {
            let mut s = c.prepare("SELECT chat_jid, id FROM msg_index")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })?;
        let gone: Vec<(String, String)> = keys.into_iter().filter(|k| !seen.contains(k)).collect();
        for chunk in gone.chunks(BATCH) {
            st.removed += self.core.app.write(|tx| AppDb::delete_index(tx, chunk))? as u64;
        }
        let cur = self.core.app.watermark()?;
        if wm > cur {
            self.core.app.set_kv(KV_WATERMARK, &wm.to_string())?;
        }
        Ok(st)
    }
}
