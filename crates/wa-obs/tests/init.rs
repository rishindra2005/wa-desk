//! Global subscriber setup (own test binary: the global default can be set once).

use std::sync::Arc;

use wa_core::config::Paths;
use wa_core::types::{LogFilter, LogLevel};
use wa_obs::{IssueStore, init_logging, init_logging_with_issues, query_logs};

#[test]
fn init_writes_json_file_and_records_errors() {
    let d = tempfile::tempdir().unwrap();
    let p = Paths::under_root(d.path(), "http://127.0.0.1:8080".into());
    let store = Arc::new(IssueStore::open(&p).unwrap());
    let guard = init_logging_with_issues(&p, Some("info,wa_=debug"), Some(store.clone())).unwrap();
    tracing::debug!(target: "wa_jobs::x", "debug kept for wa_ targets");
    tracing::debug!(target: "hyper", "debug dropped for others");
    tracing::error!(target: "wa_bridge", kind = "auth", "token rejected");
    drop(guard); // flush the non-blocking writer

    let files = wa_obs::log_files(&p.logs);
    assert_eq!(files.len(), 1);
    let name = files[0].file_name().unwrap().to_str().unwrap().to_string();
    assert!(
        name.starts_with("wa-desk.") && name.ends_with(".jsonl"),
        "{name}"
    );
    let lines = query_logs(
        &p,
        &LogFilter {
            level: Some(LogLevel::Trace),
            ..Default::default()
        },
    )
    .unwrap();
    let msgs: Vec<_> = lines.iter().map(|l| l.message.as_str()).collect();
    assert_eq!(msgs, ["token rejected", "debug kept for wa_ targets"]);
    let issues = store.list().unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, wa_core::ErrorKind::Auth);
    assert_eq!(issues[0].source, "wa_bridge");

    // A second global init is refused, not a panic.
    assert!(init_logging(&p, None).is_err());
}
