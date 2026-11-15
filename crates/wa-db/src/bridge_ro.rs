//! Read-only connections to the bridge's SQLite files.
//!
//! The Go bridge owns `messages.db` (rollback-journal mode) and gives up after
//! 5 s on a lock, losing the message it was storing. Every connection here is
//! therefore opened with `file:<p>?mode=ro`, `PRAGMA query_only=1`, a short
//! `busy_timeout`, and a progress handler that interrupts any statement that
//! runs longer than the pool's deadline (300 ms by default). Statements are
//! always fully drained and reset before a helper returns, so no SHARED lock
//! outlives a single call.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags, Params, Row};
use wa_core::{AppError, Result};

use crate::util::sql_err;

/// Default per-statement deadline.
pub const DEFAULT_DEADLINE: Duration = Duration::from_millis(300);
/// busy_timeout for read-only connections (must stay ≤ 2000 ms).
pub const RO_BUSY_TIMEOUT: Duration = Duration::from_millis(1000);
/// Maximum rows a single paged read may return.
pub const PAGE_ROWS: usize = 500;
const MAX_IDLE: usize = 4;
const DISARMED: u64 = u64::MAX;

/// Build a `file:` URI for `path` with the given query string.
pub(crate) fn file_uri(path: &Path, query: &str) -> String {
    let raw = path.to_string_lossy();
    let mut s = String::with_capacity(raw.len() + 16);
    s.push_str("file:");
    for c in raw.chars() {
        match c {
            '%' => s.push_str("%25"),
            '?' => s.push_str("%3f"),
            '#' => s.push_str("%23"),
            _ => s.push(c),
        }
    }
    s.push('?');
    s.push_str(query);
    s
}

/// One read-only connection with its deadline cell.
pub struct RoConn {
    conn: Connection,
    deadline: Arc<AtomicU64>,
    base: Instant,
    limit: Duration,
}

impl RoConn {
    /// Open `path` read-only with the safety settings described in the module docs.
    pub fn open(path: &Path, limit: Duration) -> Result<Self> {
        if !path.exists() {
            return Err(AppError::not_found("database file not found")
                .with_detail(path.display().to_string()));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn =
            Connection::open_with_flags(file_uri(path, "mode=ro"), flags).map_err(sql_err)?;
        conn.busy_timeout(RO_BUSY_TIMEOUT).map_err(sql_err)?;
        conn.execute_batch("PRAGMA query_only=1;")
            .map_err(sql_err)?;
        let deadline = Arc::new(AtomicU64::new(DISARMED));
        let base = Instant::now();
        let cell = Arc::clone(&deadline);
        conn.progress_handler(
            1000,
            Some(move || {
                let d = cell.load(Ordering::Relaxed);
                d != DISARMED && base.elapsed().as_nanos() as u64 > d
            }),
        );
        Ok(Self {
            conn,
            deadline,
            base,
            limit,
        })
    }

    fn arm(&self) {
        let at = self.base.elapsed() + self.limit;
        self.deadline.store(at.as_nanos() as u64, Ordering::Relaxed);
    }

    fn disarm(&self) {
        self.deadline.store(DISARMED, Ordering::Relaxed);
    }

    /// Run a query under the deadline and collect every row.
    pub fn rows<T, P, F>(&self, sql: &str, params: P, mut f: F) -> Result<Vec<T>>
    where
        P: Params,
        F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
    {
        self.arm();
        let res = (|| {
            let mut st = self.conn.prepare_cached(sql)?;
            let mut rows = st.query(params)?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(f(r)?);
            }
            Ok(out)
        })();
        self.disarm();
        res.map_err(sql_err)
    }

    /// Run a query under the deadline and return the first row, if any.
    pub fn row<T, P, F>(&self, sql: &str, params: P, f: F) -> Result<Option<T>>
    where
        P: Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        self.arm();
        let res = (|| {
            let mut st = self.conn.prepare_cached(sql)?;
            let mut rows = st.query(params)?;
            match rows.next()? {
                Some(r) => f(r).map(Some),
                None => Ok(None),
            }
        })();
        self.disarm();
        res.map_err(sql_err)
    }

    /// Whether a table exists (schema lookups are cheap).
    pub fn has_table(&self, name: &str) -> Result<bool> {
        Ok(self
            .row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |_| Ok(()),
            )?
            .is_some())
    }

    /// Column names of a table.
    pub fn columns(&self, table: &str) -> Result<Vec<String>> {
        // Table names come from our own constants, never from input.
        let sql = format!("PRAGMA table_info({table})");
        self.rows(&sql, [], |r| r.get::<_, String>(1))
    }

    /// `PRAGMA data_version` (changes when another connection commits).
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .row("PRAGMA data_version", [], |r| r.get(0))?
            .unwrap_or(0))
    }

    /// Raw connection access for tests.
    #[cfg(test)]
    pub(crate) fn raw(&self) -> &Connection {
        &self.conn
    }
}

/// A small checkout pool of `RoConn`s for one database file.
pub struct RoPool {
    path: PathBuf,
    limit: Duration,
    idle: Mutex<Vec<RoConn>>,
}

/// A checked-out connection; returned to the pool on drop.
pub struct Pooled<'a> {
    conn: Option<RoConn>,
    pool: &'a RoPool,
}

impl std::ops::Deref for Pooled<'_> {
    type Target = RoConn;
    fn deref(&self) -> &RoConn {
        // Invariant: `conn` is only taken in `drop`.
        self.conn.as_ref().unwrap_or_else(|| unreachable!())
    }
}

impl Drop for Pooled<'_> {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            let mut idle = self.pool.idle.lock();
            if idle.len() < MAX_IDLE {
                idle.push(c);
            }
        }
    }
}

impl RoPool {
    /// Pool for `path`; connections are opened lazily.
    pub fn new(path: impl Into<PathBuf>, limit: Duration) -> Self {
        Self {
            path: path.into(),
            limit,
            idle: Mutex::new(Vec::new()),
        }
    }

    /// The database file this pool reads.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the database file exists right now.
    pub fn available(&self) -> bool {
        self.path.exists()
    }

    /// Check out a connection (opens a new one if none is idle).
    pub fn get(&self) -> Result<Pooled<'_>> {
        let existing = self.idle.lock().pop();
        let conn = match existing {
            Some(c) => c,
            None => RoConn::open(&self.path, self.limit)?,
        };
        Ok(Pooled {
            conn: Some(conn),
            pool: self,
        })
    }

    /// Drop idle connections (e.g. after the file was replaced).
    pub fn reset(&self) {
        self.idle.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wa_core::ErrorKind;
    use wa_testkit::Fixture;

    #[test]
    fn uri_escapes_special_chars() {
        assert_eq!(
            file_uri(Path::new("/a b/c?d#e%f"), "mode=ro"),
            "file:/a b/c%3fd%23e%25f?mode=ro"
        );
    }

    #[test]
    fn read_only_refuses_writes() {
        let f = Fixture::standard();
        let c = RoConn::open(&f.paths.messages_db, DEFAULT_DEADLINE).unwrap();
        assert!(c.raw().execute("DELETE FROM messages", []).is_err());
        let n: Vec<i64> = c
            .rows("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, vec![13]);
    }

    #[test]
    fn missing_file_is_not_found() {
        let f = Fixture::new();
        let e = RoConn::open(&f.root.join("nope.db"), DEFAULT_DEADLINE)
            .err()
            .unwrap();
        assert_eq!(e.kind, ErrorKind::NotFound);
        assert!(!f.root.join("nope.db").exists(), "must not create the file");
    }

    #[test]
    fn slow_query_aborts_at_deadline() {
        let f = Fixture::standard();
        let pool = RoPool::new(&f.paths.messages_db, Duration::from_millis(300));
        let c = pool.get().unwrap();
        let t = Instant::now();
        let e = c
            .rows(
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n) \
                 SELECT count(*) FROM n, messages",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert!(
            t.elapsed() < Duration::from_millis(1500),
            "{:?}",
            t.elapsed()
        );
        // The connection stays usable afterwards.
        let n: Option<i64> = c
            .row("SELECT count(*) FROM chats", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, Some(7));
    }

    #[test]
    fn writer_succeeds_during_long_read() {
        let f = Fixture::standard();
        let path = f.paths.messages_db.clone();
        let reader = std::thread::spawn(move || {
            let pool = RoPool::new(&path, Duration::from_millis(300));
            let c = pool.get().unwrap();
            c.rows(
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n) \
                 SELECT count(*) FROM messages, n",
                [],
                |r| r.get::<_, i64>(0),
            )
        });
        std::thread::sleep(Duration::from_millis(50));
        let w = f.bridge_conn();
        w.busy_timeout(Duration::from_millis(5000)).unwrap();
        let t = Instant::now();
        w.execute(
            "INSERT INTO messages (id, chat_jid, sender, content, timestamp, is_from_me, media_type) \
             VALUES ('W1', ?1, 'x', 'during read', '2026-11-16 00:00:00+05:30', 0, '')",
            [wa_testkit::ids::DM_ALICE],
        )
        .expect("bridge write must succeed while we read");
        assert!(
            t.elapsed() < Duration::from_millis(2000),
            "{:?}",
            t.elapsed()
        );
        let r = reader.join().unwrap();
        assert_eq!(r.unwrap_err().kind, ErrorKind::Timeout);
    }

    #[test]
    fn pool_reuses_connections() {
        let f = Fixture::standard();
        let pool = RoPool::new(&f.paths.messages_db, DEFAULT_DEADLINE);
        {
            let _a = pool.get().unwrap();
            let _b = pool.get().unwrap();
        }
        assert_eq!(pool.idle.lock().len(), 2);
        let _c = pool.get().unwrap();
        assert_eq!(pool.idle.lock().len(), 1);
        pool.reset();
        assert_eq!(pool.idle.lock().len(), 0);
    }
}
