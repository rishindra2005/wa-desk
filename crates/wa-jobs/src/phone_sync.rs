//! Copy `WhatsApp/Media` from a USB (MTP, via GVFS) phone into `phone_media`
//! with rsync. Never deletes anything at the destination.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use wa_core::traits::ProgressSink;
use wa_core::{AppError, ErrorKind, Result};

use crate::manager::{JobCtl, JobDeps};
use crate::tools::Tool;
use crate::util::blocking;

/// Media sub-folders that are never copied.
pub const EXCLUDED: &[&str] = &[
    "Stickers",
    "Backup Excluded Stickers",
    "Bug Report Attachments",
    "Profile Photos",
    "WallPaper",
    "Sticker Packs",
];
const MEDIA_REL: &str = "Android/media/com.whatsapp/WhatsApp/Media";
const ATTEMPTS: u32 = 3;
const FOLDER_TIMEOUT: Duration = Duration::from_secs(6 * 3600);

/// A phone's WhatsApp Media folder and the GVFS mount it lives on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneMedia {
    pub mount: PathBuf,
    pub media: PathBuf,
}

/// Find `<root>/mtp:host=*/<storage>/Android/media/com.whatsapp/WhatsApp/Media`.
pub fn discover(root: &Path) -> Vec<PhoneMedia> {
    let Ok(mounts) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut mounts: Vec<PathBuf> = mounts
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("mtp:host="))
        .map(|e| e.path())
        .collect();
    mounts.sort();
    for mount in mounts {
        let Ok(storages) = std::fs::read_dir(&mount) else {
            continue;
        };
        let mut storages: Vec<PathBuf> = storages.flatten().map(|e| e.path()).collect();
        storages.sort();
        for s in storages {
            let media = s.join(MEDIA_REL);
            if media.is_dir() {
                found.push(PhoneMedia {
                    mount: mount.clone(),
                    media,
                });
            }
        }
    }
    found
}

/// Parse an rsync `--info=progress2` line: `(bytes, percent)`.
pub fn parse_progress2(line: &str) -> Option<(u64, u8)> {
    let mut it = line.split_whitespace();
    let bytes: u64 = it.next()?.replace(',', "").parse().ok()?;
    let pct: u8 = it.next()?.strip_suffix('%')?.parse().ok()?;
    Some((bytes, pct.min(100)))
}

fn folders(media: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(media)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !EXCLUDED.contains(&n.as_str()) && !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn disconnected() -> AppError {
    AppError::external("Phone disconnected")
}

/// Run the sync. Returns the summary message.
pub async fn run(deps: &JobDeps, ctl: &Arc<JobCtl>) -> Result<String> {
    ctl.phase("looking for phone");
    let root = deps.phone_root.clone();
    let phones = blocking(move || Ok(discover(&root))).await?;
    let Some(phone) = phones.into_iter().next() else {
        return Err(AppError::external(
            "No phone connected over USB (enable File transfer)",
        ));
    };
    let media = phone.media.clone();
    let names = blocking(move || Ok(folders(&media))).await?;
    std::fs::create_dir_all(&deps.paths.phone_media)?;
    let n = names.len() as u64;
    let mut failed: Vec<String> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        ctl.check()?;
        let base = i as u64 * 100;
        ctl.progress(base, n * 100, name, None);
        let src = phone.media.join(name);
        let dest = deps.paths.phone_media.join(name);
        std::fs::create_dir_all(&dest)?;
        let mut last_err = None;
        for attempt in 0..ATTEMPTS {
            if !phone.mount.exists() {
                return Err(disconnected());
            }
            match sync_folder(deps, &src, &dest, name, base, n, ctl).await {
                Ok(()) => {
                    last_err = None;
                    break;
                }
                Err(e)
                    if e.kind == ErrorKind::Cancelled
                        || e.message.ends_with("is not installed") =>
                {
                    return Err(e);
                }
                Err(e) => {
                    if !phone.mount.exists() {
                        return Err(disconnected());
                    }
                    tracing::warn!(folder = %name, attempt, error = %e.message, "rsync attempt failed");
                    last_err = Some(e);
                }
            }
        }
        if last_err.is_some() {
            failed.push(name.clone());
        }
        ctl.progress(base + 100, n * 100, name, None);
    }
    if !failed.is_empty() {
        return Err(AppError::external(format!(
            "Could not copy {} folder(s): {}",
            failed.len(),
            failed.join(", ")
        )));
    }
    Ok(format!("Copied {n} folder(s) from the phone"))
}

async fn sync_folder(
    deps: &JobDeps,
    src: &Path,
    dest: &Path,
    name: &str,
    base: u64,
    n: u64,
    ctl: &Arc<JobCtl>,
) -> Result<()> {
    // Absolute paths start with '/', so rsync never mistakes "mtp:host=" for a remote host.
    let mut src_arg = src.as_os_str().to_os_string();
    src_arg.push("/");
    let mut dest_arg = dest.as_os_str().to_os_string();
    dest_arg.push("/");
    let ctl2 = ctl.clone();
    let name2 = name.to_string();
    let mut on_line = move |l: &str| {
        if let Some((bytes, pct)) = parse_progress2(l) {
            let mb = bytes / (1024 * 1024);
            ctl2.progress(
                base + u64::from(pct),
                n * 100,
                &name2,
                Some(&format!("{mb} MB copied")),
            );
        }
    };
    Tool::new("rsync", &deps.tools.rsync)
        .args([
            "-rt",
            "--no-perms",
            "--partial",
            "--timeout=60",
            "--info=progress2",
            "--",
        ])
        .arg(src_arg)
        .arg(dest_arg)
        .timeout(FOLDER_TIMEOUT)
        .ok_code(24) // some files vanished during transfer: fine
        .run(ctl.cancel_token(), Some(&mut on_line))
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress2() {
        assert_eq!(
            parse_progress2("    1,234,567  45%   1.23MB/s    0:00:12 (xfr#3, to-chk=10/20)"),
            Some((1_234_567, 45))
        );
        assert_eq!(parse_progress2("0 100% 0.00kB/s 0:00:00"), Some((0, 100)));
        assert_eq!(parse_progress2("sending incremental file list"), None);
        assert_eq!(parse_progress2(""), None);
    }

    #[test]
    fn discovers_any_storage_name() {
        let d = tempfile::tempdir().unwrap();
        let m = d.path().join("mtp:host=SAMSUNG_SM-S911B_R5CW");
        std::fs::create_dir_all(m.join("Internal storage").join(MEDIA_REL)).unwrap();
        std::fs::create_dir_all(d.path().join("smb-share:server=x")).unwrap();
        std::fs::create_dir_all(d.path().join("mtp:host=Other/Card/DCIM")).unwrap();
        let found = discover(d.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mount, m);
        assert!(discover(&d.path().join("missing")).is_empty());
    }
}
