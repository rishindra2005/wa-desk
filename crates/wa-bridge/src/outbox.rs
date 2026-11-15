//! Staging of attachments into the bridge outbox.
//!
//! The bridge only accepts `media_path` values that are regular files under
//! the outbox. `stage` validates a user-chosen file and copies it to
//! `outbox/<uuid>/<sanitized name>` (dir 0700, file 0600). Blocking: call it
//! from `spawn_blocking` in async code.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use wa_core::config::Paths;
use wa_core::paths::{SECRET_NAMES, sanitize_file_name};
use wa_core::{AppError, Result};

/// Largest attachment we stage (WhatsApp's document limit is 2 GiB).
pub const MAX_STAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn looks_like_db(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".db")
        || n.contains(".db-")
        || n.ends_with(".sqlite")
        || n.ends_with(".sqlite3")
}

/// Validate `src` and copy it into a fresh outbox subdirectory. Returns the staged path.
pub fn stage(paths: &Paths, src: &Path) -> Result<PathBuf> {
    stage_with_limit(paths, src, MAX_STAGE_BYTES)
}

/// `stage` with an explicit size limit (tests).
pub fn stage_with_limit(paths: &Paths, src: &Path, max_bytes: u64) -> Result<PathBuf> {
    if !src.is_absolute() {
        return Err(AppError::invalid("attachment path must be absolute"));
    }
    let real = fs::canonicalize(src).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => AppError::not_found("attachment file does not exist"),
        _ => AppError::from(e),
    })?;
    let meta = fs::metadata(&real)?;
    if !meta.file_type().is_file() {
        return Err(AppError::invalid(
            "attachment must be a regular file (not a folder, device or pipe)",
        ));
    }
    if meta.len() > max_bytes {
        return Err(AppError::invalid("attachment is larger than 2 GiB"));
    }
    let orig_name = real
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if SECRET_NAMES.contains(&orig_name.as_str()) || looks_like_db(&orig_name) {
        return Err(AppError::forbidden(
            "refusing to send a secret or database file",
        ));
    }
    let name = sanitize_file_name(
        &src.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(orig_name),
    );
    ensure_dir(&paths.outbox)?;
    let dir = paths.outbox.join(uuid::Uuid::new_v4().to_string());
    DirBuilder::new().mode(0o700).create(&dir)?;
    let dest = dir.join(&name);
    if let Err(e) = copy_private(&real, &dest) {
        let _ = fs::remove_dir_all(&dir);
        return Err(e);
    }
    tracing::debug!(size = meta.len(), "staged attachment in outbox");
    Ok(fs::canonicalize(&dest).unwrap_or(dest))
}

fn ensure_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        DirBuilder::new().mode(0o700).recursive(true).create(dir)?;
    }
    Ok(())
}

fn copy_private(src: &Path, dest: &Path) -> Result<()> {
    let mut input = fs::File::open(src)?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dest)?;
    io::copy(&mut input, &mut out)?;
    out.sync_all()?;
    // The umask may have narrowed the mode further; make it exactly 0600.
    fs::set_permissions(dest, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Remove staged attachment folders older than `age`. Only `<uuid>` folders
/// created by `stage` are touched. Returns how many were removed.
pub fn cleanup_older_than(paths: &Paths, age: Duration) -> Result<usize> {
    let entries = match fs::read_dir(&paths.outbox) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let cutoff = SystemTime::now()
        .checked_sub(age)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if uuid::Uuid::parse_str(name).is_err() {
            continue;
        }
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::now());
        if modified <= cutoff && fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use wa_core::ErrorKind;

    fn setup() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::under_root(d.path(), "http://127.0.0.1:8080".into());
        (d, p)
    }

    #[test]
    fn stages_with_private_perms() {
        let (d, p) = setup();
        let src = d.path().join("Report (final).pdf");
        fs::write(&src, b"%PDF-1.4").unwrap();
        let out = stage(&p, &src).unwrap();
        assert!(out.starts_with(fs::canonicalize(&p.outbox).unwrap()));
        assert_eq!(out.file_name().unwrap(), "Report (final).pdf");
        assert_eq!(fs::read(&out).unwrap(), b"%PDF-1.4");
        let fmode = fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        let dmode = fs::metadata(out.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!((fmode, dmode), (0o600, 0o700));
        // Two stagings of the same file never collide.
        let out2 = stage(&p, &src).unwrap();
        assert_ne!(out, out2);
    }

    #[test]
    fn sanitizes_names() {
        let (d, p) = setup();
        let src = d.path().join("a:b*c?.txt");
        fs::write(&src, b"x").unwrap();
        let out = stage(&p, &src).unwrap();
        assert_eq!(out.file_name().unwrap(), "a_b_c_.txt");
        let src = d.path().join("...");
        fs::write(&src, b"x").unwrap();
        assert_eq!(stage(&p, &src).unwrap().file_name().unwrap(), "file");
    }

    #[test]
    fn rejects_bad_inputs() {
        let (d, p) = setup();
        let k = |path: &Path| stage(&p, path).unwrap_err().kind;
        assert_eq!(k(Path::new("relative.txt")), ErrorKind::Invalid);
        assert_eq!(k(&d.path().join("missing.txt")), ErrorKind::NotFound);
        assert_eq!(k(d.path()), ErrorKind::Invalid, "directory");
        let sub = d.path().join("sub");
        fs::create_dir(&sub).unwrap();
        symlink(&sub, d.path().join("dirlink")).unwrap();
        assert_eq!(k(&d.path().join("dirlink")), ErrorKind::Invalid, "symlink to dir");
        assert_eq!(k(Path::new("/dev/null")), ErrorKind::Invalid, "device");
        let fifo = d.path().join("pipe");
        let st = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(st.success());
        assert_eq!(k(&fifo), ErrorKind::Invalid, "fifo");
        fs::write(d.path().join(".bridge-token"), b"t").unwrap();
        assert_eq!(k(&d.path().join(".bridge-token")), ErrorKind::Forbidden);
        fs::write(d.path().join("messages.db"), b"t").unwrap();
        assert_eq!(k(&d.path().join("messages.db")), ErrorKind::Forbidden);
        // Nothing was left behind by the failures.
        let left = fs::read_dir(&p.outbox).map(|r| r.count()).unwrap_or(0);
        assert_eq!(left, 0);
    }

    #[test]
    fn symlink_to_file_is_resolved() {
        let (d, p) = setup();
        let real = d.path().join("real.txt");
        fs::write(&real, b"data").unwrap();
        symlink(&real, d.path().join("link.txt")).unwrap();
        let out = stage(&p, &d.path().join("link.txt")).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"data");
        assert!(!out.symlink_metadata().unwrap().file_type().is_symlink());
    }

    #[test]
    fn rejects_oversize() {
        let (d, p) = setup();
        let src = d.path().join("big.bin");
        fs::write(&src, vec![0u8; 100]).unwrap();
        assert_eq!(
            stage_with_limit(&p, &src, 99).unwrap_err().kind,
            ErrorKind::Invalid
        );
        assert!(stage_with_limit(&p, &src, 100).is_ok());
    }

    #[test]
    fn cleanup_removes_only_old_uuid_dirs() {
        let (d, p) = setup();
        let src = d.path().join("x.txt");
        fs::write(&src, b"x").unwrap();
        let staged = stage(&p, &src).unwrap();
        fs::create_dir_all(p.outbox.join("keep-me")).unwrap();
        assert_eq!(cleanup_older_than(&p, Duration::from_secs(3600)).unwrap(), 0);
        assert!(staged.exists());
        assert_eq!(cleanup_older_than(&p, Duration::ZERO).unwrap(), 1);
        assert!(!staged.exists());
        assert!(p.outbox.join("keep-me").exists());
        let (_d2, empty) = setup();
        assert_eq!(cleanup_older_than(&empty, Duration::ZERO).unwrap(), 0);
    }
}
