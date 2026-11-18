//! `export_media_files`: save several media/document files at once, either as
//! one streamed `.zip` or as individual files in a folder.
//!
//! Sources are resolved only through `AppCore::resolve_media`, which goes
//! through `wa_core::paths::bridge_media_path` and `resolve_within`, so
//! `.bridge-token` and database files can never be exported. This is a local
//! export, not a WhatsApp write: it works in read-only mode, where media that
//! is not on disk cannot be fetched and is reported as failed.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use wa_core::config::Paths;
use wa_core::types::{
    MediaExportFailure, MediaExportMode, MediaExportProgress, MediaExportReq, MediaExportSummary,
    MediaTarget, events,
};
use wa_core::{AppError, Result};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use super::{AppCore, Emitter, blocking, emit_typed, open_file_name, require};

/// Most files per export.
pub const EXPORT_MAX_ITEMS: usize = 500;

/// A file ready to be written.
struct Source {
    target: MediaTarget,
    name: String,
    path: PathBuf,
}

/// Pick a file name not in `used` (case-insensitive) and for which `taken`
/// is false: `a.pdf`, `a (1).pdf`, `a (2).pdf`, ... The result is recorded
/// in `used`.
pub fn unique_name(used: &mut HashSet<String>, name: &str, taken: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut candidate = name.to_string();
    let mut n = 0u32;
    while used.contains(&candidate.to_lowercase()) || taken(&candidate) {
        n += 1;
        candidate = format!("{stem} ({n}){ext}");
    }
    used.insert(candidate.to_lowercase());
    candidate
}

/// Validate the destination folder of a "save files to folder" export:
/// it must exist and must not be inside the bridge's own folders.
pub fn check_export_dir(paths: &Paths, dir: &Path) -> Result<PathBuf> {
    if !dir.is_absolute() {
        return Err(AppError::invalid("Choose an absolute destination folder"));
    }
    let real = std::fs::canonicalize(dir)
        .map_err(|_| AppError::invalid("The destination folder does not exist"))?;
    if !real.is_dir() {
        return Err(AppError::invalid("The destination is not a folder"));
    }
    for root in [&paths.bridge_store, &paths.phone_media, &paths.outbox] {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if real.starts_with(&root) {
            return Err(AppError::forbidden(
                "Refusing to write into the WhatsApp bridge's folders",
            ));
        }
    }
    Ok(real)
}

fn compression_for(name: &str) -> CompressionMethod {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        // Already compressed: storing is faster and no bigger.
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "mp4" | "mov" | "mkv" | "webm" | "ogg"
        | "opus" | "mp3" | "m4a" | "aac" | "zip" | "docx" | "xlsx" | "pptx" | "pdf" => {
            CompressionMethod::Stored
        }
        _ => CompressionMethod::Deflated,
    }
}

struct Progress<'a> {
    em: &'a dyn Emitter,
    request_id: &'a str,
    total: u32,
}

impl Progress<'_> {
    fn emit(&self, done: u32, current: Option<&str>) {
        emit_typed(
            self.em,
            events::MEDIA_EXPORT_PROGRESS,
            &MediaExportProgress {
                request_id: self.request_id.to_string(),
                done,
                total: self.total,
                current: current.map(str::to_string),
            },
        );
    }
}

fn fail(t: &MediaTarget, reason: impl Into<String>) -> MediaExportFailure {
    MediaExportFailure {
        chat_jid: t.chat_jid.clone(),
        message_id: t.message_id.clone(),
        reason: reason.into(),
    }
}

/// Stream `sources` into a new zip at `dest` (via a `.partial` sibling that
/// is renamed on success). Returns per-file failures (unreadable sources).
fn write_zip(
    dest: &Path,
    sources: &[Source],
    prog: &Progress<'_>,
    base: u32,
) -> Result<Vec<MediaExportFailure>> {
    let mut tmp_name = dest.as_os_str().to_owned();
    tmp_name.push(".partial");
    let tmp = PathBuf::from(tmp_name);
    let run = || -> Result<Vec<MediaExportFailure>> {
        let mut zip = zip::ZipWriter::new(File::create(&tmp)?);
        let mut failures = Vec::new();
        for (i, s) in sources.iter().enumerate() {
            prog.emit(base + i as u32, Some(&s.name));
            // Open before starting the entry so an unreadable file never
            // leaves a half-written entry behind.
            let mut src = match File::open(&s.path) {
                Ok(f) => f,
                Err(e) => {
                    failures.push(fail(&s.target, format!("Could not read the file: {e}")));
                    continue;
                }
            };
            let opts = SimpleFileOptions::default()
                .compression_method(compression_for(&s.name))
                .large_file(true);
            zip.start_file(s.name.as_str(), opts).map_err(|e| {
                AppError::internal("could not write the zip").with_detail(e.to_string())
            })?;
            io::copy(&mut src, &mut zip)?;
        }
        zip.finish()
            .map_err(|e| AppError::internal("could not finish the zip").with_detail(e.to_string()))?
            .flush()?;
        Ok(failures)
    };
    match run() {
        Ok(f) if f.len() < sources.len() => {
            std::fs::rename(&tmp, dest)?;
            Ok(f)
        }
        Ok(f) => {
            let _ = std::fs::remove_file(&tmp);
            Ok(f)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Copy `sources` into `dir` with unique names, never overwriting anything.
fn write_folder(
    dir: &Path,
    sources: &[Source],
    prog: &Progress<'_>,
    base: u32,
) -> Vec<MediaExportFailure> {
    let mut used = HashSet::new();
    let mut failures = Vec::new();
    for (i, s) in sources.iter().enumerate() {
        prog.emit(base + i as u32, Some(&s.name));
        if let Err(e) = copy_unique(dir, s, &mut used) {
            failures.push(fail(&s.target, format!("Could not save the file: {e}")));
        }
    }
    failures
}

fn copy_unique(dir: &Path, s: &Source, used: &mut HashSet<String>) -> io::Result<()> {
    let mut src = File::open(&s.path)?;
    // `create_new` also guards against a race with another writer.
    for _ in 0..50 {
        let name = unique_name(used, &s.name, |n| dir.join(n).exists());
        let target = dir.join(&name);
        match File::options().write(true).create_new(true).open(&target) {
            Ok(mut out) => {
                let copied = io::copy(&mut src, &mut out);
                if copied.is_err() {
                    drop(out);
                    let _ = std::fs::remove_file(&target);
                }
                return copied.map(|_| ());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("no free file name"))
}

impl AppCore {
    /// Resolve one item to a local file, downloading it first when missing
    /// (impossible in read-only mode, which reports it as failed).
    async fn export_source(
        &self,
        t: &MediaTarget,
    ) -> std::result::Result<(String, PathBuf), String> {
        require("chat", &t.chat_jid).map_err(|e| e.message)?;
        require("message", &t.message_id).map_err(|e| e.message)?;
        let mut r = self
            .media_meta(&t.chat_jid, &t.message_id)
            .await
            .map_err(|e| e.message)?
            .ok_or_else(|| "This message has no media".to_string())?;
        let mut path = self
            .resolve_media(&t.chat_jid, &t.message_id)
            .await
            .map_err(|e| e.message)?;
        if path.is_none() {
            if self.read_only() {
                return Err(
                    "Not downloaded, and wa-desk is in read-only mode so it cannot be fetched"
                        .into(),
                );
            }
            r = self
                .ensure_media(t.chat_jid.clone(), t.message_id.clone())
                .await
                .map_err(|e| e.message)?;
            path = self
                .resolve_media(&t.chat_jid, &t.message_id)
                .await
                .map_err(|e| e.message)?;
        }
        let path = path.ok_or_else(|| "The file is not available on this computer".to_string())?;
        let name = open_file_name(&r, &t.message_id);
        if wa_core::paths::is_secret_or_db_name(&name) {
            return Err("Refusing to export a database or secret file".into());
        }
        Ok((name, path))
    }

    /// `export_media_files`: see docs/CONTRACT.md.
    pub async fn export_media_files(&self, req: MediaExportReq) -> Result<MediaExportSummary> {
        require("request", &req.request_id)?;
        if req.items.is_empty() {
            return Err(AppError::invalid("Select at least one file"));
        }
        if req.items.len() > EXPORT_MAX_ITEMS {
            return Err(AppError::invalid(format!(
                "You can export at most {EXPORT_MAX_ITEMS} files at a time"
            )));
        }
        let dest = match req.mode {
            MediaExportMode::Zip => {
                let mut d = req.dest.trim().to_string();
                // Check the name as given first: `.zip` must not launder a
                // secret or database file name.
                let given = super::check_save_dest(&self.paths, Path::new(&d))?;
                if d.to_ascii_lowercase().ends_with(".zip") {
                    given
                } else {
                    d.push_str(".zip");
                    super::check_save_dest(&self.paths, Path::new(&d))?
                }
            }
            MediaExportMode::Folder => check_export_dir(&self.paths, Path::new(req.dest.trim()))?,
        };
        // Drop repeated ids, keep order.
        let mut seen = HashSet::new();
        let items: Vec<&MediaTarget> = req
            .items
            .iter()
            .filter(|t| seen.insert((t.chat_jid.as_str(), t.message_id.as_str())))
            .collect();
        let n = items.len() as u32;
        let prog = Progress {
            em: self.emitter.as_ref(),
            request_id: &req.request_id,
            total: n * 2,
        };
        let mut failures = Vec::new();
        let mut sources = Vec::new();
        let mut used = HashSet::new();
        for (i, t) in items.into_iter().enumerate() {
            prog.emit(i as u32, None);
            match self.export_source(t).await {
                Ok((name, path)) => {
                    // Zip names are unique inside the archive; folder names
                    // are made unique against the disk at write time.
                    let name = if req.mode == MediaExportMode::Zip {
                        unique_name(&mut used, &name, |_| false)
                    } else {
                        name
                    };
                    sources.push(Source {
                        target: t.clone(),
                        name,
                        path,
                    });
                }
                Err(reason) => failures.push(fail(t, reason)),
            }
        }
        let m = sources.len() as u32;
        let base = n + (n - m);
        let mode = req.mode;
        let (dest2, em, rid) = (dest.clone(), self.emitter.clone(), req.request_id.clone());
        let write_failures = if sources.is_empty() {
            Vec::new()
        } else {
            blocking(move || {
                let prog = Progress {
                    em: em.as_ref(),
                    request_id: &rid,
                    total: n * 2,
                };
                match mode {
                    MediaExportMode::Zip => write_zip(&dest2, &sources, &prog, base),
                    MediaExportMode::Folder => Ok(write_folder(&dest2, &sources, &prog, base)),
                }
            })
            .await?
        };
        failures.extend(write_failures);
        let failed = failures.len() as u32;
        let saved = n - failed;
        prog.emit(n * 2, None);
        tracing::info!(saved, failed, mode = ?mode, "media export finished");
        Ok(MediaExportSummary {
            request_id: req.request_id,
            saved,
            failed,
            failures,
            dest: (saved > 0).then(|| dest.to_string_lossy().into_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_unique() {
        let mut used = HashSet::new();
        let none = |_: &str| false;
        assert_eq!(unique_name(&mut used, "a.pdf", none), "a.pdf");
        assert_eq!(unique_name(&mut used, "a.pdf", none), "a (1).pdf");
        assert_eq!(unique_name(&mut used, "A.PDF", none), "A (2).PDF");
        assert_eq!(unique_name(&mut used, "noext", none), "noext");
        assert_eq!(unique_name(&mut used, "noext", none), "noext (1)");
        assert_eq!(unique_name(&mut used, ".hidden", none), ".hidden");
        assert_eq!(
            unique_name(&mut used, "b.jpg", |n| n == "b.jpg"),
            "b (1).jpg"
        );
    }
}
