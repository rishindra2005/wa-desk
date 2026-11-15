//! Timestamp handling for the bridge's `messages.db`.
//!
//! go-sqlite3 stores `time.Time` as TEXT. Observed forms:
//! `2026-11-15 23:04:39+05:30`, with fractional seconds, `T` separator, `Z`,
//! and (from main.go:2257) no offset at all. Text comparison across different
//! offsets is wrong, so everything is normalised to a UTC epoch here.
//! Values with no offset are interpreted as UTC, which is what go-sqlite3 does
//! when reading them back (it has no `_loc` parameter in the bridge DSN).

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};

/// Parse a stored timestamp to Unix seconds (UTC). `None` for empty/garbage.
pub fn parse_ts(raw: &str) -> Option<i64> {
    parse_ts_fixed(raw).map(|d| d.timestamp())
}

/// Parse keeping the stored offset (no-offset values become +00:00).
pub fn parse_ts_fixed(raw: &str) -> Option<DateTime<FixedOffset>> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // Normalise: date/time separator 'T' (byte 10 only — zone names like "IST" contain T) -> ' ',
    // trailing 'Z' -> '+00:00'.
    let mut norm = s.to_string();
    if norm.as_bytes().get(10) == Some(&b'T') {
        norm.replace_range(10..11, " ");
    }
    if let Some(stripped) = norm.strip_suffix('Z') {
        norm = format!("{stripped}+00:00");
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%.f%z",
        "%Y-%m-%d %H:%M:%S%.f %:z",
        "%Y-%m-%d %H:%M:%S%.f %z",
    ] {
        if let Ok(d) = DateTime::parse_from_str(&norm, fmt) {
            return Some(d);
        }
    }
    // Go's String() form: "2006-01-02 15:04:05.999999999 -0700 MST" (rare, but seen in some drivers).
    if let Some(idx) = norm.rfind(' ') {
        let head = &norm[..idx];
        if norm[idx + 1..].chars().all(|c| c.is_ascii_alphabetic())
            && let Ok(d) = DateTime::parse_from_str(head, "%Y-%m-%d %H:%M:%S%.f %z")
        {
            return Some(d);
        }
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(&norm, fmt) {
            return Some(
                FixedOffset::east_opt(0)
                    .expect("zero offset")
                    .from_utc_datetime(&n),
            );
        }
    }
    None
}

/// The literal `YYYYMMDD_HHMMSS` digits of the stored value, in its own offset.
/// This is what the bridge uses in media file names (Go `time.Format` on the
/// value as parsed, which keeps the stored offset).
pub fn store_name_stamp(raw: &str) -> Option<String> {
    parse_ts_fixed(raw).map(|d| d.format("%Y%m%d_%H%M%S").to_string())
}

/// Current Unix time in seconds.
pub fn now_epoch() -> i64 {
    Utc::now().timestamp()
}

/// Format an epoch as RFC3339 UTC (for logs, bundles, bridge requests).
pub fn epoch_to_rfc3339(epoch: i64) -> String {
    Utc.timestamp_opt(epoch, 0)
        .single()
        .map(|d| d.to_rfc3339())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_offset_forms() {
        // 23:04:39 IST == 17:34:39 UTC
        let want = 1_794_764_079;
        assert_eq!(parse_ts("2026-11-15 23:04:39+05:30"), Some(want));
        assert_eq!(parse_ts("2026-11-15T23:04:39+05:30"), Some(want));
        assert_eq!(parse_ts("2026-11-15 17:34:39Z"), Some(want));
        assert_eq!(parse_ts("2026-11-15T17:34:39Z"), Some(want));
        assert_eq!(parse_ts("2026-11-15 23:04:39.123456789+05:30"), Some(want));
        assert_eq!(parse_ts("2026-11-15 23:04:39 +0530"), Some(want));
        assert_eq!(parse_ts("2026-11-15 23:04:39.5 +0530 IST"), Some(want));
    }

    #[test]
    fn no_offset_is_utc() {
        assert_eq!(parse_ts("2026-11-15 17:34:39"), Some(1_794_764_079));
        assert_eq!(parse_ts("2026-11-15 17:34"), Some(1_794_764_040));
    }

    #[test]
    fn mixed_offsets_order_correctly() {
        // Text order says a < b, real order is b < a.
        let a = "2026-11-15 10:00:00+00:00";
        let b = "2026-11-15 13:00:00+05:30"; // 07:30 UTC
        assert!(a < b);
        assert!(parse_ts(b).unwrap() < parse_ts(a).unwrap());
    }

    #[test]
    fn garbage_and_empty() {
        assert_eq!(parse_ts(""), None);
        assert_eq!(parse_ts("   "), None);
        assert_eq!(parse_ts("yesterday"), None);
        assert_eq!(parse_ts("2026-13-45 99:99:99"), None);
    }

    #[test]
    fn store_stamp_keeps_stored_offset() {
        assert_eq!(
            store_name_stamp("2026-11-15 20:31:02+05:30").as_deref(),
            Some("20261115_203102")
        );
        assert_eq!(
            store_name_stamp("2026-11-15 20:31:02").as_deref(),
            Some("20261115_203102")
        );
        assert_eq!(
            store_name_stamp("2026-11-15T15:01:02Z").as_deref(),
            Some("20261115_150102")
        );
    }

    #[test]
    fn rfc3339_roundtrip() {
        assert_eq!(epoch_to_rfc3339(0), "1970-01-01T00:00:00+00:00");
    }
}
