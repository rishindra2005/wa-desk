//! wa-desk Tauri shell. Integration (commands, protocol, tray, watcher) is
//! owned by the integrator; see AGENTS.md.

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

pub fn run() {
    apply_platform_env();
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running wa-desk");
}
