//! Media lookup: local file resolution, `MediaRef`s and the media library.
//!
//! Resolution order for a message's file:
//! 1. the bridge store path (`wa_core::paths::bridge_media_path`) if it exists → `Store`;
//! 2. a phone link (app.db `phone_links`, then messages.db `phone_media_links`)
//!    whose store copy or phone file exists → `Phone`;
//! 3. otherwise `Missing`.

use std::path::{Path, PathBuf};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use wa_core::paths::{bridge_media_path, media_ext, resolve_within};
use wa_core::time::store_name_stamp;
use wa_core::traits::MediaInfo;
use wa_core::types::{MediaItem, MediaKind, MediaQuery, MediaRef, MediaSource, Page};
use wa_core::{AppError, Result};

use crate::Core;
use crate::app_db::StoredLink;
use crate::bridge_ro::{PAGE_ROWS, RoConn};
use crate::names::{NameCache, is_status_jid};
use crate::raw::{self, RawMsg};
use crate::util::{clamp_limit, decode_msg_cursor, encode_msg_cursor, like_escape, placeholders};

/// Characters kept verbatim in `wa-media` URL path segments.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// `wa-media://localhost/<prefix>/<chat>/<id>` with both segments percent-encoded.
pub fn media_url(prefix: &str, chat_jid: &str, message_id: &str) -> String {
    format!(
        "wa-media://localhost/{prefix}/{}/{}",
        utf8_percent_encode(chat_jid, SEGMENT),
        utf8_percent_encode(message_id, SEGMENT)
    )
}

/// Reject ids that could escape a directory or confuse path building.
pub fn validate_component(what: &str, v: &str) -> Result<()> {
    if v.is_empty()
        || v.len() > 256
        || v.contains('/')
        || v.contains('\\')
        || v.contains('\0')
        || v.contains("..")
        || v == "."
    {
        return Err(AppError::invalid(format!("invalid {what}")));
    }
    Ok(())
}

/// MIME type from file name, else from kind.
pub fn guess_mime(filename: Option<&str>, kind: MediaKind) -> String {
    if let Some(m) = filename.and_then(|f| mime_guess::from_path(f).first()) {
        return m.essence_str().to_string();
    }
    match kind {
        MediaKind::Image => "image/jpeg",
        MediaKind::Video => "video/mp4",
        MediaKind::Audio => "audio/ogg",
        MediaKind::Sticker => "image/webp",
        MediaKind::Document => "application/octet-stream",
    }
    .to_string()
}

/// Original name for documents, else the bridge's store file name.
fn display_filename(raw: &RawMsg, kind: MediaKind) -> String {
    if let Some(f) = raw.filename.as_deref().filter(|f| !f.trim().is_empty())
        && kind == MediaKind::Document
    {
        return f.to_string();
    }
    let t = kind.as_db();
    match store_name_stamp(&raw.raw_ts) {
        Some(s) => format!("{t}_{s}_{}{}", raw.id, media_ext(t)),
        None => format!("{t}_{}{}", raw.id, media_ext(t)),
    }
}

fn existing(p: &Path) -> Option<PathBuf> {
    p.is_file().then(|| p.to_path_buf())
}

fn link_path(l: &StoredLink) -> Option<PathBuf> {
    l.store_path
        .as_deref()
        .and_then(existing)
        .or_else(|| existing(&l.phone_path))
}

/// The messages.db `phone_media_links` row for a message (None if the table is absent).
fn bridge_link(conn: &RoConn, chat_jid: &str, id: &str) -> Result<Option<StoredLink>> {
    if !conn.has_table("phone_media_links")? {
        return Ok(None);
    }
    conn.row(
        "SELECT phone_path, store_path FROM phone_media_links WHERE message_id = ?1 AND chat_jid = ?2",
        [id, chat_jid],
        |r| {
            Ok(StoredLink {
                phone_path: PathBuf::from(r.get::<_, String>(0)?),
                store_path: r.get::<_, Option<String>>(1)?.map(PathBuf::from),
            })
        },
    )
}

/// Find the local file for a media row (see module docs for the order).
pub(crate) fn locate(
    core: &Core,
    conn: &RoConn,
    raw: &RawMsg,
) -> Result<(Option<PathBuf>, MediaSource)> {
    if raw.kind().is_none() {
        return Ok((None, MediaSource::Missing));
    }
    if let Ok(p) = bridge_media_path(
        &core.paths.bridge_store,
        &raw.chat_jid,
        &raw.media_type,
        &raw.id,
        &raw.raw_ts,
    ) && let Some(p) = existing(&p)
    {
        return Ok((Some(p), MediaSource::Store));
    }
    if let Some(p) = core
        .app
        .phone_link(&raw.chat_jid, &raw.id)?
        .as_ref()
        .and_then(link_path)
    {
        return Ok((Some(p), MediaSource::Phone));
    }
    if let Some(p) = bridge_link(conn, &raw.chat_jid, &raw.id)?
        .as_ref()
        .and_then(link_path)
    {
        return Ok((Some(p), MediaSource::Phone));
    }
    Ok((None, MediaSource::Missing))
}

/// Build the `MediaRef` for a (non-deleted) media row.
pub(crate) fn media_ref(core: &Core, conn: &RoConn, raw: &RawMsg) -> Result<Option<MediaRef>> {
    let Some(kind) = raw.kind() else {
        return Ok(None);
    };
    let (path, source) = locate(core, conn, raw)?;
    let filename = display_filename(raw, kind);
    let mime = guess_mime(Some(&filename), kind);
    let size = raw
        .file_length
        .filter(|n| *n > 0)
        .map(|n| n as u64)
        .or_else(|| {
            path.as_ref()
                .and_then(|p| p.metadata().ok())
                .map(|m| m.len())
        });
    let local = path.is_some();
    let thumbable = matches!(
        kind,
        MediaKind::Image | MediaKind::Video | MediaKind::Sticker
    ) || (kind == MediaKind::Document && mime == "application/pdf");
    Ok(Some(MediaRef {
        kind,
        filename: Some(filename),
        size,
        mime: Some(mime),
        source,
        url: local.then(|| media_url("m", &raw.chat_jid, &raw.id)),
        thumb_url: (local && thumbable).then(|| media_url("t", &raw.chat_jid, &raw.id)),
    }))
}

/// `MediaInfo` for a raw row.
pub(crate) fn info_of(core: &Core, conn: &RoConn, raw: &RawMsg) -> Result<Option<MediaInfo>> {
    let Some(kind) = raw.kind() else {
        return Ok(None);
    };
    let (local_path, source) = locate(core, conn, raw)?;
    Ok(Some(MediaInfo {
        chat_jid: raw.chat_jid.clone(),
        message_id: raw.id.clone(),
        kind,
        filename: raw.filename.clone(),
        file_length: raw.file_length.filter(|n| *n >= 0).map(|n| n as u64),
        file_sha256: raw.file_sha256.clone(),
        raw_ts: raw.raw_ts.clone(),
        ts: raw.ts,
        local_path,
        source,
    }))
}

/// `MessageStore::media_info`.
pub(crate) fn media_info(core: &Core, chat_jid: &str, id: &str) -> Result<Option<MediaInfo>> {
    let conn = core.bridge.get()?;
    match raw::one(&conn, chat_jid, id)? {
        Some(r) => info_of(core, &conn, &r),
        None => Ok(None),
    }
}

/// `MessageStore::media_rows`: scans messages.db in rowid windows of `PAGE_ROWS`.
pub(crate) fn media_rows(
    core: &Core,
    since: Option<i64>,
    kinds: &[MediaKind],
) -> Result<Vec<MediaInfo>> {
    let kinds: Vec<&str> = if kinds.is_empty() {
        vec!["image", "video", "audio", "document", "sticker"]
    } else {
        kinds.iter().map(|k| k.as_db()).collect()
    };
    let conn = core.bridge.get()?;
    let max = raw::max_rowid(&conn)?;
    let sql = format!(
        "SELECT {} FROM messages WHERE rowid > ? AND rowid <= ? AND media_type IN ({})",
        raw::COLS,
        placeholders(kinds.len())
    );
    let mut out = Vec::new();
    let mut lo = 0i64;
    while lo < max {
        let hi = lo + PAGE_ROWS as i64;
        let mut params: Vec<rusqlite::types::Value> = vec![lo.into(), hi.into()];
        params.extend(
            kinds
                .iter()
                .map(|k| rusqlite::types::Value::Text((*k).to_string())),
        );
        let rows = conn.rows(&sql, rusqlite::params_from_iter(params), RawMsg::from_row)?;
        for r in rows {
            if r.is_deleted() || since.is_some_and(|s| r.ts < s) {
                continue;
            }
            if let Some(i) = info_of(core, &conn, &r)? {
                out.push(i);
            }
        }
        lo = hi;
    }
    Ok(out)
}

/// Absolute, scope-checked local path for the protocol handler.
/// `Invalid` for malformed ids; `Ok(None)` when unknown or not on disk.
pub(crate) fn resolve_media_path(core: &Core, chat_jid: &str, id: &str) -> Result<Option<PathBuf>> {
    validate_component("chat JID", chat_jid)?;
    validate_component("message ID", id)?;
    let conn = core.bridge.get()?;
    let Some(r) = raw::one(&conn, chat_jid, id)? else {
        return Ok(None);
    };
    if r.is_deleted() {
        return Ok(None);
    }
    let (path, _) = locate(core, &conn, &r)?;
    drop(conn);
    match path {
        Some(p) => resolve_within(&p, &core.paths.media_roots()).map(Some),
        None => Ok(None),
    }
}

/// `list_media`: newest first from the index, cursor "ts:rowid".
pub(crate) fn list_media(core: &Core, q: &MediaQuery) -> Result<Page<MediaItem>> {
    let limit = clamp_limit(q.limit, 100, 500);
    let mut cursor = match q.cursor.as_deref() {
        Some(c) => decode_msg_cursor(c)?,
        None => (i64::MAX, i64::MAX),
    };
    let names = core.names.snapshot()?;
    let mut items = Vec::new();
    let mut scanned = 0usize;
    let batch = (limit * 2).clamp(50, PAGE_ROWS);
    loop {
        let keys = media_keys(core, q, cursor, batch)?;
        let exhausted = keys.len() < batch;
        scanned += keys.len();
        let mut full_at: Option<(i64, i64)> = None;
        let built = build_items(core, &names, &keys)?;
        for (key, item) in keys.iter().map(|k| (k.0, k.1)).zip(built) {
            cursor = key;
            let Some(item) = item else { continue };
            let local = item.media.source != MediaSource::Missing;
            if q.available.is_some_and(|want| want != local) {
                continue;
            }
            items.push(item);
            if items.len() == limit {
                full_at = Some(key);
                break;
            }
        }
        if let Some(k) = full_at {
            let more = !exhausted || keys.last().map(|l| (l.0, l.1)) != Some(k);
            return Ok(Page {
                items,
                next_cursor: more.then(|| encode_msg_cursor(k.0, k.1)),
            });
        }
        if exhausted {
            return Ok(Page {
                items,
                next_cursor: None,
            });
        }
        if scanned >= 5000 {
            return Ok(Page {
                items,
                next_cursor: Some(encode_msg_cursor(cursor.0, cursor.1)),
            });
        }
    }
}

/// (ts, src_rowid, chat_jid, id) of indexed media older than `before`.
type MediaKey = (i64, i64, String, String);

fn media_keys(core: &Core, q: &MediaQuery, before: (i64, i64), n: usize) -> Result<Vec<MediaKey>> {
    let mut sql = String::from(
        "SELECT i.ts, i.src_rowid, i.chat_jid, i.id FROM msg_index i \
         LEFT JOIN msg_fts f ON f.rowid = i.rowid \
         WHERE i.media_kind IS NOT NULL AND i.deleted = 0 AND (i.ts, i.src_rowid) < (?, ?)",
    );
    let mut p: Vec<rusqlite::types::Value> = vec![before.0.into(), before.1.into()];
    if let Some(c) = &q.chat_jid {
        sql.push_str(" AND i.chat_jid = ?");
        p.push(c.clone().into());
    }
    if !q.kinds.is_empty() {
        sql.push_str(&format!(
            " AND i.media_kind IN ({})",
            placeholders(q.kinds.len())
        ));
        p.extend(q.kinds.iter().map(|k| k.as_db().to_string().into()));
    }
    if let Some(a) = q.after {
        sql.push_str(" AND i.ts >= ?");
        p.push(a.into());
    }
    if let Some(b) = q.before {
        sql.push_str(" AND i.ts < ?");
        p.push(b.into());
    }
    if let Some(t) = q.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let pat = format!("%{}%", like_escape(t));
        sql.push_str(" AND (f.filename LIKE ? ESCAPE '\\' OR f.content LIKE ? ESCAPE '\\')");
        p.push(pat.clone().into());
        p.push(pat.into());
    }
    sql.push_str(" ORDER BY i.ts DESC, i.src_rowid DESC LIMIT ?");
    p.push((n as i64).into());
    core.app.read(|c| {
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(p), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.collect()
    })
}

/// Fetch bridge rows for index keys (falling back to (chat, id) when a rowid moved).
pub(crate) fn fetch_keyed(
    conn: &RoConn,
    keys: &[(i64, String, String)],
) -> Result<Vec<Option<RawMsg>>> {
    let rowids: Vec<i64> = keys.iter().map(|k| k.0).collect();
    let mut by_rowid: std::collections::HashMap<i64, RawMsg> = raw::by_rowids(conn, &rowids)?
        .into_iter()
        .map(|r| (r.rowid, r))
        .collect();
    let mut out = Vec::with_capacity(keys.len());
    for (rowid, chat, id) in keys {
        match by_rowid.remove(rowid) {
            Some(r) if &r.chat_jid == chat && &r.id == id => out.push(Some(r)),
            _ => out.push(raw::one(conn, chat, id)?),
        }
    }
    Ok(out)
}

fn build_items(
    core: &Core,
    names: &NameCache,
    keys: &[MediaKey],
) -> Result<Vec<Option<MediaItem>>> {
    let conn = core.bridge.get()?;
    let k: Vec<(i64, String, String)> = keys
        .iter()
        .map(|k| (k.1, k.2.clone(), k.3.clone()))
        .collect();
    let rows = fetch_keyed(&conn, &k)?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let Some(r) = r.filter(|r| !r.is_deleted() && !is_status_jid(&r.chat_jid)) else {
            out.push(None);
            continue;
        };
        let Some(media) = media_ref(core, &conn, &r)? else {
            out.push(None);
            continue;
        };
        out.push(Some(MediaItem {
            message_id: r.id.clone(),
            chat_jid: r.chat_jid.clone(),
            chat_name: names.chat_name(&r.chat_jid),
            sender_name: names.sender_name(&crate::messages::sender_of(&r), r.is_from_me),
            is_from_me: r.is_from_me,
            ts: r.ts,
            caption: r.content.clone(),
            media,
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_percent_encoded() {
        assert_eq!(
            media_url("m", "120363@g.us", "3EB0/..?x"),
            "wa-media://localhost/m/120363%40g.us/3EB0%2F..%3Fx"
        );
        assert_eq!(
            media_url("t", "a:b@s.whatsapp.net", "ID-1_x.~"),
            "wa-media://localhost/t/a%3Ab%40s.whatsapp.net/ID-1_x.~"
        );
    }

    #[test]
    fn mime_guessing() {
        assert_eq!(
            guess_mime(Some("a.pdf"), MediaKind::Document),
            "application/pdf"
        );
        assert_eq!(
            guess_mime(Some("noext"), MediaKind::Document),
            "application/octet-stream"
        );
        assert_eq!(guess_mime(None, MediaKind::Image), "image/jpeg");
        assert_eq!(guess_mime(None, MediaKind::Audio), "audio/ogg");
        assert_eq!(guess_mime(Some("x.ogg"), MediaKind::Audio), "audio/ogg");
    }

    #[test]
    fn component_validation() {
        for bad in ["", "a/b", "..", "a..b", "a\\b", "a\0b", "."] {
            assert!(validate_component("id", bad).is_err(), "{bad:?}");
        }
        assert!(validate_component("id", "3EB0ABC").is_ok());
        assert!(validate_component("id", "120363@g.us").is_ok());
    }
}
