//! Threads: paging, single messages, context, and `Message` assembly.
//!
//! Ordering is always by parsed epoch then bridge rowid. When the search index
//! is complete, keys come from `msg_index` (plus the not-yet-indexed tail,
//! rowid > watermark, read live); otherwise the chat's keys are read from
//! messages.db in rowid pages and sorted in memory.
//!
//! Reactions are **not** stored by wa-db: they are read live from messages.db
//! for the targets on each page (`chat_jid = ? AND media_type = 'reaction' AND
//! filename IN (...)`), so they are always current. Per target and sender the
//! latest reaction (by epoch, then rowid) wins; an empty emoji removes it.
//! Reaction rows never appear as thread items.

use std::collections::{HashMap, HashSet};

use rusqlite::params_from_iter;
use wa_core::types::{Message, MessagesQuery, Page, QuotedRef, Reaction};
use wa_core::{AppError, Result};

use crate::Core;
use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::chats::preview_label;
use crate::media;
use crate::names::{NameCache, is_group_jid};
use crate::raw::{self, ChatKey, RawMsg};
use crate::util::{clamp_limit, decode_msg_cursor, encode_msg_cursor, placeholders};

/// The sender value to resolve for a row (DMs with an empty sender use the chat).
pub(crate) fn sender_of(r: &RawMsg) -> String {
    if r.sender.trim().is_empty() && !r.is_from_me && !is_group_jid(&r.chat_jid) {
        r.chat_jid.clone()
    } else {
        r.sender.clone()
    }
}

/// Normalised sender JID for a row (None for own messages without sender).
pub(crate) fn sender_jid_of(r: &RawMsg, names: &NameCache) -> Option<String> {
    names.normalize(&sender_of(r))
}

/// Aggregate reaction rows into per-target reaction lists.
pub(crate) fn aggregate_reactions(
    rows: &[RawMsg],
    names: &NameCache,
) -> HashMap<String, Vec<Reaction>> {
    // (target, sender key) -> latest row
    let mut latest: HashMap<(String, String), &RawMsg> = HashMap::new();
    for r in rows.iter().filter(|r| r.is_reaction()) {
        let Some(target) = r.filename.clone() else {
            continue;
        };
        let who = if r.is_from_me || names.is_own(&r.sender) {
            "\u{0}me".to_string()
        } else {
            names
                .normalize(&r.sender)
                .unwrap_or_else(|| r.sender.clone())
        };
        let e = latest.entry((target, who)).or_insert(r);
        if r.key() > e.key() {
            *e = r;
        }
    }
    let mut picked: Vec<&RawMsg> = latest.into_values().collect();
    picked.sort_by_key(|r| r.key());
    let mut out: HashMap<String, Vec<Reaction>> = HashMap::new();
    for r in picked {
        let emoji = r.content.trim();
        if emoji.is_empty() {
            continue;
        }
        let Some(target) = r.filename.clone() else {
            continue;
        };
        out.entry(target).or_default().push(Reaction {
            emoji: emoji.to_string(),
            sender_name: names.sender_name(&r.sender, r.is_from_me),
            is_from_me: r.is_from_me || names.is_own(&r.sender),
        });
    }
    out
}

fn reactions_for(conn: &RoConn, chat: &str, targets: &[String]) -> Result<Vec<RawMsg>> {
    let mut out = Vec::new();
    for chunk in targets.chunks(PAGE_ROWS - 1) {
        let sql = format!(
            "SELECT {} FROM messages WHERE chat_jid = ? AND media_type = 'reaction' AND filename IN ({})",
            raw::COLS,
            placeholders(chunk.len())
        );
        let p = std::iter::once(chat).chain(chunk.iter().map(String::as_str));
        out.extend(conn.rows(&sql, params_from_iter(p), RawMsg::from_row)?);
    }
    Ok(out)
}

/// Turn raw rows into `Message`s (order preserved).
pub(crate) fn assemble(
    core: &Core,
    names: &NameCache,
    conn: &RoConn,
    rows: &[RawMsg],
) -> Result<Vec<Message>> {
    let mut by_chat: HashMap<&str, Vec<&RawMsg>> = HashMap::new();
    for r in rows {
        by_chat.entry(r.chat_jid.as_str()).or_default().push(r);
    }
    let mut reactions: HashMap<(String, String), Vec<Reaction>> = HashMap::new();
    let mut quoted: HashMap<(String, String), RawMsg> = HashMap::new();
    for (chat, rs) in &by_chat {
        let targets: Vec<String> = rs
            .iter()
            .filter(|r| !r.is_reaction())
            .map(|r| r.id.clone())
            .collect();
        if !targets.is_empty() {
            let rx = reactions_for(conn, chat, &targets)?;
            for (t, v) in aggregate_reactions(&rx, names) {
                reactions.insert(((*chat).to_string(), t), v);
            }
        }
        let have: HashSet<&str> = rs.iter().map(|r| r.id.as_str()).collect();
        let want: Vec<String> = rs
            .iter()
            .filter_map(|r| r.quoted_id.clone())
            .filter(|q| !have.contains(q.as_str()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for q in raw::by_ids(conn, chat, &want)? {
            quoted.insert((q.chat_jid.clone(), q.id.clone()), q);
        }
        for r in rs {
            quoted.insert((r.chat_jid.clone(), r.id.clone()), (*r).clone());
        }
    }
    let keys: Vec<(&str, &str)> = rows
        .iter()
        .filter(|r| r.is_from_me && !r.is_reaction())
        .map(|r| (r.chat_jid.as_str(), r.id.as_str()))
        .collect();
    let receipts = crate::receipts::statuses(conn, &keys)?;
    let mut starred: HashSet<(String, String)> = HashSet::new();
    for (chat, rs) in &by_chat {
        let ids: Vec<&str> = rs.iter().map(|r| r.id.as_str()).collect();
        for id in core.app.starred_among(chat, &ids)? {
            starred.insert(((*chat).to_string(), id));
        }
    }
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let q = r.quoted_id.as_ref().map(|qid| {
            let found = quoted.get(&(r.chat_jid.clone(), qid.clone()));
            QuotedRef {
                id: qid.clone(),
                sender_name: found.map(|f| names.sender_name(&sender_of(f), f.is_from_me)),
                preview: found.map(preview_label),
            }
        });
        let rx = reactions
            .remove(&(r.chat_jid.clone(), r.id.clone()))
            .unwrap_or_default();
        let mut m = to_message(core, names, conn, r, q, rx)?;
        m.status = crate::receipts::status_of(&receipts, &r.chat_jid, &r.id, r.is_from_me);
        m.starred = starred.contains(&(r.chat_jid.clone(), r.id.clone()));
        out.push(m);
    }
    Ok(out)
}

fn to_message(
    core: &Core,
    names: &NameCache,
    conn: &RoConn,
    r: &RawMsg,
    quoted: Option<QuotedRef>,
    reactions: Vec<Reaction>,
) -> Result<Message> {
    let deleted = r.is_deleted();
    let media = if deleted || r.is_reaction() {
        None
    } else {
        media::media_ref(core, conn, r)?
    };
    let sender = sender_of(r);
    Ok(Message {
        id: r.id.clone(),
        chat_jid: r.chat_jid.clone(),
        sender_jid: names.normalize(&sender),
        sender_name: names.sender_name(&sender, r.is_from_me),
        is_from_me: r.is_from_me,
        ts: r.ts,
        content: if deleted {
            String::new()
        } else {
            r.content.clone()
        },
        media,
        quoted,
        reactions,
        deleted,
        status: None,
        starred: false,
    })
}

/// Direction for `keys_around`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Older,
    Newer,
}

/// (ts, rowid, id) keys of non-reaction rows strictly older/newer than `pivot`,
/// nearest first, at most `n`.
fn keys_around(
    core: &Core,
    conn: &RoConn,
    chat: &str,
    pivot: (i64, i64),
    dir: Dir,
    n: usize,
) -> Result<Vec<ChatKey>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut keys: Vec<ChatKey> = if core.index_ready()? && is_indexed_chat(chat) {
        let wm = core.app.watermark()?;
        let sql = match dir {
            Dir::Older => {
                "SELECT ts, src_rowid FROM msg_index WHERE chat_jid = ?1 AND (ts, src_rowid) < (?2, ?3) \
                 ORDER BY ts DESC, src_rowid DESC LIMIT ?4"
            }
            Dir::Newer => {
                "SELECT ts, src_rowid FROM msg_index WHERE chat_jid = ?1 AND (ts, src_rowid) > (?2, ?3) \
                 ORDER BY ts ASC, src_rowid ASC LIMIT ?4"
            }
        };
        let mut k: Vec<ChatKey> = core.app.read(|c| {
            let mut st = c.prepare_cached(sql)?;
            let rows = st.query_map(rusqlite::params![chat, pivot.0, pivot.1, n as i64], |r| {
                Ok(ChatKey {
                    ts: r.get(0)?,
                    rowid: r.get(1)?,
                })
            })?;
            rows.collect()
        })?;
        k.extend(tail_keys(conn, chat, wm)?);
        k
    } else {
        raw::chat_keys(conn, chat)?
    };
    keys.retain(|k| match dir {
        Dir::Older => (k.ts, k.rowid) < pivot,
        Dir::Newer => (k.ts, k.rowid) > pivot,
    });
    keys.sort_by_key(|k| (k.ts, k.rowid));
    keys.dedup_by_key(|k| k.rowid);
    if dir == Dir::Older {
        keys.reverse();
    }
    keys.truncate(n);
    Ok(keys)
}

/// Whether the search index holds this chat's rows (channels, broadcast lists
/// and status are read straight from messages.db).
pub(crate) fn is_indexed_chat(chat: &str) -> bool {
    !crate::names::is_status_jid(chat)
}

/// Non-reaction rows of `chat` above the index watermark (not yet indexed).
fn tail_keys(conn: &RoConn, chat: &str, watermark: i64) -> Result<Vec<ChatKey>> {
    let mut out = Vec::new();
    let mut last = watermark;
    loop {
        let page = raw::chat_after_rowid(conn, chat, last)?;
        let n = page.len();
        for r in page {
            last = r.rowid;
            if !r.is_reaction() {
                out.push(ChatKey {
                    ts: r.ts,
                    rowid: r.rowid,
                });
            }
        }
        if n < PAGE_ROWS {
            return Ok(out);
        }
    }
}

/// Fetch rows for keys, keeping only rows of `chat`, ascending.
fn rows_for(conn: &RoConn, chat: &str, keys: &[ChatKey]) -> Result<Vec<RawMsg>> {
    let ids: Vec<i64> = keys.iter().map(|k| k.rowid).collect();
    let mut rows: Vec<RawMsg> = raw::by_rowids(conn, &ids)?
        .into_iter()
        .filter(|r| r.chat_jid == chat && !r.is_reaction())
        .collect();
    rows.sort_by_key(RawMsg::key);
    Ok(rows)
}

/// `get_messages`: newest page first, items ascending, cursor pages older.
pub(crate) fn get_messages(core: &Core, q: &MessagesQuery) -> Result<Page<Message>> {
    if q.chat_jid.trim().is_empty() {
        return Err(AppError::invalid("chat_jid is required"));
    }
    let limit = clamp_limit(q.limit, 80, 500);
    if q.before.is_some() && q.after.is_some() {
        return Err(AppError::invalid("use either before or after, not both"));
    }
    let (pivot, dir) = match (q.before.as_deref(), q.after.as_deref()) {
        (_, Some(a)) => (decode_msg_cursor(a)?, Dir::Newer),
        (Some(b), None) => (decode_msg_cursor(b)?, Dir::Older),
        (None, None) => ((i64::MAX, i64::MAX), Dir::Older),
    };
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let mut keys = keys_around(core, &conn, &q.chat_jid, pivot, dir, limit + 1)?;
    let more = keys.len() > limit;
    keys.truncate(limit);
    let next_cursor = if more {
        keys.last().map(|k| encode_msg_cursor(k.ts, k.rowid))
    } else {
        None
    };
    let rows = rows_for(&conn, &q.chat_jid, &keys)?;
    let items = assemble(core, &names, &conn, &rows)?;
    Ok(Page { items, next_cursor })
}

/// Messages of one chat by id (unknown ids skipped), ascending.
pub(crate) fn messages_by_ids(core: &Core, chat_jid: &str, ids: &[String]) -> Result<Vec<Message>> {
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let mut rows: Vec<RawMsg> = raw::by_ids(&conn, chat_jid, ids)?
        .into_iter()
        .filter(|r| !r.is_reaction())
        .collect();
    rows.sort_by_key(RawMsg::key);
    assemble(core, &names, &conn, &rows)
}

/// `get_message` by primary key.
pub(crate) fn get_message(core: &Core, chat_jid: &str, id: &str) -> Result<Option<Message>> {
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let Some(r) = raw::one(&conn, chat_jid, id)? else {
        return Ok(None);
    };
    Ok(assemble(core, &names, &conn, std::slice::from_ref(&r))?.pop())
}

/// `get_message_context`: up to `radius` messages either side, same chat, ascending.
pub(crate) fn get_message_context(
    core: &Core,
    chat_jid: &str,
    id: &str,
    radius: u32,
) -> Result<Vec<Message>> {
    let radius = radius.min(200) as usize;
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let target =
        raw::one(&conn, chat_jid, id)?.ok_or_else(|| AppError::not_found("message not found"))?;
    let mut keys = keys_around(core, &conn, chat_jid, target.key(), Dir::Older, radius)?;
    keys.extend(keys_around(
        core,
        &conn,
        chat_jid,
        target.key(),
        Dir::Newer,
        radius,
    )?);
    let mut rows = rows_for(&conn, chat_jid, &keys)?;
    rows.push(target);
    rows.sort_by_key(RawMsg::key);
    assemble(core, &names, &conn, &rows)
}
