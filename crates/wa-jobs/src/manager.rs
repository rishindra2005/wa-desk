//! `JobManager`: starts, tracks, throttles, persists and cancels jobs.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use wa_core::config::Paths;
use wa_core::time::now_epoch;
use wa_core::traits::{AppStore, BridgeApi, IssueSink, LinkStore, MessageStore, ProgressSink};
use wa_core::types::{JobProgress, JobRecord, JobSpec, JobStatus};
use wa_core::{AppError, ErrorKind, Result};

use crate::tools::Tools;
use crate::util::media_url;

/// Maximum conversions running at once (LibreOffice is further serialised).
pub const MAX_CONVERT: usize = 4;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
const PERSIST_INTERVAL: Duration = Duration::from_secs(2);

/// Emitted to the app (mapped to `job:progress` / `job:finished`).
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Progress(JobProgress),
    Finished(JobRecord),
}

/// Event callback.
pub type JobEventFn = Arc<dyn Fn(JobEvent) + Send + Sync>;

/// Future returned by a custom job: `Ok(Some(msg))` sets the final message.
pub type JobFuture = Pin<Box<dyn Future<Output = Result<Option<String>>> + Send>>;

/// Everything jobs need from the rest of the app.
#[derive(Clone)]
pub struct JobDeps {
    pub paths: Paths,
    pub messages: Arc<dyn MessageStore>,
    pub links: Arc<dyn LinkStore>,
    pub app: Arc<dyn AppStore>,
    pub bridge: Arc<dyn BridgeApi>,
    pub tools: Tools,
    /// Where GVFS mounts MTP phones (default `$XDG_RUNTIME_DIR/gvfs`).
    pub phone_root: PathBuf,
    pub issues: Option<Arc<dyn IssueSink>>,
    /// Pause before retrying a backfill download when the bridge returns 503.
    pub backfill_retry_delay: Duration,
}

impl JobDeps {
    pub fn new(
        paths: Paths,
        messages: Arc<dyn MessageStore>,
        links: Arc<dyn LinkStore>,
        app: Arc<dyn AppStore>,
        bridge: Arc<dyn BridgeApi>,
    ) -> Self {
        Self {
            paths,
            messages,
            links,
            app,
            bridge,
            tools: Tools::default(),
            phone_root: default_phone_root(),
            issues: None,
            backfill_retry_delay: Duration::from_secs(15),
        }
    }
}

/// `$XDG_RUNTIME_DIR/gvfs`, else `/run/user/<uid>/gvfs`.
pub fn default_phone_root() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d).join("gvfs");
    }
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(1000);
    PathBuf::from(format!("/run/user/{uid}/gvfs"))
}

/// Handle given to a running job: progress, outputs and cancellation.
pub struct JobCtl {
    id: i64,
    cancel: CancellationToken,
    record: Mutex<JobRecord>,
    emit: JobEventFn,
    app: Arc<dyn AppStore>,
    last_emit: Mutex<Option<Instant>>,
    last_persist: Mutex<Instant>,
}

impl JobCtl {
    pub fn id(&self) -> i64 {
        self.id
    }

    /// Token cancelled by `JobManager::cancel`; pass to external tools.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// `Err(Cancelled)` once cancellation was requested.
    pub fn check(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(AppError::cancelled())
        } else {
            Ok(())
        }
    }

    /// Current snapshot of the record.
    pub fn snapshot(&self) -> JobRecord {
        self.record.lock().clone()
    }

    /// Set only the phase (keeps counters).
    pub fn phase(&self, phase: &str) {
        let (d, t) = {
            let r = self.record.lock();
            (r.done, r.total)
        };
        self.progress(d, t, phase, None);
    }
}

impl ProgressSink for JobCtl {
    fn progress(&self, done: u64, total: u64, phase: &str, message: Option<&str>) {
        let p = {
            let mut r = self.record.lock();
            r.done = done;
            r.total = total;
            r.phase = phase.to_string();
            if let Some(m) = message {
                r.message = Some(m.to_string());
            }
            JobProgress {
                id: self.id,
                done,
                total,
                phase: r.phase.clone(),
                message: r.message.clone(),
            }
        };
        let now = Instant::now();
        let due = {
            let mut last = self.last_emit.lock();
            let due = last.is_none_or(|t| now.duration_since(t) >= PROGRESS_INTERVAL);
            if due {
                *last = Some(now);
            }
            due
        };
        if due {
            (self.emit)(JobEvent::Progress(p));
        }
        let persist = {
            let mut last = self.last_persist.lock();
            let due = now.duration_since(*last) >= PERSIST_INTERVAL;
            if due {
                *last = now;
            }
            due
        };
        if persist {
            let _ = self.app.update_job(&self.snapshot());
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn output(&self, path: &Path) {
        self.record.lock().outputs.push(media_url(path));
    }
}

struct Running {
    kind: &'static str,
    ctl: Arc<JobCtl>,
}

struct Inner {
    deps: JobDeps,
    emit: JobEventFn,
    running: Mutex<HashMap<i64, Running>>,
    convert_slots: Arc<Semaphore>,
    handle: tokio::runtime::Handle,
}

/// Starts and supervises background jobs. Clone freely.
#[derive(Clone)]
pub struct JobManager {
    inner: Arc<Inner>,
}

fn kind_key(spec: &JobSpec) -> &'static str {
    match spec {
        JobSpec::Backfill { .. } => "backfill",
        JobSpec::PhoneSync => "phone_sync",
        JobSpec::LinkPhoneMedia { .. } => "link_phone_media",
        JobSpec::Convert(_) => "convert",
        JobSpec::Reindex => "reindex",
        JobSpec::Thumbnails => "thumbnails",
    }
}

impl JobManager {
    /// Must be called inside a Tokio runtime (jobs are spawned on it).
    pub fn new(deps: JobDeps, on_event: JobEventFn) -> Result<Self> {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| AppError::internal("JobManager needs a Tokio runtime"))?;
        Ok(Self {
            inner: Arc::new(Inner {
                deps,
                emit: on_event,
                running: Mutex::new(HashMap::new()),
                convert_slots: Arc::new(Semaphore::new(MAX_CONVERT)),
                handle,
            }),
        })
    }

    /// Dependencies the manager was built with.
    pub fn deps(&self) -> &JobDeps {
        &self.inner.deps
    }

    /// Start a built-in job. `Reindex` must use `start_custom`.
    pub fn start(&self, spec: JobSpec) -> Result<JobRecord> {
        if matches!(spec, JobSpec::Reindex) {
            return Err(AppError::invalid(
                "the search index job is provided by the app (start_custom)",
            ));
        }
        if let JobSpec::Convert(c) = &spec {
            crate::convert::validate_kind(&c.kind)?;
        }
        let deps = self.inner.deps.clone();
        let spec2 = spec.clone();
        self.launch(
            spec,
            Box::new(move |ctl| Box::pin(async move { run_builtin(deps, spec2, ctl).await })),
        )
    }

    /// Start an app-provided job (e.g. Reindex) under the same bookkeeping.
    pub fn start_custom<F>(&self, spec: JobSpec, f: F) -> Result<JobRecord>
    where
        F: FnOnce(Arc<JobCtl>) -> JobFuture + Send + 'static,
    {
        self.launch(spec, Box::new(f))
    }

    fn launch(
        &self,
        spec: JobSpec,
        run: Box<dyn FnOnce(Arc<JobCtl>) -> JobFuture + Send>,
    ) -> Result<JobRecord> {
        let kind = kind_key(&spec);
        let inner = self.inner.clone();
        let ctl = {
            let mut running = inner.running.lock();
            if kind != "convert" && running.values().any(|r| r.kind == kind) {
                return Err(AppError::busy(format!("{} is already running", spec.label())));
            }
            let mut rec = JobRecord {
                id: 0,
                label: spec.label(),
                spec,
                status: JobStatus::Queued,
                done: 0,
                total: 0,
                phase: "queued".into(),
                message: None,
                error: None,
                outputs: Vec::new(),
                created_at: now_epoch(),
                finished_at: None,
            };
            rec.id = inner.deps.app.insert_job(&rec)?;
            let ctl = Arc::new(JobCtl {
                id: rec.id,
                cancel: CancellationToken::new(),
                record: Mutex::new(rec),
                emit: inner.emit.clone(),
                app: inner.deps.app.clone(),
                last_emit: Mutex::new(None),
                last_persist: Mutex::new(Instant::now()),
            });
            running.insert(
                ctl.id,
                Running {
                    kind,
                    ctl: ctl.clone(),
                },
            );
            ctl
        };
        let snapshot = ctl.snapshot();
        let task_ctl = ctl.clone();
        inner.handle.clone().spawn(async move {
            supervise(inner, task_ctl, kind, run).await;
        });
        Ok(snapshot)
    }

    /// Request cancellation. `NotFound` if the job is not running.
    pub fn cancel(&self, id: i64) -> Result<()> {
        match self.inner.running.lock().get(&id) {
            Some(r) => {
                r.ctl.cancel.cancel();
                Ok(())
            }
            None => Err(AppError::not_found("job is not running")),
        }
    }

    /// Cancel everything (app shutdown).
    pub fn cancel_all(&self) {
        for r in self.inner.running.lock().values() {
            r.ctl.cancel.cancel();
        }
    }

    /// Live record of a running job.
    pub fn get(&self, id: i64) -> Option<JobRecord> {
        self.inner.running.lock().get(&id).map(|r| r.ctl.snapshot())
    }

    /// Newest first, running jobs overlaid with their live progress.
    pub fn list(&self, limit: u32) -> Result<Vec<JobRecord>> {
        let mut v = self.inner.deps.app.list_jobs(limit)?;
        let running = self.inner.running.lock();
        for r in &mut v {
            if let Some(live) = running.get(&r.id) {
                *r = live.ctl.snapshot();
            }
        }
        Ok(v)
    }
}

async fn supervise(
    inner: Arc<Inner>,
    ctl: Arc<JobCtl>,
    kind: &'static str,
    run: Box<dyn FnOnce(Arc<JobCtl>) -> JobFuture + Send>,
) {
    let _permit = if kind == "convert" {
        tokio::select! {
            p = inner.convert_slots.clone().acquire_owned() => p.ok(),
            () = ctl.cancel.cancelled() => None,
        }
    } else {
        None
    };
    let result = if ctl.cancel.is_cancelled() {
        Err(AppError::cancelled())
    } else {
        {
            let mut r = ctl.record.lock();
            r.status = JobStatus::Running;
            r.phase = "starting".into();
        }
        let _ = inner.deps.app.update_job(&ctl.snapshot());
        ctl.progress(0, 0, "starting", None);
        match tokio::spawn(run(ctl.clone())).await {
            Ok(r) => r,
            Err(e) => Err(AppError::internal("job crashed").with_detail(e.to_string())),
        }
    };
    let rec = {
        let mut r = ctl.record.lock();
        r.finished_at = Some(now_epoch());
        match result {
            Ok(msg) => {
                r.status = JobStatus::Succeeded;
                r.phase = "done".into();
                if r.total > 0 {
                    r.done = r.total;
                }
                if msg.is_some() {
                    r.message = msg;
                }
            }
            Err(e) if e.kind == ErrorKind::Cancelled || ctl.cancel.is_cancelled() => {
                r.status = JobStatus::Cancelled;
                r.phase = "cancelled".into();
            }
            Err(e) => {
                r.status = JobStatus::Failed;
                r.phase = "failed".into();
                r.error = Some(e.message.clone());
                if let Some(sink) = &inner.deps.issues {
                    sink.record(
                        e.kind,
                        &format!("jobs.{kind}"),
                        &format!("{}: {}", r.label, e.message),
                        e.detail.as_deref(),
                    );
                }
                tracing::warn!(job = r.id, kind, error = %e.message, "job failed");
            }
        }
        r.clone()
    };
    if let Err(e) = inner.deps.app.update_job(&rec) {
        tracing::warn!(job = rec.id, error = %e, "could not persist job result");
    }
    inner.running.lock().remove(&rec.id);
    (inner.emit)(JobEvent::Finished(rec));
}

async fn run_builtin(deps: JobDeps, spec: JobSpec, ctl: Arc<JobCtl>) -> Result<Option<String>> {
    match spec {
        JobSpec::Backfill {
            days,
            kinds,
            workers,
            retry_failed,
        } => crate::backfill::run(&deps, days, &kinds, workers, retry_failed, &ctl)
            .await
            .map(Some),
        JobSpec::PhoneSync => crate::phone_sync::run(&deps, &ctl).await.map(Some),
        JobSpec::LinkPhoneMedia { extra_dirs } => {
            crate::linker::run(&deps, &extra_dirs, ctl).await.map(Some)
        }
        JobSpec::Convert(c) => crate::convert::run(&deps, &c, &ctl).await.map(Some),
        JobSpec::Thumbnails => crate::thumbs::run_job(&deps, &ctl).await.map(Some),
        JobSpec::Reindex => Err(AppError::invalid("reindex is app-provided")),
    }
}
