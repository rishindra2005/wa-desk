//! Long-running background loops. Each tick runs in its own task so a panic
//! only loses that tick; errors are logged, recorded as issues and retried
//! with exponential backoff. Nothing here ever panics the app.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wa_core::{AppError, ErrorKind, Result};

use crate::core::AppCore;

/// Watcher poll interval.
pub const WATCH_EVERY: Duration = Duration::from_secs(1);
/// Bridge state poll interval.
pub const BRIDGE_EVERY: Duration = Duration::from_secs(3);
/// Index reconcile interval.
pub const RECONCILE_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Index completeness re-check interval (restarts an interrupted build).
pub const INDEX_CHECK_EVERY: Duration = Duration::from_secs(300);
/// Outbox cleanup interval.
pub const CLEANUP_EVERY: Duration = Duration::from_secs(24 * 3600);
/// Upper bound for error backoff.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Delay after `failures` consecutive errors (base · 2^n, capped).
pub fn backoff(base: Duration, failures: u32) -> Duration {
    let factor = 1u32 << failures.min(10);
    (base * factor).min(MAX_BACKOFF.max(base))
}

/// Run one tick in its own task; a panic becomes an `Internal` error.
async fn guarded<T, F>(fut: F) -> Result<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T>> + Send + 'static,
{
    tokio::spawn(fut)
        .await
        .map_err(|e| AppError::internal("background task panicked").with_detail(e.to_string()))?
}

/// Generic resilient loop: run `tick` every `every`, backing off on errors.
async fn run_loop<F, Fut>(
    core: Arc<AppCore>,
    name: &'static str,
    title: &'static str,
    every: Duration,
    initial_delay: Duration,
    shutdown: CancellationToken,
    tick: F,
) where
    F: Fn(Arc<AppCore>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    tracing::info!(
        task = name,
        every_s = every.as_secs_f64(),
        "background loop started"
    );
    let mut delay = initial_delay;
    let mut failures: u32 = 0;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(delay) => {}
        }
        match guarded(tick(core.clone())).await {
            Ok(()) => {
                if failures > 0 {
                    tracing::info!(task = name, after = failures, "background task recovered");
                }
                failures = 0;
                delay = every;
            }
            Err(e) => {
                if failures == 0 {
                    tracing::warn!(task = name, error = %e, "background task failed; backing off");
                    let kind = if e.kind == ErrorKind::Cancelled {
                        ErrorKind::Internal
                    } else {
                        e.kind
                    };
                    core.record_issue(
                        kind,
                        &format!("app.{name}"),
                        title,
                        e.detail.as_deref().unwrap_or(&e.message),
                    );
                } else {
                    tracing::debug!(task = name, error = %e, failures, "background task still failing");
                }
                failures = failures.saturating_add(1);
                delay = backoff(every, failures);
            }
        }
    }
    tracing::info!(task = name, "background loop stopped");
}

/// Start every background loop on the current Tokio runtime.
pub fn spawn_all(core: Arc<AppCore>, shutdown: CancellationToken) {
    let sd = shutdown.clone();
    tokio::spawn(run_loop(
        core.clone(),
        "index",
        "Search index could not be updated",
        INDEX_CHECK_EVERY,
        Duration::ZERO,
        sd,
        |c| async move { c.ensure_index().await.map(|_| ()) },
    ));
    tokio::spawn(run_loop(
        core.clone(),
        "watcher",
        "Cannot read new messages from the bridge database",
        WATCH_EVERY,
        Duration::from_millis(500),
        shutdown.clone(),
        |c| async move { c.watch_tick().await.map(|_| ()) },
    ));
    tokio::spawn(run_loop(
        core.clone(),
        "bridge",
        "Bridge state check failed",
        BRIDGE_EVERY,
        Duration::ZERO,
        shutdown.clone(),
        |c| async move {
            c.bridge_tick().await;
            Ok(())
        },
    ));
    tokio::spawn(run_loop(
        core.clone(),
        "reconcile",
        "Search index reconcile failed",
        RECONCILE_EVERY,
        RECONCILE_EVERY,
        shutdown.clone(),
        |c| async move { c.reconcile_tick().await },
    ));
    tokio::spawn(run_loop(
        core,
        "outbox_cleanup",
        "Outbox cleanup failed",
        CLEANUP_EVERY,
        Duration::from_secs(60),
        shutdown,
        |c| async move { c.outbox_cleanup_tick().await.map(|_| ()) },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let s = Duration::from_secs(1);
        assert_eq!(backoff(s, 1), Duration::from_secs(2));
        assert_eq!(backoff(s, 3), Duration::from_secs(8));
        assert_eq!(backoff(s, 30), MAX_BACKOFF);
        // Slow loops never back off below their own interval.
        let day = Duration::from_secs(86_400);
        assert_eq!(backoff(day, 2), day);
    }
}
