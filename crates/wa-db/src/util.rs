//! Small shared helpers: error mapping, row hashing, cursors, SQL fragments.

use rusqlite::ErrorCode;
use sha2::{Digest, Sha256};
use wa_core::{AppError, ErrorKind};

/// Map a rusqlite error to the shared error type.
pub(crate) fn sql_err(e: rusqlite::Error) -> AppError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _) => match f.code {
            ErrorCode::OperationInterrupted => {
                AppError::timeout("database query exceeded its deadline").with_detail(e.to_string())
            }
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => {
                AppError::busy("database is busy").with_detail(e.to_string())
            }
            ErrorCode::CannotOpen => {
                AppError::not_found("database not available").with_detail(e.to_string())
            }
            _ => AppError::internal("database error").with_detail(e.to_string()),
        },
        rusqlite::Error::QueryReturnedNoRows => AppError::not_found("no such row"),
        _ => AppError::internal("database error").with_detail(e.to_string()),
    }
}

/// `true` when the error means "try again later" rather than a bug.
#[allow(dead_code)]
pub(crate) fn is_transient(e: &AppError) -> bool {
    matches!(e.kind, ErrorKind::Busy | ErrorKind::Timeout)
}

/// Fields of a bridge row that define its visible state (see `row_hash`).
pub(crate) struct HashFields<'a> {
    pub content: &'a str,
    pub deleted_at: Option<&'a str>,
    pub sender: &'a str,
    pub media_type: &'a str,
    pub filename: Option<&'a str>,
    pub timestamp: &'a str,
}

/// Stable 63-bit hash of `content|deleted_at|sender|media_type|filename|timestamp`.
pub(crate) fn row_hash(f: &HashFields<'_>) -> i64 {
    let mut h = Sha256::new();
    for part in [
        f.content,
        f.deleted_at.unwrap_or("\u{0}"),
        f.sender,
        f.media_type,
        f.filename.unwrap_or("\u{0}"),
        f.timestamp,
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    let d = h.finalize();
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    (u64::from_le_bytes(b) >> 1) as i64
}

/// "?,?,?" with `n` placeholders (n ≥ 1).
pub(crate) fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

/// Escape `%`, `_` and `\` for `LIKE ... ESCAPE '\'`.
pub(crate) fn like_escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 4);
    for c in v.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Thread cursor `"<ts>:<rowid>"`.
pub(crate) fn encode_msg_cursor(ts: i64, rowid: i64) -> String {
    format!("{ts}:{rowid}")
}

/// Parse a thread cursor; `Invalid` on garbage.
pub(crate) fn decode_msg_cursor(c: &str) -> wa_core::Result<(i64, i64)> {
    let (a, b) = c
        .split_once(':')
        .ok_or_else(|| AppError::invalid("malformed cursor"))?;
    let ts = a
        .parse::<i64>()
        .map_err(|_| AppError::invalid("malformed cursor"))?;
    let rowid = b
        .parse::<i64>()
        .map_err(|_| AppError::invalid("malformed cursor"))?;
    Ok((ts, rowid))
}

/// Offset cursor `"o:<n>"`.
pub(crate) fn encode_offset_cursor(n: usize) -> String {
    format!("o:{n}")
}

/// Parse an offset cursor (None → 0).
pub(crate) fn decode_offset_cursor(c: Option<&str>) -> wa_core::Result<usize> {
    match c {
        None => Ok(0),
        Some(s) => s
            .strip_prefix("o:")
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or_else(|| AppError::invalid("malformed cursor")),
    }
}

/// Clamp an optional page size.
pub(crate) fn clamp_limit(l: Option<u32>, default: u32, max: u32) -> usize {
    l.unwrap_or(default).clamp(1, max) as usize
}

/// First `max` chars of `s` (on a char boundary), newlines folded to spaces.
pub(crate) fn preview_text(s: &str, max: usize) -> String {
    let folded: String = s
        .trim()
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .take(max)
        .collect();
    folded
}

/// Case-insensitive, char-boundary-safe match ranges of `needle` in `hay` (byte offsets).
pub(crate) fn highlight_ranges(hay: &str, needle: &str) -> Vec<(u32, u32)> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let idx: Vec<(usize, char)> = hay.char_indices().collect();
    let mut i = 0;
    while i < idx.len() {
        if let Some(end) = match_at(&idx, i, &needle) {
            let start_b = idx[i].0;
            let end_b = idx.get(end).map_or(hay.len(), |x| x.0);
            out.push((start_b as u32, end_b as u32));
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// If `needle` (lowercased chars) matches starting at char index `i`, return the end char index.
fn match_at(idx: &[(usize, char)], i: usize, needle: &[char]) -> Option<usize> {
    let mut n = 0;
    let mut j = i;
    while n < needle.len() {
        let (_, c) = *idx.get(j)?;
        for lc in c.to_lowercase() {
            if needle.get(n) != Some(&lc) {
                return None;
            }
            n += 1;
        }
        j += 1;
    }
    Some(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_differs_per_field() {
        let base = HashFields {
            content: "a",
            deleted_at: None,
            sender: "s",
            media_type: "",
            filename: None,
            timestamp: "t",
        };
        let h = row_hash(&base);
        assert!(h >= 0);
        let d = HashFields {
            deleted_at: Some("x"),
            ..base
        };
        assert_ne!(h, row_hash(&d));
        // Field boundaries matter.
        let a = HashFields {
            content: "ab",
            sender: "c",
            ..d
        };
        let b = HashFields {
            content: "a",
            sender: "bc",
            ..a
        };
        assert_ne!(row_hash(&a), row_hash(&b));
    }

    #[test]
    fn cursors_roundtrip_and_reject_garbage() {
        assert_eq!(
            decode_msg_cursor(&encode_msg_cursor(-5, 9)).unwrap(),
            (-5, 9)
        );
        assert!(decode_msg_cursor("x").is_err());
        assert!(decode_msg_cursor("1:y").is_err());
        assert_eq!(decode_offset_cursor(Some("o:7")).unwrap(), 7);
        assert!(decode_offset_cursor(Some("7")).is_err());
        assert_eq!(decode_offset_cursor(None).unwrap(), 0);
    }

    #[test]
    fn highlights_are_char_safe() {
        let s = "नमस्ते सभी 🙏 exam कल है EXAM";
        let r = highlight_ranges(s, "exam");
        assert_eq!(r.len(), 2);
        for (a, b) in &r {
            assert!(s.is_char_boundary(*a as usize) && s.is_char_boundary(*b as usize));
            assert_eq!(s[*a as usize..*b as usize].to_lowercase(), "exam");
        }
        let r = highlight_ranges(s, "🙏");
        assert_eq!(&s[r[0].0 as usize..r[0].1 as usize], "🙏");
        let r = highlight_ranges(s, "सभी");
        assert_eq!(&s[r[0].0 as usize..r[0].1 as usize], "सभी");
        assert!(highlight_ranges(s, "").is_empty());
        assert_eq!(highlight_ranges("aaaa", "aa"), vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn like_escape_and_preview() {
        assert_eq!(like_escape("50%_\\"), "50\\%\\_\\\\");
        let long = "é".repeat(200);
        assert_eq!(preview_text(&long, 120).chars().count(), 120);
        assert_eq!(preview_text(" a\nb ", 120), "a b");
    }
}
