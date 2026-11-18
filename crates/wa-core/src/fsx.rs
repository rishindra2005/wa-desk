//! Owner-only file and directory helpers that work on every platform.
//!
//! On Unix, directories are created `0700` and files `0600`, as the app's media,
//! outbox and report files may hold private messages. On Windows, files under the
//! user's profile already inherit an ACL that limits them to that user, so these
//! helpers create them normally.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

/// Create `dir` (and, with `recursive`, its parents) as an owner-only directory.
pub fn create_private_dir(dir: &Path, recursive: bool) -> io::Result<()> {
    let mut b = DirBuilder::new();
    b.recursive(recursive);
    #[cfg(unix)]
    b.mode(0o700);
    b.create(dir)
}

/// Options that create a new owner-only file and fail if it already exists.
pub fn private_create_new() -> OpenOptions {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    o.mode(0o600);
    o
}

/// Open (create or truncate) an owner-only file for writing.
pub fn private_write(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    o.mode(0o600);
    o.open(path)
}

/// Restrict an existing file to its owner (`0600` on Unix; no-op elsewhere).
pub fn make_private_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Restrict an existing directory to its owner (`0700` on Unix; no-op elsewhere).
pub fn make_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// The Unix permission bits of `path` (`None` on other platforms).
pub fn unix_mode(path: &Path) -> io::Result<Option<u32>> {
    #[cfg(unix)]
    {
        Ok(Some(fs::metadata(path)?.permissions().mode() & 0o777))
    }
    #[cfg(not(unix))]
    {
        let _ = fs::metadata(path)?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn creates_private_dir_and_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("a/b");
        create_private_dir(&dir, true).unwrap();
        let f = dir.join("x.txt");
        private_create_new()
            .open(&f)
            .unwrap()
            .write_all(b"hi")
            .unwrap();
        assert!(
            private_create_new().open(&f).is_err(),
            "create_new must not overwrite"
        );
        private_write(&f).unwrap().write_all(b"yo").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"yo");
        make_private_file(&f).unwrap();
        make_private_dir(&dir).unwrap();
        #[cfg(unix)]
        {
            assert_eq!(unix_mode(&dir).unwrap(), Some(0o700));
            assert_eq!(unix_mode(&f).unwrap(), Some(0o600));
        }
        #[cfg(not(unix))]
        assert_eq!(unix_mode(&f).unwrap(), None);
    }
}
