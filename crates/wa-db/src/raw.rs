//! Raw `messages` rows and the paged readers every module shares.
//!
//! All readers return at most `PAGE_ROWS` rows per statement and go through
//! `RoConn` (deadline + read-only).

use rusqlite::types::ValueRef;
use rusqlite::{Row, params_from_iter};
use wa_core::Result;
use wa_core::time::parse_ts;
use wa_core::types::MediaKind;

use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::util::{HashFields, placeholders, row_hash};

/// Column list matching `RawMsg::from_row`.
pub(crate) const COLS: &str = "rowid, id, chat_jid, sender, content, timestamp, is_from_me, media_type, \
     filename, file_length, deleted_at, quoted_message_id, file_sha256";

/// One `messages` row, tolerant of NULLs and odd column types.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawMsg {
    pub rowid: i64,
    pub id: String,
    pub chat_jid: String,
    pub sender: String,
    pub content: String,
    pub raw_ts: String,
    pub ts: i64,
    pub is_from_me: bool,
    pub media_type: String,
    pub filename: Option<String>,
    pub file_length: Option<i64>,
    pub deleted_at: Option<String>,
    pub quoted_id: Option<String>,
    pub file_sha256: Option<Vec<u8>>,
}

fn text(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<String>> {
    Ok(match r.get_ref(i)? {
        ValueRef::Null => None,
        ValueRef::Integer(n) => Some(n.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) | ValueRef::Blob(t) => Some(String::from_utf8_lossy(t).into_owned()),
    })
}

fn boolish(r: &Row<'_>, i: usize) -> rusqlite::Result<bool> {
    Ok(match r.get_ref(i)? {
        ValueRef::Null => false,
        ValueRef::Integer(n) => n != 0,
        ValueRef::Real(f) => f != 0.0,
        ValueRef::Text(t) | ValueRef::Blob(t) => {
            matches!(t, b"1" | b"true" | b"TRUE" | b"True" | b"t")
        }
    })
}

/// Parse a stored timestamp value (text, or integer epoch seconds). 0 when unparseable.
pub(crate) fn ts_of(raw: &str) -> i64 {
    parse_ts(raw)
        .or_else(|| raw.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

impl RawMsg {
    /// Build from a row selected with `COLS`.
    pub(crate) fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        let raw_ts = text(r, 5)?.unwrap_or_default();
        Ok(Self {
            rowid: r.get(0)?,
            id: text(r, 1)?.unwrap_or_default(),
            chat_jid: text(r, 2)?.unwrap_or_default(),
            sender: text(r, 3)?.unwrap_or_default(),
            content: text(r, 4)?.unwrap_or_default(),
            ts: ts_of(&raw_ts),
            raw_ts,
            is_from_me: boolish(r, 6)?,
            media_type: text(r, 7)?.unwrap_or_default(),
            filename: text(r, 8)?.filter(|s| !s.is_empty()),
            file_length: r.get::<_, Option<i64>>(9).ok().flatten(),
            deleted_at: text(r, 10)?.filter(|s| !s.is_empty()),
            quoted_id: text(r, 11)?.filter(|s| !s.is_empty()),
            file_sha256: match r.get_ref(12)? {
                ValueRef::Blob(b) if !b.is_empty() => Some(b.to_vec()),
                _ => None,
            },
        })
    }

    /// Reaction rows (emoji in content, target id in filename).
    pub fn is_reaction(&self) -> bool {
        self.media_type == "reaction"
    }

    /// Media kind, if this is a media message.
    pub fn kind(&self) -> Option<MediaKind> {
        MediaKind::from_db(&self.media_type)
    }

    /// Revoked ("delete for everyone").
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Change-detection hash of the visible state.
    pub fn hash(&self) -> i64 {
        row_hash(&HashFields {
            content: &self.content,
            deleted_at: self.deleted_at.as_deref(),
            sender: &self.sender,
            media_type: &self.media_type,
            filename: self.filename.as_deref(),
            timestamp: &self.raw_ts,
        })
    }

    /// Ordering key within a chat.
    pub fn key(&self) -> (i64, i64) {
        (self.ts, self.rowid)
    }
}

/// Rows by rowid (any order; missing rowids are skipped).
pub(crate) fn by_rowids(c: &RoConn, rowids: &[i64]) -> Result<Vec<RawMsg>> {
    let mut out = Vec::with_capacity(rowids.len());
    for chunk in rowids.chunks(PAGE_ROWS) {
        let sql = format!(
            "SELECT {COLS} FROM messages WHERE rowid IN ({})",
            placeholders(chunk.len())
        );
        out.extend(c.rows(&sql, params_from_iter(chunk.iter()), RawMsg::from_row)?);
    }
    Ok(out)
}

/// Rows of one chat by message id.
pub(crate) fn by_ids(c: &RoConn, chat_jid: &str, ids: &[String]) -> Result<Vec<RawMsg>> {
    let mut out = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(PAGE_ROWS - 1) {
        let sql = format!(
            "SELECT {COLS} FROM messages WHERE chat_jid = ? AND id IN ({})",
            placeholders(chunk.len())
        );
        let params = std::iter::once(chat_jid).chain(chunk.iter().map(String::as_str));
        out.extend(c.rows(&sql, params_from_iter(params), RawMsg::from_row)?);
    }
    Ok(out)
}

/// One row by primary key.
pub(crate) fn one(c: &RoConn, chat_jid: &str, id: &str) -> Result<Option<RawMsg>> {
    c.row(
        &format!("SELECT {COLS} FROM messages WHERE chat_jid = ?1 AND id = ?2"),
        [chat_jid, id],
        RawMsg::from_row,
    )
}

/// Up to `limit` (≤ PAGE_ROWS) rows with rowid > `after`, ascending.
pub(crate) fn after_rowid(c: &RoConn, after: i64, limit: usize) -> Result<Vec<RawMsg>> {
    c.rows(
        &format!("SELECT {COLS} FROM messages WHERE rowid > ?1 ORDER BY rowid LIMIT ?2"),
        [after, limit.min(PAGE_ROWS) as i64],
        RawMsg::from_row,
    )
}

/// Rows of one chat with rowid > `after`, ascending, one page.
pub(crate) fn chat_after_rowid(c: &RoConn, chat_jid: &str, after: i64) -> Result<Vec<RawMsg>> {
    c.rows(
        &format!(
            "SELECT {COLS} FROM messages WHERE chat_jid = ?1 AND rowid > ?2 ORDER BY rowid LIMIT {PAGE_ROWS}"
        ),
        rusqlite::params![chat_jid, after],
        RawMsg::from_row,
    )
}

/// `max(rowid)` of messages (0 when empty).
pub(crate) fn max_rowid(c: &RoConn) -> Result<i64> {
    Ok(c.row("SELECT max(rowid) FROM messages", [], |r| {
        r.get::<_, Option<i64>>(0)
    })?
    .flatten()
    .unwrap_or(0))
}

/// Light ordering key of a chat row: (ts, rowid, is_reaction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChatKey {
    pub ts: i64,
    pub rowid: i64,
}

/// Every non-reaction key of a chat, sorted ascending by (ts, rowid).
/// Paged by rowid (≤ PAGE_ROWS per statement); used when the index is not ready.
pub(crate) fn chat_keys(c: &RoConn, chat_jid: &str) -> Result<Vec<ChatKey>> {
    let mut out = Vec::new();
    let mut last = i64::MIN;
    loop {
        let page: Vec<(i64, Option<String>, Option<String>)> = c.rows(
            &format!(
                "SELECT rowid, timestamp, media_type FROM messages WHERE chat_jid = ?1 AND rowid > ?2 \
                 ORDER BY rowid LIMIT {PAGE_ROWS}"
            ),
            rusqlite::params![chat_jid, last],
            |r| Ok((r.get(0)?, text(r, 1)?, text(r, 2)?)),
        )?;
        let n = page.len();
        for (rowid, ts, mt) in page {
            last = rowid;
            if mt.as_deref() != Some("reaction") {
                out.push(ChatKey {
                    ts: ts.as_deref().map_or(0, ts_of),
                    rowid,
                });
            }
        }
        if n < PAGE_ROWS {
            break;
        }
    }
    out.sort_by_key(|k| (k.ts, k.rowid));
    Ok(out)
}
