//! Per-chat extras for the info panel: links, stats, starred messages and
//! groups in common. Everything reads `msg_index` / `msg_fts` (app.db), so it
//! never holds a messages.db statement longer than one page.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::types::Value;
use wa_core::types::{ChatRef, ChatStats, LinkItem, Page, SearchHit};
use wa_core::{AppError, Result};

use crate::Core;
use crate::messages::assemble;
use crate::raw::{self, RawMsg};
use crate::util::{clamp_limit, decode_offset_cursor, encode_offset_cursor, placeholders};

/// Rows fetched per index statement while scanning for links.
const LINK_SCAN: i64 = 200;
/// Maximum characters of context around a link.
pub const LINK_CONTEXT: usize = 200;

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    // A constant pattern; compiling it cannot fail.
    #[allow(clippy::expect_used)]
    Regex::new(r#"(?i)(?:https?://|\bwww\.)[^\s<>"'`]+"#).expect("valid URL regex")
});

/// Trailing characters that end a sentence rather than a URL.
const TRAILING: &[char] = &[
    '.', ',', ';', ':', '!', '?', '\'', '"', '*', '_', '~', '»', '”', '’', '…', '।', '、', '。',
];

fn trim_url(mut u: &str) -> &str {
    loop {
        let Some(last) = u.chars().last() else {
            return u;
        };
        let unbalanced = |open: char, close: char| {
            last == close && u.matches(open).count() < u.matches(close).count()
        };
        if TRAILING.contains(&last)
            || unbalanced('(', ')')
            || unbalanced('[', ']')
            || unbalanced('{', '}')
        {
            u = &u[..u.len() - last.len_utf8()];
        } else {
            return u;
        }
    }
}

/// URLs in `text` (http, https, www.), trailing punctuation removed, deduplicated,
/// in order of appearance, with their byte offsets.
pub fn extract_urls(text: &str) -> Vec<(usize, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in URL_RE.find_iter(text) {
        let u = trim_url(m.as_str());
        let lower = u.to_ascii_lowercase();
        let rest = ["https://", "http://", "www."]
            .iter()
            .find_map(|p| lower.strip_prefix(p))
            .unwrap_or("");
        if !rest.chars().any(char::is_alphanumeric) {
            continue;
        }
        if seen.insert(u.to_string()) {
            out.push((m.start(), u.to_string()));
        }
    }
    out
}

/// ≤ `LINK_CONTEXT` characters of `text` around byte offset `at`, one line.
fn context_around(text: &str, at: usize) -> String {
    let folded: Vec<char> = text
        .trim()
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if folded.len() <= LINK_CONTEXT {
        return folded.into_iter().collect();
    }
    let lead = text.trim_start().len();
    let skipped = text.len() - lead;
    let at_char = text
        .get(skipped..at.max(skipped))
        .map_or(0, |s| s.chars().count());
    let start = at_char.saturating_sub(60).min(folded.len() - LINK_CONTEXT);
    folded[start..start + LINK_CONTEXT].iter().collect()
}

fn link_cursor(c: Option<&str>) -> Result<(i64, i64, usize)> {
    let Some(c) = c else {
        return Ok((i64::MAX, i64::MAX, 0));
    };
    let bad = || AppError::invalid("malformed cursor");
    let mut it = c.split(':');
    let ts = it.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let rowid = it.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let skip = it.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    if it.next().is_some() {
        return Err(bad());
    }
    Ok((ts, rowid, skip))
}

struct LinkRow {
    ts: i64,
    rowid: i64,
    chat_jid: String,
    id: String,
    sender_jid: Option<String>,
    is_from_me: bool,
    content: String,
}

fn link_rows(core: &Core, chat: Option<&str>, pivot: (i64, i64)) -> Result<Vec<LinkRow>> {
    let mut sql = String::from(
        "SELECT i.ts, i.src_rowid, i.chat_jid, i.id, i.sender_jid, i.is_from_me, f.content \
         FROM msg_index i JOIN msg_fts f ON f.rowid = i.rowid \
         WHERE i.deleted = 0 AND (i.ts, i.src_rowid) <= (?1, ?2) \
         AND (f.content LIKE '%http%' OR f.content LIKE '%www.%')",
    );
    let mut p: Vec<Value> = vec![pivot.0.into(), pivot.1.into()];
    if let Some(c) = chat {
        sql.push_str(" AND i.chat_jid = ?3");
        p.push(c.to_string().into());
    }
    sql.push_str(&format!(
        " ORDER BY i.ts DESC, i.src_rowid DESC LIMIT {LINK_SCAN}"
    ));
    core.app.read(|c| {
        let mut st = c.prepare_cached(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(LinkRow {
                ts: r.get(0)?,
                rowid: r.get(1)?,
                chat_jid: r.get(2)?,
                id: r.get(3)?,
                sender_jid: r.get(4)?,
                is_from_me: r.get(5)?,
                content: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        })?;
        rows.collect()
    })
}

/// `list_links`: newest first; cursor `"<ts>:<rowid>:<skip>"`.
pub(crate) fn list_links(
    core: &Core,
    chat: Option<&str>,
    cursor: Option<&str>,
    limit: u32,
) -> Result<Page<LinkItem>> {
    let limit = clamp_limit(Some(limit), 50, 500);
    let (mut ts, mut rowid, mut skip) = link_cursor(cursor)?;
    let names = core.names.snapshot()?;
    let mut items = Vec::new();
    loop {
        let rows = link_rows(core, chat, (ts, rowid))?;
        let exhausted = (rows.len() as i64) < LINK_SCAN;
        let first = (ts, rowid);
        for r in &rows {
            let urls = extract_urls(&r.content);
            let from = if (r.ts, r.rowid) == first { skip } else { 0 };
            for (k, (at, url)) in urls.iter().enumerate().skip(from) {
                if items.len() == limit {
                    return Ok(Page {
                        items,
                        next_cursor: Some(format!("{}:{}:{k}", r.ts, r.rowid)),
                    });
                }
                items.push(LinkItem {
                    url: url.clone(),
                    message_id: r.id.clone(),
                    chat_jid: r.chat_jid.clone(),
                    sender_name: names
                        .sender_name(r.sender_jid.as_deref().unwrap_or(""), r.is_from_me),
                    ts: r.ts,
                    context: context_around(&r.content, *at),
                });
            }
        }
        let Some(last) = rows.last() else {
            break;
        };
        if exhausted {
            break;
        }
        // Continue strictly below the last scanned message.
        (ts, rowid, skip) = if last.rowid == i64::MIN {
            (last.ts - 1, i64::MAX, 0)
        } else {
            (last.ts, last.rowid - 1, 0)
        };
    }
    Ok(Page {
        items,
        next_cursor: None,
    })
}

/// `chat_stats` from the index (+ starred from app.db).
pub(crate) fn chat_stats(core: &Core, chat: &str) -> Result<ChatStats> {
    let (messages, media, docs, contents): (i64, i64, i64, Vec<String>) = core.app.read(|c| {
        let (m, md, d) = c.query_row(
            "SELECT count(*), \
               coalesce(sum(media_kind IN ('image','video')), 0), \
               coalesce(sum(media_kind = 'document'), 0) \
             FROM msg_index WHERE chat_jid = ?1 AND deleted = 0",
            [chat],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut st = c.prepare_cached(
            "SELECT f.content FROM msg_index i JOIN msg_fts f ON f.rowid = i.rowid \
             WHERE i.chat_jid = ?1 AND i.deleted = 0 \
             AND (f.content LIKE '%http%' OR f.content LIKE '%www.%')",
        )?;
        let rows = st.query_map([chat], |r| {
            Ok(r.get::<_, Option<String>>(0)?.unwrap_or_default())
        })?;
        Ok((m, md, d, rows.collect::<rusqlite::Result<_>>()?))
    })?;
    let links = contents.iter().map(|c| extract_urls(c).len() as u64).sum();
    Ok(ChatStats {
        messages: messages.max(0) as u64,
        media: media.max(0) as u64,
        docs: docs.max(0) as u64,
        links,
        starred: core.app.starred_count(chat)?,
    })
}

/// `set_starred`: unknown ids are ignored; starring only unknown ids is `NotFound`.
pub(crate) fn set_starred(core: &Core, chat: &str, ids: &[String], starred: bool) -> Result<()> {
    if chat.trim().is_empty() {
        return Err(AppError::invalid("chat_jid is required"));
    }
    if ids.is_empty() {
        return Ok(());
    }
    let items: Vec<(String, i64)> = if starred {
        let conn = core.bridge.get()?;
        let rows = raw::by_ids(&conn, chat, ids)?;
        drop(conn);
        let v: Vec<(String, i64)> = rows
            .into_iter()
            .filter(|r| !r.is_reaction())
            .map(|r| (r.id, r.ts))
            .collect();
        if v.is_empty() {
            return Err(AppError::not_found("message not found"));
        }
        v
    } else {
        ids.iter().map(|i| (i.clone(), 0)).collect()
    };
    core.app.set_starred(chat, &items, starred)
}

/// `list_starred`: newest first, offset cursor `"o:<n>"`.
pub(crate) fn list_starred(
    core: &Core,
    chat: Option<&str>,
    cursor: Option<&str>,
    limit: u32,
) -> Result<Page<SearchHit>> {
    let limit = clamp_limit(Some(limit), 50, 500);
    let offset = decode_offset_cursor(cursor)?;
    let mut keys = core.app.starred_page(chat, offset, limit + 1)?;
    let more = keys.len() > limit;
    keys.truncate(limit);
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let mut by_chat: HashMap<&str, Vec<String>> = HashMap::new();
    for (c, id, _) in &keys {
        by_chat.entry(c.as_str()).or_default().push(id.clone());
    }
    let mut rows: HashMap<(String, String), RawMsg> = HashMap::new();
    for (c, ids) in by_chat {
        for r in raw::by_ids(&conn, c, &ids)? {
            rows.insert((r.chat_jid.clone(), r.id.clone()), r);
        }
    }
    let ordered: Vec<RawMsg> = keys
        .iter()
        .filter_map(|(c, id, _)| rows.remove(&(c.clone(), id.clone())))
        .collect();
    let msgs = assemble(core, &names, &conn, &ordered)?;
    Ok(Page {
        items: msgs
            .into_iter()
            .map(|m| SearchHit {
                chat_name: names.chat_name(&m.chat_jid),
                message: m,
                highlights: Vec::new(),
            })
            .collect(),
        next_cursor: more.then(|| encode_offset_cursor(offset + limit)),
    })
}

/// Groups (not communities) in which `jid` has sent messages, by name.
pub(crate) fn common_groups(core: &Core, jid: &str) -> Result<Vec<ChatRef>> {
    let names = core.names.snapshot()?;
    let mut forms: Vec<String> = Vec::new();
    for a in names.sender_aliases(jid) {
        forms.extend(names.normalize(&a));
        forms.push(a);
    }
    forms.extend(names.normalize(jid));
    forms.sort();
    forms.dedup();
    if forms.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT DISTINCT chat_jid FROM msg_index WHERE sender_jid IN ({}) AND chat_jid LIKE '%@g.us'",
        placeholders(forms.len())
    );
    let chats: Vec<String> = core.app.read(|c| {
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(forms.iter()), |r| r.get(0))?;
        rows.collect()
    })?;
    let mut out: Vec<ChatRef> = chats.iter().map(|c| names.chat_ref(c)).collect();
    out.sort_by_key(|c| c.name.to_lowercase());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(t: &str) -> Vec<String> {
        extract_urls(t).into_iter().map(|(_, u)| u).collect()
    }

    #[test]
    fn extracts_and_trims() {
        assert_eq!(urls("see https://a.com/x."), ["https://a.com/x"]);
        assert_eq!(urls("(link: http://b.org/p)"), ["http://b.org/p"]);
        assert_eq!(
            urls("wiki https://en.wikipedia.org/wiki/Rust_(language)!"),
            ["https://en.wikipedia.org/wiki/Rust_(language)"]
        );
        assert_eq!(urls("go to www.example.com, now"), ["www.example.com"]);
        assert_eq!(
            urls("HTTPS://X.COM/A?b=1&c=2…"),
            ["HTTPS://X.COM/A?b=1&c=2"]
        );
        assert!(urls("just https:// and www. and http://...").is_empty());
        assert!(urls("no links here, swww.x").is_empty());
    }

    #[test]
    fn unicode_and_dedupe() {
        assert_eq!(
            urls("देखो https://example.in/पथ। फिर https://example.in/पथ"),
            ["https://example.in/पथ"]
        );
        assert_eq!(
            urls("🙏https://a.b/c 🙏 «https://d.e/f»"),
            ["https://a.b/c", "https://d.e/f"]
        );
        assert_eq!(
            urls("x https://a.com y https://a.com/"),
            ["https://a.com", "https://a.com/"]
        );
    }

    #[test]
    fn context_is_bounded_and_char_safe() {
        let long = format!("{} https://z.com {}", "अ".repeat(300), "b".repeat(300));
        let at = long.find("https").unwrap();
        let c = context_around(&long, at);
        assert_eq!(c.chars().count(), LINK_CONTEXT);
        assert!(c.contains("https://z.com"));
        assert_eq!(context_around(" a\nb https://q.io ", 5), "a b https://q.io");
    }

    #[test]
    fn link_cursor_parsing() {
        assert_eq!(link_cursor(Some("5:6:1")).unwrap(), (5, 6, 1));
        assert!(link_cursor(Some("5:6")).is_err());
        assert!(link_cursor(Some("5:6:1:2")).is_err());
        assert_eq!(link_cursor(None).unwrap().2, 0);
    }
}
