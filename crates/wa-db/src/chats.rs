//! Chat list and chat summaries.
//!
//! The ~1000 chat rows are loaded into memory (paged), filtered, sorted by
//! parsed epoch (never by raw timestamp text) and paged with an opaque offset
//! cursor (`"o:<n>"`). The last message of each chat (non-reaction) comes from
//! `msg_index` when it is complete, otherwise from the newest rowids of the chat.

use std::collections::HashMap;

use wa_core::types::{ChatKind, ChatQuery, ChatSummary, MediaKind, Page};
use wa_core::{AppError, Result};

use crate::Core;
use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::messages::sender_of;
use crate::names::{NameCache, is_group_jid, is_status_jid};
use crate::raw::{self, RawMsg, ts_of};
use crate::util::{clamp_limit, decode_offset_cursor, encode_offset_cursor, preview_text};

/// Maximum characters of text in a preview.
pub const PREVIEW_CHARS: usize = 120;

/// One-line preview of a message: media label, deleted marker or trimmed text.
pub fn preview_label(r: &RawMsg) -> String {
    if r.is_deleted() {
        return "🚫 Deleted message".to_string();
    }
    let caption = preview_text(&r.content, PREVIEW_CHARS);
    let with = |icon: &str, default: &str| {
        if caption.is_empty() {
            format!("{icon} {default}")
        } else {
            preview_text(&format!("{icon} {caption}"), PREVIEW_CHARS)
        }
    };
    match r.kind() {
        Some(MediaKind::Document) => {
            let name = r
                .filename
                .as_deref()
                .map(str::trim)
                .filter(|f| !f.is_empty());
            preview_text(&format!("📄 {}", name.unwrap_or("Document")), PREVIEW_CHARS)
        }
        Some(MediaKind::Image) => with("📷", "Photo"),
        Some(MediaKind::Video) => with("🎥", "Video"),
        Some(MediaKind::Audio) => "🎤 Audio".to_string(),
        Some(MediaKind::Sticker) => "🏷️ Sticker".to_string(),
        None => caption,
    }
}

/// Python's unread rule on parsed epochs: the last (non-reaction) message is
/// inbound and newer than the read marker (no marker → unread).
pub fn is_unread(last_ts: Option<i64>, last_from_me: Option<bool>, last_read: Option<i64>) -> bool {
    match (last_ts, last_from_me) {
        (Some(ts), Some(false)) => last_read.is_none_or(|r| ts > r),
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct ChatRow {
    jid: String,
    last_message_time: Option<i64>,
    last_read_time: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct LastMsg {
    ts: i64,
    rowid: i64,
    is_from_me: bool,
}

fn load_chats(conn: &RoConn, only: Option<&str>) -> Result<Vec<ChatRow>> {
    let has_read = conn.columns("chats")?.iter().any(|c| c == "last_read_time");
    let read_col = if has_read { "last_read_time" } else { "NULL" };
    if let Some(j) = only {
        let sql = format!("SELECT jid, last_message_time, {read_col} FROM chats WHERE jid = ?1");
        return conn.rows(&sql, [j], map_chat);
    }
    let sql = format!(
        "SELECT jid, last_message_time, {read_col} FROM chats WHERE jid > ?1 ORDER BY jid LIMIT {PAGE_ROWS}"
    );
    let mut out = Vec::new();
    let mut last = String::new();
    loop {
        let page = conn.rows(&sql, [&last], map_chat)?;
        let n = page.len();
        if let Some(c) = page.last() {
            last = c.jid.clone();
        }
        out.extend(page);
        if n < PAGE_ROWS {
            return Ok(out);
        }
    }
}

fn map_chat(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatRow> {
    let t = |i: usize| -> rusqlite::Result<Option<i64>> {
        Ok(match r.get_ref(i)? {
            rusqlite::types::ValueRef::Text(t) => {
                Some(ts_of(&String::from_utf8_lossy(t))).filter(|v| *v != 0)
            }
            rusqlite::types::ValueRef::Integer(n) => Some(n),
            _ => None,
        })
    };
    Ok(ChatRow {
        jid: r.get(0)?,
        last_message_time: t(1)?,
        last_read_time: t(2)?,
    })
}

/// Last non-reaction message per chat from the index (+ unindexed tail).
fn last_from_index(core: &Core, conn: &RoConn) -> Result<HashMap<String, LastMsg>> {
    let mut map: HashMap<String, LastMsg> = core.app.read(|c| {
        let mut st = c.prepare_cached(
            "SELECT chat_jid, max(ts), src_rowid, is_from_me FROM msg_index GROUP BY chat_jid",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                LastMsg {
                    ts: r.get(1)?,
                    rowid: r.get(2)?,
                    is_from_me: r.get(3)?,
                },
            ))
        })?;
        rows.collect()
    })?;
    let mut last = core.app.watermark()?;
    loop {
        let page = raw::after_rowid(conn, last, PAGE_ROWS)?;
        let n = page.len();
        for r in page {
            last = r.rowid;
            if r.is_reaction() {
                continue;
            }
            let m = LastMsg {
                ts: r.ts,
                rowid: r.rowid,
                is_from_me: r.is_from_me,
            };
            let e = map.entry(r.chat_jid.clone()).or_insert(m);
            if (m.ts, m.rowid) > (e.ts, e.rowid) {
                *e = m;
            }
        }
        if n < PAGE_ROWS {
            return Ok(map);
        }
    }
}

/// Last non-reaction message of one chat straight from messages.db (newest 50 rowids).
fn last_from_bridge(conn: &RoConn, jid: &str) -> Result<Option<LastMsg>> {
    let rows = conn.rows(
        &format!(
            "SELECT {} FROM messages WHERE chat_jid = ?1 AND media_type IS NOT 'reaction' \
             ORDER BY rowid DESC LIMIT 50",
            raw::COLS
        ),
        [jid],
        RawMsg::from_row,
    )?;
    Ok(rows.iter().max_by_key(|r| r.key()).map(|r| LastMsg {
        ts: r.ts,
        rowid: r.rowid,
        is_from_me: r.is_from_me,
    }))
}

struct Candidate {
    row: ChatRow,
    last: Option<LastMsg>,
    name: String,
    sort_ts: Option<i64>,
}

fn candidates(
    core: &Core,
    conn: &RoConn,
    names: &NameCache,
    only: Option<&str>,
) -> Result<Vec<Candidate>> {
    let rows = load_chats(conn, only)?;
    let indexed = if core.index_ready()? && only.is_none() {
        Some(last_from_index(core, conn)?)
    } else {
        None
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if only.is_none() && is_status_jid(&row.jid) {
            continue;
        }
        let last = match &indexed {
            Some(m) => m.get(&row.jid).copied(),
            None => last_from_bridge(conn, &row.jid)?,
        };
        if only.is_none() && last.is_none() && row.last_message_time.is_none() {
            continue;
        }
        let sort_ts = last.map(|l| l.ts).or(row.last_message_time);
        out.push(Candidate {
            name: names.chat_name(&row.jid),
            row,
            last,
            sort_ts,
        });
    }
    Ok(out)
}

fn matches(c: &Candidate, q: &ChatQuery) -> bool {
    let group = is_group_jid(&c.row.jid);
    match q.kind {
        ChatKind::Direct if group => return false,
        ChatKind::Group if !group => return false,
        _ => {}
    }
    if q.unread_only && !unread_of(c) {
        return false;
    }
    match q.filter.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => {
            let f = f.to_lowercase();
            c.name.to_lowercase().contains(&f) || c.row.jid.to_lowercase().contains(&f)
        }
        None => true,
    }
}

fn unread_of(c: &Candidate) -> bool {
    is_unread(
        c.last.map(|l| l.ts),
        c.last.map(|l| l.is_from_me),
        c.row.last_read_time,
    )
}

fn summarize(
    core: &Core,
    conn: &RoConn,
    names: &NameCache,
    cands: Vec<Candidate>,
) -> Result<Vec<ChatSummary>> {
    let rowids: Vec<i64> = cands
        .iter()
        .filter_map(|c| c.last.map(|l| l.rowid))
        .collect();
    let raws: HashMap<i64, RawMsg> = raw::by_rowids(conn, &rowids)?
        .into_iter()
        .map(|r| (r.rowid, r))
        .collect();
    let _ = core;
    Ok(cands
        .into_iter()
        .map(|c| {
            let unread = unread_of(&c);
            let last = c
                .last
                .and_then(|l| raws.get(&l.rowid))
                .filter(|r| r.chat_jid == c.row.jid);
            ChatSummary {
                is_group: is_group_jid(&c.row.jid),
                last_message_ts: c.sort_ts,
                last_preview: last.map(preview_label),
                last_sender_name: last.map(|r| names.sender_name(&sender_of(r), r.is_from_me)),
                last_is_from_me: c.last.is_some_and(|l| l.is_from_me),
                unread,
                name: c.name,
                jid: c.row.jid,
            }
        })
        .collect())
}

/// `list_chats`.
pub(crate) fn list_chats(core: &Core, q: &ChatQuery) -> Result<Page<ChatSummary>> {
    let limit = clamp_limit(q.limit, 100, 500);
    let offset = decode_offset_cursor(q.cursor.as_deref())?;
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let mut cands: Vec<Candidate> = candidates(core, &conn, &names, None)?
        .into_iter()
        .filter(|c| matches(c, q))
        .collect();
    cands.sort_by(|a, b| {
        b.sort_ts
            .unwrap_or(i64::MIN)
            .cmp(&a.sort_ts.unwrap_or(i64::MIN))
            .then_with(|| a.row.jid.cmp(&b.row.jid))
    });
    let total = cands.len();
    let page: Vec<Candidate> = cands.into_iter().skip(offset).take(limit).collect();
    let next = offset + page.len();
    let items = summarize(core, &conn, &names, page)?;
    Ok(Page {
        items,
        next_cursor: (next < total).then(|| encode_offset_cursor(next)),
    })
}

/// `get_chat`: summary of one chat (any chat row, including status).
pub(crate) fn get_chat(core: &Core, jid: &str) -> Result<ChatSummary> {
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let cands = candidates(core, &conn, &names, Some(jid))?;
    if cands.is_empty() {
        return Err(AppError::not_found("chat not found"));
    }
    summarize(core, &conn, &names, cands)?
        .pop()
        .ok_or_else(|| AppError::not_found("chat not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(media: &str, content: &str, filename: Option<&str>) -> RawMsg {
        RawMsg {
            media_type: media.into(),
            content: content.into(),
            filename: filename.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn preview_labels() {
        assert_eq!(
            preview_label(&raw("document", "here", Some("ML Notes.pdf"))),
            "📄 ML Notes.pdf"
        );
        assert_eq!(preview_label(&raw("document", "", None)), "📄 Document");
        assert_eq!(preview_label(&raw("image", "", None)), "📷 Photo");
        assert_eq!(preview_label(&raw("image", "look", None)), "📷 look");
        assert_eq!(preview_label(&raw("video", "", None)), "🎥 Video");
        assert_eq!(preview_label(&raw("audio", "", None)), "🎤 Audio");
        assert_eq!(preview_label(&raw("sticker", "", None)), "🏷️ Sticker");
        let mut d = raw("", "secret", None);
        d.deleted_at = Some("x".into());
        assert_eq!(preview_label(&d), "🚫 Deleted message");
        let long = "अ".repeat(300);
        let p = preview_label(&raw("", &long, None));
        assert_eq!(p.chars().count(), PREVIEW_CHARS);
    }

    #[test]
    fn unread_rules() {
        assert!(!is_unread(None, None, None));
        assert!(!is_unread(Some(10), Some(true), None));
        assert!(is_unread(Some(10), Some(false), None));
        assert!(is_unread(Some(10), Some(false), Some(9)));
        assert!(!is_unread(Some(10), Some(false), Some(10)));
        assert!(!is_unread(Some(10), Some(false), Some(11)));
        assert!(!is_unread(Some(10), None, Some(1)));
    }
}
