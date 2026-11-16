//! Call log: `list_calls` over the bridge's `calls` table (read-only) and
//! change detection helpers for the watcher.
//!
//! Rows are loaded in rowid pages (≤ 500), sorted by parsed epoch (never raw
//! timestamp text) and paged with a `"ts:rowid"` cursor, newest first. A
//! database without a `calls` table yields an empty page.

use rusqlite::Row;
use wa_core::Result;
use wa_core::types::{CallMedia, CallQuery, CallRecord, CallResult, Page};

use crate::Core;
use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::media::avatar_url;
use crate::names::{NameCache, is_group_jid};
use crate::raw::{boolish, text, ts_of};
use crate::util::{HashFields, clamp_limit, decode_msg_cursor, encode_msg_cursor, row_hash};

/// Rows hashed by the watcher on each change (the newest by rowid).
pub const WATCH_ROWS: usize = 50;
/// Upper bound of rows scanned for one `list_calls`.
const MAX_SCAN: usize = 20_000;

const COLS: &str = "rowid, call_id, chat_jid, from_jid, timestamp, is_from_me, call_type, \
                    is_group, result, duration_sec, ended_at, reason";

/// A `calls` row as stored.
#[derive(Debug, Clone, Default)]
pub(crate) struct RawCall {
    pub rowid: i64,
    pub call_id: String,
    pub chat_jid: String,
    pub from_jid: String,
    pub raw_ts: String,
    pub ts: i64,
    pub is_from_me: bool,
    pub video: bool,
    pub is_group: bool,
    pub result: String,
    pub duration: Option<i64>,
    pub raw_ended: Option<String>,
    pub reason: Option<String>,
}

impl RawCall {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        let raw_ts = text(r, 4)?.unwrap_or_default();
        let call_type = text(r, 6)?.unwrap_or_default();
        Ok(Self {
            rowid: r.get(0)?,
            call_id: text(r, 1)?.unwrap_or_default(),
            chat_jid: text(r, 2)?.unwrap_or_default(),
            from_jid: text(r, 3)?.unwrap_or_default(),
            ts: ts_of(&raw_ts),
            raw_ts,
            is_from_me: boolish(r, 5)?,
            video: call_type.trim().eq_ignore_ascii_case("video"),
            is_group: boolish(r, 7)?,
            result: text(r, 8)?.unwrap_or_default().trim().to_lowercase(),
            duration: text(r, 9)?.and_then(|d| d.trim().parse::<i64>().ok()),
            raw_ended: text(r, 10)?.filter(|s| !s.is_empty()),
            reason: text(r, 11)?.filter(|s| !s.is_empty()),
        })
    }

    /// Stable hash of everything the UI shows (detects state changes).
    pub(crate) fn hash(&self) -> i64 {
        let dur = self.duration.map(|d| d.to_string()).unwrap_or_default();
        let ended = self.raw_ended.clone().unwrap_or_default();
        let content = format!(
            "{}|{}|{}|{}|{}",
            self.result,
            dur,
            ended,
            self.reason.as_deref().unwrap_or(""),
            u8::from(self.video)
        );
        row_hash(&HashFields {
            content: &content,
            deleted_at: None,
            sender: &self.from_jid,
            media_type: &self.call_id,
            filename: Some(&self.chat_jid),
            timestamp: &self.raw_ts,
        })
    }

    fn missed_for_me(&self) -> bool {
        !self.is_from_me && matches!(self.result.as_str(), "missed" | "rejected")
    }

    pub(crate) fn to_record(&self, names: &NameCache) -> CallRecord {
        let caller = if self.from_jid.trim().is_empty() {
            self.chat_jid.as_str()
        } else {
            self.from_jid.as_str()
        };
        CallRecord {
            call_id: self.call_id.clone(),
            chat_name: names.chat_name(&self.chat_jid),
            caller_name: names.sender_name(caller, self.is_from_me),
            avatar_url: avatar_url(&self.chat_jid),
            is_group: self.is_group || is_group_jid(&self.chat_jid),
            chat_jid: self.chat_jid.clone(),
            caller_jid: caller.to_string(),
            is_from_me: self.is_from_me,
            media: if self.video {
                CallMedia::Video
            } else {
                CallMedia::Voice
            },
            result: CallResult::from_db(&self.result),
            ts: self.ts,
            duration_secs: self
                .duration
                .filter(|d| *d >= 0)
                .map(|d| d.min(i64::from(u32::MAX)) as u32),
            ended_at: self.raw_ended.as_deref().map(ts_of).filter(|t| *t != 0),
            reason: self.reason.clone(),
        }
    }
}

/// Newest `limit` rows by rowid (empty without a `calls` table).
pub(crate) fn newest(conn: &RoConn, limit: usize) -> Result<Vec<RawCall>> {
    if !conn.has_table("calls")? {
        return Ok(Vec::new());
    }
    conn.rows(
        &format!("SELECT {COLS} FROM calls ORDER BY rowid DESC LIMIT {limit}"),
        [],
        RawCall::from_row,
    )
}

fn scan(conn: &RoConn, q: &CallQuery) -> Result<Vec<RawCall>> {
    let mut sql = format!("SELECT {COLS} FROM calls WHERE rowid > ?1");
    if q.chat_jid.is_some() {
        sql.push_str(" AND chat_jid = ?2");
    }
    sql.push_str(&format!(" ORDER BY rowid LIMIT {PAGE_ROWS}"));
    let mut out = Vec::new();
    let mut last = 0i64;
    while out.len() < MAX_SCAN {
        let page = match &q.chat_jid {
            Some(j) => conn.rows(&sql, rusqlite::params![last, j], RawCall::from_row)?,
            None => conn.rows(&sql, rusqlite::params![last], RawCall::from_row)?,
        };
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

fn wanted(c: &RawCall, q: &CallQuery) -> bool {
    if q.missed_only && !c.missed_for_me() {
        return false;
    }
    match q.media {
        Some(CallMedia::Video) => c.video,
        Some(CallMedia::Voice) => !c.video,
        None => true,
    }
}

/// `list_calls`.
pub(crate) fn list_calls(core: &Core, q: &CallQuery) -> Result<Page<CallRecord>> {
    let limit = clamp_limit(q.limit, 100, 500);
    let cursor = q.cursor.as_deref().map(decode_msg_cursor).transpose()?;
    let conn = core.bridge.get()?;
    if !conn.has_table("calls")? {
        return Ok(Page {
            items: Vec::new(),
            next_cursor: None,
        });
    }
    let mut rows: Vec<RawCall> = scan(&conn, q)?
        .into_iter()
        .filter(|c| wanted(c, q))
        .filter(|c| cursor.is_none_or(|k| (c.ts, c.rowid) < k))
        .collect();
    drop(conn);
    rows.sort_by_key(|r| std::cmp::Reverse((r.ts, r.rowid)));
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = more
        .then(|| rows.last().map(|r| encode_msg_cursor(r.ts, r.rowid)))
        .flatten();
    let names = core.names.snapshot()?;
    Ok(Page {
        items: rows.iter().map(|r| r.to_record(&names)).collect(),
        next_cursor,
    })
}
