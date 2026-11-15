//! Phone-media linker (port of `whatsapp-link-phone-media`).
//!
//! Indexes `phone_media` plus extra folders, matches media messages by
//! SHA-256 (then documents by normalised name + exact size) and places the
//! file where the bridge expects it (hard link for phone files, copy for
//! extra folders). Existing store files are never overwritten.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use parking_lot::Mutex;
use wa_core::paths::bridge_media_path;
use wa_core::traits::{LinkRecord, MediaInfo, ProgressSink};
use wa_core::types::MediaKind;
use wa_core::{AppError, Result};

use crate::manager::{JobCtl, JobDeps};
use crate::util::{blocking, sha256_file};

/// Files above this size in extra folders are ignored.
pub const MAX_EXTRA_BYTES: u64 = 200 * 1024 * 1024;
const BATCH: usize = 500;
const SKIP_DIRS: &[&str] = &["node_modules", "__pycache__", "scratch_venv"];

/// One indexed candidate file.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
    /// From `phone_media` (hard-link) rather than an extra folder (copy).
    pub is_phone: bool,
}

/// Lowercase, trim, strip a trailing " (1)"-style counter from the stem.
pub fn norm_name(name: &str) -> String {
    let n = name.trim().to_lowercase();
    let (stem, ext) = match n.rfind('.') {
        Some(i) if i > 0 => (&n[..i], &n[i..]),
        _ => (n.as_str(), ""),
    };
    let stem = strip_counter(stem);
    format!("{stem}{ext}")
}

fn strip_counter(stem: &str) -> &str {
    let Some(inner) = stem.strip_suffix(')') else {
        return stem;
    };
    let Some(open) = inner.rfind('(') else {
        return stem;
    };
    let digits = &inner[open + 1..];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return stem;
    }
    inner[..open].trim_end()
}

/// Walk `root` (no symlinks, hidden dirs, node_modules, `.nomedia`).
pub fn walk(root: &Path, is_phone: bool, out: &mut Vec<Candidate>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name();
            let name = name.to_string_lossy();
            if ft.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref()) {
                    stack.push(e.path());
                }
            } else if ft.is_file() && name != ".nomedia" {
                let Ok(meta) = e.metadata() else { continue };
                if !is_phone && meta.len() > MAX_EXTRA_BYTES {
                    continue;
                }
                out.push(Candidate {
                    path: e.path(),
                    size: meta.len(),
                    is_phone,
                });
            }
        }
    }
}

struct Index {
    by_hash: HashMap<[u8; 32], usize>,
    by_name: HashMap<(String, u64), usize>,
    files: Vec<Candidate>,
}

fn build_index(files: Vec<Candidate>, ctl: &JobCtl) -> Result<Index> {
    let total = files.len() as u64;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8);
    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let hashes: Mutex<Vec<Option<[u8; 32]>>> = Mutex::new(vec![None; files.len()]);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    if ctl.is_cancelled() {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(c) = files.get(i) else { break };
                    let h = sha256_file(&c.path).ok();
                    hashes.lock()[i] = h;
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    ctl.progress(d, total, "hashing", None);
                }
            });
        }
    });
    ctl.check()?;
    let hashes = hashes.into_inner();
    let mut by_hash = HashMap::new();
    let mut by_name = HashMap::new();
    for (i, c) in files.iter().enumerate() {
        let Some(h) = hashes[i] else { continue };
        by_hash.entry(h).or_insert(i);
        if let Some(n) = c.path.file_name() {
            by_name
                .entry((norm_name(&n.to_string_lossy()), c.size))
                .or_insert(i);
        }
    }
    Ok(Index {
        by_hash,
        by_name,
        files,
    })
}

fn find_match<'a>(idx: &'a Index, row: &MediaInfo) -> Option<(&'a Candidate, &'static str)> {
    if let Some(sha) = row
        .file_sha256
        .as_deref()
        .and_then(|s| <[u8; 32]>::try_from(s).ok())
        && let Some(&i) = idx.by_hash.get(&sha)
    {
        return Some((&idx.files[i], "hash"));
    }
    if row.kind == MediaKind::Document
        && let (Some(name), Some(len)) = (row.filename.as_deref(), row.file_length)
        && let Some(&i) = idx.by_name.get(&(norm_name(name), len))
    {
        return Some((&idx.files[i], "name+size"));
    }
    None
}

fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        DirBuilder::new().mode(0o700).recursive(true).create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Place `src` at `dest` (hard link when allowed, else exclusive copy). Never overwrites.
fn place(src: &Candidate, dest: &Path) -> io::Result<&'static str> {
    if let Some(dir) = dest.parent() {
        ensure_private_dir(dir)?;
    }
    let how = if src.is_phone && fs::hard_link(&src.path, dest).is_ok() {
        "hardlink"
    } else {
        let mut out = OpenOptions::new().write(true).create_new(true).open(dest)?;
        let mut input = fs::File::open(&src.path)?;
        if let Err(e) = io::copy(&mut input, &mut out) {
            let _ = fs::remove_file(dest);
            return Err(e);
        }
        "copy"
    };
    fs::set_permissions(dest, fs::Permissions::from_mode(0o600))?;
    Ok(how)
}

#[derive(Default)]
struct Stats {
    hash: u64,
    name_size: u64,
    already: u64,
    linked: u64,
    unmatched: u64,
    errors: u64,
    per_kind: BTreeMap<&'static str, (u64, u64)>,
}

fn link_all(deps: &JobDeps, extra_dirs: &[String], ctl: &JobCtl) -> Result<String> {
    ctl.phase("indexing");
    let mut files = Vec::new();
    walk(&deps.paths.phone_media, true, &mut files);
    for d in extra_dirs {
        let p = Path::new(d);
        if !p.is_absolute() {
            return Err(AppError::invalid(format!(
                "extra folder must be absolute: {d}"
            )));
        }
        match fs::canonicalize(p) {
            Ok(real) if real.is_dir() => walk(&real, false, &mut files),
            _ => tracing::warn!(dir = %d, "extra folder missing; skipped"),
        }
    }
    let indexed = files.len();
    let idx = build_index(files, ctl)?;
    ctl.phase("matching");
    let rows = deps.messages.media_rows(None, &[])?;
    let total = rows.len() as u64;
    let mut st = Stats::default();
    let mut batch: Vec<LinkRecord> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        ctl.check()?;
        let entry = st.per_kind.entry(row.kind.as_db()).or_default();
        let Some((cand, how)) = find_match(&idx, row) else {
            st.unmatched += 1;
            entry.1 += 1;
            continue;
        };
        entry.0 += 1;
        if how == "hash" {
            st.hash += 1;
        } else {
            st.name_size += 1;
        }
        let store_path = bridge_media_path(
            &deps.paths.bridge_store,
            &row.chat_jid,
            row.kind.as_db(),
            &row.message_id,
            &row.raw_ts,
        )
        .ok();
        if let Some(sp) = &store_path {
            if sp.exists() {
                st.already += 1;
            } else {
                match place(cand, sp) {
                    Ok(_) => st.linked += 1,
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => st.already += 1,
                    Err(e) => {
                        st.errors += 1;
                        tracing::warn!(error = %e, "could not place linked media");
                        continue;
                    }
                }
            }
        }
        batch.push(LinkRecord {
            chat_jid: row.chat_jid.clone(),
            message_id: row.message_id.clone(),
            kind: row.kind,
            source_path: cand.path.clone(),
            store_path,
            matched_by: how.to_string(),
        });
        if batch.len() >= BATCH {
            deps.links.upsert_links(&batch)?;
            batch.clear();
        }
        ctl.progress(i as u64 + 1, total, "matching", None);
    }
    if !batch.is_empty() {
        deps.links.upsert_links(&batch)?;
    }
    let kinds: Vec<String> = st
        .per_kind
        .iter()
        .map(|(k, (m, u))| format!("{k} {m}/{}", m + u))
        .collect();
    let mut msg = format!(
        "Indexed {indexed} files. Matched {} (hash {}, name+size {}): {} placed now, {} already in store; {} unmatched. Per kind: {}",
        st.hash + st.name_size,
        st.hash,
        st.name_size,
        st.linked,
        st.already,
        st.unmatched,
        if kinds.is_empty() {
            "none".to_string()
        } else {
            kinds.join(", ")
        }
    );
    if st.errors > 0 {
        msg.push_str(&format!("; {} could not be placed", st.errors));
    }
    Ok(msg)
}

/// Run the linker job. Returns the summary message.
pub async fn run(deps: &JobDeps, extra_dirs: &[String], ctl: Arc<JobCtl>) -> Result<String> {
    let deps = deps.clone();
    let extra = extra_dirs.to_vec();
    blocking(move || link_all(&deps, &extra, &ctl)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_names() {
        assert_eq!(norm_name("ML Notes (1).PDF"), "ml notes.pdf");
        assert_eq!(norm_name(" Report(12).docx "), "report.docx");
        assert_eq!(norm_name("plan (v2).pdf"), "plan (v2).pdf");
        assert_eq!(norm_name("README"), "readme");
        assert_eq!(norm_name(".hidden"), ".hidden");
    }

    #[test]
    fn walk_skips_hidden_symlinks_and_big_extra() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join(".thumbs")).unwrap();
        fs::create_dir_all(r.join("node_modules")).unwrap();
        fs::create_dir_all(r.join("a")).unwrap();
        fs::write(r.join(".thumbs/x"), b"x").unwrap();
        fs::write(r.join("node_modules/y"), b"y").unwrap();
        fs::write(r.join("a/.nomedia"), b"").unwrap();
        fs::write(r.join("a/ok.jpg"), b"ok").unwrap();
        std::os::unix::fs::symlink(r.join("a/ok.jpg"), r.join("a/link.jpg")).unwrap();
        let big = fs::File::create(r.join("big.bin")).unwrap();
        big.set_len(MAX_EXTRA_BYTES + 1).unwrap();
        let mut v = Vec::new();
        walk(r, false, &mut v);
        let names: Vec<_> = v
            .iter()
            .map(|c| c.path.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["ok.jpg"]);
        let mut v = Vec::new();
        walk(r, true, &mut v);
        assert_eq!(v.len(), 2, "phone files have no size cap");
    }
}
