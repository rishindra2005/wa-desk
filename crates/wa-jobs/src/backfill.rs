//! Media backfill (port of `whatsapp-download-all`): ask the bridge to fetch
//! recent media that is still on WhatsApp's servers, never asking the phone.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use tokio::task::JoinSet;
use wa_core::time::now_epoch;
use wa_core::traits::{MediaInfo, ProgressSink};
use wa_core::types::MediaKind;
use wa_core::{AppError, ErrorKind, Result};

use crate::manager::{JobCtl, JobDeps};
use crate::util::blocking;

/// Key used in the skip list: `chat|message_id`.
pub fn skip_key(chat: &str, id: &str) -> String {
    format!("{chat}|{id}")
}

/// Read the skip list (missing file = empty).
pub fn read_skip_list(path: &Path) -> HashSet<String> {
    std::fs::read_to_string(path)
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn append_skip(path: &Path, keys: &[String]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for k in keys {
        writeln!(f, "{k}")?;
    }
    Ok(())
}

fn rewrite_skip(path: &Path, keys: &HashSet<String>) -> Result<()> {
    let mut v: Vec<_> = keys.iter().cloned().collect();
    v.sort();
    let tmp = path.with_extension("tmp");
    let mut body = v.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

enum Outcome {
    Ok,
    /// Media is gone: remember it.
    Unavailable,
    /// Transient (timeout): don't remember.
    Transient,
    /// Bridge offline / auth broken: stop the job.
    Fatal(AppError),
}

fn classify(e: AppError) -> Outcome {
    match e.kind {
        ErrorKind::BridgeDown | ErrorKind::Auth | ErrorKind::Forbidden => Outcome::Fatal(e),
        ErrorKind::Timeout | ErrorKind::Busy | ErrorKind::Cancelled => Outcome::Transient,
        _ => Outcome::Unavailable,
    }
}

fn order(rows: &mut [MediaInfo]) {
    rows.sort_by(|a, b| {
        let da = a.kind != MediaKind::Document;
        let db = b.kind != MediaKind::Document;
        da.cmp(&db).then(b.ts.cmp(&a.ts))
    });
}

/// Run the backfill. Returns the summary message.
pub async fn run(
    deps: &JobDeps,
    days: u32,
    kinds: &[MediaKind],
    workers: u8,
    retry_failed: bool,
    ctl: &Arc<JobCtl>,
) -> Result<String> {
    if days == 0 {
        return Err(AppError::invalid("days must be at least 1"));
    }
    ctl.phase("listing media");
    let since = now_epoch() - i64::from(days) * 86_400;
    let store = deps.messages.clone();
    let kinds_v = kinds.to_vec();
    let mut rows = blocking(move || store.media_rows(Some(since), &kinds_v)).await?;
    let skip_path = deps.paths.backfill_skip.clone();
    let old_skip = read_skip_list(&skip_path);
    let total_rows = rows.len();
    rows.retain(|r| {
        r.local_path.is_none() && (retry_failed || !old_skip.contains(&skip_key(&r.chat_jid, &r.message_id)))
    });
    order(&mut rows);
    let skipped = total_rows - rows.len();
    let total = rows.len() as u64;
    tracing::info!(total, skipped, days, "backfill starting");
    ctl.progress(0, total, "downloading", Some(&format!("{total} to fetch, {skipped} already done/skipped")));

    let workers = usize::from(workers.clamp(1, 8));
    let mut set: JoinSet<(String, Outcome)> = JoinSet::new();
    let (mut ok, mut failed, mut transient, mut done) = (0u64, Vec::new(), 0u64, 0u64);
    let mut succeeded = Vec::new();
    let mut fatal: Option<AppError> = None;
    let mut queue = rows.into_iter();

    loop {
        while fatal.is_none() && !ctl.is_cancelled() && set.len() < workers {
            let Some(r) = queue.next() else { break };
            let bridge = deps.bridge.clone();
            let delay = deps.backfill_retry_delay;
            set.spawn(async move {
                let key = skip_key(&r.chat_jid, &r.message_id);
                let mut res = bridge.download(&r.chat_jid, &r.message_id, true).await;
                if matches!(&res, Err(e) if e.kind == ErrorKind::BridgeDown) {
                    tokio::time::sleep(delay).await;
                    res = bridge.download(&r.chat_jid, &r.message_id, true).await;
                }
                (key, res.map_or_else(classify, |_| Outcome::Ok))
            });
        }
        let Some(joined) = set.join_next().await else { break };
        done += 1;
        match joined {
            Ok((key, Outcome::Ok)) => {
                ok += 1;
                succeeded.push(key);
            }
            Ok((key, Outcome::Unavailable)) => {
                append_skip(&skip_path, std::slice::from_ref(&key))?;
                failed.push(key);
            }
            Ok((_, Outcome::Transient)) | Err(_) => transient += 1,
            Ok((_, Outcome::Fatal(e))) => {
                transient += 1;
                fatal.get_or_insert(e);
            }
        }
        ctl.progress(done, total, "downloading", Some(&format!("ok {ok}, unavailable {}", failed.len())));
    }

    if retry_failed {
        let mut keep: HashSet<String> = read_skip_list(&skip_path);
        for k in &succeeded {
            keep.remove(k);
        }
        if keep.len() != read_skip_list(&skip_path).len() {
            rewrite_skip(&skip_path, &keep)?;
        }
    }
    if let Some(e) = fatal {
        return Err(AppError::new(
            e.kind,
            format!("{} ({ok} downloaded before stopping)", e.message),
        ));
    }
    if ctl.is_cancelled() {
        return Err(AppError::cancelled());
    }
    let mut msg = format!(
        "Downloaded {ok}; {} unavailable (won't be retried); {skipped} already done or skipped",
        failed.len()
    );
    if transient > 0 {
        msg.push_str(&format!("; {transient} timed out (will retry next run)"));
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_list_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/skip.txt");
        assert!(read_skip_list(&p).is_empty());
        append_skip(&p, &["a|1".into(), "b|2".into()]).unwrap();
        append_skip(&p, &["c|3".into()]).unwrap();
        let s = read_skip_list(&p);
        assert_eq!(s.len(), 3);
        let keep: HashSet<String> = ["b|2".to_string()].into();
        rewrite_skip(&p, &keep).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "b|2\n");
    }

    #[test]
    fn classifies_errors() {
        assert!(matches!(classify(AppError::external("expired")), Outcome::Unavailable));
        assert!(matches!(classify(AppError::not_found("x")), Outcome::Unavailable));
        assert!(matches!(classify(AppError::timeout("x")), Outcome::Transient));
        assert!(matches!(classify(AppError::bridge_down("x")), Outcome::Fatal(_)));
        assert!(matches!(classify(AppError::auth("x")), Outcome::Fatal(_)));
    }
}
