//! Cheap change detection on messages.db for the ~1 s poll loop.
//!
//! An idle poll costs one `stat`, `SELECT max(rowid)` and `PRAGMA
//! data_version` on a persistent read-only connection. When something changed:
//! - rows above the last seen rowid are returned as `new` (≤ `MAX_NEW` per poll;
//!   the rest arrive on the next polls);
//! - the chats table is re-hashed (name / read marker) to report renames and
//!   read-state changes;
//! - if the index is ready, indexed rows from the last 48 h are re-hashed and
//!   compared with `msg_index`: edits and revokes (same rowid, new hash) and
//!   vanished rows (LID delete+reinsert) report their chats as changed, and the
//!   index is updated so the next poll stays quiet;
//! - the newest 50 `calls` rows are re-hashed to report new or changed calls;
//! - finally the index catches up incrementally (skipped while a full build runs).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use wa_core::Result;
use wa_core::time::now_epoch;
use wa_core::traits::IncomingMessage;
use wa_core::types::CallRecord;

use crate::Core;
use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::calls;
use crate::indexer::Indexer;
use crate::raw::{self, RawMsg};
use crate::util::{HashFields, row_hash};

/// Maximum new rows returned by one poll.
pub const MAX_NEW: usize = 2000;
/// How far back edits/revokes are checked.
pub const RECENT_SECS: i64 = 48 * 3600;

/// Result of one poll.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WatchOutcome {
    /// New rows (all kinds, including reactions; see `IncomingMessage::raw_media_type`).
    pub new: Vec<IncomingMessage>,
    /// Chats with edits, revokes, removals, renames or read-marker changes.
    pub changed_chats: Vec<String>,
    /// Calls that are new or changed since the previous poll.
    pub calls: Vec<CallRecord>,
    /// Highest rowid delivered so far.
    pub max_rowid: i64,
}

#[derive(Default)]
struct State {
    init: bool,
    last_rowid: i64,
    data_version: i64,
    file_sig: Option<(SystemTime, u64)>,
    chats: HashMap<String, i64>,
    /// (call_id, chat_jid) → hash of the newest `calls` rows.
    calls: HashMap<(String, String), i64>,
    /// `chat_state` key → hash of (archived, pinned, muted_until).
    chat_state: HashMap<String, i64>,
}

/// Polls messages.db for changes. One instance per app.
pub struct Watcher {
    core: Arc<Core>,
    indexer: Indexer,
    start_after: Option<i64>,
    conn: Mutex<Option<RoConn>>,
    state: Mutex<State>,
}

fn file_sig(p: &std::path::Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

impl Watcher {
    /// `start_after`: deliver rows after this rowid; `None` = start at the current max (no backlog).
    pub(crate) fn new(core: Arc<Core>, start_after: Option<i64>) -> Self {
        Self {
            indexer: Indexer::new(Arc::clone(&core)),
            core,
            start_after,
            conn: Mutex::new(None),
            state: Mutex::new(State::default()),
        }
    }

    /// Last rowid delivered.
    pub fn last_rowid(&self) -> i64 {
        self.state.lock().last_rowid
    }

    /// One poll (see module docs). Safe to call from one thread at a time;
    /// concurrent calls serialise.
    pub fn poll(&self) -> Result<WatchOutcome> {
        let mut st = self.state.lock();
        let mut slot = self.conn.lock();
        if slot.is_none() {
            *slot = Some(RoConn::open(
                &self.core.paths.messages_db,
                crate::bridge_ro::DEFAULT_DEADLINE,
            )?);
        }
        let res = self.poll_inner(&mut st, slot.as_ref().unwrap_or_else(|| unreachable!()));
        if res.is_err() {
            // Reopen next time (file may have been replaced).
            *slot = None;
        }
        res
    }

    fn poll_inner(&self, st: &mut State, conn: &RoConn) -> Result<WatchOutcome> {
        let sig = file_sig(&self.core.paths.messages_db);
        let max = raw::max_rowid(conn)?;
        let dv = conn.data_version()?;
        if !st.init {
            st.init = true;
            st.last_rowid = self.start_after.unwrap_or(max);
            st.data_version = dv;
            st.file_sig = sig;
            st.chats = chat_hashes(conn)?;
            st.calls = call_hashes(&calls::newest(conn, calls::WATCH_ROWS)?);
            st.chat_state = state_hashes(conn)?;
            if st.last_rowid >= max {
                return Ok(WatchOutcome {
                    max_rowid: st.last_rowid,
                    ..Default::default()
                });
            }
        }
        let changed = dv != st.data_version || sig != st.file_sig;
        if !changed && max <= st.last_rowid {
            return Ok(WatchOutcome {
                max_rowid: st.last_rowid,
                ..Default::default()
            });
        }
        let mut out = WatchOutcome::default();
        if max > st.last_rowid {
            let rows = new_rows(conn, st.last_rowid)?;
            if let Some(l) = rows.last() {
                st.last_rowid = l.rowid;
            }
            out.new = crate::incoming(&self.core, conn, rows)?;
        }
        let mut changed_chats = BTreeSet::new();
        if changed {
            let chats = chat_hashes(conn)?;
            let mut renamed = false;
            for (jid, h) in &chats {
                if st.chats.get(jid).is_some_and(|old| old != h) {
                    changed_chats.insert(jid.clone());
                    renamed = true;
                }
            }
            if renamed || chats.len() != st.chats.len() {
                self.core.names.invalidate();
            }
            st.chats = chats;
            changed_chats.extend(self.changed_states(st, conn)?);
            if self.core.index_ready()? {
                changed_chats.extend(self.recent_changes(conn)?);
            }
        }
        out.calls = self.changed_calls(st, conn, changed)?;
        st.data_version = dv;
        st.file_sig = sig;
        if self.core.index_ready()?
            && let Err(e) = self.indexer.try_incremental()
        {
            tracing::warn!(kind = ?e.kind, "incremental index after poll failed");
        }
        out.changed_chats = changed_chats.into_iter().collect();
        out.max_rowid = st.last_rowid;
        Ok(out)
    }

    /// Chats whose archive/pin/mute row was added, changed or removed.
    fn changed_states(&self, st: &mut State, conn: &RoConn) -> Result<Vec<String>> {
        let now = state_hashes(conn)?;
        let mut keys: Vec<String> = now
            .iter()
            .filter(|(k, h)| st.chat_state.get(*k) != Some(*h))
            .map(|(k, _)| k.clone())
            .collect();
        keys.extend(
            st.chat_state
                .keys()
                .filter(|k| !now.contains_key(*k))
                .cloned(),
        );
        st.chat_state = now;
        if keys.is_empty() {
            return Ok(keys);
        }
        // Report the chat under its normalised JID as well as the stored key.
        let names = self.core.names.snapshot()?;
        let mut out = Vec::new();
        for k in keys {
            if let Some(n) = names.normalize(&k)
                && n != k
            {
                out.push(n);
            }
            out.push(k);
        }
        Ok(out)
    }

    /// New or changed rows among the newest `calls` rows (only when the db changed).
    fn changed_calls(
        &self,
        st: &mut State,
        conn: &RoConn,
        changed: bool,
    ) -> Result<Vec<CallRecord>> {
        if !changed {
            return Ok(Vec::new());
        }
        let mut rows = calls::newest(conn, calls::WATCH_ROWS)?;
        let hashes = call_hashes(&rows);
        rows.retain(|r| {
            let key = (r.call_id.clone(), r.chat_jid.clone());
            st.calls.get(&key) != hashes.get(&key)
        });
        st.calls.extend(hashes);
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        rows.sort_by_key(|r| r.rowid);
        let names = self.core.names.snapshot()?;
        Ok(rows.iter().map(|r| r.to_record(&names)).collect())
    }

    /// Re-hash indexed rows from the last 48 h; fix the index and return affected chats.
    fn recent_changes(&self, conn: &RoConn) -> Result<BTreeSet<String>> {
        let since = now_epoch() - RECENT_SECS;
        let wm = self.core.app.watermark()?;
        let idx: Vec<(i64, String, String, i64)> = self.core.app.read(|c| {
            let mut s = c.prepare_cached(
                "SELECT src_rowid, chat_jid, id, row_hash FROM msg_index WHERE ts >= ?1 AND src_rowid <= ?2",
            )?;
            let rows = s.query_map([since, wm], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })?;
        let mut chats = BTreeSet::new();
        if idx.is_empty() {
            return Ok(chats);
        }
        let rowids: Vec<i64> = idx.iter().map(|k| k.0).collect();
        let live: HashMap<i64, RawMsg> = raw::by_rowids(conn, &rowids)?
            .into_iter()
            .map(|r| (r.rowid, r))
            .collect();
        let mut changed_rows = Vec::new();
        let mut vanished = Vec::new();
        for (rowid, chat, id, h) in idx {
            match live.get(&rowid) {
                Some(r) if r.chat_jid == chat && r.id == id => {
                    if r.hash() != h {
                        chats.insert(chat);
                        changed_rows.push(r.clone());
                    }
                }
                _ => match raw::one(conn, &chat, &id)? {
                    // Same message under a new rowid (re-insert): just re-point the index.
                    Some(r) => {
                        if r.hash() != h {
                            chats.insert(chat);
                        }
                        changed_rows.push(r);
                    }
                    None => {
                        chats.insert(chat.clone());
                        vanished.push((chat, id));
                    }
                },
            }
        }
        self.indexer.apply_changes(&changed_rows, &vanished)?;
        Ok(chats)
    }
}

fn new_rows(conn: &RoConn, after: i64) -> Result<Vec<RawMsg>> {
    let mut out = Vec::new();
    let mut last = after;
    while out.len() < MAX_NEW {
        let page = raw::after_rowid(conn, last, PAGE_ROWS)?;
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

/// jid → hash(name, last_read_time) for every chat (paged).
fn chat_hashes(conn: &RoConn) -> Result<HashMap<String, i64>> {
    let has_read = conn.columns("chats")?.iter().any(|c| c == "last_read_time");
    let sql = format!(
        "SELECT jid, COALESCE(name, ''), COALESCE(CAST({} AS TEXT), '') FROM chats WHERE jid > ?1 ORDER BY jid LIMIT {PAGE_ROWS}",
        if has_read { "last_read_time" } else { "NULL" }
    );
    let mut out = HashMap::new();
    let mut last = String::new();
    loop {
        let page: Vec<(String, String, String)> =
            conn.rows(&sql, [&last], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let n = page.len();
        for (jid, name, read) in page {
            let h = row_hash(&HashFields {
                content: &name,
                deleted_at: None,
                sender: "",
                media_type: "",
                filename: None,
                timestamp: &read,
            });
            last = jid.clone();
            out.insert(jid, h);
        }
        if n < PAGE_ROWS {
            return Ok(out);
        }
    }
}

/// `chat_state` key → hash of its values (empty without the table).
fn state_hashes(conn: &RoConn) -> Result<HashMap<String, i64>> {
    Ok(crate::chat_state::raw_rows(conn)?
        .into_iter()
        .map(|(jid, a, p, m)| {
            let vals = format!("{a}|{p}|{}", m.map(|v| v.to_string()).unwrap_or_default());
            let h = row_hash(&HashFields {
                content: &vals,
                deleted_at: None,
                sender: "",
                media_type: "",
                filename: None,
                timestamp: "",
            });
            (jid, h)
        })
        .collect())
}

fn call_hashes(rows: &[calls::RawCall]) -> HashMap<(String, String), i64> {
    rows.iter()
        .map(|r| ((r.call_id.clone(), r.chat_jid.clone()), r.hash()))
        .collect()
}
