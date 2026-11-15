//! Where everything lives. Defaults match the live install; `WA_DESK_DATA_ROOT`
//! relocates *all* paths under one directory (tests, copies of real DBs).

use std::path::{Path, PathBuf};

/// Bridge REST base (no trailing slash). Host header must be exactly this host:port.
pub const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Bridge store dir (holds messages.db, whatsapp.db, .bridge-token, media).
    pub bridge_store: PathBuf,
    pub messages_db: PathBuf,
    pub whatsapp_db: PathBuf,
    pub token_file: PathBuf,
    /// Media copied from the phone (WhatsApp/Media tree).
    pub phone_media: PathBuf,
    /// Only directory the bridge accepts `media_path` from.
    pub outbox: PathBuf,
    /// Skip list written by the backfill job.
    pub backfill_skip: PathBuf,
    /// wa-desk own state: app.db, settings.
    pub app_data: PathBuf,
    pub app_db: PathBuf,
    /// Thumbnails, converted previews, LibreOffice profile.
    pub cache: PathBuf,
    pub logs: PathBuf,
    /// Default output folder for conversions and issue bundles.
    pub exports: PathBuf,
    pub bridge_url: String,
}

impl Paths {
    /// Resolve from environment: `WA_DESK_DATA_ROOT` (all under one root) or user defaults.
    pub fn from_env() -> Self {
        let bridge_url =
            std::env::var("WA_DESK_BRIDGE_URL").unwrap_or_else(|_| DEFAULT_BRIDGE_URL.to_string());
        match std::env::var_os("WA_DESK_DATA_ROOT") {
            Some(root) if !root.is_empty() => Self::under_root(Path::new(&root), bridge_url),
            _ => Self::user_defaults(bridge_url),
        }
    }

    /// Everything beneath `root` (used by tests and `WA_DESK_DATA_ROOT`).
    pub fn under_root(root: &Path, bridge_url: String) -> Self {
        let wa = root.join("whatsapp-mcp");
        let app = root.join("wa-desk");
        Self::build(
            wa,
            app.join("data"),
            app.join("cache"),
            app.join("logs"),
            root.join("exports"),
            bridge_url,
        )
    }

    fn user_defaults(bridge_url: String) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        let cache = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
        let state = dirs::state_dir().unwrap_or_else(|| home.join(".local/state"));
        let downloads = dirs::download_dir().unwrap_or_else(|| home.join("Downloads"));
        Self::build(
            data.join("whatsapp-mcp"),
            data.join("wa-desk"),
            cache.join("wa-desk"),
            state.join("wa-desk/logs"),
            downloads.join("wa-desk"),
            bridge_url,
        )
    }

    fn build(
        wa: PathBuf,
        app_data: PathBuf,
        cache: PathBuf,
        logs: PathBuf,
        exports: PathBuf,
        bridge_url: String,
    ) -> Self {
        let store = wa.join("runtime/store");
        Self {
            messages_db: store.join("messages.db"),
            whatsapp_db: store.join("whatsapp.db"),
            token_file: store.join(".bridge-token"),
            backfill_skip: wa.join("runtime/download-skip.txt"),
            bridge_store: store,
            phone_media: wa.join("phone-media"),
            outbox: wa.join("outbox"),
            app_db: app_data.join("app.db"),
            app_data,
            cache,
            logs,
            exports,
            bridge_url,
        }
    }

    /// Roots whose files may be served to the UI (secrets/DBs are still refused).
    pub fn media_roots(&self) -> Vec<PathBuf> {
        vec![
            self.bridge_store.clone(),
            self.phone_media.clone(),
            self.outbox.clone(),
            self.cache.clone(),
            self.exports.clone(),
        ]
    }

    /// Create the directories wa-desk owns (never the bridge's).
    pub fn ensure_own_dirs(&self) -> std::io::Result<()> {
        for d in [&self.app_data, &self.cache, &self.logs, &self.exports] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_root_layout() {
        let p = Paths::under_root(Path::new("/r"), DEFAULT_BRIDGE_URL.into());
        assert_eq!(
            p.messages_db,
            PathBuf::from("/r/whatsapp-mcp/runtime/store/messages.db")
        );
        assert_eq!(
            p.token_file,
            PathBuf::from("/r/whatsapp-mcp/runtime/store/.bridge-token")
        );
        assert_eq!(p.outbox, PathBuf::from("/r/whatsapp-mcp/outbox"));
        assert_eq!(p.app_db, PathBuf::from("/r/wa-desk/data/app.db"));
        assert!(p.media_roots().contains(&p.phone_media));
    }

    #[test]
    fn user_defaults_point_at_live_install() {
        let p = Paths::user_defaults(DEFAULT_BRIDGE_URL.into());
        assert!(
            p.messages_db
                .ends_with("whatsapp-mcp/runtime/store/messages.db")
        );
        assert!(p.app_db.ends_with("wa-desk/app.db"));
    }
}
