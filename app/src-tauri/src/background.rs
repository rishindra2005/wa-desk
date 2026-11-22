//! Long-running background loops. Each tick runs in its own task so a panic
//! only loses that tick; errors are logged, recorded as issues and retried
//! with exponential backoff. Nothing here ever panics the app.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use wa_core::transient::{FailureTracker, is_transient_kind};
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
/// Chat-name refresh interval (also run once shortly after startup).
pub const NAMES_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Cache-warm pass interval (also kicked after link/backfill jobs).
pub const WARM_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Delay before the first warm pass, so startup stays snappy.
pub const WARM_FIRST: Duration = Duration::from_secs(20);
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

/// Tasks whose already-open timeout issues (from before transient timeouts
/// stopped raising issues) are resolved once their first run succeeds.
const STALE_TIMEOUT_CLEANUP: &[&str] = &["watcher", "index"];

/// Per-task failure state used by [`on_failure`] and [`on_success`].
#[derive(Debug, Default)]
pub struct TaskHealth {
    tracker: FailureTracker,
    /// Kinds of issues this task raised during the current failure streak.
    raised: Vec<ErrorKind>,
    cleaned: bool,
}

impl TaskHealth {
    /// Consecutive failures so far.
    pub fn attempts(&self) -> u32 {
        self.tracker.attempts()
    }
}

/// Handle one failed run at `now`.
///
/// Transient errors (timeouts, busy) are logged at DEBUG and only escalate to
/// WARN plus an issue once the streak has lasted 60 s over 5 attempts. Any
/// other error warns and raises an issue on its first occurrence.
pub fn on_failure(
    core: &AppCore,
    name: &'static str,
    title: &str,
    health: &mut TaskHealth,
    now: Instant,
    e: &AppError,
) {
    let kind = if e.kind == ErrorKind::Cancelled {
        ErrorKind::Internal
    } else {
        e.kind
    };
    let attempts_before = health.tracker.attempts();
    let escalate = health.tracker.failure(now);
    let attempts = health.tracker.attempts();
    let raise = if is_transient_kind(kind) {
        escalate
    } else {
        !health.raised.contains(&kind)
    };
    if raise {
        tracing::warn!(task = name, error = %e, attempts, "background task failed; backing off");
        if !health.raised.contains(&kind) {
            health.raised.push(kind);
        }
        core.record_issue(
            kind,
            &format!("app.{name}"),
            title,
            e.detail.as_deref().unwrap_or(&e.message),
        );
    } else if attempts_before == 0 {
        tracing::debug!(task = name, error = %e, kind = ?kind, "background task failed; retrying");
    } else {
        tracing::debug!(task = name, error = %e, attempts, "background task still failing");
    }
}

/// Handle one successful run: resolve issues this task raised and, once per
/// process, stale timeout issues left by earlier versions.
pub fn on_success(core: &AppCore, name: &'static str, title: &str, health: &mut TaskHealth) {
    let source = format!("app.{name}");
    let (failures, _) = health.tracker.success();
    let raised = std::mem::take(&mut health.raised);
    let mut resolved = 0;
    for kind in &raised {
        match core.issues().resolve_matching(*kind, &source, title) {
            Ok(n) => resolved += n,
            Err(err) => tracing::debug!(task = name, error = %err, "could not resolve issue"),
        }
    }
    if !health.cleaned {
        health.cleaned = true;
        if STALE_TIMEOUT_CLEANUP.contains(&name) {
            match core
                .issues()
                .resolve_open_by_source(&source, ErrorKind::Timeout)
            {
                Ok(n) if n > 0 => {
                    tracing::info!(task = name, resolved = n, "resolved stale timeout issues");
                }
                Ok(_) => {}
                Err(err) => tracing::debug!(task = name, error = %err, "stale cleanup failed"),
            }
        }
    }
    if failures == 0 {
        return;
    }
    if raised.is_empty() {
        tracing::debug!(task = name, after = failures, "background task recovered");
    } else {
        tracing::info!(
            task = name,
            after = failures,
            resolved,
            "background task recovered"
        );
    }
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
    let mut health = TaskHealth::default();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(delay) => {}
        }
        match guarded(tick(core.clone())).await {
            Ok(()) => {
                on_success(&core, name, title, &mut health);
                delay = every;
            }
            Err(e) => {
                on_failure(&core, name, title, &mut health, Instant::now(), &e);
                delay = backoff(every, health.attempts());
            }
        }
    }
    tracing::info!(task = name, "background loop stopped");
}

/// Delay before the one-time chat-state resync (after the first name refresh).
pub const RESYNC_STATE_FIRST: Duration = Duration::from_secs(12);

/// Replay archive/pin/mute state once at startup, after the name refresh.
async fn resync_state_once(core: Arc<AppCore>, shutdown: CancellationToken) {
    tokio::select! {
        _ = shutdown.cancelled() => return,
        _ = tokio::time::sleep(RESYNC_STATE_FIRST) => {}
    }
    if let Err(e) = guarded(async move { core.resync_state_tick().await }).await {
        tracing::warn!(error = %e, "chat state resync failed");
    }
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
        core.clone(),
        "chat_names",
        "Chat names could not be refreshed",
        NAMES_EVERY,
        Duration::from_secs(5),
        shutdown.clone(),
        |c| async move { c.refresh_names_tick().await },
    ));
    tokio::spawn(resync_state_once(core.clone(), shutdown.clone()));
    tokio::spawn(warm_loop(core.clone(), shutdown.clone()));
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

/// Cache warmer: first pass shortly after startup, then every 6 h or when kicked.
async fn warm_loop(core: Arc<AppCore>, shutdown: CancellationToken) {
    tokio::select! {
        _ = shutdown.cancelled() => return,
        _ = tokio::time::sleep(WARM_FIRST) => {}
    }
    loop {
        let c = core.clone();
        let sd = shutdown.clone();
        if let Err(e) = guarded(async move { c.warm_pass(&sd).await.map(|_| ()) }).await {
            tracing::warn!(error = %e, "cache warm pass failed");
        }
        let state = core.warm_state().clone();
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = state.wait(WARM_EVERY) => {}
        }
    }
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
