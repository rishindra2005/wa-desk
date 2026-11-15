//! Reading the JSON log files back for the Logs view and report bundles.

use std::path::{Path, PathBuf};

use serde_json::Value;
use wa_core::Result;
use wa_core::config::Paths;
use wa_core::types::{LogFilter, LogLevel, LogLine};

use crate::logging::{LOG_PREFIX, LOG_SUFFIX};
use crate::redact::redact;

const DEFAULT_LIMIT: u32 = 500;
const MAX_LIMIT: u32 = 5000;

/// Log files in `dir`, newest first (rotated names sort by date).
pub fn log_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with(LOG_PREFIX) && n.ends_with(&format!(".{LOG_SUFFIX}"))
                })
        })
        .collect();
    v.sort();
    v.reverse();
    v
}

fn parse_level(s: &str) -> Option<LogLevel> {
    match s.to_ascii_uppercase().as_str() {
        "TRACE" => Some(LogLevel::Trace),
        "DEBUG" => Some(LogLevel::Debug),
        "INFO" => Some(LogLevel::Info),
        "WARN" | "WARNING" => Some(LogLevel::Warn),
        "ERROR" => Some(LogLevel::Error),
        _ => None,
    }
}

/// Parse one line written by the JSON file layer (None if partial/corrupt).
pub fn parse_line(line: &str) -> Option<LogLine> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let ts = chrono::DateTime::parse_from_rfc3339(v.get("timestamp")?.as_str()?)
        .ok()?
        .timestamp();
    let level = parse_level(v.get("level")?.as_str()?)?;
    let target = v
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut fields = v.get("fields").cloned().unwrap_or(Value::Null);
    let message = match fields.as_object_mut().and_then(|m| m.remove("message")) {
        Some(Value::String(s)) => s,
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let fields = match fields {
        Value::Object(m) if !m.is_empty() => Value::Object(m).to_string(),
        _ => String::new(),
    };
    Some(LogLine {
        ts,
        level,
        target,
        message,
        fields,
    })
}

fn matches(l: &LogLine, f: &LogFilter, needle: Option<&str>) -> bool {
    if l.level < f.level.unwrap_or(LogLevel::Info) {
        return false;
    }
    if f.since.is_some_and(|s| l.ts < s) {
        return false;
    }
    if let Some(t) = f.target.as_deref().filter(|t| !t.is_empty())
        && !l.target.starts_with(t)
    {
        return false;
    }
    if let Some(n) = needle {
        let hay = format!("{} {}", l.message, l.fields).to_lowercase();
        if !hay.contains(n) {
            return false;
        }
    }
    true
}

/// Newest-first log lines across rotated files; corrupt lines are skipped.
/// Results are redacted.
pub fn query_logs(paths: &Paths, filter: &LogFilter) -> Result<Vec<LogLine>> {
    let limit = filter.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;
    let needle = filter
        .contains
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let mut out = Vec::new();
    for file in log_files(&paths.logs) {
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines().rev() {
            let Some(mut l) = parse_line(line) else {
                continue;
            };
            if !matches(&l, filter, needle.as_deref()) {
                continue;
            }
            l.message = redact(&l.message);
            l.fields = redact(&l.fields);
            out.push(l);
            if out.len() >= limit {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// Last `n` raw lines across files, oldest first (for bundles; not redacted).
pub fn tail_raw_lines(dir: &Path, n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for file in log_files(dir) {
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines().rev() {
            if line.trim().is_empty() {
                continue;
            }
            out.push(line.to_string());
            if out.len() >= n {
                out.reverse();
                return out;
            }
        }
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(ts: &str, level: &str, target: &str, msg: &str) -> String {
        serde_json::json!({
            "timestamp": ts, "level": level, "target": target,
            "fields": {"message": msg, "chat": "x"}
        })
        .to_string()
    }

    fn setup() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::under_root(d.path(), "http://127.0.0.1:8080".into());
        std::fs::create_dir_all(&p.logs).unwrap();
        let mut old = std::fs::File::create(p.logs.join("wa-desk.2026-11-14.jsonl")).unwrap();
        writeln!(
            old,
            "{}",
            line("2026-11-14T10:00:00.000Z", "INFO", "wa_db", "old info")
        )
        .unwrap();
        writeln!(
            old,
            "{}",
            line(
                "2026-11-14T11:00:00.000Z",
                "ERROR",
                "wa_jobs::backfill",
                "old error"
            )
        )
        .unwrap();
        writeln!(old, "{{\"timestamp\":\"2026-11-14T12:00").unwrap(); // partial line (crash mid-write)
        let mut new = std::fs::File::create(p.logs.join("wa-desk.2026-11-15.jsonl")).unwrap();
        writeln!(new, "garbage not json").unwrap();
        writeln!(
            new,
            "{}",
            line(
                "2026-11-15T09:00:00.123456Z",
                "DEBUG",
                "wa_bridge",
                "debug detail"
            )
        )
        .unwrap();
        writeln!(
            new,
            "{}",
            line(
                "2026-11-15T10:00:00Z",
                "WARN",
                "wa_bridge",
                "token 0123456789abcdef0123456789abcdef from 919811111234"
            )
        )
        .unwrap();
        new.write_all(b"\xff\xfe broken utf8\n").unwrap();
        std::fs::write(p.logs.join("other.txt"), "ignored").unwrap();
        (d, p)
    }

    #[test]
    fn newest_first_across_rotated_files() {
        let (_d, p) = setup();
        let l = query_logs(&p, &LogFilter::default()).unwrap();
        let msgs: Vec<_> = l.iter().map(|l| l.message.as_str()).collect();
        assert_eq!(msgs.len(), 3, "{msgs:?}");
        assert!(msgs[0].starts_with("token [redacted]"));
        assert_eq!(msgs[1..], ["old error", "old info"]);
        assert_eq!(l[0].fields, r#"{"chat":"x"}"#);
        assert!(!msgs[0].contains("919811111234"));
    }

    #[test]
    fn filters_level_target_contains_since_limit() {
        let (_d, p) = setup();
        let q = |f: LogFilter| query_logs(&p, &f).unwrap();
        assert_eq!(
            q(LogFilter {
                level: Some(LogLevel::Debug),
                ..Default::default()
            })
            .len(),
            4
        );
        assert_eq!(
            q(LogFilter {
                level: Some(LogLevel::Error),
                ..Default::default()
            })
            .len(),
            1
        );
        assert_eq!(
            q(LogFilter {
                target: Some("wa_jobs".into()),
                ..Default::default()
            })
            .len(),
            1
        );
        assert_eq!(
            q(LogFilter {
                contains: Some("OLD".into()),
                ..Default::default()
            })
            .len(),
            2
        );
        assert_eq!(
            q(LogFilter {
                contains: Some("CHAT".into()),
                ..Default::default()
            })
            .len(),
            3,
            "matches fields"
        );
        let since = chrono::DateTime::parse_from_rfc3339("2026-11-15T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            q(LogFilter {
                since: Some(since),
                ..Default::default()
            })
            .len(),
            1
        );
        assert_eq!(
            q(LogFilter {
                limit: Some(1),
                level: Some(LogLevel::Trace),
                ..Default::default()
            })
            .len(),
            1
        );
    }

    #[test]
    fn missing_dir_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::under_root(&d.path().join("nope"), "http://127.0.0.1:8080".into());
        assert!(query_logs(&p, &LogFilter::default()).unwrap().is_empty());
        assert!(tail_raw_lines(&p.logs, 10).is_empty());
    }

    #[test]
    fn tail_is_chronological() {
        let (_d, p) = setup();
        let t = tail_raw_lines(&p.logs, 3);
        assert_eq!(t.len(), 3);
        assert!(t[2].contains("broken utf8"));
        assert!(t[0].contains("debug detail"));
    }

    #[test]
    fn parses_real_subscriber_output() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.jsonl");
        let f = std::fs::File::create(&file).unwrap();
        let sub = tracing_subscriber::fmt()
            .json()
            .with_current_span(false)
            .with_span_list(false)
            .with_writer(std::sync::Mutex::new(f))
            .finish();
        tracing::subscriber::with_default(sub, || {
            tracing::info!(target: "wa_test", n = 3, "hello {}", "world");
        });
        let text = std::fs::read_to_string(&file).unwrap();
        let l = parse_line(text.lines().next().unwrap()).unwrap();
        assert_eq!(
            (l.level, l.target.as_str(), l.message.as_str()),
            (LogLevel::Info, "wa_test", "hello world")
        );
        assert_eq!(l.fields, r#"{"n":3}"#);
        assert!(l.ts > 1_700_000_000);
    }
}
