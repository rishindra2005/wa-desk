//! `wa-media://` custom protocol (see docs/CONTRACT.md).
//!
//! [`handle`] maps a request to a response without touching Tauri, so it is
//! tested directly. Files are never read whole: only the requested range (or
//! the first [`MAX_CHUNK`] bytes of large files) is loaded.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tauri::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use wa_core::types::MediaKind;
use wa_core::{AppError, ErrorKind};

use crate::core::{AppCore, OFFICE_EXTS, blocking};
use crate::media_url::{MediaRequest, parse};
use crate::range::{ByteRange, MAX_CHUNK, RangeOutcome, parse_range};

/// Files up to this size are answered with a plain 200; larger files get
/// 206 + the first chunk when no Range was asked for.
pub const FULL_LIMIT: u64 = 32 * 1024 * 1024;
/// Cache lifetime of original media.
pub const CACHE_MEDIA: u32 = 3600;
/// Cache lifetime of generated thumbnails/previews.
pub const CACHE_DERIVED: u32 = 7 * 24 * 3600;

/// Served file with its metadata.
struct Target {
    path: PathBuf,
    content_type: String,
    max_age: u32,
}

fn status_for(e: &AppError) -> StatusCode {
    match e.kind {
        ErrorKind::Invalid => StatusCode::BAD_REQUEST,
        ErrorKind::Forbidden | ErrorKind::Auth => StatusCode::FORBIDDEN,
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::Busy | ErrorKind::Timeout => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Origins allowed to read responses cross-origin (the app itself, dev server).
fn allowed_origin(origin: &str) -> bool {
    origin.starts_with("tauri://")
        || origin == "http://tauri.localhost"
        || origin == "https://tauri.localhost"
        || origin.starts_with("http://localhost:")
        || origin.starts_with("http://127.0.0.1:")
}

fn base(status: StatusCode, origin: Option<&str>) -> tauri::http::response::Builder {
    let mut b = Response::builder()
        .status(status)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:",
        )
        .header(header::VARY, "Origin");
    if let Some(o) = origin.filter(|o| allowed_origin(o))
        && let Ok(v) = HeaderValue::from_str(o)
    {
        b = b
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, v)
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Range")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
            .header(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                "Content-Range, Content-Length, Accept-Ranges, Content-Type",
            );
    }
    b
}

fn text(status: StatusCode, origin: Option<&str>, msg: &str) -> Response<Vec<u8>> {
    base(status, origin)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(msg.as_bytes().to_vec())
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

fn error(e: &AppError, origin: Option<&str>) -> Response<Vec<u8>> {
    let st = status_for(e);
    if st == StatusCode::INTERNAL_SERVER_ERROR {
        tracing::warn!(error = %e, "wa-media request failed");
    }
    text(st, origin, &e.message)
}

/// Content type safe to serve: active documents are downgraded to text.
fn safe_content_type(mime: &str) -> String {
    let m = mime.trim().to_ascii_lowercase();
    let essence = m.split(';').next().unwrap_or("").trim().to_string();
    if essence.is_empty() {
        return "application/octet-stream".into();
    }
    if matches!(
        essence.as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "text/xml"
            | "application/xml"
            | "text/javascript"
            | "application/javascript"
    ) {
        return "text/plain; charset=utf-8".into();
    }
    if essence.starts_with("text/") || essence == "application/json" {
        return format!("{essence}; charset=utf-8");
    }
    essence
}

fn guess(path: &Path) -> String {
    mime_guess::from_path(path)
        .first()
        .map(|m| m.essence_str().to_string())
        .unwrap_or_else(|| "application/octet-stream".into())
}

async fn resolve_message(
    core: &AppCore,
    chat: &str,
    id: &str,
) -> Result<(PathBuf, Option<wa_core::types::MediaRef>), AppError> {
    let path = core
        .resolve_media(chat, id)
        .await?
        .ok_or_else(|| AppError::not_found("media is not available locally"))?;
    let meta = core.media_meta(chat, id).await?;
    Ok((path, meta))
}

async fn target_for(core: &AppCore, req: MediaRequest) -> Result<Target, AppError> {
    match req {
        MediaRequest::Original {
            chat_jid,
            message_id,
        } => {
            let (path, meta) = resolve_message(core, &chat_jid, &message_id).await?;
            let ct = meta.and_then(|m| m.mime).unwrap_or_else(|| guess(&path));
            Ok(Target {
                path,
                content_type: ct,
                max_age: CACHE_MEDIA,
            })
        }
        MediaRequest::Thumb {
            chat_jid,
            message_id,
        } => {
            let (path, meta) = resolve_message(core, &chat_jid, &message_id).await?;
            let kind = meta.map(|m| m.kind).unwrap_or(MediaKind::Document);
            let thumb = wa_jobs::thumbnail(core.paths(), &path, kind)
                .await?
                .ok_or_else(|| AppError::not_found("no thumbnail for this file"))?;
            Ok(Target {
                content_type: guess(&thumb),
                path: thumb,
                max_age: CACHE_DERIVED,
            })
        }
        MediaRequest::Preview {
            chat_jid,
            message_id,
        } => {
            let (path, meta) = resolve_message(core, &chat_jid, &message_id).await?;
            let ext = meta
                .and_then(|m| m.filename)
                .and_then(|f| {
                    Path::new(&f)
                        .extension()
                        .map(|e| e.to_string_lossy().to_ascii_lowercase())
                })
                .unwrap_or_default();
            if !OFFICE_EXTS.contains(&ext.as_str()) {
                return Err(AppError::not_found("no preview for this file type"));
            }
            let pdf = wa_jobs::office_preview(core.paths(), &path).await?;
            Ok(Target {
                path: pdf,
                content_type: "application/pdf".into(),
                max_age: CACHE_DERIVED,
            })
        }
        MediaRequest::Export { path } => {
            let roots = [core.paths().exports.clone(), core.paths().cache.clone()];
            let p = wa_core::paths::resolve_within(Path::new(&path), &roots)?;
            if !p.is_file() {
                return Err(AppError::not_found("not a file"));
            }
            Ok(Target {
                content_type: guess(&p),
                path: p,
                max_age: CACHE_MEDIA,
            })
        }
    }
}

/// Read `[start, start+len)` of a file (fewer bytes if it shrank meanwhile).
fn read_range(path: &Path, start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity(len.min(MAX_CHUNK) as usize);
    f.take(len).read_to_end(&mut buf)?;
    Ok(buf)
}

async fn serve(
    t: Target,
    range: Option<String>,
    head: bool,
    origin: Option<&str>,
) -> Result<Response<Vec<u8>>, AppError> {
    let p = t.path.clone();
    let len = blocking(move || Ok(std::fs::metadata(&p)?.len())).await?;
    let outcome = match parse_range(range.as_deref(), len) {
        RangeOutcome::Full if len > FULL_LIMIT => RangeOutcome::Partial(ByteRange {
            start: 0,
            end: MAX_CHUNK - 1,
        }),
        RangeOutcome::Partial(r) if r.byte_len() > MAX_CHUNK => RangeOutcome::Partial(ByteRange {
            start: r.start,
            end: r.start + MAX_CHUNK - 1,
        }),
        o => o,
    };
    let ct = safe_content_type(&t.content_type);
    let cache = format!("private, max-age={}", t.max_age);
    let b = |st| {
        base(st, origin)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CACHE_CONTROL, cache.clone())
    };
    let (status, start, want) = match outcome {
        RangeOutcome::Unsatisfiable => {
            return b(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(b"range not satisfiable".to_vec())
                .map_err(|e| AppError::internal("bad response").with_detail(e.to_string()));
        }
        RangeOutcome::Full => (StatusCode::OK, 0, len),
        RangeOutcome::Partial(r) => (StatusCode::PARTIAL_CONTENT, r.start, r.byte_len()),
    };
    let body = if head {
        Vec::new()
    } else {
        let p = t.path.clone();
        blocking(move || Ok(read_range(&p, start, want)?)).await?
    };
    let sent = if head { want } else { body.len() as u64 };
    let mut rb = b(status)
        .header(header::CONTENT_TYPE, ct)
        .header(header::CONTENT_LENGTH, sent.to_string());
    if status == StatusCode::PARTIAL_CONTENT {
        let end = start + sent.max(1) - 1;
        rb = rb.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
    }
    rb.body(body)
        .map_err(|e| AppError::internal("bad response").with_detail(e.to_string()))
}

/// Answer one `wa-media://` request.
pub async fn handle(core: &AppCore, req: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let origin = origin.as_deref();
    let head = match *req.method() {
        Method::GET => false,
        Method::HEAD => true,
        Method::OPTIONS => {
            return base(StatusCode::NO_CONTENT, origin)
                .body(Vec::new())
                .unwrap_or_else(|_| Response::new(Vec::new()));
        }
        _ => return text(StatusCode::METHOD_NOT_ALLOWED, origin, "method not allowed"),
    };
    let Some(parsed) = parse(req.uri().path()) else {
        return text(StatusCode::BAD_REQUEST, origin, "bad wa-media URL");
    };
    let range = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let target = match target_for(core, parsed).await {
        Ok(t) => t,
        Err(e) => return error(&e, origin),
    };
    match serve(target, range, head, origin).await {
        Ok(r) => r,
        Err(e) => error(&e, origin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_types_are_downgraded() {
        assert_eq!(safe_content_type("text/html"), "text/plain; charset=utf-8");
        assert_eq!(safe_content_type("text/plain"), "text/plain; charset=utf-8");
        assert_eq!(safe_content_type("image/jpeg"), "image/jpeg");
        assert_eq!(safe_content_type(""), "application/octet-stream");
    }

    #[test]
    fn origins() {
        assert!(allowed_origin("tauri://localhost"));
        assert!(allowed_origin("http://127.0.0.1:1420"));
        assert!(!allowed_origin("https://evil.example"));
    }

    #[test]
    fn reads_only_the_range() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, b"0123456789").unwrap();
        assert_eq!(read_range(&p, 3, 4).unwrap(), b"3456");
        assert_eq!(read_range(&p, 8, 10).unwrap(), b"89");
    }
}
