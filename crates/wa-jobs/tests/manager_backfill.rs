mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::*;
use wa_core::time::now_epoch;
use wa_core::traits::ProgressSink;
use wa_core::types::*;
use wa_core::{AppError, ErrorKind};
use wa_jobs::{JobEvent, Tool};

fn convert_spec() -> JobSpec {
    JobSpec::Convert(ConvertSpec {
        input: ConvertInput::File {
            path: "/nonexistent".into(),
        },
        kind: ConvertKind::OfficeToPdf,
        output_dir: None,
    })
}

#[tokio::test]
async fn custom_job_lifecycle_is_persisted() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let rec = m
        .start_custom(JobSpec::Reindex, |ctl| {
            Box::pin(async move {
                ctl.progress(5, 10, "indexing", Some("half"));
                Ok(Some("indexed 10".into()))
            })
        })
        .unwrap();
    assert_eq!(rec.status, JobStatus::Queued);
    assert_eq!(rec.label, "Rebuild search index");
    let done = ev.finished(rec.id).await;
    assert_eq!(done.status, JobStatus::Succeeded);
    assert_eq!(done.message.as_deref(), Some("indexed 10"));
    assert_eq!(done.done, 10);
    assert!(done.finished_at.is_some());
    assert_eq!(env.app.jobs.lock()[0], done, "final state persisted");
    assert_eq!(m.list(10).unwrap()[0].status, JobStatus::Succeeded);
}

#[tokio::test]
async fn same_kind_is_busy_and_cancel_works() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let job = |ctl: Arc<wa_jobs::JobCtl>| -> wa_jobs::JobFuture {
        Box::pin(async move {
            ctl.cancel_token().cancelled().await;
            Err(AppError::cancelled())
        })
    };
    let a = m.start_custom(JobSpec::Reindex, job).unwrap();
    let e = m.start_custom(JobSpec::Reindex, job).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
    // A different kind may run alongside.
    let b = m.start_custom(JobSpec::Thumbnails, job).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(m.get(a.id).unwrap().status, JobStatus::Running);
    assert_eq!(
        m.list(10)
            .unwrap()
            .iter()
            .filter(|j| j.status == JobStatus::Running)
            .count(),
        2
    );
    m.cancel(a.id).unwrap();
    assert_eq!(ev.finished(a.id).await.status, JobStatus::Cancelled);
    m.cancel_all();
    assert_eq!(ev.finished(b.id).await.status, JobStatus::Cancelled);
    assert_eq!(m.cancel(a.id).unwrap_err().kind, ErrorKind::NotFound);
    // Kind is free again.
    let c = m
        .start_custom(JobSpec::Reindex, |_| Box::pin(async { Ok(None) }))
        .unwrap();
    assert_eq!(ev.finished(c.id).await.status, JobStatus::Succeeded);
}

#[tokio::test]
async fn reindex_needs_custom_and_bad_convert_rejected_upfront() {
    let env = Env::new();
    let (m, _ev) = env.manager();
    assert_eq!(
        m.start(JobSpec::Reindex).unwrap_err().kind,
        ErrorKind::Invalid
    );
    let e = m
        .start(JobSpec::Convert(ConvertSpec {
            input: ConvertInput::File { path: "/x".into() },
            kind: ConvertKind::PdfCompress {
                level: "prepress".into(),
            },
            output_dir: None,
        }))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert!(env.app.jobs.lock().is_empty());
}

#[tokio::test]
async fn progress_is_throttled() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let rec = m
        .start_custom(JobSpec::Reindex, |ctl| {
            Box::pin(async move {
                for i in 0..2000u64 {
                    ctl.progress(i, 2000, "work", None);
                    if i % 100 == 0 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                Ok(None)
            })
        })
        .unwrap();
    let t0 = Instant::now();
    ev.finished(rec.id).await;
    let secs = t0.elapsed().as_secs_f64();
    let n = ev.progress_count(rec.id);
    assert!(n >= 2, "some progress emitted");
    assert!((n as f64) <= secs * 5.0 + 2.0, "{n} events in {secs}s");
}

#[tokio::test]
async fn panicking_job_fails_and_records_issue() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let rec = m
        .start_custom(JobSpec::Reindex, |_| Box::pin(async { panic!("boom") }))
        .unwrap();
    let done = ev.finished(rec.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    assert_eq!(done.error.as_deref(), Some("job crashed"));
    let issues = env.issues.0.lock();
    assert_eq!(issues[0].1, "jobs.reindex");
}

#[tokio::test]
async fn converts_limited_to_four_concurrent() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut ids = vec![];
    for _ in 0..7 {
        let (l, p) = (live.clone(), peak.clone());
        let r = m
            .start_custom(convert_spec(), move |_| {
                Box::pin(async move {
                    let now = l.fetch_add(1, Ordering::SeqCst) + 1;
                    p.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    l.fetch_sub(1, Ordering::SeqCst);
                    Ok(None)
                })
            })
            .unwrap();
        ids.push(r.id);
    }
    for id in ids {
        assert_eq!(ev.finished(id).await.status, JobStatus::Succeeded);
    }
    assert_eq!(peak.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn cancel_kills_long_child_quickly() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let rec = m
        .start_custom(convert_spec(), |ctl| {
            Box::pin(async move {
                Tool::new("sleep", Path::new("sleep"))
                    .arg("30")
                    .run(ctl.cancel_token(), None)
                    .await?;
                Ok(None)
            })
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t0 = Instant::now();
    m.cancel(rec.id).unwrap();
    let done = ev.finished(rec.id).await;
    assert_eq!(done.status, JobStatus::Cancelled);
    assert!(
        t0.elapsed() < Duration::from_millis(3500),
        "{:?}",
        t0.elapsed()
    );
}

// ---------- backfill ----------

const CHAT: &str = "919811111111@s.whatsapp.net";

fn seed_backfill(env: &Env) {
    let now = now_epoch();
    let mut local = media(CHAT, "LOCAL", MediaKind::Image, now - 100);
    local.local_path = Some(env.paths.bridge_store.join("x.jpg"));
    *env.store.rows.lock() = vec![
        media(CHAT, "IMG_OK", MediaKind::Image, now - 50),
        media(CHAT, "DOC_OK", MediaKind::Document, now - 500),
        media(CHAT, "GONE", MediaKind::Video, now - 60),
        local,
        media(CHAT, "SKIPPED", MediaKind::Image, now - 70),
        media(CHAT, "OLD", MediaKind::Image, now - 40 * 86_400),
    ];
    std::fs::create_dir_all(env.paths.backfill_skip.parent().unwrap()).unwrap();
    std::fs::write(&env.paths.backfill_skip, format!("{CHAT}|SKIPPED\n")).unwrap();
    env.bridge
        .fail
        .lock()
        .insert("GONE".into(), AppError::external("media expired"));
}

fn backfill(retry: bool) -> JobSpec {
    JobSpec::Backfill {
        days: 30,
        kinds: vec![],
        workers: 1,
        retry_failed: retry,
    }
}

#[tokio::test]
async fn backfill_downloads_skips_and_remembers_failures() {
    let env = Env::new();
    seed_backfill(&env);
    let (m, mut ev) = env.manager();
    let r = m.start(backfill(false)).unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Succeeded, "{done:?}");
    // Documents first, then newest first; local/skip-listed/old never requested.
    assert_eq!(env.bridge.ids(), ["DOC_OK", "IMG_OK", "GONE"]);
    assert!(
        env.bridge.calls.lock().iter().all(|c| c.2),
        "no_retry always set"
    );
    let skip = std::fs::read_to_string(&env.paths.backfill_skip).unwrap();
    assert!(skip.contains(&format!("{CHAT}|GONE")) && skip.contains("SKIPPED"));
    let msg = done.message.unwrap();
    assert!(msg.starts_with("Downloaded 2; 1 unavailable"), "{msg}");
    assert_eq!(done.total, 3);

    // Second run: the failure is not retried.
    env.bridge.calls.lock().clear();
    let r = m.start(backfill(false)).unwrap();
    ev.finished(r.id).await;
    assert_eq!(env.bridge.ids(), ["DOC_OK", "IMG_OK"]);
}

#[tokio::test]
async fn backfill_retry_failed_ignores_and_prunes_skip_list() {
    let env = Env::new();
    seed_backfill(&env);
    let (m, mut ev) = env.manager();
    let r = m.start(backfill(true)).unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Succeeded);
    let ids = env.bridge.ids();
    assert!(ids.contains(&"SKIPPED".to_string()) && ids.contains(&"GONE".to_string()));
    let skip = std::fs::read_to_string(&env.paths.backfill_skip).unwrap();
    assert!(!skip.contains("SKIPPED"), "succeeded now → pruned: {skip}");
    assert!(skip.contains("GONE"));
}

#[tokio::test]
async fn backfill_bridge_down_stops_without_poisoning_skip_list() {
    let env = Env::new();
    seed_backfill(&env);
    *env.bridge.all_fail.lock() = Some(AppError::bridge_down("WhatsApp is not connected"));
    let (m, mut ev) = env.manager();
    let r = m.start(backfill(false)).unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    assert!(done.error.unwrap().contains("not connected"));
    let skip = std::fs::read_to_string(&env.paths.backfill_skip).unwrap();
    assert_eq!(skip.trim(), format!("{CHAT}|SKIPPED"));
    // Retried once after the delay, then stopped scheduling.
    assert_eq!(env.bridge.calls.lock().len(), 2);
}

#[tokio::test]
async fn backfill_timeouts_are_not_remembered_and_kinds_filter() {
    let env = Env::new();
    seed_backfill(&env);
    env.bridge
        .fail
        .lock()
        .insert("IMG_OK".into(), AppError::timeout("slow"));
    let (m, mut ev) = env.manager();
    let r = m
        .start(JobSpec::Backfill {
            days: 30,
            kinds: vec![MediaKind::Image],
            workers: 4,
            retry_failed: false,
        })
        .unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(env.bridge.ids(), ["IMG_OK"]);
    assert!(done.message.unwrap().contains("1 timed out"));
    assert!(
        !std::fs::read_to_string(&env.paths.backfill_skip)
            .unwrap()
            .contains("IMG_OK")
    );
}

#[tokio::test]
async fn backfill_zero_days_invalid() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let r = m
        .start(JobSpec::Backfill {
            days: 0,
            kinds: vec![],
            workers: 2,
            retry_failed: false,
        })
        .unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    let _ = JobEvent::Progress(JobProgress {
        id: 0,
        done: 0,
        total: 0,
        phase: String::new(),
        message: None,
    });
}
