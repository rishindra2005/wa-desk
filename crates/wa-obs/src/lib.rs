//! Structured logging, redaction, issue tracking and issue-report bundles.
//!
//! - [`init_logging`]: pretty stderr + daily rolling JSON files (7 kept).
//! - [`redact`]: scrubbing of tokens, phone numbers and home paths.
//! - [`query_logs`]: newest-first log lines for the Logs view.
//! - [`IssueStore`]: deduplicated issues in `obs.db`; implements `IssueSink`.
//! - [`IssueLayer`] / [`install_panic_hook`]: ERROR events and panics → issues.
//! - [`export_bundle`]: redacted zip for bug reports.

mod bundle;
mod issues;
mod logging;
mod query;
pub mod redact;

pub use bundle::{BUNDLE_LOG_LINES, export_bundle};
pub use issues::{IssueListener, IssueStore, fingerprint, normalize_title};
pub use logging::{
    DEFAULT_FILTER, IssueLayer, KEEP_FILES, LOG_ENV, LOG_PREFIX, LOG_SUFFIX, LogGuard,
    init_logging, init_logging_with_issues, install_panic_hook,
};
pub use query::{log_files, parse_line, query_logs, tail_raw_lines};
