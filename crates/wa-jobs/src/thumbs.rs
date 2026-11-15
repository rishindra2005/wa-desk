//! Cached thumbnails and office previews for the `wa-media://` protocol handler.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, imageops::FilterType};
use tokio_util::sync::CancellationToken;
use wa_core::config::Paths;
use wa_core::traits::ProgressSink;
use wa_core::types::MediaKind;
use wa_core::{AppError, ErrorKind, Result};

use crate::convert::{load_oriented, office_to_pdf};
use crate::manager::{JobCtl, JobDeps};
use crate::tools::{Tool, Tools};
use crate::util::{WorkDir, blocking, canonical_input_file, file_identity, sha256_hex};

/// Longest side of generated thumbnails.
pub const THUMB_SIZE: u32 = 256;
const THUMB_TIMEOUT: Duration = Duration::from_secs(60);

fn cache_key(src: &Path, tag: &str) -> Result<String> {
    let (mtime, size) = file_identity(src)?;
    Ok(sha256_hex(&format!("{}|{mtime}|{size}|{tag}", src.display())))
}

/// Office document → cached PDF in `cache/previews` (key: path|mtime|size).
pub async fn office_preview(paths: &Paths, src: &Path) -> Result<PathBuf> {
    office_preview_with(paths, &Tools::default(), src, &CancellationToken::new()).await
}

/// `office_preview` with explicit tools/cancellation.
pub async fn office_preview_with(
    paths: &Paths,
    tools: &Tools,
    src: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    let src = canonical_input_file(src)?;
    let dir = paths.cache.join("previews");
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(format!("{}.pdf", cache_key(&src, "pdf")?));
    if out.is_file() {
        return Ok(out);
    }
    let work = WorkDir::new(&paths.cache)?;
    let pdf = office_to_pdf(tools, &paths.cache, &src, work.path(), cancel).await?;
    std::fs::rename(&pdf, &out).or_else(|_| std::fs::copy(&pdf, &out).map(|_| ()))?;
    Ok(out)
}

fn is_pdf(src: &Path) -> bool {
    if src.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        return true;
    }
    let mut head = [0u8; 5];
    std::fs::File::open(src)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok()
        && &head == b"%PDF-"
}

/// 256 px thumbnail cached in `cache/thumbs`. `Ok(None)` when the kind/file
/// has no thumbnail (audio, non-PDF documents, undecodable images).
pub async fn thumbnail(paths: &Paths, src: &Path, kind: MediaKind) -> Result<Option<PathBuf>> {
    thumbnail_with(paths, &Tools::default(), src, kind, &CancellationToken::new()).await
}

/// `thumbnail` with explicit tools/cancellation.
pub async fn thumbnail_with(
    paths: &Paths,
    tools: &Tools,
    src: &Path,
    kind: MediaKind,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>> {
    let src = canonical_input_file(src)?;
    let dir = paths.cache.join("thumbs");
    std::fs::create_dir_all(&dir)?;
    let key = cache_key(&src, "thumb256")?;
    for ext in ["jpg", "webp"] {
        let p = dir.join(format!("{key}.{ext}"));
        if p.is_file() {
            return Ok(Some(p));
        }
    }
    let jpg = dir.join(format!("{key}.jpg"));
    match kind {
        MediaKind::Image | MediaKind::Sticker => {
            let (s, d, k) = (src.clone(), dir.clone(), key.clone());
            blocking(move || image_thumb(&s, &d, &k)).await
        }
        MediaKind::Video => video_thumb(paths, tools, &src, &jpg, cancel).await,
        MediaKind::Document if is_pdf(&src) => pdf_thumb(paths, tools, &src, &jpg, cancel).await,
        MediaKind::Document | MediaKind::Audio => Ok(None),
    }
}

fn image_thumb(src: &Path, dir: &Path, key: &str) -> Result<Option<PathBuf>> {
    let img = match load_oriented(src) {
        Ok(i) => i,
        Err(e) if e.kind == ErrorKind::Invalid => return Ok(None),
        Err(e) => return Err(e),
    };
    let t = img.resize(THUMB_SIZE, THUMB_SIZE, FilterType::Lanczos3);
    let enc = |e: image::ImageError| AppError::internal("could not encode thumbnail").with_detail(e.to_string());
    let tmp = dir.join(format!(".{key}.{}.tmp", uuid::Uuid::new_v4()));
    let out = if t.color().has_alpha() {
        let w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        DynamicImage::ImageRgba8(t.to_rgba8())
            .write_with_encoder(WebPEncoder::new_lossless(w))
            .map_err(enc)?;
        dir.join(format!("{key}.webp"))
    } else {
        let w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        DynamicImage::ImageRgb8(t.to_rgb8())
            .write_with_encoder(JpegEncoder::new_with_quality(w, 80))
            .map_err(enc)?;
        dir.join(format!("{key}.jpg"))
    };
    std::fs::rename(&tmp, &out)?;
    Ok(Some(out))
}

async fn video_thumb(
    paths: &Paths,
    tools: &Tools,
    src: &Path,
    out: &Path,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>> {
    let work = WorkDir::new(&paths.cache)?;
    let tmp = work.path().join("t.jpg");
    let scale = format!("scale={THUMB_SIZE}:{THUMB_SIZE}:force_original_aspect_ratio=decrease");
    // 1 s in avoids black first frames; very short clips fall back to 0 s.
    for seek in ["1", "0"] {
        let r = Tool::new("ffmpeg", &tools.ffmpeg)
            .args(["-y", "-hide_banner", "-nostdin", "-loglevel", "error", "-ss", seek, "-i"])
            .arg(src)
            .args(["-frames:v", "1", "-vf", &scale])
            .arg(&tmp)
            .timeout(THUMB_TIMEOUT)
            .run(cancel, None)
            .await;
        match r {
            Ok(_) if tmp.is_file() => {
                std::fs::rename(&tmp, out).or_else(|_| std::fs::copy(&tmp, out).map(|_| ()))?;
                return Ok(Some(out.to_path_buf()));
            }
            Ok(_) => continue,
            Err(e) if e.kind == ErrorKind::External && !e.message.contains("not installed") => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

async fn pdf_thumb(
    paths: &Paths,
    tools: &Tools,
    src: &Path,
    out: &Path,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>> {
    let work = WorkDir::new(&paths.cache)?;
    let prefix = work.path().join("t");
    let r = Tool::new("pdftoppm", &tools.pdftoppm)
        .args(["-f", "1", "-l", "1", "-scale-to", &THUMB_SIZE.to_string(), "-jpeg", "-singlefile", "--"])
        .arg(src)
        .arg(&prefix)
        .timeout(THUMB_TIMEOUT)
        .run(cancel, None)
        .await;
    match r {
        Ok(_) => {
            let made = work.path().join("t.jpg");
            if !made.is_file() {
                return Ok(None);
            }
            std::fs::rename(&made, out).or_else(|_| std::fs::copy(&made, out).map(|_| ()))?;
            Ok(Some(out.to_path_buf()))
        }
        Err(e) if e.kind == ErrorKind::External && !e.message.contains("not installed") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Thumbnails job: every local image/video/sticker/PDF.
pub async fn run_job(deps: &JobDeps, ctl: &Arc<JobCtl>) -> Result<String> {
    ctl.phase("listing media");
    let store = deps.messages.clone();
    let rows = blocking(move || {
        store.media_rows(
            None,
            &[MediaKind::Image, MediaKind::Video, MediaKind::Sticker, MediaKind::Document],
        )
    })
    .await?;
    let rows: Vec<_> = rows.into_iter().filter(|r| r.local_path.is_some()).collect();
    let total = rows.len() as u64;
    let (mut made, mut none, mut failed) = (0u64, 0u64, 0u64);
    for (i, r) in rows.iter().enumerate() {
        ctl.check()?;
        let Some(p) = &r.local_path else { continue };
        match thumbnail_with(&deps.paths, &deps.tools, p, r.kind, ctl.cancel_token()).await {
            Ok(Some(_)) => made += 1,
            Ok(None) => none += 1,
            Err(e) if e.kind == ErrorKind::Cancelled => return Err(e),
            Err(e) if e.message.contains("not installed") => return Err(e),
            Err(_) => failed += 1,
        }
        ctl.progress(i as u64 + 1, total, "generating", None);
    }
    Ok(format!(
        "{made} thumbnails ready; {none} files have no preview; {failed} failed"
    ))
}
