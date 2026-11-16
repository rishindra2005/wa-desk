//! Preview latency bench against the real data, read-only.
//!
//! The bridge DBs and store are only read; app.db, cache and exports go to a
//! fresh directory (`$WA_BENCH_DIR`, default a temp dir), so the first
//! measurement of each item is a true cold start and the second a cache hit.
//!
//! ```sh
//! cargo run --release -p wa-desk --example preview_bench
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tauri::http::{Request, header};
use wa_core::Result;
use wa_core::config::Paths;
use wa_core::traits::{MediaInfo, MessageStore};
use wa_core::types::MediaKind;
use wa_desk_lib::core::{AppCore, CoreDeps, Emitter, Platform};

struct Nop;
impl Emitter for Nop {
    fn emit(&self, _: &str, _: Value) {}
}
impl Platform for Nop {
    fn open_path(&self, _: &Path) -> Result<()> {
        Ok(())
    }
    fn reveal_path(&self, _: &Path) -> Result<()> {
        Ok(())
    }
    fn set_autostart(&self, _: bool) -> Result<()> {
        Ok(())
    }
    fn notify(&self, _: &str, _: &str) {}
    fn main_window_focused(&self) -> bool {
        true
    }
}

fn ext_of(i: &MediaInfo) -> String {
    i.filename
        .as_deref()
        .and_then(|f| Path::new(f).extension())
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn size(i: &MediaInfo) -> u64 {
    i.local_path
        .as_deref()
        .and_then(|p| p.metadata().ok())
        .map_or(0, |m| m.len())
}

async fn timed_get(core: &AppCore, url: &str, range: Option<&str>) -> (f64, u16, usize) {
    let mut b = Request::builder().method("GET").uri(url);
    if let Some(r) = range {
        b = b.header(header::RANGE, r);
    }
    let req = b.body(Vec::new()).unwrap_or_default();
    let t = Instant::now();
    let r = wa_desk_lib::protocol::handle(core, &req).await;
    (
        t.elapsed().as_secs_f64() * 1000.0,
        r.status().as_u16(),
        r.body().len(),
    )
}

fn pick(rows: &[MediaInfo], f: impl Fn(&MediaInfo) -> bool) -> Option<&MediaInfo> {
    rows.iter()
        .filter(|r| r.local_path.is_some())
        .find(|r| f(r))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let real = Paths::from_env();
    let dir = std::env::var_os("WA_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("wa-bench-{}", std::process::id())));
    let mut paths = real.clone();
    paths.app_data = dir.join("data");
    paths.app_db = dir.join("data/app.db");
    paths.cache = dir.join("cache");
    paths.logs = dir.join("logs");
    paths.exports = dir.join("exports");
    paths.ensure_own_dirs().ok();
    let bridge = Arc::new(wa_bridge::BridgeClient::new(&paths).expect("bridge client"));
    let core = AppCore::new(CoreDeps {
        paths: paths.clone(),
        bridge,
        issues: Arc::new(wa_obs::IssueStore::in_memory().expect("issues")),
        emitter: Arc::new(Nop),
        platform: Arc::new(Nop),
    })
    .await
    .expect("core");
    core.set_read_only(true);
    let db = core.db().clone();
    let t = Instant::now();
    let rows = tokio::task::spawn_blocking(move || db.media_rows(None, &[]))
        .await
        .expect("join")
        .expect("media rows");
    println!(
        "media rows: {} ({} local) in {:.0} ms",
        rows.len(),
        rows.iter().filter(|r| r.local_path.is_some()).count(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    let cases: Vec<(&str, Option<&MediaInfo>)> = vec![
        (
            "image ~5MB",
            pick(&rows, |r| r.kind == MediaKind::Image && size(r) > 2_000_000)
                .or_else(|| pick(&rows, |r| r.kind == MediaKind::Image && size(r) > 300_000)),
        ),
        ("video", pick(&rows, |r| r.kind == MediaKind::Video)),
        (
            "pdf",
            pick(&rows, |r| ext_of(r) == "pdf" && size(r) > 200_000),
        ),
        ("docx", pick(&rows, |r| ext_of(r) == "docx")),
        ("pptx", pick(&rows, |r| ext_of(r) == "pptx")),
    ];
    for (label, row) in cases {
        let Some(r) = row else {
            println!("{label:>11}: no local sample");
            continue;
        };
        let (chat, id) = (&r.chat_jid, &r.message_id);
        let kb = size(r) / 1024;
        let office = matches!(ext_of(r).as_str(), "docx" | "pptx");
        let prefix = if office { "p" } else { "t" };
        let url = wa_db::media::media_url(prefix, chat, id);
        let cold = timed_get(&core, &url, None).await;
        let warm = timed_get(&core, &url, None).await;
        let what = if office { "preview pdf" } else { "thumbnail" };
        println!(
            "{label:>11} ({kb} KB): {what} cold {:.0} ms (HTTP {}), cached {:.1} ms",
            cold.0, cold.1, warm.0
        );
        let m = wa_db::media::media_url("m", chat, id);
        let full = timed_get(&core, &m, None).await;
        let first = timed_get(&core, &m, Some("bytes=0-65535")).await;
        println!(
            "{:>11}   original: full {:.1} ms ({} bytes), first 64 KB {:.1} ms",
            "", full.0, full.2, first.0
        );
    }
    // A media grid opening: 40 thumbnails at once, plus the user opening one
    // full image in the middle of it. Cold = no warm cache, then after a warm pass.
    let imgs: Vec<&MediaInfo> = rows
        .iter()
        .filter(|r| r.local_path.is_some() && r.kind == MediaKind::Image)
        .skip(5)
        .take(40)
        .collect();
    // Name snapshot reload (every 5 min TTL, on the first command after it).
    if let Some(r) = rows.first() {
        let db = core.db().clone();
        let (c, i) = (r.chat_jid.clone(), r.message_id.clone());
        let ms = tokio::task::spawn_blocking(move || {
            db.names().invalidate();
            let t = Instant::now();
            let _ = db.get_message(&c, &i);
            let cold = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let _ = db.get_message(&c, &i);
            (cold, t.elapsed().as_secs_f64() * 1000.0)
        })
        .await
        .unwrap_or_default();
        println!(
            "get_message: with name reload {:.1} ms, cached names {:.2} ms",
            ms.0, ms.1
        );
    }
    let core = Arc::new(core);
    for round in ["cold", "after warm"] {
        if round == "after warm" {
            let t = Instant::now();
            // Warm only what the grid needs (a full pass covers all local media).
            for r in &imgs {
                if let Some(p) = &r.local_path {
                    let _ = wa_jobs::thumbnail(&paths, p, r.kind).await;
                }
            }
            println!(
                "warmed {} thumbnails in {:.0} ms",
                imgs.len(),
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
        let t = Instant::now();
        let mut set = tokio::task::JoinSet::new();
        for r in &imgs {
            let (c, url) = (
                core.clone(),
                wa_db::media::media_url("t", &r.chat_jid, &r.message_id),
            );
            set.spawn(async move { timed_get(&c, &url, None).await.0 });
        }
        let probe = imgs
            .first()
            .map(|r| wa_db::media::media_url("t", &r.chat_jid, &r.message_id));
        let one = match &probe {
            Some(u) => timed_get(&core, u, None).await.0,
            None => 0.0,
        };
        let mut worst: f64 = 0.0;
        while let Some(Ok(ms)) = set.join_next().await {
            worst = worst.max(ms);
        }
        println!(
            "grid of {} thumbnails ({round}): all done in {:.0} ms, slowest {:.0} ms, one opened meanwhile {:.0} ms",
            imgs.len(),
            t.elapsed().as_secs_f64() * 1000.0,
            worst,
            one
        );
    }
    let a = timed_get(&core, "wa-media://localhost/a/status%40broadcast", None).await;
    println!("avatar status@broadcast: {:.1} ms HTTP {}", a.0, a.1);
    if let Some(g) = rows.iter().find(|r| r.chat_jid.ends_with("@g.us")) {
        let url = wa_db::media::avatar_url(&g.chat_jid);
        let a = timed_get(&core, &url, None).await;
        let b = timed_get(&core, &url, None).await;
        println!(
            "avatar group: first {:.1} ms HTTP {}, cached {:.2} ms",
            a.0, a.1, b.0
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
