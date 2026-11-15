//! System tray: Show/Hide, "Pause automations", Quit. Closing the window
//! only hides it (see `lib.rs`), so automations keep running in the tray.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::core::AppCore;

/// Label of the main window.
pub const MAIN: &str = "main";

/// Show + focus the main window.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn toggle_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(MAIN) {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            show_main(app);
        }
    }
}

/// Build the tray icon and its menu.
pub fn build<R: Runtime>(app: &AppHandle<R>, core: Arc<AppCore>) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide wa-desk", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(
        app,
        "pause",
        "Pause automations",
        true,
        core.automation_paused(),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit wa-desk", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &pause, &sep, &quit])?;
    let pause_item = pause.clone();
    let mut b = TrayIconBuilder::with_id("wa-desk")
        .tooltip("wa-desk")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, ev| match ev.id().as_ref() {
            "toggle" => toggle_main(app),
            "pause" => {
                let paused = pause_item.is_checked().unwrap_or(false);
                let core = core.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = core.set_automation_paused(paused).await {
                        tracing::warn!(error = %e, "could not toggle automation pause");
                    }
                });
            }
            "quit" => {
                tracing::info!("quit requested from tray");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        b = b.icon(icon);
    }
    b.build(app)?;
    Ok(())
}
