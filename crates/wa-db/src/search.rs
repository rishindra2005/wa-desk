//! Full-text search over `msg_fts` (FTS5, trigram tokenizer).
//!
//! - ≥ 3 characters: `MATCH` with the whole query as one quoted phrase (all
//!   FTS syntax characters are literal inside a phrase; `"` is doubled).
//! - 1–2 characters: substring scan with `LIKE` (wildcards escaped).
//! - empty text: filter-only browse.
//!
//! Hits are newest first; message bodies come live from messages.db.

use std::time::Instant;

use rusqlite::types::Value;
use wa_core::Result;
use wa_core::types::{SearchHit, SearchQuery, SearchResults};

use crate::Core;
use crate::media::fetch_keyed;
use crate::messages::assemble;
use crate::util::{clamp_limit, highlight_ranges, like_escape, placeholders};

/// Cap on `total`.
pub const TOTAL_CAP: u32 = 10_000;

/// Quote free text as a single FTS5 phrase.
pub fn fts_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

struct Built {
    from: String,
    wh: Vec<String>,
    params: Vec<Value>,
}

fn build(core: &Core, q: &SearchQuery, text: &str) -> Result<Built> {
    let n = text.chars().count();
    let mut b = Built {
        from: "msg_index i".into(),
        wh: Vec::new(),
        params: Vec::new(),
    };
    if n >= 3 {
        b.from = "msg_fts JOIN msg_index i ON i.rowid = msg_fts.rowid".into();
        b.wh.push("msg_fts MATCH ?".into());
        b.params.push(fts_phrase(text).into());
    } else if n > 0 {
        b.from = "msg_fts JOIN msg_index i ON i.rowid = msg_fts.rowid".into();
        b.wh.push(
            "(msg_fts.content LIKE ? ESCAPE '\\' OR msg_fts.filename LIKE ? ESCAPE '\\')".into(),
        );
        let pat = format!("%{}%", like_escape(text));
        b.params.push(pat.clone().into());
        b.params.push(pat.into());
    }
    if !q.chat_jids.is_empty() {
        b.wh.push(format!(
            "i.chat_jid IN ({})",
            placeholders(q.chat_jids.len())
        ));
        b.params
            .extend(q.chat_jids.iter().map(|c| Value::from(c.clone())));
    }
    if !q.senders.is_empty() {
        let names = core.names.snapshot()?;
        let mut forms: Vec<String> = Vec::new();
        for s in &q.senders {
            for a in names.sender_aliases(s) {
                if let Some(n) = names.normalize(&a) {
                    forms.push(n);
                }
                forms.push(a);
            }
        }
        forms.sort();
        forms.dedup();
        b.wh.push(format!(
            "i.sender_jid IN ({})",
            placeholders(forms.len().max(1))
        ));
        if forms.is_empty() {
            b.params.push(Value::Null);
        }
        b.params.extend(forms.into_iter().map(Value::from));
    }
    if !q.media_kinds.is_empty() {
        b.wh.push(format!(
            "i.media_kind IN ({})",
            placeholders(q.media_kinds.len())
        ));
        b.params.extend(
            q.media_kinds
                .iter()
                .map(|k| Value::from(k.as_db().to_string())),
        );
    }
    if let Some(m) = q.from_me {
        b.wh.push("i.is_from_me = ?".into());
        b.params.push(Value::from(m as i64));
    }
    if let Some(a) = q.after {
        b.wh.push("i.ts >= ?".into());
        b.params.push(a.into());
    }
    if let Some(bf) = q.before {
        b.wh.push("i.ts < ?".into());
        b.params.push(bf.into());
    }
    if n > 0 {
        b.wh.push("i.deleted = 0".into());
    }
    Ok(b)
}

impl Built {
    fn where_sql(&self) -> String {
        if self.wh.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.wh.join(" AND "))
        }
    }
}

/// `search`.
pub(crate) fn search(core: &Core, q: &SearchQuery) -> Result<SearchResults> {
    let start = Instant::now();
    let text = q.text.trim();
    let limit = clamp_limit(q.limit, 50, 500);
    let offset = q.offset.unwrap_or(0) as i64;
    let b = build(core, q, text)?;
    let wsql = b.where_sql();
    let count_sql = format!(
        "SELECT count(*) FROM (SELECT 1 FROM {}{} LIMIT {TOTAL_CAP})",
        b.from, wsql
    );
    let page_sql = format!(
        "SELECT i.src_rowid, i.chat_jid, i.id FROM {}{} ORDER BY i.ts DESC, i.src_rowid DESC LIMIT ? OFFSET ?",
        b.from, wsql
    );
    let (total, keys) = core.app.read(|c| {
        let total: i64 = c
            .prepare(&count_sql)?
            .query_row(rusqlite::params_from_iter(b.params.iter()), |r| r.get(0))?;
        let mut p = b.params.clone();
        p.push(Value::from(limit as i64));
        p.push(Value::from(offset));
        let mut st = c.prepare(&page_sql)?;
        let keys = st
            .query_map(rusqlite::params_from_iter(p), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((total, keys))
    })?;
    let names = core.names.snapshot()?;
    let conn = core.bridge.get()?;
    let rows: Vec<_> = fetch_keyed(&conn, &keys)?.into_iter().flatten().collect();
    let msgs = assemble(core, &names, &conn, &rows)?;
    drop(conn);
    let hits = msgs
        .into_iter()
        .map(|m| {
            let mut highlights = highlight_ranges(&m.content, text);
            if highlights.is_empty()
                && let Some(f) = m.media.as_ref().and_then(|x| x.filename.as_deref())
            {
                highlights = highlight_ranges(f, text);
            }
            SearchHit {
                chat_name: names.chat_name(&m.chat_jid),
                message: m,
                highlights,
            }
        })
        .collect();
    Ok(SearchResults {
        hits,
        total: (total.max(0) as u32).min(TOTAL_CAP),
        took_ms: start.elapsed().as_millis().min(u32::MAX as u128) as u32,
        index_building: !core.index_ready()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_quoting() {
        assert_eq!(fts_phrase("a\"b"), "\"a\"\"b\"");
        assert_eq!(fts_phrase("x* -y ^z:(w)"), "\"x* -y ^z:(w)\"");
    }
}
