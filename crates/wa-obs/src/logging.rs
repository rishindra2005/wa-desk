//! Subscriber setup, the ERROR→issue layer and the panic hook.

use std::fmt::Write as _;
use std::sync::Arc;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use wa_core::config::Paths;
use wa_core::traits::IssueSink;
use wa_core::{AppError, ErrorKind, Result};

use crate::issues::is_recording;
use crate::redact::truncate;

/// Env var holding an `EnvFilter` directive string.
pub const LOG_ENV: &str = "WA_DESK_LOG";
/// Filter used when neither an explicit level nor `WA_DESK_LOG` is given.
pub const DEFAULT_FILTER: &str = "info,wa_=debug";
/// Log files are `<LOG_PREFIX>.<YYYY-MM-DD>.<LOG_SUFFIX>` in `Paths.logs`.
pub const LOG_PREFIX: &str = "wa-desk";
pub const LOG_SUFFIX: &str = "jsonl";
/// Daily files kept.
pub const KEEP_FILES: usize = 7;

/// Keeps the background log writer alive; drop it at shutdown to flush.
pub struct LogGuard {
    _file: WorkerGuard,
}

/// Initialise logging: pretty stderr + daily rolling JSON in `Paths.logs`.
/// `level` overrides `WA_DESK_LOG` (an `EnvFilter` directive string).
pub fn init_logging(paths: &Paths, level: Option<&str>) -> Result<LogGuard> {
    init_logging_with_issues(paths, level, None)
}

/// Like `init_logging`, also turning ERROR events into issues via `issues`.
pub fn init_logging_with_issues(
    paths: &Paths,
    level: Option<&str>,
    issues: Option<Arc<dyn IssueSink>>,
) -> Result<LogGuard> {
    std::fs::create_dir_all(&paths.logs)?;
    let directives = level
        .map(str::to_string)
        .or_else(|| std::env::var(LOG_ENV).ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FILTER.to_string());
    let filter = EnvFilter::try_new(&directives)
        .or_else(|_| EnvFilter::try_new(DEFAULT_FILTER))
        .map_err(|e| AppError::invalid("bad log filter").with_detail(e.to_string()))?;
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_PREFIX)
        .filename_suffix(LOG_SUFFIX)
        .max_log_files(KEEP_FILES)
        .build(&paths.logs)
        .map_err(|e| AppError::internal("cannot open log file").with_detail(e.to_string()))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(writer);
    let stderr_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_writer(std::io::stderr);
    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .with(issues.map(IssueLayer::new))
        .try_init()
        .map_err(|e| {
            AppError::internal("logging already initialised").with_detail(e.to_string())
        })?;
    Ok(LogGuard { _file: guard })
}

/// Tracing layer that records every ERROR event as an issue (source = target).
/// Events may carry `kind = "bridge_down"` etc. to choose the `ErrorKind`.
pub struct IssueLayer {
    sink: Arc<dyn IssueSink>,
}

impl IssueLayer {
    pub fn new(sink: Arc<dyn IssueSink>) -> Self {
        Self { sink }
    }
}

#[derive(Default)]
struct Collect {
    message: String,
    kind: Option<String>,
    fields: String,
}

impl Collect {
    fn put(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            "kind" => self.kind = Some(value),
            name => {
                if !self.fields.is_empty() {
                    self.fields.push(' ');
                }
                let _ = write!(self.fields, "{name}={value}");
            }
        }
    }
}

impl Visit for Collect {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_string());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, format!("{value:?}"));
    }
}

impl<S: Subscriber> Layer<S> for IssueLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() != Level::ERROR || meta.target().starts_with("wa_obs") || is_recording() {
            return;
        }
        let mut c = Collect::default();
        event.record(&mut c);
        let kind = c
            .kind
            .as_deref()
            .and_then(|k| serde_json::from_value(serde_json::Value::String(k.into())).ok())
            .unwrap_or(ErrorKind::Internal);
        let title = if c.message.is_empty() {
            meta.target().to_string()
        } else {
            c.message
        };
        let detail = (!c.fields.is_empty()).then_some(c.fields);
        self.sink
            .record(kind, meta.target(), &title, detail.as_deref());
    }
}

/// Record panics as issues (source "panic"), then run the previous hook.
pub fn install_panic_hook(sink: Arc<dyn IssueSink>) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let title = format!("Crash: {}", truncate(&payload, 150));
        let detail = format!("at {location} (thread {thread})");
        tracing::error!(target: "wa_obs::panic", %location, "panic: {}", truncate(&payload, 300));
        sink.record(ErrorKind::Internal, "panic", &title, Some(&detail));
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issues::IssueStore;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn error_events_become_issues() {
        let store = Arc::new(IssueStore::in_memory().unwrap());
        let sub = tracing_subscriber::registry().with(IssueLayer::new(store.clone()));
        tracing::subscriber::with_default(sub, || {
            tracing::error!(target: "wa_jobs::backfill", kind = "bridge_down", chat = "x", "bridge offline 3 times");
            tracing::error!(target: "wa_jobs::backfill", kind = "bridge_down", "bridge offline 4 times");
            tracing::warn!(target: "wa_jobs::backfill", "only a warning");
            tracing::error!(target: "wa_obs::issues", "never recorded");
            tracing::error!(target: "wa_db", "plain error");
        });
        let l = store.list().unwrap();
        assert_eq!(l.len(), 2, "{l:?}");
        let b = l.iter().find(|i| i.source == "wa_jobs::backfill").unwrap();
        assert_eq!(b.kind, ErrorKind::BridgeDown);
        assert_eq!(b.count, 2);
        let d = l.iter().find(|i| i.source == "wa_db").unwrap();
        assert_eq!(d.kind, ErrorKind::Internal);
        assert_eq!(d.title, "plain error");
    }

    #[test]
    fn panic_hook_records_issue() {
        let store = Arc::new(IssueStore::in_memory().unwrap());
        install_panic_hook(store.clone());
        let r = std::thread::Builder::new()
            .name("boom-thread".into())
            .spawn(|| panic!("kaboom 42"))
            .unwrap()
            .join();
        assert!(r.is_err());
        let _ = std::panic::take_hook();
        let l = store.list().unwrap();
        let i = l.iter().find(|i| i.source == "panic").expect("panic issue");
        assert!(i.title.contains("kaboom"));
        assert!(i.last_detail.as_deref().unwrap().contains("boom-thread"));
    }
}
