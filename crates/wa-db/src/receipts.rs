//! Delivery ticks from the bridge's `message_receipts` table.
//!
//! The table is optional (older bridges): when it is missing every own
//! message is `Sent`. Inbound messages never have a status.

use std::collections::HashMap;

use rusqlite::params_from_iter;
use wa_core::Result;
use wa_core::types::MessageStatus;

use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::util::placeholders;

/// Stored statuses for `(chat_jid, message_id)` keys (only keys with a row).
pub(crate) fn statuses(
    conn: &RoConn,
    keys: &[(&str, &str)],
) -> Result<HashMap<(String, String), MessageStatus>> {
    let mut out = HashMap::new();
    if keys.is_empty() || !conn.has_table("message_receipts")? {
        return Ok(out);
    }
    let mut by_chat: HashMap<&str, Vec<&str>> = HashMap::new();
    for (c, i) in keys {
        by_chat.entry(c).or_default().push(i);
    }
    for (chat, ids) in by_chat {
        for chunk in ids.chunks(PAGE_ROWS - 1) {
            let sql = format!(
                "SELECT message_id, status FROM message_receipts WHERE chat_jid = ? AND message_id IN ({})",
                placeholders(chunk.len())
            );
            let p = std::iter::once(chat).chain(chunk.iter().copied());
            let rows: Vec<(String, Option<String>)> =
                conn.rows(&sql, params_from_iter(p), |r| Ok((r.get(0)?, r.get(1)?)))?;
            for (id, st) in rows {
                if let Some(s) = st.as_deref().and_then(MessageStatus::from_db) {
                    let e = out.entry((chat.to_string(), id)).or_insert(s);
                    *e = (*e).max(s);
                }
            }
        }
    }
    Ok(out)
}

/// Status for one message: `None` inbound, stored receipt or `Sent` for own.
pub(crate) fn status_of(
    map: &HashMap<(String, String), MessageStatus>,
    chat: &str,
    id: &str,
    is_from_me: bool,
) -> Option<MessageStatus> {
    if !is_from_me {
        return None;
    }
    Some(
        map.get(&(chat.to_string(), id.to_string()))
            .copied()
            .unwrap_or(MessageStatus::Sent),
    )
}
