//! Archive / pin / mute state from the bridge's `chat_state` table (read-only).
//!
//! The table is keyed by phone-form JIDs, with `@lid` keys for LIDs that have
//! no phone mapping. Keys are normalised through the name snapshot so a chat
//! row stored under its LID still finds state stored under its phone JID.
//! A database without the table yields no state (everything is plain).

use std::collections::HashMap;

use wa_core::Result;
use wa_core::time::now_epoch;

use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::names::NameCache;

/// State of one chat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChatState {
    pub archived: bool,
    pub pinned: bool,
    /// Raw `muted_until` (epoch secs, `-1` forever), expired values kept.
    pub muted_until: Option<i64>,
}

impl ChatState {
    /// Mute end as the UI should see it: expired mutes become `None`.
    pub fn active_mute(&self, now: i64) -> Option<i64> {
        self.muted_until.filter(|&m| m == -1 || m > now)
    }
}

/// A raw `chat_state` row: `(jid, archived, pinned, muted_until)`.
pub(crate) type RawState = (String, i64, i64, Option<i64>);

/// Raw rows of `chat_state`, paged.
pub(crate) fn raw_rows(conn: &RoConn) -> Result<Vec<RawState>> {
    if !conn.has_table("chat_state")? {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT jid, COALESCE(archived, 0), COALESCE(pinned, 0), CAST(muted_until AS INTEGER) \
         FROM chat_state WHERE jid > ?1 ORDER BY jid LIMIT {PAGE_ROWS}"
    );
    let mut out = Vec::new();
    let mut last = String::new();
    loop {
        let page: Vec<RawState> = conn.rows(&sql, [&last], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        let n = page.len();
        if let Some(l) = page.last() {
            last = l.0.clone();
        }
        out.extend(page);
        if n < PAGE_ROWS {
            return Ok(out);
        }
    }
}

/// All states keyed by normalised JID. A `@lid`-keyed row wins over a phone row.
pub(crate) fn load(conn: &RoConn, names: &NameCache) -> Result<HashMap<String, ChatState>> {
    let mut out: HashMap<String, (bool, ChatState)> = HashMap::new();
    for (jid, archived, pinned, muted) in raw_rows(conn)? {
        let is_lid = jid.ends_with("@lid");
        let key = names.normalize(&jid).unwrap_or(jid);
        let st = ChatState {
            archived: archived != 0,
            pinned: pinned != 0,
            muted_until: muted,
        };
        match out.get(&key) {
            Some((true, _)) if !is_lid => {}
            _ => {
                out.insert(key, (is_lid, st));
            }
        }
    }
    Ok(out.into_iter().map(|(k, (_, s))| (k, s)).collect())
}

/// State of a chat (any JID form).
pub(crate) fn of(map: &HashMap<String, ChatState>, names: &NameCache, jid: &str) -> ChatState {
    let key = names.normalize(jid).unwrap_or_else(|| jid.to_string());
    map.get(&key).copied().unwrap_or_default()
}

/// Current time for mute expiry (one place so it is easy to find).
pub(crate) fn now() -> i64 {
    now_epoch()
}
