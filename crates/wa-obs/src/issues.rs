//! Deduplicated issue tracking in its own SQLite file (`obs.db`).

use std::cell::Cell;
use std::path::Path;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use wa_core::config::Paths;
use wa_core::time::now_epoch;
use wa_core::traits::IssueSink;
use wa_core::types::{Issue, IssueStatus};
use wa_core::{AppError, ErrorKind, Result};

use crate::redact::{redact, truncate};

/// Called (outside any lock) when an issue is new or reopened.
pub type IssueListener = Arc<dyn Fn(&Issue) + Send + Sync>;

const MAX_TITLE: usize = 200;
const MAX_DETAIL: usize = 4000;

thread_local! {
    /// Set while recording, so logging inside the store cannot recurse into it.
    static RECORDING: Cell<bool> = const { Cell::new(false) };
}

/// True while this thread is inside `IssueStore::record_issue`.
pub(crate) fn is_recording() -> bool {
    RECORDING.with(Cell::get)
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS issues (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fingerprint TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 1,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    last_detail TEXT,
    status TEXT NOT NULL DEFAULT 'open'
);
CREATE INDEX IF NOT EXISTS idx_issues_status_seen ON issues(status, last_seen);
";

fn db_err(e: rusqlite::Error) -> AppError {
    let busy = matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    );
    let base = if busy {
        AppError::busy("issue database is busy")
    } else {
        AppError::internal("issue database error")
    };
    base.with_detail(e.to_string())
}

fn kind_str(k: ErrorKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "internal".into())
}

fn kind_from(s: &str) -> ErrorKind {
    serde_json::from_value(serde_json::Value::String(s.to_string())).unwrap_or(ErrorKind::Internal)
}

/// Title normalisation for dedup: lowercase, trimmed, every digit run → `#`.
pub fn normalize_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut in_digits = false;
    for c in title.trim().chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            in_digits = false;
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// `sha256(kind|source|normalized title)` as hex.
pub fn fingerprint(kind: ErrorKind, source: &str, title: &str) -> String {
    let mut h = Sha256::new();
    h.update(kind_str(kind).as_bytes());
    h.update(b"|");
    h.update(source.as_bytes());
    h.update(b"|");
    h.update(normalize_title(title).as_bytes());
    hex::encode(h.finalize())
}

/// Issue store backed by `<app_data>/obs.db`.
pub struct IssueStore {
    conn: Mutex<Connection>,
    listener: RwLock<Option<IssueListener>>,
}

impl IssueStore {
    /// Open (or create) `Paths.app_data/obs.db`.
    pub fn open(paths: &Paths) -> Result<Self> {
        std::fs::create_dir_all(&paths.app_data)?;
        Self::open_at(&paths.app_data.join("obs.db"))
    }

    /// Open (or create) an issue database at `path`.
    pub fn open_at(path: &Path) -> Result<Self> {
        Self::from_conn(Connection::open(path).map_err(db_err)?)
    }

    /// Ephemeral store (tests, headless tools).
    pub fn in_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory().map_err(db_err)?)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_millis(2000))
            .map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(Self {
            conn: Mutex::new(conn),
            listener: RwLock::new(None),
        })
    }

    /// Install the new/reopened callback (e.g. emits `issue:new`).
    pub fn set_listener(&self, f: IssueListener) {
        *self.listener.write() = Some(f);
    }

    /// Record an occurrence. Returns the stored issue and whether it was new or reopened.
    pub fn record_issue(
        &self,
        kind: ErrorKind,
        source: &str,
        title: &str,
        detail: Option<&str>,
    ) -> Result<(Issue, bool)> {
        let was = RECORDING.with(|r| r.replace(true));
        let res = self.record_inner(kind, source, title, detail);
        RECORDING.with(|r| r.set(was));
        let (issue, notify) = res?;
        if notify && let Some(l) = self.listener.read().clone() {
            l(&issue);
        }
        Ok((issue, notify))
    }

    fn record_inner(
        &self,
        kind: ErrorKind,
        source: &str,
        title: &str,
        detail: Option<&str>,
    ) -> Result<(Issue, bool)> {
        let title = truncate(&redact(title.trim()), MAX_TITLE);
        let title = if title.is_empty() {
            "Unknown problem".to_string()
        } else {
            title
        };
        let source = truncate(source.trim(), 120);
        let detail = detail.map(|d| truncate(&redact(d), MAX_DETAIL));
        let fp = fingerprint(kind, &source, &title);
        let now = now_epoch();
        let conn = self.conn.lock();
        let prev: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, status FROM issues WHERE fingerprint = ?1",
                [&fp],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_err)?;
        let (id, notify) = match prev {
            Some((id, status)) => {
                conn.execute(
                    "UPDATE issues SET count = count + 1, last_seen = ?2, last_detail = COALESCE(?3, last_detail),
                     title = ?4, status = 'open' WHERE id = ?1",
                    params![id, now, detail, title],
                )
                .map_err(db_err)?;
                (id, status != "open")
            }
            None => {
                conn.execute(
                    "INSERT INTO issues (fingerprint, title, kind, source, count, first_seen, last_seen, last_detail, status)
                     VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5, ?6, 'open')",
                    params![fp, title, kind_str(kind), source, now, detail],
                )
                .map_err(db_err)?;
                (conn.last_insert_rowid(), true)
            }
        };
        let issue = get_issue(&conn, id)?
            .ok_or_else(|| AppError::internal("issue vanished after write"))?;
        Ok((issue, notify))
    }

    /// Open issues first, then by `last_seen` (newest first).
    pub fn list(&self) -> Result<Vec<Issue>> {
        let conn = self.conn.lock();
        let mut st = conn
            .prepare(
                "SELECT id, fingerprint, title, kind, source, count, first_seen, last_seen, last_detail, status
                 FROM issues ORDER BY (status = 'open') DESC, last_seen DESC, id DESC",
            )
            .map_err(db_err)?;
        let rows = st.query_map([], row_to_issue).map_err(db_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)
    }

    /// One issue by id.
    pub fn get(&self, id: i64) -> Result<Option<Issue>> {
        get_issue(&self.conn.lock(), id)
    }

    /// Mark resolved (it reopens automatically if it happens again).
    pub fn resolve(&self, id: i64) -> Result<()> {
        let n = self
            .conn
            .lock()
            .execute("UPDATE issues SET status = 'resolved' WHERE id = ?1", [id])
            .map_err(db_err)?;
        if n == 0 {
            return Err(AppError::not_found("issue not found"));
        }
        Ok(())
    }

    /// Resolve the open issue with this kind, source and title (same
    /// fingerprint as `record_issue`). Returns how many were resolved (0 or 1).
    pub fn resolve_matching(&self, kind: ErrorKind, source: &str, title: &str) -> Result<u32> {
        let title = truncate(&redact(title.trim()), MAX_TITLE);
        let source = truncate(source.trim(), 120);
        let fp = fingerprint(kind, &source, &title);
        let n = self
            .conn
            .lock()
            .execute(
                "UPDATE issues SET status = 'resolved' WHERE fingerprint = ?1 AND status = 'open'",
                [&fp],
            )
            .map_err(db_err)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Resolve every open issue of `kind` raised by `source`; returns how many.
    pub fn resolve_open_by_source(&self, source: &str, kind: ErrorKind) -> Result<u32> {
        let n = self
            .conn
            .lock()
            .execute(
                "UPDATE issues SET status = 'resolved' WHERE source = ?1 AND kind = ?2 AND status = 'open'",
                params![source.trim(), kind_str(kind)],
            )
            .map_err(db_err)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Delete all resolved issues; returns how many.
    pub fn clear_resolved(&self) -> Result<u32> {
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM issues WHERE status = 'resolved'", [])
            .map_err(db_err)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}

fn get_issue(conn: &Connection, id: i64) -> Result<Option<Issue>> {
    conn.query_row(
        "SELECT id, fingerprint, title, kind, source, count, first_seen, last_seen, last_detail, status
         FROM issues WHERE id = ?1",
        [id],
        row_to_issue,
    )
    .optional()
    .map_err(db_err)
}

fn row_to_issue(r: &rusqlite::Row<'_>) -> rusqlite::Result<Issue> {
    let kind: String = r.get(3)?;
    let status: String = r.get(9)?;
    let count: i64 = r.get(5)?;
    Ok(Issue {
        id: r.get(0)?,
        fingerprint: r.get(1)?,
        title: r.get(2)?,
        kind: kind_from(&kind),
        source: r.get(4)?,
        count: u64::try_from(count).unwrap_or(0),
        first_seen: r.get(6)?,
        last_seen: r.get(7)?,
        last_detail: r.get(8)?,
        status: if status == "open" {
            IssueStatus::Open
        } else {
            IssueStatus::Resolved
        },
    })
}

impl IssueSink for IssueStore {
    fn record(&self, kind: ErrorKind, source: &str, title: &str, detail: Option<&str>) {
        if is_recording() {
            return;
        }
        if let Err(e) = self.record_issue(kind, source, title, detail) {
            // Not ERROR: that would loop back through the issue layer.
            tracing::warn!(target: "wa_obs::issues", error = %e, "could not record issue");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn dedups_by_normalized_title() {
        let s = IssueStore::in_memory().unwrap();
        let (a, new_a) = s
            .record_issue(
                ErrorKind::External,
                "jobs.backfill",
                "Failed 12 downloads",
                None,
            )
            .unwrap();
        let (b, new_b) = s
            .record_issue(
                ErrorKind::External,
                "jobs.backfill",
                "failed 3 downloads ",
                Some("d2"),
            )
            .unwrap();
        assert!(new_a && !new_b);
        assert_eq!(a.id, b.id);
        assert_eq!(b.count, 2);
        assert_eq!(b.last_detail.as_deref(), Some("d2"));
        assert!(b.last_seen >= a.first_seen);
        // Different kind or source → different issue.
        let (c, _) = s
            .record_issue(
                ErrorKind::Timeout,
                "jobs.backfill",
                "Failed 1 downloads",
                None,
            )
            .unwrap();
        let (d, _) = s
            .record_issue(ErrorKind::External, "bridge", "Failed 1 downloads", None)
            .unwrap();
        assert_ne!(c.id, a.id);
        assert_ne!(d.id, a.id);
        assert_eq!(s.list().unwrap().len(), 3);
    }

    #[test]
    fn normalize_collapses_digit_runs() {
        assert_eq!(
            normalize_title(" Port 8080 refused 3x "),
            "port # refused #x"
        );
        assert_eq!(
            fingerprint(ErrorKind::Busy, "db", "a 1"),
            fingerprint(ErrorKind::Busy, "db", "A 22")
        );
    }

    #[test]
    fn resolve_reopen_and_clear() {
        let s = IssueStore::in_memory().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        s.set_listener(Arc::new(move |_| {
            h.fetch_add(1, Ordering::SeqCst);
        }));
        let (i, _) = s
            .record_issue(ErrorKind::Busy, "db", "locked", None)
            .unwrap();
        s.record_issue(ErrorKind::Busy, "db", "locked", None)
            .unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "repeat of open issue is silent"
        );
        s.resolve(i.id).unwrap();
        assert_eq!(s.get(i.id).unwrap().unwrap().status, IssueStatus::Resolved);
        let (again, notified) = s
            .record_issue(ErrorKind::Busy, "db", "locked", None)
            .unwrap();
        assert!(notified);
        assert_eq!(again.status, IssueStatus::Open);
        assert_eq!(again.count, 3);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        s.resolve(i.id).unwrap();
        s.record_issue(ErrorKind::Internal, "x", "other", None)
            .unwrap();
        assert_eq!(s.clear_resolved().unwrap(), 1);
        assert!(s.get(i.id).unwrap().is_none());
        assert_eq!(s.resolve(9999).unwrap_err().kind, ErrorKind::NotFound);
    }

    #[test]
    fn resolve_matching_and_by_source() {
        let s = IssueStore::in_memory().unwrap();
        let (a, _) = s
            .record_issue(ErrorKind::Timeout, "app.watcher", "Cannot read 5", None)
            .unwrap();
        let (b, _) = s
            .record_issue(ErrorKind::Internal, "app.watcher", "Other", None)
            .unwrap();
        let (c, _) = s
            .record_issue(ErrorKind::Timeout, "app.index", "Index", None)
            .unwrap();
        // Wrong kind / source: nothing happens.
        assert_eq!(
            s.resolve_matching(ErrorKind::Busy, "app.watcher", "Cannot read 5")
                .unwrap(),
            0
        );
        assert_eq!(
            s.resolve_open_by_source("app.watcher", ErrorKind::Timeout)
                .unwrap(),
            1
        );
        assert_eq!(s.get(a.id).unwrap().unwrap().status, IssueStatus::Resolved);
        assert_eq!(s.get(b.id).unwrap().unwrap().status, IssueStatus::Open);
        assert_eq!(s.get(c.id).unwrap().unwrap().status, IssueStatus::Open);
        // Already resolved: idempotent.
        assert_eq!(
            s.resolve_matching(ErrorKind::Timeout, "app.watcher", "cannot read 9")
                .unwrap(),
            0
        );
        assert_eq!(
            s.resolve_matching(ErrorKind::Timeout, "app.index", "INDEX")
                .unwrap(),
            1
        );
        // It reopens on recurrence.
        let (again, notified) = s
            .record_issue(ErrorKind::Timeout, "app.index", "Index", None)
            .unwrap();
        assert!(notified && again.status == IssueStatus::Open);
    }

    #[test]
    fn list_orders_open_first() {
        let s = IssueStore::in_memory().unwrap();
        let (a, _) = s.record_issue(ErrorKind::Busy, "a", "first", None).unwrap();
        let (b, _) = s
            .record_issue(ErrorKind::Busy, "b", "second", None)
            .unwrap();
        s.resolve(b.id).unwrap();
        let l = s.list().unwrap();
        assert_eq!(l[0].id, a.id);
        assert_eq!(l[1].status, IssueStatus::Resolved);
    }

    #[test]
    fn titles_and_details_are_redacted_and_bounded() {
        let s = IssueStore::in_memory().unwrap();
        let tok = "ab".repeat(32);
        let (i, _) = s
            .record_issue(
                ErrorKind::Auth,
                "bridge",
                &format!("bad token {tok} {}", "x".repeat(500)),
                Some(&format!("Bearer {tok} from 919811111234")),
            )
            .unwrap();
        assert!(!i.title.contains(&tok));
        assert!(i.title.chars().count() <= MAX_TITLE + 1);
        let d = i.last_detail.unwrap();
        assert!(!d.contains(&tok) && !d.contains("919811111234"));
        let (e, _) = s
            .record_issue(ErrorKind::Auth, "bridge", "   ", None)
            .unwrap();
        assert_eq!(e.title, "Unknown problem");
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("obs.db");
        let id = {
            let s = IssueStore::open_at(&p).unwrap();
            s.record_issue(ErrorKind::NotFound, "x", "gone", None)
                .unwrap()
                .0
                .id
        };
        let s = IssueStore::open_at(&p).unwrap();
        assert_eq!(s.get(id).unwrap().unwrap().kind, ErrorKind::NotFound);
    }

    #[test]
    fn listener_may_record_without_deadlock() {
        let s = Arc::new(IssueStore::in_memory().unwrap());
        let s2 = Arc::downgrade(&s);
        s.set_listener(Arc::new(move |_| {
            if let Some(s) = s2.upgrade() {
                s.record(ErrorKind::Internal, "listener", "nested", None);
            }
        }));
        s.record(ErrorKind::Internal, "x", "outer", None);
        assert!(!s.list().unwrap().is_empty());
    }
}
