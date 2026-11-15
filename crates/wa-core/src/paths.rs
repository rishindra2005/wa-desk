//! Path rules shared by every crate that touches files.
//!
//! `bridge_media_path` reproduces the Go bridge's `mediaDownloadStorePaths`
//! (whatsapp-bridge/main.go) exactly, so files we place are found by the
//! bridge and vice versa. `resolve_within` is the one gate for any path that
//! is served to the webview or handed to an external tool.

use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, Result};
use crate::time::store_name_stamp;

/// File names that must never be served or copied out of the store.
pub const SECRET_NAMES: &[&str] = &[".bridge-token"];

fn valid_component(v: &str) -> bool {
    !v.is_empty()
        && v != "."
        && v != ".."
        && !v.contains('/')
        && !v.contains('\\')
        && !v.contains('\0')
}

/// Extension the bridge uses for each media type (documents keep none).
pub fn media_ext(media_type: &str) -> &'static str {
    match media_type {
        "image" => ".jpg",
        "video" => ".mp4",
        "audio" => ".ogg",
        "sticker" => ".webp",
        _ => "",
    }
}

/// `store/<chat_jid with ':'→'_'>/<type>_<YYYYMMDD_HHMMSS>_<msgid><ext>`.
pub fn bridge_media_path(
    store_root: &Path,
    chat_jid: &str,
    media_type: &str,
    message_id: &str,
    raw_ts: &str,
) -> Result<PathBuf> {
    let chat = chat_jid.replace(':', "_");
    for (name, v) in [
        ("chat JID", chat.as_str()),
        ("message ID", message_id),
        ("media type", media_type),
    ] {
        if !valid_component(v) {
            return Err(AppError::invalid(format!(
                "invalid {name} for media store path"
            )));
        }
    }
    let stamp = store_name_stamp(raw_ts)
        .ok_or_else(|| AppError::invalid("unparseable message timestamp"))?;
    let file = format!("{media_type}_{stamp}_{message_id}{}", media_ext(media_type));
    Ok(store_root.join(chat).join(file))
}

/// Lexically normalise without touching the filesystem (removes `.`/`..`).
fn lexical_normalize(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// Canonicalise `path` (following symlinks) and require it to be inside one
/// of `roots` and not a secret/database file. Use for everything served to
/// the UI or passed to external tools.
pub fn resolve_within(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(AppError::forbidden("path must be absolute"));
    }
    if lexical_normalize(path).is_none() {
        return Err(AppError::forbidden("path escapes root"));
    }
    let real = std::fs::canonicalize(path).map_err(AppError::from)?;
    let name = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if SECRET_NAMES.contains(&name) || is_db_file(name) {
        return Err(AppError::forbidden(
            "refusing to expose a secret or database file",
        ));
    }
    let inside = roots.iter().any(|r| match std::fs::canonicalize(r) {
        Ok(root) => real.starts_with(&root) && real != root,
        Err(_) => false,
    });
    if !inside {
        return Err(AppError::forbidden("path is outside the allowed folders"));
    }
    Ok(real)
}

fn is_db_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".db")
        || n.contains(".db-")
        || n.contains(".db.")
        || n.ends_with(".sqlite")
        || n.ends_with(".sqlite3")
}

/// Make a user/remote supplied file name safe as a single path component.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.chars().take(180).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn matches_bridge_layout() {
        let p = bridge_media_path(
            Path::new("/s"),
            "120363@g.us",
            "document",
            "3EB0ABC",
            "2026-11-15 20:31:02+05:30",
        )
        .unwrap();
        assert_eq!(
            p,
            PathBuf::from("/s/120363@g.us/document_20261115_203102_3EB0ABC")
        );
        let p = bridge_media_path(
            Path::new("/s"),
            "91999:12@s.whatsapp.net",
            "image",
            "AC12",
            "2026-01-02 03:04:05+05:30",
        )
        .unwrap();
        assert_eq!(
            p,
            PathBuf::from("/s/91999_12@s.whatsapp.net/image_20260102_030405_AC12.jpg")
        );
        for (t, e) in [
            ("video", ".mp4"),
            ("audio", ".ogg"),
            ("sticker", ".webp"),
            ("document", ""),
        ] {
            assert!(
                bridge_media_path(Path::new("/s"), "a@b", t, "X", "2026-01-01 00:00:00")
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(&format!("_X{e}"))
            );
        }
    }

    #[test]
    fn rejects_traversal_components() {
        for (chat, id, ty) in [
            ("..", "X", "image"),
            ("a@b", "../x", "image"),
            ("a@b", "X", ""),
            ("a/b", "X", "image"),
            ("a@b", "a\\b", "image"),
            ("a@b", ".", "image"),
        ] {
            assert!(
                bridge_media_path(Path::new("/s"), chat, ty, id, "2026-01-01 00:00:00").is_err(),
                "{chat} {id} {ty}"
            );
        }
        assert!(bridge_media_path(Path::new("/s"), "a@b", "image", "X", "not a time").is_err());
    }

    #[test]
    fn resolve_within_blocks_escapes_and_secrets() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let r = root.path().to_path_buf();
        fs::create_dir(r.join("chat")).unwrap();
        fs::write(r.join("chat/ok.jpg"), b"x").unwrap();
        fs::write(r.join(".bridge-token"), b"secret").unwrap();
        fs::write(r.join("messages.db"), b"x").unwrap();
        fs::write(r.join("messages.db-journal"), b"x").unwrap();
        fs::write(outside.path().join("evil"), b"x").unwrap();
        std::os::unix::fs::symlink(outside.path().join("evil"), r.join("chat/link")).unwrap();
        let roots = vec![r.clone()];

        assert!(resolve_within(&r.join("chat/ok.jpg"), &roots).is_ok());
        assert_eq!(
            resolve_within(&r.join(".bridge-token"), &roots)
                .unwrap_err()
                .kind,
            crate::ErrorKind::Forbidden
        );
        assert!(resolve_within(&r.join("messages.db"), &roots).is_err());
        assert!(resolve_within(&r.join("messages.db-journal"), &roots).is_err());
        assert!(
            resolve_within(&r.join("chat/link"), &roots).is_err(),
            "symlink escape"
        );
        assert!(resolve_within(&r.join("chat/../../etc/passwd"), &roots).is_err());
        assert!(resolve_within(Path::new("relative.jpg"), &roots).is_err());
        assert!(
            resolve_within(&r, &roots).is_err(),
            "root itself is not a file to serve"
        );
        assert_eq!(
            resolve_within(&r.join("chat/missing.jpg"), &roots)
                .unwrap_err()
                .kind,
            crate::ErrorKind::NotFound
        );
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("a:b*c?.pdf"), "a_b_c_.pdf");
        assert_eq!(sanitize_file_name("..."), "file");
        assert_eq!(sanitize_file_name(""), "file");
        assert_eq!(
            sanitize_file_name("C:\\x\\Report (1).docx"),
            "Report (1).docx"
        );
        assert_eq!(sanitize_file_name(&"a".repeat(500)).len(), 180);
    }
}
