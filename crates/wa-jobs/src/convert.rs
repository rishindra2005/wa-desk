//! File conversions (LibreOffice, image crate, ffmpeg, Ghostscript, pdftoppm).

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use image::ImageReader;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::{DynamicImage, ImageDecoder, imageops::FilterType};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use wa_core::config::Paths;
use wa_core::paths::sanitize_file_name;
use wa_core::traits::ProgressSink;
use wa_core::types::{ConvertInput, ConvertKind, ConvertSpec, ImageFormat};
use wa_core::{AppError, Result};

use crate::manager::{JobCtl, JobDeps};
use crate::tools::{Tool, Tools};
use crate::util::{WorkDir, blocking, canonical_input_file, output_base, place_unique};

/// LibreOffice cannot run two conversions on one profile: serialise them all.
pub(crate) static LIBREOFFICE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(1));

const OFFICE_TIMEOUT: Duration = Duration::from_secs(120);
const VIDEO_TIMEOUT: Duration = Duration::from_secs(4 * 3600);
const TOOL_TIMEOUT: Duration = Duration::from_secs(600);

/// Reject bad parameters before a job is created.
pub fn validate_kind(kind: &ConvertKind) -> Result<()> {
    match kind {
        ConvertKind::OfficeToPdf => Ok(()),
        ConvertKind::ImageCompress {
            quality, max_dim, ..
        } => {
            if !(1..=100).contains(quality) {
                return Err(AppError::invalid("image quality must be 1–100"));
            }
            if !(16..=16_384).contains(max_dim) {
                return Err(AppError::invalid("max dimension must be 16–16384"));
            }
            Ok(())
        }
        ConvertKind::VideoCompress { crf, max_height } => {
            if *crf > 51 {
                return Err(AppError::invalid("CRF must be 0–51"));
            }
            if !(16..=8640).contains(max_height) {
                return Err(AppError::invalid("max height must be 16–8640"));
            }
            Ok(())
        }
        ConvertKind::PdfCompress { level } => pdf_setting(level).map(|_| ()),
        ConvertKind::AudioToOpus { bitrate_kbps } => {
            if !(6..=510).contains(bitrate_kbps) {
                return Err(AppError::invalid("Opus bitrate must be 6–510 kbps"));
            }
            Ok(())
        }
        ConvertKind::PdfToImages { dpi } => {
            if !(36..=600).contains(dpi) {
                return Err(AppError::invalid("DPI must be 36–600"));
            }
            Ok(())
        }
    }
}

fn pdf_setting(level: &str) -> Result<&'static str> {
    match level {
        "screen" => Ok("/screen"),
        "ebook" => Ok("/ebook"),
        "printer" => Ok("/printer"),
        _ => Err(AppError::invalid(
            "PDF compression level must be screen, ebook or printer",
        )),
    }
}

/// Resolve the input to a local file and a display stem.
async fn resolve_input(deps: &JobDeps, input: &ConvertInput) -> Result<(PathBuf, String)> {
    let (path, name) = match input {
        ConvertInput::File { path } => {
            let p = PathBuf::from(path);
            (
                p.clone(),
                p.file_name().map(|n| n.to_string_lossy().into_owned()),
            )
        }
        ConvertInput::Message {
            chat_jid,
            message_id,
        } => {
            let store = deps.messages.clone();
            let (c, m) = (chat_jid.clone(), message_id.clone());
            let info = blocking(move || store.media_info(&c, &m))
                .await?
                .ok_or_else(|| AppError::not_found("message has no media"))?;
            let local = match info.local_path.filter(|p| p.is_file()) {
                Some(p) => p,
                None => {
                    deps.bridge
                        .download(chat_jid, message_id, false)
                        .await?
                        .path
                }
            };
            let name = info
                .filename
                .clone()
                .or_else(|| local.file_name().map(|n| n.to_string_lossy().into_owned()));
            (local, name)
        }
    };
    let real = canonical_input_file(&path)?;
    let name = sanitize_file_name(&name.unwrap_or_else(|| "file".into()));
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name,
    };
    Ok((real, stem))
}

fn output_dir(paths: &Paths, requested: Option<&str>) -> Result<PathBuf> {
    match requested.filter(|s| !s.trim().is_empty()) {
        Some(d) => {
            let p = Path::new(d);
            if !p.is_absolute() {
                return Err(AppError::invalid("output folder must be absolute"));
            }
            let real = std::fs::canonicalize(p)
                .map_err(|_| AppError::not_found("output folder does not exist"))?;
            if !real.is_dir() {
                return Err(AppError::invalid("output folder is not a folder"));
            }
            Ok(real)
        }
        None => {
            std::fs::create_dir_all(&paths.exports)?;
            Ok(paths.exports.clone())
        }
    }
}

/// Run one conversion job. Returns the summary message.
pub async fn run(deps: &JobDeps, spec: &ConvertSpec, ctl: &Arc<JobCtl>) -> Result<String> {
    validate_kind(&spec.kind)?;
    ctl.phase("preparing input");
    let (src, stem) = resolve_input(deps, &spec.input).await?;
    let out_dir = output_dir(&deps.paths, spec.output_dir.as_deref())?;
    let work = WorkDir::new(&deps.paths.cache)?;
    ctl.phase("converting");
    let produced = convert_file(deps, &spec.kind, &src, work.path(), ctl).await?;
    let mut outs = Vec::new();
    for (tmp, suffix, ext) in produced {
        let p = place_unique(&tmp, &out_dir, &output_base(&stem, &suffix), &ext)?;
        ctl.output(&p);
        outs.push(p);
    }
    let names: Vec<String> = outs
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    Ok(match names.len() {
        0 => "No output produced".into(),
        1 => format!("Saved {}", names[0]),
        n => format!("Saved {n} files ({} …)", names[0]),
    })
}

/// Produced files: (temp path, name suffix, extension).
type Produced = Vec<(PathBuf, String, String)>;

async fn convert_file(
    deps: &JobDeps,
    kind: &ConvertKind,
    src: &Path,
    work: &Path,
    ctl: &Arc<JobCtl>,
) -> Result<Produced> {
    let tools = &deps.tools;
    let cancel = ctl.cancel_token();
    match kind {
        ConvertKind::OfficeToPdf => {
            let pdf = office_to_pdf(tools, &deps.paths.cache, src, work, cancel).await?;
            Ok(vec![(pdf, String::new(), "pdf".into())])
        }
        ConvertKind::ImageCompress {
            quality,
            max_dim,
            format,
        } => {
            let (q, m, f) = (*quality, *max_dim, *format);
            let out = match f {
                ImageFormat::Webp => {
                    let out = work.join("out.webp");
                    let scale = format!(
                        "scale=w='min(iw,{m})':h='min(ih,{m})':force_original_aspect_ratio=decrease"
                    );
                    ffmpeg(tools)
                        .arg("-i")
                        .arg(src)
                        .args(["-vf", &scale, "-c:v", "libwebp", "-lossless", "0"])
                        .args(["-quality", &q.to_string(), "-frames:v", "1"])
                        .arg(&out)
                        .timeout(TOOL_TIMEOUT)
                        .run(cancel, None)
                        .await?;
                    out
                }
                _ => {
                    let (s, w) = (src.to_path_buf(), work.to_path_buf());
                    blocking(move || compress_image(&s, &w, q, m, f)).await?
                }
            };
            let ext = match f {
                ImageFormat::Jpeg => "jpg",
                ImageFormat::Png => "png",
                ImageFormat::Webp => "webp",
            };
            Ok(vec![(out, "compressed".into(), ext.into())])
        }
        ConvertKind::VideoCompress { crf, max_height } => {
            let out = work.join("out.mp4");
            let duration = probe_duration(tools, src, cancel).await;
            let total_ms = duration.map_or(0, |d| (d * 1000.0) as u64);
            let ctl2 = ctl.clone();
            let mut on_line = move |l: &str| {
                if let Some(us) = l
                    .strip_prefix("out_time_us=")
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    let ms = us / 1000;
                    let done = if total_ms > 0 { ms.min(total_ms) } else { ms };
                    ctl2.progress(done, total_ms, "encoding video", None);
                }
            };
            let scale = format!("scale=-2:'trunc(min(ih,{max_height})/2)*2'");
            ffmpeg(tools)
                .arg("-i")
                .arg(src)
                .args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    &crf.to_string(),
                ])
                .args(["-vf", &scale, "-pix_fmt", "yuv420p"])
                .args(["-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"])
                .args(["-progress", "pipe:1", "-nostats"])
                .arg(&out)
                .timeout(VIDEO_TIMEOUT)
                .run(cancel, Some(&mut on_line))
                .await?;
            Ok(vec![(out, "compressed".into(), "mp4".into())])
        }
        ConvertKind::PdfCompress { level } => {
            let setting = pdf_setting(level)?;
            let out = work.join("out.pdf");
            let mut out_arg = std::ffi::OsString::from("-sOutputFile=");
            out_arg.push(&out);
            Tool::new("gs", &tools.gs)
                .args([
                    "-sDEVICE=pdfwrite",
                    "-dCompatibilityLevel=1.5",
                    "-dNOPAUSE",
                    "-dQUIET",
                ])
                .args(["-dBATCH", "-dSAFER"])
                .arg(format!("-dPDFSETTINGS={setting}"))
                .arg(out_arg)
                .arg("--")
                .arg(src)
                .timeout(TOOL_TIMEOUT)
                .run(cancel, None)
                .await?;
            Ok(vec![(out, "compressed".into(), "pdf".into())])
        }
        ConvertKind::AudioToOpus { bitrate_kbps } => {
            let out = work.join("out.ogg");
            ffmpeg(tools)
                .arg("-i")
                .arg(src)
                .args([
                    "-vn",
                    "-c:a",
                    "libopus",
                    "-b:a",
                    &format!("{bitrate_kbps}k"),
                ])
                .args(["-vbr", "on", "-application", "voip"])
                .arg(&out)
                .timeout(TOOL_TIMEOUT)
                .run(cancel, None)
                .await?;
            Ok(vec![(out, "opus".into(), "ogg".into())])
        }
        ConvertKind::PdfToImages { dpi } => {
            let prefix = work.join("page");
            Tool::new("pdftoppm", &tools.pdftoppm)
                .args(["-r", &dpi.to_string(), "-png", "--"])
                .arg(src)
                .arg(&prefix)
                .timeout(TOOL_TIMEOUT)
                .run(cancel, None)
                .await?;
            let mut pages: Vec<(u32, PathBuf)> = std::fs::read_dir(work)?
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let num = n
                        .strip_prefix("page-")?
                        .strip_suffix(".png")?
                        .parse()
                        .ok()?;
                    Some((num, e.path()))
                })
                .collect();
            pages.sort();
            if pages.is_empty() {
                return Err(AppError::external("pdftoppm produced no pages"));
            }
            Ok(pages
                .into_iter()
                .map(|(n, p)| (p, format!("page-{n}"), "png".to_string()))
                .collect())
        }
    }
}

fn ffmpeg(tools: &Tools) -> Tool {
    Tool::new("ffmpeg", &tools.ffmpeg).args([
        "-y",
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
    ])
}

async fn probe_duration(tools: &Tools, src: &Path, cancel: &CancellationToken) -> Option<f64> {
    let out = Tool::new("ffprobe", &tools.ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(src)
        .timeout(Duration::from_secs(30))
        .run(cancel, None)
        .await
        .ok()?;
    out.stdout.trim().parse().ok()
}

/// Decode (respecting EXIF orientation), resize to fit `max_dim` with Lanczos3.
pub(crate) fn load_oriented(src: &Path) -> Result<DynamicImage> {
    let bad = |e: image::ImageError| {
        AppError::invalid("unsupported or damaged image").with_detail(e.to_string())
    };
    let mut dec = ImageReader::open(src)?
        .with_guessed_format()?
        .into_decoder()
        .map_err(bad)?;
    let orientation = dec.orientation().ok();
    let mut img = DynamicImage::from_decoder(dec).map_err(bad)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

fn compress_image(
    src: &Path,
    work: &Path,
    quality: u8,
    max_dim: u32,
    f: ImageFormat,
) -> Result<PathBuf> {
    let mut img = load_oriented(src)?;
    if img.width() > max_dim || img.height() > max_dim {
        img = img.resize(max_dim, max_dim, FilterType::Lanczos3);
    }
    let enc = |e: image::ImageError| {
        AppError::internal("could not encode image").with_detail(e.to_string())
    };
    match f {
        ImageFormat::Png => {
            let out = work.join("out.png");
            let w = std::io::BufWriter::new(std::fs::File::create(&out)?);
            img.write_with_encoder(PngEncoder::new_with_quality(
                w,
                CompressionType::Best,
                PngFilter::Adaptive,
            ))
            .map_err(enc)?;
            Ok(out)
        }
        _ => {
            let out = work.join("out.jpg");
            let w = std::io::BufWriter::new(std::fs::File::create(&out)?);
            DynamicImage::ImageRgb8(img.to_rgb8())
                .write_with_encoder(JpegEncoder::new_with_quality(w, quality))
                .map_err(enc)?;
            Ok(out)
        }
    }
}

const PATH_SET: &AsciiSet = &CONTROLS.add(b' ').add(b'%').add(b'#').add(b'?').add(b'"');

/// soffice → PDF in `work` (serialised; isolated profile under `cache/lo-profile`).
pub(crate) async fn office_to_pdf(
    tools: &Tools,
    cache: &Path,
    src: &Path,
    work: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    let profile = cache.join("lo-profile");
    std::fs::create_dir_all(&profile)?;
    let profile_url = format!(
        "file://{}",
        utf8_percent_encode(&profile.to_string_lossy(), PATH_SET)
    );
    let _permit = tokio::select! {
        p = LIBREOFFICE.acquire() => p.map_err(|_| AppError::internal("LibreOffice lock closed"))?,
        () = cancel.cancelled() => return Err(AppError::cancelled()),
    };
    Tool::new("soffice", &tools.soffice)
        .arg(format!("-env:UserInstallation={profile_url}"))
        .args([
            "--headless",
            "--norestore",
            "--nolockcheck",
            "--convert-to",
            "pdf",
            "--outdir",
        ])
        .arg(work)
        .arg(src)
        .timeout(OFFICE_TIMEOUT)
        .nice(tools.low_priority)
        .run(cancel, None)
        .await?;
    std::fs::read_dir(work)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
        .ok_or_else(|| AppError::external("LibreOffice could not convert this document"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_parameters() {
        let ok = [
            ConvertKind::OfficeToPdf,
            ConvertKind::ImageCompress {
                quality: 80,
                max_dim: 1600,
                format: ImageFormat::Jpeg,
            },
            ConvertKind::VideoCompress {
                crf: 28,
                max_height: 720,
            },
            ConvertKind::PdfCompress {
                level: "ebook".into(),
            },
            ConvertKind::AudioToOpus { bitrate_kbps: 32 },
            ConvertKind::PdfToImages { dpi: 150 },
        ];
        for k in ok {
            assert!(validate_kind(&k).is_ok(), "{k:?}");
        }
        let bad = [
            ConvertKind::ImageCompress {
                quality: 0,
                max_dim: 1600,
                format: ImageFormat::Jpeg,
            },
            ConvertKind::ImageCompress {
                quality: 80,
                max_dim: 1,
                format: ImageFormat::Png,
            },
            ConvertKind::VideoCompress {
                crf: 60,
                max_height: 720,
            },
            ConvertKind::PdfCompress {
                level: "prepress".into(),
            },
            ConvertKind::PdfCompress {
                level: "/screen".into(),
            },
            ConvertKind::AudioToOpus { bitrate_kbps: 1 },
            ConvertKind::PdfToImages { dpi: 5000 },
        ];
        for k in bad {
            assert_eq!(
                validate_kind(&k).unwrap_err().kind,
                wa_core::ErrorKind::Invalid,
                "{k:?}"
            );
        }
    }
}
