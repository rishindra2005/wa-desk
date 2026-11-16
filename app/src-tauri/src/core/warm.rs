//! Background cache warmer: thumbnails for every local image/video/sticker/PDF
//! and PDF previews for every local office document, so opening a preview is
//! a cache hit instead of a multi-second LibreOffice/ffmpeg run.
//!
//! Runs one item at a time with external tools under `nice -n 19`, yields
//! while the user is previewing or converting, skips cached items (resumable
//! by construction: the cache *is* the progress) and stops on shutdown.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use wa_core::types::{JobSpec, JobStatus, MediaKind};
use wa_core::{ErrorKind, Result};

use super::{AppCore, OFFICE_EXTS, blocking};

/// Office documents larger than this are not pre-converted.
pub const WARM_OFFICE_MAX: u64 = 30 * 1024 * 1024;
/// Pause between items so the warmer never hogs a core.
const ITEM_GAP: Duration = Duration::from_millis(5);
/// How often to re-check while the user is busy.
const BUSY_POLL: Duration = Duration::from_millis(300);

/// Shared warmer state.
#[derive(Default)]
pub struct WarmState {
    user_busy: AtomicUsize,
    running: AtomicBool,
    kick: Notify,
    done: AtomicU64,
    total: AtomicU64,
}

/// Marks a user-facing preview/conversion in flight (the warmer yields).
pub struct UserBusy(Arc<WarmState>);

impl Drop for UserBusy {
    fn drop(&mut self) {
        self.0.user_busy.fetch_sub(1, Ordering::SeqCst);
    }
}

impl WarmState {
    /// Ask for a new pass (after link/backfill jobs).
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    /// Wait for a kick or `max`, whichever comes first.
    pub async fn wait(&self, max: Duration) {
        let _ = tokio::time::timeout(max, self.kick.notified()).await;
    }

    /// Hold while doing user-visible work.
    pub fn busy(self: &Arc<Self>) -> UserBusy {
        self.user_busy.fetch_add(1, Ordering::SeqCst);
        UserBusy(self.clone())
    }

    /// `(done, total)` of the current/last pass.
    pub fn progress(&self) -> (u64, u64) {
        (
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
}

/// Outcome of one warm pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WarmStats {
    pub scanned: u64,
    pub made: u64,
    pub cached: u64,
    /// No thumbnail possible (audio-like, non-PDF documents, too large).
    pub skipped: u64,
    pub failed: u64,
}

/// Should this job finishing trigger a new warm pass?
pub(crate) fn kicks_warm(spec: &JobSpec, status: JobStatus) -> bool {
    status == JobStatus::Succeeded
        && matches!(
            spec,
            JobSpec::Backfill { .. } | JobSpec::LinkPhoneMedia { .. } | JobSpec::PhoneSync
        )
}

fn office_ext(name: Option<&str>, path: &Path) -> bool {
    let ext = name
        .and_then(|n| Path::new(n).extension())
        .or_else(|| path.extension())
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    OFFICE_EXTS.contains(&ext.as_str())
}

impl AppCore {
    /// The warmer's shared state.
    pub fn warm_state(&self) -> &Arc<WarmState> {
        &self.warm
    }

    async fn user_converting(&self) -> bool {
        if self.warm.user_busy.load(Ordering::SeqCst) > 0 {
            return true;
        }
        let jobs = self.jobs.clone();
        blocking(move || jobs.list(20))
            .await
            .map(|v| {
                v.iter().any(|j| {
                    j.status == JobStatus::Running
                        && matches!(j.spec, JobSpec::Convert(_) | JobSpec::Thumbnails)
                })
            })
            .unwrap_or(false)
    }

    /// One full pass over local media. Cached items cost one `stat`.
    pub async fn warm_pass(&self, shutdown: &CancellationToken) -> Result<WarmStats> {
        if self.warm.running.swap(true, Ordering::SeqCst) {
            return Ok(WarmStats::default());
        }
        struct Reset<'a>(&'a AtomicBool);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset(&self.warm.running);
        let t0 = Instant::now();
        let db = self.db.clone();
        let rows = blocking(move || {
            wa_core::traits::MessageStore::media_rows(
                &*db,
                None,
                &[
                    MediaKind::Image,
                    MediaKind::Video,
                    MediaKind::Sticker,
                    MediaKind::Document,
                ],
            )
        })
        .await?;
        let mut rows: Vec<_> = rows
            .into_iter()
            .filter(|r| r.local_path.is_some())
            .collect();
        // Newest first: what the user is most likely to open.
        rows.sort_by_key(|r| std::cmp::Reverse(r.ts));
        self.warm.total.store(rows.len() as u64, Ordering::Relaxed);
        self.warm.done.store(0, Ordering::Relaxed);
        tracing::info!(items = rows.len(), "cache warm pass started");
        let tools = wa_jobs::Tools {
            low_priority: true,
            ..Default::default()
        };
        let cancel = shutdown.child_token();
        let mut st = WarmStats::default();
        for (i, r) in rows.iter().enumerate() {
            while self.user_converting().await {
                tokio::select! {
                    _ = shutdown.cancelled() => return Ok(st),
                    _ = tokio::time::sleep(BUSY_POLL) => {}
                }
            }
            if shutdown.is_cancelled() {
                break;
            }
            let Some(path) = r.local_path.as_deref() else {
                continue;
            };
            st.scanned += 1;
            let office = r.kind == MediaKind::Document && office_ext(r.filename.as_deref(), path);
            let cached = if office {
                wa_jobs::cached_office_preview(&self.paths, path)
            } else {
                wa_jobs::cached_thumbnail(&self.paths, path)
            };
            if matches!(cached, Ok(Some(_))) {
                st.cached += 1;
                self.warm.done.store(i as u64 + 1, Ordering::Relaxed);
                continue;
            }
            let res = match r.kind {
                MediaKind::Document if office => {
                    if std::fs::metadata(path).is_ok_and(|m| m.len() > WARM_OFFICE_MAX) {
                        Ok(false)
                    } else {
                        wa_jobs::office_preview_with(&self.paths, &tools, path, &cancel)
                            .await
                            .map(|_| true)
                    }
                }
                kind => wa_jobs::thumbnail_with(&self.paths, &tools, path, kind, &cancel)
                    .await
                    .map(|o| o.is_some()),
            };
            match res {
                Ok(true) => st.made += 1,
                Ok(false) => st.skipped += 1,
                Err(e) if e.kind == ErrorKind::Cancelled => break,
                Err(e) => {
                    st.failed += 1;
                    tracing::debug!(kind = ?e.kind, error = %e.message, "warm item failed");
                }
            }
            self.warm.done.store(i as u64 + 1, Ordering::Relaxed);
            if (i + 1) % 250 == 0 {
                tracing::info!(
                    done = i + 1,
                    total = rows.len(),
                    made = st.made,
                    failed = st.failed,
                    "cache warm progress"
                );
            }
            tokio::time::sleep(ITEM_GAP).await;
        }
        tracing::info!(
            scanned = st.scanned,
            made = st.made,
            cached = st.cached,
            skipped = st.skipped,
            failed = st.failed,
            secs = t0.elapsed().as_secs(),
            "cache warm pass finished"
        );
        Ok(st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_media_jobs_kick() {
        assert!(kicks_warm(&JobSpec::PhoneSync, JobStatus::Succeeded));
        assert!(!kicks_warm(&JobSpec::PhoneSync, JobStatus::Failed));
        assert!(!kicks_warm(&JobSpec::Reindex, JobStatus::Succeeded));
    }

    #[test]
    fn busy_guard_counts() {
        let s = Arc::new(WarmState::default());
        let a = s.busy();
        let b = s.busy();
        assert_eq!(s.user_busy.load(Ordering::SeqCst), 2);
        drop(a);
        drop(b);
        assert_eq!(s.user_busy.load(Ordering::SeqCst), 0);
    }
}
