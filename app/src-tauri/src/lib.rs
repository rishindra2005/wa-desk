//! wa-desk Tauri shell.
//!
//! - [`core`]: all application logic (`AppCore`), Tauri-independent and tested.
//! - [`commands`]: thin `#[tauri::command]` wrappers (docs/CONTRACT.md).
//! - [`protocol`]: the `wa-media://` handler (Range, scope checks).
//! - [`background`]: watcher, bridge state, index, reconcile, outbox loops.
//! - [`tray`]: tray icon/menu; closing the window hides it to the tray.

pub mod background;
pub mod commands;
pub mod core;
pub mod media_url;
pub mod protocol;
pub mod range;
pub mod tray;

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::Value;
use tauri::{AppHandle, Manager, RunEvent, Wry};
use tokio_util::sync::CancellationToken;
use wa_core::config::Paths;
use wa_core::{AppError, Result};
use wa_obs::{IssueStore, LogGuard};

use crate::core::{AppCore, CoreDeps, Emitter, Platform};

/// Command-line flag used by autostart to start in the tray.
pub const HIDDEN_ARG: &str = "--hidden";

/// Work around blank/black WebKitGTK windows on NVIDIA + Wayland. Must run
/// before any GTK/WebKit initialisation, i.e. before the Tauri builder.
pub fn apply_platform_env() {
    // SAFETY: called at the very start of `run()`, before any threads are spawned.
    unsafe {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
}

/// Emits events to every webview.
struct TauriEmitter {
    app: AppHandle<Wry>,
}

impl Emitter for TauriEmitter {
    fn emit(&self, event: &str, payload: Value) {
        use tauri::Emitter as _;
        if let Err(e) = self.app.emit(event, payload) {
            tracing::debug!(event, error = %e, "emit failed");
        }
    }
}

/// Opener / autostart / notification plugins and window focus.
struct TauriPlatform {
    app: AppHandle<Wry>,
}

impl Platform for TauriPlatform {
    fn open_path(&self, path: &Path) -> Result<()> {
        use tauri_plugin_opener::OpenerExt;
        self.app
            .opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|e| AppError::external("Could not open the file").with_detail(e.to_string()))
    }

    fn reveal_path(&self, path: &Path) -> Result<()> {
        use tauri_plugin_opener::OpenerExt;
        self.app.opener().reveal_item_in_dir(path).map_err(|e| {
            AppError::external("Could not show the file in its folder").with_detail(e.to_string())
        })
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        use tauri_plugin_autostart::ManagerExt;
        let al = self.app.autolaunch();
        let r = if enabled { al.enable() } else { al.disable() };
        r.map_err(|e| AppError::external("Could not change autostart").with_detail(e.to_string()))
    }

    fn notify(&self, title: &str, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::debug!(error = %e, "notification failed");
        }
    }

    fn main_window_focused(&self) -> bool {
        self.app
            .get_webview_window(tray::MAIN)
            .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
    }
}

/// Shared state for shutdown.
struct Shutdown {
    token: CancellationToken,
    log: Mutex<Option<LogGuard>>,
}

fn open_issues(paths: &Paths) -> Arc<IssueStore> {
    match IssueStore::open(paths) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("wa-desk: issue store unavailable ({e}); using memory");
            // An in-memory SQLite database cannot fail to open short of OOM.
            Arc::new(IssueStore::in_memory().expect("in-memory issue store"))
        }
    }
}

fn setup(
    app: &mut tauri::App<Wry>,
    paths: Paths,
    issues: Arc<IssueStore>,
    start_hidden_arg: bool,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let bridge = Arc::new(wa_bridge::BridgeClient::new(&paths)?);
    let deps = CoreDeps {
        paths,
        bridge,
        issues,
        emitter: Arc::new(TauriEmitter {
            app: handle.clone(),
        }),
        platform: Arc::new(TauriPlatform {
            app: handle.clone(),
        }),
    };
    let t0 = std::time::Instant::now();
    let core = tauri::async_runtime::block_on(AppCore::new(deps)).inspect_err(|e| {
        tracing::error!(error = %e, "startup failed");
    })?;
    if std::env::var("WA_DESK_READ_ONLY").is_ok_and(|v| v == "1") {
        tracing::warn!("read-only mode: sends, reactions, receipts and automations are disabled");
        core.set_read_only(true);
    }
    app.manage(core.clone());
    let token = app.state::<Shutdown>().token.clone();
    let bg = core.clone();
    tauri::async_runtime::spawn(async move { background::spawn_all(bg, token) });
    if let Err(e) = tray::build(&handle, core.clone()) {
        tracing::warn!(error = %e, "tray icon unavailable");
    }
    let settings = tauri::async_runtime::block_on(core.get_settings()).unwrap_or_default();
    {
        use tauri_plugin_autostart::ManagerExt;
        let al = handle.autolaunch();
        if al.is_enabled().ok() != Some(settings.autostart) {
            let r = if settings.autostart {
                al.enable()
            } else {
                al.disable()
            };
            if let Err(e) = r {
                tracing::warn!(error = %e, "could not sync autostart");
            }
        }
    }
    let hidden = start_hidden_arg || settings.start_hidden;
    if !hidden {
        tray::show_main(&handle);
    }
    tracing::info!(
        hidden,
        ms = t0.elapsed().as_millis() as u64,
        "startup complete"
    );
    Ok(())
}

/// Entry point.
pub fn run() {
    apply_platform_env();
    let paths = Paths::from_env();
    if let Err(e) = paths.ensure_own_dirs() {
        eprintln!("wa-desk: cannot create data folders: {e}");
    }
    let issues = open_issues(&paths);
    let log = match wa_obs::init_logging_with_issues(&paths, None, Some(issues.clone())) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("wa-desk: logging unavailable: {e}");
            None
        }
    };
    wa_obs::install_panic_hook(issues.clone());
    let hidden_arg = std::env::args().any(|a| a == HIDDEN_ARG);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_root = std::env::var_os("WA_DESK_DATA_ROOT").is_some(),
        hidden_arg,
        "wa-desk starting"
    );

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tracing::info!("second instance started; focusing the existing window");
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Shutdown {
            token: CancellationToken::new(),
            log: Mutex::new(log),
        })
        .register_asynchronous_uri_scheme_protocol("wa-media", |ctx, req, responder| {
            let core = ctx
                .app_handle()
                .try_state::<Arc<AppCore>>()
                .map(|s| s.inner().clone());
            tauri::async_runtime::spawn(async move {
                let resp = match core {
                    Some(c) => protocol::handle(&c, &req).await,
                    None => tauri::http::Response::builder()
                        .status(503)
                        .header("Content-Type", "text/plain")
                        .body(b"starting".to_vec())
                        .unwrap_or_default(),
                };
                responder.respond(resp);
            });
        })
        .on_window_event(|w, ev| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = ev
                && w.label() == tray::MAIN
            {
                api.prevent_close();
                let _ = w.hide();
                tracing::debug!("main window hidden to tray");
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::overview,
            commands::list_chats,
            commands::get_chat,
            commands::get_messages,
            commands::get_message_context,
            commands::search,
            commands::list_media,
            commands::ensure_media,
            commands::preview_doc,
            commands::open_media_external,
            commands::save_media_as,
            commands::open_path,
            commands::reveal_path,
            commands::send_message,
            commands::react,
            commands::list_calls,
            commands::reject_call,
            commands::mark_read,
            commands::connection_status,
            commands::reconnect,
            commands::start_job,
            commands::cancel_job,
            commands::list_jobs,
            commands::list_rules,
            commands::save_rule,
            commands::delete_rule,
            commands::test_rule,
            commands::automation_log,
            commands::set_automation_enabled,
            commands::query_logs,
            commands::list_issues,
            commands::resolve_issue,
            commands::clear_resolved_issues,
            commands::export_issue_bundle,
            commands::get_settings,
            commands::set_settings,
            commands::get_profile,
            commands::chat_stats,
            commands::list_links,
            commands::set_starred,
            commands::list_starred,
            commands::export_chat,
            commands::forward_messages,
            commands::copy_messages_text,
            commands::refresh_chat_names,
            commands::archive_count,
            commands::set_archived,
            commands::set_pinned,
            commands::set_muted,
            commands::resync_chat_state,
        ]);
    let builder = builder.setup(move |app| setup(app, paths, issues, hidden_arg));
    let app = match builder.build(tauri::generate_context!()) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "could not start the app window");
            eprintln!("wa-desk: {e}");
            std::process::exit(1);
        }
    };
    app.run(|app, ev| {
        if let RunEvent::Exit = ev {
            tracing::info!("wa-desk exiting");
            if let Some(core) = app.try_state::<Arc<AppCore>>() {
                core.shutdown();
            }
            let sd = app.state::<Shutdown>();
            sd.token.cancel();
            sd.log.lock().take();
        }
    });
}
