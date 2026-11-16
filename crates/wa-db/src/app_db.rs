//! wa-desk's own database (`app.db`): settings, kv, rules, automation log,
//! jobs, phone-media links and the search index (`msg_index` + `msg_fts`).
//!
//! WAL mode, `foreign_keys=ON`, schema migrations keyed on `user_version`.
//! One writer connection (mutex) plus a small pool of reader connections, so
//! searches never wait for an index batch to commit.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use wa_core::time::now_epoch;
use wa_core::traits::{AppStore, LinkRecord};
use wa_core::types::{AutomationEvent, JobRecord, JobSpec, JobStatus, Rule, Settings};
use wa_core::{AppError, Result};

use crate::util::sql_err;

/// Rows kept in `automation_log`.
pub const AUTOMATION_LOG_CAP: i64 = 5000;
/// kv key: highest bridge rowid indexed.
pub const KV_WATERMARK: &str = "index.watermark";
/// kv key: "1" once a full build finished (and no rebuild is pending).
pub const KV_COMPLETE: &str = "index.complete";

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);
CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    body TEXT NOT NULL,
    fired_count INTEGER NOT NULL DEFAULT 0,
    last_fired_at INTEGER,
    updated_at INTEGER NOT NULL
);
CREATE TABLE automation_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id INTEGER NOT NULL,
    rule_name TEXT NOT NULL,
    chat_jid TEXT NOT NULL,
    message_id TEXT NOT NULL,
    dry_run INTEGER NOT NULL,
    outcomes TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE TABLE jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    spec TEXT NOT NULL,
    label TEXT NOT NULL,
    status TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    phase TEXT NOT NULL DEFAULT '',
    message TEXT,
    error TEXT,
    outputs TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    finished_at INTEGER
);
CREATE INDEX jobs_status ON jobs(status);
CREATE TABLE phone_links (
    chat_jid TEXT NOT NULL,
    message_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    source_path TEXT NOT NULL,
    store_path TEXT,
    matched_by TEXT NOT NULL,
    linked_at INTEGER NOT NULL,
    PRIMARY KEY (chat_jid, message_id)
);
CREATE TABLE msg_index (
    chat_jid TEXT NOT NULL,
    id TEXT NOT NULL,
    src_rowid INTEGER NOT NULL,
    ts INTEGER NOT NULL,
    sender_jid TEXT,
    is_from_me INTEGER NOT NULL,
    media_kind TEXT,
    filename TEXT,
    file_length INTEGER,
    deleted INTEGER NOT NULL DEFAULT 0,
    row_hash INTEGER NOT NULL,
    PRIMARY KEY (chat_jid, id)
);
CREATE INDEX msg_index_chat_ts ON msg_index(chat_jid, ts, src_rowid);
CREATE INDEX msg_index_media_ts ON msg_index(media_kind, ts);
CREATE INDEX msg_index_ts ON msg_index(ts);
CREATE INDEX msg_index_sender ON msg_index(sender_jid);
CREATE VIRTUAL TABLE msg_fts USING fts5(content, filename, tokenize = 'trigram');
"#,
    // v2: local bookmarks ("starred")
    r#"
CREATE TABLE starred (
    chat_jid TEXT NOT NULL,
    id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (chat_jid, id)
);
CREATE INDEX starred_ts ON starred(ts);
"#,
];

/// One row of the search index (FTS rowid = `msg_index.rowid`).
#[derive(Debug, Clone, PartialEq)]
pub struct IndexRow {
    pub chat_jid: String,
    pub id: String,
    pub src_rowid: i64,
    pub ts: i64,
    pub sender_jid: Option<String>,
    pub is_from_me: bool,
    pub media_kind: Option<&'static str>,
    pub filename: Option<String>,
    pub file_length: Option<i64>,
    pub deleted: bool,
    pub row_hash: i64,
    /// Searchable text ("" for revoked messages).
    pub content: String,
}

/// A stored phone link (app.db or the messages.db mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredLink {
    pub phone_path: PathBuf,
    pub store_path: Option<PathBuf>,
}

/// Handle to app.db.
pub struct AppDb {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

fn configure(c: &Connection) -> rusqlite::Result<()> {
    c.busy_timeout(std::time::Duration::from_millis(5000))?;
    c.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;",
    )
}

impl AppDb {
    /// Open (creating parent dirs and migrating) app.db at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut c = Connection::open(path).map_err(sql_err)?;
        configure(&c).map_err(sql_err)?;
        migrate(&mut c).map_err(sql_err)?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(c),
            readers: Mutex::new(Vec::new()),
        })
    }

    /// Path of the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run `f` on a reader connection.
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let existing = self.readers.lock().pop();
        let conn = match existing {
            Some(c) => c,
            None => {
                let c = Connection::open(&self.path).map_err(sql_err)?;
                c.busy_timeout(std::time::Duration::from_millis(5000))
                    .map_err(sql_err)?;
                c
            }
        };
        let r = f(&conn).map_err(sql_err);
        let mut idle = self.readers.lock();
        if idle.len() < 4 {
            idle.push(conn);
        }
        r
    }

    /// Run `f` inside a write transaction.
    pub(crate) fn write<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T> {
        let mut c = self.writer.lock();
        let tx = c.transaction().map_err(sql_err)?;
        let v = f(&tx).map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(v)
    }

    // ---------- index ----------

    /// Upsert index rows (unchanged rows are skipped). Call inside `write`.
    pub(crate) fn upsert_index(tx: &Transaction<'_>, rows: &[IndexRow]) -> rusqlite::Result<usize> {
        let mut up = tx.prepare_cached(
            "INSERT INTO msg_index (chat_jid, id, src_rowid, ts, sender_jid, is_from_me, media_kind, filename, file_length, deleted, row_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(chat_jid, id) DO UPDATE SET
               src_rowid=excluded.src_rowid, ts=excluded.ts, sender_jid=excluded.sender_jid,
               is_from_me=excluded.is_from_me, media_kind=excluded.media_kind, filename=excluded.filename,
               file_length=excluded.file_length, deleted=excluded.deleted, row_hash=excluded.row_hash
             WHERE msg_index.row_hash != excluded.row_hash OR msg_index.src_rowid != excluded.src_rowid
                OR msg_index.sender_jid IS NOT excluded.sender_jid
             RETURNING rowid",
        )?;
        let mut del = tx.prepare_cached("DELETE FROM msg_fts WHERE rowid = ?1")?;
        let mut ins = tx
            .prepare_cached("INSERT INTO msg_fts (rowid, content, filename) VALUES (?1, ?2, ?3)")?;
        let mut changed = 0;
        for r in rows {
            let rid: Option<i64> = up
                .query_row(
                    params![
                        r.chat_jid,
                        r.id,
                        r.src_rowid,
                        r.ts,
                        r.sender_jid,
                        r.is_from_me,
                        r.media_kind,
                        r.filename,
                        r.file_length,
                        r.deleted,
                        r.row_hash
                    ],
                    |x| x.get(0),
                )
                .optional()?;
            if let Some(rid) = rid {
                del.execute([rid])?;
                ins.execute(params![rid, r.content, r.filename.as_deref().unwrap_or("")])?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Delete index rows by (chat_jid, id). Call inside `write`.
    pub(crate) fn delete_index(
        tx: &Transaction<'_>,
        keys: &[(String, String)],
    ) -> rusqlite::Result<usize> {
        let mut sel =
            tx.prepare_cached("SELECT rowid FROM msg_index WHERE chat_jid=?1 AND id=?2")?;
        let mut d1 = tx.prepare_cached("DELETE FROM msg_index WHERE rowid=?1")?;
        let mut d2 = tx.prepare_cached("DELETE FROM msg_fts WHERE rowid=?1")?;
        let mut n = 0;
        for (c, i) in keys {
            if let Some(rid) = sel
                .query_row(params![c, i], |r| r.get::<_, i64>(0))
                .optional()?
            {
                d1.execute([rid])?;
                d2.execute([rid])?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Remove everything from the index and reset its watermark.
    pub fn clear_index(&self) -> Result<()> {
        self.write(|tx| {
            tx.execute_batch(
                "DELETE FROM msg_index; DELETE FROM msg_fts; DELETE FROM kv WHERE key IN ('index.watermark','index.complete');",
            )
        })
    }

    /// Index watermark (0 if never indexed).
    pub fn watermark(&self) -> Result<i64> {
        Ok(self
            .get_kv(KV_WATERMARK)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    /// Whether a full build has completed.
    pub fn index_complete(&self) -> Result<bool> {
        Ok(self.get_kv(KV_COMPLETE)?.as_deref() == Some("1"))
    }

    /// Number of indexed messages.
    pub fn indexed_count(&self) -> Result<u64> {
        self.read(|c| c.query_row("SELECT count(*) FROM msg_index", [], |r| r.get::<_, i64>(0)))
            .map(|n| n.max(0) as u64)
    }

    // ---------- phone links ----------

    /// Upsert links into app.db (one transaction).
    pub fn upsert_phone_links(&self, links: &[LinkRecord]) -> Result<()> {
        let now = now_epoch();
        self.write(|tx| {
            let mut st = tx.prepare_cached(
                "INSERT INTO phone_links (chat_jid, message_id, kind, source_path, store_path, matched_by, linked_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(chat_jid, message_id) DO UPDATE SET kind=excluded.kind, source_path=excluded.source_path,
                   store_path=excluded.store_path, matched_by=excluded.matched_by, linked_at=excluded.linked_at",
            )?;
            for l in links {
                st.execute(params![
                    l.chat_jid,
                    l.message_id,
                    l.kind.as_db(),
                    l.source_path.to_string_lossy(),
                    l.store_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                    l.matched_by,
                    now
                ])?;
            }
            Ok(())
        })
    }

    /// The app.db phone link for a message, if any.
    pub fn phone_link(&self, chat_jid: &str, message_id: &str) -> Result<Option<StoredLink>> {
        self.read(|c| {
            c.prepare_cached(
                "SELECT source_path, store_path FROM phone_links WHERE chat_jid=?1 AND message_id=?2",
            )?
            .query_row([chat_jid, message_id], |r| {
                Ok(StoredLink {
                    phone_path: PathBuf::from(r.get::<_, String>(0)?),
                    store_path: r.get::<_, Option<String>>(1)?.map(PathBuf::from),
                })
            })
            .optional()
        })
    }

    /// Number of stored phone links.
    pub fn phone_link_count(&self) -> Result<u64> {
        self.read(|c| {
            c.query_row("SELECT count(*) FROM phone_links", [], |r| {
                r.get::<_, i64>(0)
            })
        })
        .map(|n| n.max(0) as u64)
    }
}

impl AppDb {
    // ---------- starred ----------

    /// Star (upsert with the message time) or unstar messages of one chat.
    pub fn set_starred(
        &self,
        chat_jid: &str,
        items: &[(String, i64)],
        starred: bool,
    ) -> Result<()> {
        let now = now_epoch();
        self.write(|tx| {
            if starred {
                let mut st = tx.prepare_cached(
                    "INSERT INTO starred (chat_jid, id, ts, created_at) VALUES (?1,?2,?3,?4)
                     ON CONFLICT(chat_jid, id) DO UPDATE SET ts=excluded.ts",
                )?;
                for (id, ts) in items {
                    st.execute(params![chat_jid, id, ts, now])?;
                }
            } else {
                let mut st =
                    tx.prepare_cached("DELETE FROM starred WHERE chat_jid=?1 AND id=?2")?;
                for (id, _) in items {
                    st.execute(params![chat_jid, id])?;
                }
            }
            Ok(())
        })
    }

    /// Which of `ids` (in `chat_jid`) are starred.
    pub fn starred_among(
        &self,
        chat_jid: &str,
        ids: &[&str],
    ) -> Result<std::collections::HashSet<String>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        self.read(|c| {
            let mut out = std::collections::HashSet::new();
            for chunk in ids.chunks(400) {
                let sql = format!(
                    "SELECT id FROM starred WHERE chat_jid = ? AND id IN ({})",
                    crate::util::placeholders(chunk.len())
                );
                let mut st = c.prepare_cached(&sql)?;
                let p = std::iter::once(chat_jid).chain(chunk.iter().copied());
                let rows =
                    st.query_map(rusqlite::params_from_iter(p), |r| r.get::<_, String>(0))?;
                for r in rows {
                    out.insert(r?);
                }
            }
            Ok(out)
        })
    }

    /// Starred keys newest first: `(chat_jid, id, ts)`, offset paging.
    pub fn starred_page(
        &self,
        chat_jid: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(String, String, i64)>> {
        self.read(|c| {
            let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
            match chat_jid {
                Some(j) => c
                    .prepare_cached(
                        "SELECT chat_jid, id, ts FROM starred WHERE chat_jid = ?1
                         ORDER BY ts DESC, id DESC LIMIT ?2 OFFSET ?3",
                    )?
                    .query_map(params![j, limit as i64, offset as i64], map)?
                    .collect(),
                None => c
                    .prepare_cached(
                        "SELECT chat_jid, id, ts FROM starred ORDER BY ts DESC, chat_jid, id DESC LIMIT ?1 OFFSET ?2",
                    )?
                    .query_map(params![limit as i64, offset as i64], map)?
                    .collect(),
            }
        })
    }

    /// Number of starred messages in a chat.
    pub fn starred_count(&self, chat_jid: &str) -> Result<u64> {
        self.read(|c| {
            c.query_row(
                "SELECT count(*) FROM starred WHERE chat_jid = ?1",
                [chat_jid],
                |r| r.get::<_, i64>(0),
            )
        })
        .map(|n| n.max(0) as u64)
    }
}

fn migrate(c: &mut Connection) -> rusqlite::Result<()> {
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let target = i as i64 + 1;
        if v < target {
            let tx = c.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute_batch(&format!("PRAGMA user_version = {target}"))?;
            tx.commit()?;
        }
    }
    Ok(())
}

fn job_status_str(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Succeeded => "succeeded",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
}

fn job_status_parse(s: &str) -> JobStatus {
    match s {
        "queued" => JobStatus::Queued,
        "running" => JobStatus::Running,
        "succeeded" => JobStatus::Succeeded,
        "cancelled" => JobStatus::Cancelled,
        _ => JobStatus::Failed,
    }
}

fn json_col<T: serde::de::DeserializeOwned>(s: &str, idx: usize) -> rusqlite::Result<T> {
    serde_json::from_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string(v)?)
}

/// Merge stored settings JSON over the defaults so new fields never break loading.
fn merge_settings(stored: &str) -> Settings {
    let defaults = serde_json::to_value(Settings::default()).unwrap_or_default();
    let stored: serde_json::Value = serde_json::from_str(stored).unwrap_or_default();
    let mut merged = defaults;
    if let (Some(m), Some(s)) = (merged.as_object_mut(), stored.as_object()) {
        for (k, v) in s {
            if m.contains_key(k) {
                m.insert(k.clone(), v.clone());
            }
        }
    }
    serde_json::from_value(merged.clone())
        .or_else(|_| {
            // A field had the wrong type: fall back field by field.
            let mut d = serde_json::to_value(Settings::default()).unwrap_or_default();
            if let (Some(dm), Some(mm)) = (d.as_object_mut(), merged.as_object()) {
                for (k, v) in mm {
                    let mut trial = dm.clone();
                    trial.insert(k.clone(), v.clone());
                    if serde_json::from_value::<Settings>(serde_json::Value::Object(trial)).is_ok()
                    {
                        dm.insert(k.clone(), v.clone());
                    }
                }
            }
            serde_json::from_value(d)
        })
        .unwrap_or_default()
}

fn row_to_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    let spec: JobSpec = json_col(&r.get::<_, String>(1)?, 1)?;
    let outputs: Vec<String> = json_col(&r.get::<_, String>(9)?, 9)?;
    Ok(JobRecord {
        id: r.get(0)?,
        spec,
        label: r.get(2)?,
        status: job_status_parse(&r.get::<_, String>(3)?),
        done: r.get::<_, i64>(4)?.max(0) as u64,
        total: r.get::<_, i64>(5)?.max(0) as u64,
        phase: r.get(6)?,
        message: r.get(7)?,
        error: r.get(8)?,
        outputs,
        created_at: r.get(10)?,
        finished_at: r.get(11)?,
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RuleBody {
    name: String,
    enabled: bool,
    dry_run: bool,
    trigger: wa_core::types::Trigger,
    actions: Vec<wa_core::types::Action>,
    cooldown_secs: u32,
}

impl AppStore for AppDb {
    fn get_settings(&self) -> Result<Settings> {
        let s: Option<String> = self.read(|c| {
            c.query_row("SELECT json FROM settings WHERE id=1", [], |r| r.get(0))
                .optional()
        })?;
        Ok(s.map(|s| merge_settings(&s)).unwrap_or_default())
    }

    fn put_settings(&self, s: &Settings) -> Result<()> {
        let j = to_json(s)?;
        self.write(|tx| {
            tx.execute(
                "INSERT INTO settings (id, json) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
                [j],
            )
            .map(|_| ())
        })
    }

    fn list_rules(&self) -> Result<Vec<Rule>> {
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT id, body, fired_count, last_fired_at FROM rules ORDER BY id",
            )?;
            let rows = st.query_map([], |r| {
                let body: RuleBody = json_col(&r.get::<_, String>(1)?, 1)?;
                Ok(Rule {
                    id: Some(r.get(0)?),
                    name: body.name,
                    enabled: body.enabled,
                    dry_run: body.dry_run,
                    trigger: body.trigger,
                    actions: body.actions,
                    cooldown_secs: body.cooldown_secs,
                    fired_count: r.get::<_, i64>(2)?.max(0) as u64,
                    last_fired_at: r.get(3)?,
                })
            })?;
            rows.collect()
        })
    }

    fn save_rule(&self, rule: &Rule) -> Result<Rule> {
        let body = to_json(&RuleBody {
            name: rule.name.clone(),
            enabled: rule.enabled,
            dry_run: rule.dry_run,
            trigger: rule.trigger.clone(),
            actions: rule.actions.clone(),
            cooldown_secs: rule.cooldown_secs,
        })?;
        let now = now_epoch();
        let id = self.write(|tx| match rule.id {
            Some(id) => {
                let n = tx.execute(
                    "UPDATE rules SET body=?2, updated_at=?3 WHERE id=?1",
                    params![id, body, now],
                )?;
                Ok((n > 0).then_some(id))
            }
            None => {
                tx.execute(
                    "INSERT INTO rules (body, fired_count, last_fired_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                    params![body, rule.fired_count as i64, rule.last_fired_at, now],
                )?;
                Ok(Some(tx.last_insert_rowid()))
            }
        })?;
        let id = id.ok_or_else(|| AppError::not_found("rule not found"))?;
        self.list_rules()?
            .into_iter()
            .find(|r| r.id == Some(id))
            .ok_or_else(|| AppError::internal("rule vanished after save"))
    }

    fn delete_rule(&self, id: i64) -> Result<()> {
        let n = self.write(|tx| tx.execute("DELETE FROM rules WHERE id=?1", [id]))?;
        if n == 0 {
            return Err(AppError::not_found("rule not found"));
        }
        Ok(())
    }

    fn record_rule_fired(&self, id: i64, at: i64) -> Result<()> {
        self.write(|tx| {
            tx.execute(
                "UPDATE rules SET fired_count = fired_count + 1, last_fired_at = ?2 WHERE id=?1",
                params![id, at],
            )
            .map(|_| ())
        })
    }

    fn log_automation(&self, ev: &AutomationEvent) -> Result<()> {
        let outcomes = to_json(&ev.outcomes)?;
        self.write(|tx| {
            tx.execute(
                "INSERT INTO automation_log (rule_id, rule_name, chat_jid, message_id, dry_run, outcomes, at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![ev.rule_id, ev.rule_name, ev.chat_jid, ev.message_id, ev.dry_run, outcomes, ev.at],
            )?;
            tx.execute(
                "DELETE FROM automation_log WHERE id <= (SELECT id FROM automation_log ORDER BY id DESC LIMIT 1 OFFSET ?1)",
                [AUTOMATION_LOG_CAP],
            )?;
            Ok(())
        })
    }

    fn recent_automation(&self, limit: u32) -> Result<Vec<AutomationEvent>> {
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT rule_id, rule_name, chat_jid, message_id, dry_run, outcomes, at FROM automation_log ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = st.query_map([limit.min(AUTOMATION_LOG_CAP as u32) as i64], |r| {
                Ok(AutomationEvent {
                    rule_id: r.get(0)?,
                    rule_name: r.get(1)?,
                    chat_jid: r.get(2)?,
                    message_id: r.get(3)?,
                    dry_run: r.get(4)?,
                    outcomes: json_col(&r.get::<_, String>(5)?, 5)?,
                    at: r.get(6)?,
                })
            })?;
            rows.collect()
        })
    }

    fn get_kv(&self, key: &str) -> Result<Option<String>> {
        self.read(|c| {
            c.prepare_cached("SELECT value FROM kv WHERE key=?1")?
                .query_row([key], |r| r.get(0))
                .optional()
        })
    }

    fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.write(|tx| {
            tx.execute(
                "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [key, value],
            )
            .map(|_| ())
        })
    }

    fn insert_job(&self, job: &JobRecord) -> Result<i64> {
        let spec = to_json(&job.spec)?;
        let outputs = to_json(&job.outputs)?;
        self.write(|tx| {
            tx.execute(
                "INSERT INTO jobs (spec, label, status, done, total, phase, message, error, outputs, created_at, finished_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    spec,
                    job.label,
                    job_status_str(job.status),
                    job.done as i64,
                    job.total as i64,
                    job.phase,
                    job.message,
                    job.error,
                    outputs,
                    job.created_at,
                    job.finished_at
                ],
            )?;
            Ok(tx.last_insert_rowid())
        })
    }

    fn update_job(&self, job: &JobRecord) -> Result<()> {
        let spec = to_json(&job.spec)?;
        let outputs = to_json(&job.outputs)?;
        let n = self.write(|tx| {
            tx.execute(
                "UPDATE jobs SET spec=?2, label=?3, status=?4, done=?5, total=?6, phase=?7, message=?8, error=?9,
                   outputs=?10, finished_at=?11 WHERE id=?1",
                params![
                    job.id,
                    spec,
                    job.label,
                    job_status_str(job.status),
                    job.done as i64,
                    job.total as i64,
                    job.phase,
                    job.message,
                    job.error,
                    outputs,
                    job.finished_at
                ],
            )
        })?;
        if n == 0 {
            return Err(AppError::not_found("job not found"));
        }
        Ok(())
    }

    fn list_jobs(&self, limit: u32) -> Result<Vec<JobRecord>> {
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT id, spec, label, status, done, total, phase, message, error, outputs, created_at, finished_at
                 FROM jobs ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = st.query_map([limit.clamp(1, 10_000) as i64], row_to_job)?;
            rows.collect()
        })
    }

    fn fail_interrupted_jobs(&self) -> Result<u32> {
        let now = now_epoch();
        self.write(|tx| {
            tx.execute(
                "UPDATE jobs SET status='failed', error='interrupted', finished_at=?1 WHERE status IN ('queued','running')",
                [now],
            )
        })
        .map(|n| n as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wa_core::types::{Action, ConvertKind, ConvertSpec, MediaKind, Theme, Trigger};

    fn db() -> (tempfile::TempDir, AppDb) {
        let d = tempfile::tempdir().unwrap();
        let db = AppDb::open(&d.path().join("sub/app.db")).unwrap();
        (d, db)
    }

    fn rule(name: &str) -> Rule {
        Rule {
            id: None,
            name: name.into(),
            enabled: true,
            dry_run: true,
            trigger: Trigger {
                keywords: vec!["hi".into()],
                ..Default::default()
            },
            actions: vec![Action::Reply { text: "yo".into() }],
            cooldown_secs: 600,
            fired_count: 0,
            last_fired_at: None,
        }
    }

    fn job(status: JobStatus) -> JobRecord {
        JobRecord {
            id: 0,
            spec: JobSpec::Reindex,
            label: JobSpec::Reindex.label(),
            status,
            done: 0,
            total: 10,
            phase: "start".into(),
            message: None,
            error: None,
            outputs: vec![],
            created_at: 100,
            finished_at: None,
        }
    }

    #[test]
    fn migrations_are_idempotent_and_wal() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("app.db");
        drop(AppDb::open(&p).unwrap());
        let db = AppDb::open(&p).unwrap();
        let (v, mode): (i64, String) = db
            .read(|c| {
                Ok((
                    c.query_row("PRAGMA user_version", [], |r| r.get(0))?,
                    c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
                ))
            })
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        assert_eq!(mode, "wal");
    }

    #[test]
    fn settings_roundtrip_and_lenient_load() {
        let (_d, db) = db();
        assert_eq!(db.get_settings().unwrap(), Settings::default());
        let s = Settings {
            theme: Theme::Dark,
            backfill_days: 7,
            ..Default::default()
        };
        db.put_settings(&s).unwrap();
        assert_eq!(db.get_settings().unwrap(), s);
        // Old/partial/wrongly typed JSON still loads.
        db.write(|tx| {
            tx.execute(
                "UPDATE settings SET json='{\"theme\":\"light\",\"backfill_days\":\"x\",\"gone\":1}'",
                [],
            )
        })
        .unwrap();
        let s = db.get_settings().unwrap();
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.backfill_days, 30);
    }

    #[test]
    fn kv_roundtrip() {
        let (_d, db) = db();
        assert_eq!(db.get_kv("a").unwrap(), None);
        db.set_kv("a", "1").unwrap();
        db.set_kv("a", "2").unwrap();
        assert_eq!(db.get_kv("a").unwrap().as_deref(), Some("2"));
        assert_eq!(db.watermark().unwrap(), 0);
        assert!(!db.index_complete().unwrap());
    }

    #[test]
    fn rules_crud_and_stats() {
        let (_d, db) = db();
        let r = db.save_rule(&rule("one")).unwrap();
        let id = r.id.unwrap();
        assert_eq!(r.name, "one");
        let mut r2 = r.clone();
        r2.name = "renamed".into();
        r2.fired_count = 99; // stats are not overwritten by save
        assert_eq!(db.save_rule(&r2).unwrap().fired_count, 0);
        db.record_rule_fired(id, 1234).unwrap();
        db.record_rule_fired(id, 1300).unwrap();
        let got = &db.list_rules().unwrap()[0];
        assert_eq!(got.name, "renamed");
        assert_eq!(got.fired_count, 2);
        assert_eq!(got.last_fired_at, Some(1300));
        db.delete_rule(id).unwrap();
        assert!(db.list_rules().unwrap().is_empty());
        assert_eq!(
            db.delete_rule(id).unwrap_err().kind,
            wa_core::ErrorKind::NotFound
        );
        let mut ghost = rule("ghost");
        ghost.id = Some(4242);
        assert!(db.save_rule(&ghost).is_err());
    }

    #[test]
    fn automation_log_is_capped() {
        let (_d, db) = db();
        for i in 0..(AUTOMATION_LOG_CAP + 25) {
            db.log_automation(&AutomationEvent {
                rule_id: 1,
                rule_name: "r".into(),
                chat_jid: "c".into(),
                message_id: format!("m{i}"),
                dry_run: true,
                outcomes: vec!["ok".into()],
                at: i,
            })
            .unwrap();
        }
        let n: i64 = db
            .read(|c| c.query_row("SELECT count(*) FROM automation_log", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, AUTOMATION_LOG_CAP);
        let recent = db.recent_automation(3).unwrap();
        assert_eq!(
            recent[0].message_id,
            format!("m{}", AUTOMATION_LOG_CAP + 24)
        );
        assert_eq!(recent[0].outcomes, vec!["ok".to_string()]);
    }

    #[test]
    fn jobs_lifecycle_and_interrupted() {
        let (_d, db) = db();
        let id1 = db.insert_job(&job(JobStatus::Running)).unwrap();
        let id2 = db.insert_job(&job(JobStatus::Queued)).unwrap();
        let mut done = job(JobStatus::Succeeded);
        done.spec = JobSpec::Convert(ConvertSpec {
            input: wa_core::types::ConvertInput::File { path: "/x".into() },
            kind: ConvertKind::OfficeToPdf,
            output_dir: None,
        });
        let id3 = db.insert_job(&done).unwrap();
        done.id = id3;
        done.outputs = vec!["wa-media://localhost/x/a".into()];
        done.finished_at = Some(5);
        db.update_job(&done).unwrap();
        assert_eq!(db.fail_interrupted_jobs().unwrap(), 2);
        let jobs = db.list_jobs(10).unwrap();
        assert_eq!(
            jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
            vec![id3, id2, id1]
        );
        assert_eq!(jobs[0], done);
        assert!(jobs[1..].iter().all(|j| j.status == JobStatus::Failed
            && j.error.as_deref() == Some("interrupted")
            && j.finished_at.is_some()));
        let mut ghost = job(JobStatus::Queued);
        ghost.id = 999;
        assert!(db.update_job(&ghost).is_err());
    }

    #[test]
    fn index_upsert_skips_unchanged_and_fts_tracks() {
        let (_d, db) = db();
        let row = IndexRow {
            chat_jid: "c".into(),
            id: "1".into(),
            src_rowid: 1,
            ts: 10,
            sender_jid: None,
            is_from_me: false,
            media_kind: None,
            filename: None,
            file_length: None,
            deleted: false,
            row_hash: 1,
            content: "hello world".into(),
        };
        assert_eq!(
            db.write(|tx| AppDb::upsert_index(tx, std::slice::from_ref(&row)))
                .unwrap(),
            1
        );
        assert_eq!(
            db.write(|tx| AppDb::upsert_index(tx, std::slice::from_ref(&row)))
                .unwrap(),
            0
        );
        let mut r2 = row.clone();
        r2.row_hash = 2;
        r2.content = "goodbye".into();
        assert_eq!(db.write(|tx| AppDb::upsert_index(tx, &[r2])).unwrap(), 1);
        let hits: i64 = db
            .read(|c| {
                c.query_row(
                    "SELECT count(*) FROM msg_fts WHERE msg_fts MATCH '\"hello\"'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(hits, 0);
        assert_eq!(
            db.write(|tx| AppDb::delete_index(tx, &[("c".into(), "1".into())]))
                .unwrap(),
            1
        );
        assert_eq!(db.indexed_count().unwrap(), 0);
    }

    #[test]
    fn phone_links_upsert() {
        let (_d, db) = db();
        let l = LinkRecord {
            chat_jid: "c".into(),
            message_id: "m".into(),
            kind: MediaKind::Image,
            source_path: "/p/a.jpg".into(),
            store_path: None,
            matched_by: "hash".into(),
        };
        db.upsert_phone_links(std::slice::from_ref(&l)).unwrap();
        db.upsert_phone_links(&[LinkRecord {
            store_path: Some("/s/x.jpg".into()),
            ..l
        }])
        .unwrap();
        let got = db.phone_link("c", "m").unwrap().unwrap();
        assert_eq!(got.store_path, Some(PathBuf::from("/s/x.jpg")));
        assert_eq!(db.phone_link_count().unwrap(), 1);
        assert!(db.phone_link("c", "zz").unwrap().is_none());
    }
}
