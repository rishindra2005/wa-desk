//! Small filesystem helpers shared by the jobs.

use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use sha2::{Digest, Sha256};
use wa_core::paths::SECRET_NAMES;
use wa_core::{AppError, Result};

/// `wa-media://localhost/x/<urlenc absolute path>` for an output file.
pub fn media_url(path: &Path) -> String {
    let p = path.to_string_lossy();
    format!(
        "wa-media://localhost/x/{}",
        utf8_percent_encode(&p, NON_ALPHANUMERIC)
    )
}

/// Streaming SHA-256 of a file (1 MiB buffer).
pub fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// Hex SHA-256 of a string (cache keys).
pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Run blocking work on Tokio's blocking pool.
pub async fn blocking<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal("background task failed").with_detail(e.to_string()))?
}

/// True for the bridge token and SQLite files.
pub fn is_secret_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    SECRET_NAMES.contains(&name)
        || n.ends_with(".db")
        || n.contains(".db-")
        || n.ends_with(".sqlite")
        || n.ends_with(".sqlite3")
}

/// Canonicalise an absolute path that must be an existing regular, non-secret file.
pub fn canonical_input_file(p: &Path) -> Result<PathBuf> {
    if !p.is_absolute() {
        return Err(AppError::invalid("input path must be absolute"));
    }
    let real = fs::canonicalize(p).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => AppError::not_found("input file does not exist"),
        _ => AppError::from(e),
    })?;
    if !fs::metadata(&real)?.is_file() {
        return Err(AppError::invalid("input must be a regular file"));
    }
    let name = real.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if is_secret_name(name) {
        return Err(AppError::forbidden("refusing to read a secret or database file"));
    }
    Ok(real)
}

/// Candidate output names: `<base>.<ext>`, `<base> (1).<ext>`, `<base> (2).<ext>`, …
fn candidate(dir: &Path, base: &str, ext: &str, n: u32) -> PathBuf {
    let name = match (n, ext.is_empty()) {
        (0, true) => base.to_string(),
        (0, false) => format!("{base}.{ext}"),
        (n, true) => format!("{base} ({n})"),
        (n, false) => format!("{base} ({n}).{ext}"),
    };
    dir.join(name)
}

/// Output base name `<stem>-<suffix>` (or just `<stem>` with an empty suffix).
pub fn output_base(stem: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}-{suffix}")
    }
}

/// Move/copy `tmp` into `dir` under a unique name; never overwrites.
pub fn place_unique(tmp: &Path, dir: &Path, base: &str, ext: &str) -> Result<PathBuf> {
    for n in 0..10_000 {
        let cand = candidate(dir, base, ext, n);
        match fs::hard_link(tmp, &cand) {
            Ok(()) => return Ok(cand),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => {}
        }
        // Different filesystem (or no hard links): exclusive-create copy.
        match OpenOptions::new().write(true).create_new(true).open(&cand) {
            Ok(mut out) => {
                let mut src = fs::File::open(tmp)?;
                if let Err(e) = io::copy(&mut src, &mut out) {
                    let _ = fs::remove_file(&cand);
                    return Err(e.into());
                }
                return Ok(cand);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(AppError::internal("could not find a free output file name"))
}

/// Temporary working directory under `cache/tmp`, removed on drop.
pub struct WorkDir(PathBuf);

impl WorkDir {
    pub fn new(cache: &Path) -> Result<Self> {
        let p = cache.join("tmp").join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&p)?;
        Ok(Self(p))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `(mtime nanos, size)` identity used in cache keys.
pub fn file_identity(p: &Path) -> Result<(i128, u64)> {
    let m = fs::metadata(p)?;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    Ok((mtime, m.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_never_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let tmp = d.path().join("t");
        fs::write(&tmp, b"new").unwrap();
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("a-compressed.jpg"), b"old").unwrap();
        let p1 = place_unique(&tmp, &out, "a-compressed", "jpg").unwrap();
        assert_eq!(p1.file_name().unwrap(), "a-compressed (1).jpg");
        let p2 = place_unique(&tmp, &out, "a-compressed", "jpg").unwrap();
        assert_eq!(p2.file_name().unwrap(), "a-compressed (2).jpg");
        assert_eq!(fs::read(out.join("a-compressed.jpg")).unwrap(), b"old");
        assert_eq!(fs::read(p2).unwrap(), b"new");
        assert_eq!(output_base("x", ""), "x");
        assert_eq!(output_base("x", "y"), "x-y");
    }

    #[test]
    fn media_url_encodes() {
        assert_eq!(
            media_url(Path::new("/a b/ü.pdf")),
            "wa-media://localhost/x/%2Fa%20b%2F%C3%BC%2Epdf"
        );
    }

    #[test]
    fn input_file_checks() {
        let d = tempfile::tempdir().unwrap();
        assert!(canonical_input_file(Path::new("rel")).is_err());
        assert!(canonical_input_file(d.path()).is_err());
        fs::write(d.path().join("app.db"), b"x").unwrap();
        assert!(canonical_input_file(&d.path().join("app.db")).is_err());
        fs::write(d.path().join("ok.txt"), b"x").unwrap();
        assert!(canonical_input_file(&d.path().join("ok.txt")).is_ok());
        assert_eq!(
            sha256_file(&d.path().join("ok.txt")).unwrap()[..2],
            [0x2d, 0x71]
        );
    }
}
