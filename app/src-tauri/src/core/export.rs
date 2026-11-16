//! `export_chat` (txt / json / html / pdf) and `copy_messages_text`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;
use wa_core::types::{ExportFormat, ExportReq, MediaKind, Message, MessagesQuery};
use wa_core::{AppError, Result};

use super::{AppCore, blocking, require};

/// Most messages in one export.
pub const EXPORT_MAX: usize = 200_000;
/// Most messages copied to the clipboard at once.
pub const COPY_MAX: usize = 1000;

/// "dd/mm/yy, HH:MM" in local time.
pub fn wa_time(ts: i64) -> String {
    use chrono::TimeZone as _;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(t) => t.format("%d/%m/%y, %H:%M").to_string(),
        None => "??/??/??, ??:??".into(),
    }
}

/// The text of one message as WhatsApp exports it (media as `<attached: …>`).
pub fn body_text(m: &Message) -> String {
    if m.deleted {
        return "This message was deleted".into();
    }
    let mut parts = Vec::new();
    if let Some(r) = &m.media {
        let name = r.filename.clone().unwrap_or_else(|| match r.kind {
            MediaKind::Image => "image".into(),
            MediaKind::Video => "video".into(),
            MediaKind::Audio => "audio".into(),
            MediaKind::Sticker => "sticker".into(),
            MediaKind::Document => "document".into(),
        });
        parts.push(format!("<attached: {name}>"));
    }
    if !m.content.trim().is_empty() {
        parts.push(m.content.clone());
    }
    parts.join(" ")
}

/// `[dd/mm/yy, HH:MM] Name: text` lines.
pub fn whatsapp_lines(msgs: &[Message]) -> String {
    let mut out = String::new();
    for m in msgs {
        let _ = writeln!(
            out,
            "[{}] {}: {}",
            wa_time(m.ts),
            m.sender_name,
            body_text(m)
        );
    }
    out
}

/// Escape text for HTML element content and attribute values.
pub fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// File-name-safe version of a chat name (never empty).
pub fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let t = cleaned.trim().trim_matches('.').trim();
    let t: String = t.chars().take(80).collect();
    if t.is_empty() { "chat".into() } else { t }
}

const CSS: &str = "body{font:15px/1.45 system-ui,-apple-system,'Segoe UI',Roboto,'Noto Sans',sans-serif;\
background:#efeae2;color:#111b21;margin:0}header{background:#075e54;color:#fff;padding:16px 24px}\
header h1{margin:0;font-size:20px}header p{margin:4px 0 0;opacity:.8;font-size:13px}\
main{max-width:860px;margin:0 auto;padding:16px}.day{text-align:center;margin:16px 0 8px}\
.day span{background:#fff;border-radius:8px;padding:3px 10px;font-size:12px;color:#54656f}\
.m{max-width:75%;margin:4px 0;padding:6px 10px 4px;border-radius:8px;background:#fff;\
box-shadow:0 1px .5px rgba(0,0,0,.13);word-wrap:break-word;overflow-wrap:anywhere}\
.me{margin-left:auto;background:#d9fdd3}.s{font-size:12.5px;font-weight:600;color:#1f7aad}\
.t{font-size:11px;color:#667781;text-align:right}.del{font-style:italic;color:#667781}\
.m img{max-width:100%;border-radius:6px;display:block;margin:4px 0}.att{font-size:13px}\
@media print{body{background:#fff}.m{box-shadow:none;border:1px solid #ddd}}";

/// One media file copied next to an HTML export.
pub struct CopiedMedia {
    pub rel: String,
    pub is_image: bool,
}

/// Self-contained HTML; `media(msg)` returns copied attachments.
pub fn render_html(
    chat_name: &str,
    msgs: &[Message],
    media: &dyn Fn(&Message) -> Option<CopiedMedia>,
) -> String {
    let mut h = String::with_capacity(msgs.len() * 200 + 2048);
    let _ = write!(
        h,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'\">\
<title>{}</title><style>{CSS}</style></head><body><header><h1>{}</h1><p>{} messages · exported by wa-desk</p></header><main>",
        html_escape(chat_name),
        html_escape(chat_name),
        msgs.len()
    );
    let mut last_day = String::new();
    for m in msgs {
        let t = wa_time(m.ts);
        let (day, time) = t.split_once(", ").unwrap_or((&t, ""));
        if day != last_day {
            let _ = write!(
                h,
                "<div class=\"day\"><span>{}</span></div>",
                html_escape(day)
            );
            last_day = day.to_string();
        }
        let cls = if m.is_from_me { "m me" } else { "m" };
        let _ = write!(
            h,
            "<div class=\"{cls}\"><div class=\"s\">{}</div>",
            html_escape(&m.sender_name)
        );
        if m.deleted {
            h.push_str("<div class=\"del\">This message was deleted</div>");
        } else {
            if let Some(r) = &m.media {
                let name = r.filename.clone().unwrap_or_else(|| "attachment".into());
                match media(m) {
                    Some(c) if c.is_image => {
                        let _ = write!(
                            h,
                            "<a href=\"{0}\"><img src=\"{0}\" alt=\"{1}\"></a>",
                            html_escape(&c.rel),
                            html_escape(&name)
                        );
                    }
                    Some(c) => {
                        let _ = write!(
                            h,
                            "<div class=\"att\">📎 <a href=\"{}\">{}</a></div>",
                            html_escape(&c.rel),
                            html_escape(&name)
                        );
                    }
                    None => {
                        let _ = write!(h, "<div class=\"att\">📎 {}</div>", html_escape(&name));
                    }
                }
            }
            if !m.content.trim().is_empty() {
                let _ = write!(
                    h,
                    "<div>{}</div>",
                    html_escape(&m.content).replace('\n', "<br>")
                );
            }
        }
        let _ = write!(h, "<div class=\"t\">{}</div></div>", html_escape(time));
    }
    h.push_str("</main></body></html>\n");
    h
}

/// Local timestamp for export file names.
fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H%M").to_string()
}

impl AppCore {
    /// Messages for an export: selection, else `[after, before)` (whole chat by default).
    async fn export_messages(&self, req: &ExportReq) -> Result<Vec<Message>> {
        let db = self.db.clone();
        let (chat, ids, after, before) = (
            req.chat_jid.clone(),
            req.message_ids.clone(),
            req.after,
            req.before,
        );
        blocking(move || {
            if !ids.is_empty() {
                return db.messages_by_ids(&chat, &ids);
            }
            let mut out: Vec<Message> = Vec::new();
            let mut cursor = Some(format!("{}:{}", after.unwrap_or(i64::MIN), i64::MIN));
            while let Some(c) = cursor {
                let p = db.get_messages(&MessagesQuery {
                    chat_jid: chat.clone(),
                    before: None,
                    after: Some(c),
                    limit: Some(500),
                })?;
                let done = p.items.iter().any(|m| before.is_some_and(|b| m.ts >= b));
                out.extend(
                    p.items
                        .into_iter()
                        .filter(|m| before.is_none_or(|b| m.ts < b)),
                );
                if done || out.len() >= EXPORT_MAX {
                    break;
                }
                cursor = p.next_cursor;
            }
            out.truncate(EXPORT_MAX);
            Ok(out)
        })
        .await
    }

    /// `export_chat`: writes into the exports folder and returns the path.
    pub async fn export_chat(&self, req: ExportReq) -> Result<String> {
        require("chat", &req.chat_jid)?;
        if let (Some(a), Some(b)) = (req.after, req.before)
            && a >= b
        {
            return Err(AppError::invalid(
                "The start of the range must be before its end",
            ));
        }
        let chat = self.get_chat(req.chat_jid.clone()).await?;
        let msgs = self.export_messages(&req).await?;
        if msgs.is_empty() {
            return Err(AppError::not_found("There are no messages to export"));
        }
        let base = format!("{} {}", safe_name(&chat.name), stamp());
        let exports = self.paths.exports.clone();
        std::fs::create_dir_all(&exports)?;
        let out = match req.format {
            ExportFormat::Txt => {
                let text = whatsapp_lines(&msgs);
                self.write_unique(&base, "txt", text.into_bytes()).await?
            }
            ExportFormat::Json => {
                let v = serde_json::to_vec_pretty(&msgs).map_err(|e| {
                    AppError::internal("could not encode the export").with_detail(e.to_string())
                })?;
                self.write_unique(&base, "json", v).await?
            }
            ExportFormat::Html if req.include_media => {
                self.export_html_folder(&chat.name, &base, &msgs).await?
            }
            ExportFormat::Html => {
                let html = render_html(&chat.name, &msgs, &|_| None);
                self.write_unique(&base, "html", html.into_bytes()).await?
            }
            ExportFormat::Pdf => self.export_pdf(&chat.name, &base, &msgs).await?,
        };
        tracing::info!(format = ?req.format, messages = msgs.len(), "chat exported");
        Ok(out.to_string_lossy().into_owned())
    }

    async fn write_unique(&self, base: &str, ext: &str, data: Vec<u8>) -> Result<PathBuf> {
        let (cache, exports, base, ext) = (
            self.paths.cache.join("tmp"),
            self.paths.exports.clone(),
            base.to_string(),
            ext.to_string(),
        );
        blocking(move || {
            std::fs::create_dir_all(&cache)?;
            let tmp = cache.join(format!("export-{}.{ext}", std::process::id()));
            let tmp = unique_tmp(&tmp);
            std::fs::write(&tmp, &data)?;
            let r = wa_jobs::place_unique(&tmp, &exports, &base, &ext);
            let _ = std::fs::remove_file(&tmp);
            r
        })
        .await
    }

    /// HTML + `<name>_media/` inside a new folder `exports/<name>/`.
    async fn export_html_folder(
        &self,
        chat_name: &str,
        base: &str,
        msgs: &[Message],
    ) -> Result<PathBuf> {
        let mut dir = self.paths.exports.join(base);
        let mut n = 1;
        while dir.exists() {
            n += 1;
            dir = self.paths.exports.join(format!("{base} ({n})"));
        }
        let media_name = format!("{}_media", safe_name(chat_name));
        let media_dir = dir.join(&media_name);
        std::fs::create_dir_all(&media_dir)?;
        let mut copied = std::collections::HashMap::new();
        for m in msgs.iter().filter(|m| m.media.is_some() && !m.deleted) {
            let Some(src) = self.resolve_media(&m.chat_jid, &m.id).await.ok().flatten() else {
                continue;
            };
            let r = m.media.as_ref();
            let fname = safe_name(
                &r.and_then(|r| r.filename.clone())
                    .unwrap_or_else(|| m.id.clone()),
            );
            let (stem, ext) = match fname.rsplit_once('.') {
                Some((s, e)) if !s.is_empty() => (s.to_string(), e.to_string()),
                _ => (fname.clone(), String::new()),
            };
            let md = media_dir.clone();
            let placed = blocking(move || wa_jobs::place_unique(&src, &md, &stem, &ext)).await;
            if let Ok(p) = placed
                && let Some(file) = p.file_name()
            {
                let rel = format!("{media_name}/{}", file.to_string_lossy());
                let is_image = matches!(
                    r.map(|r| r.kind),
                    Some(MediaKind::Image | MediaKind::Sticker)
                );
                copied.insert(m.id.clone(), (rel, is_image));
            }
        }
        let html = render_html(chat_name, msgs, &|m| {
            copied.get(&m.id).map(|(rel, img)| CopiedMedia {
                rel: rel.clone(),
                is_image: *img,
            })
        });
        let page = dir.join(format!("{}.html", safe_name(chat_name)));
        let p2 = page.clone();
        blocking(move || Ok(std::fs::write(&p2, html)?)).await?;
        Ok(dir)
    }

    /// HTML rendered to PDF by LibreOffice.
    async fn export_pdf(&self, chat_name: &str, base: &str, msgs: &[Message]) -> Result<PathBuf> {
        let html = render_html(chat_name, msgs, &|_| None);
        let work = self
            .paths
            .cache
            .join("tmp")
            .join(format!("export-pdf-{}", uuid_like()));
        std::fs::create_dir_all(&work)?;
        let src = work.join("chat.html");
        std::fs::write(&src, html)?;
        let pdf = work.join("chat.pdf");
        let r = wa_jobs::html_to_pdf(
            &self.paths,
            &wa_jobs::Tools::default(),
            &src,
            &pdf,
            &CancellationToken::new(),
        )
        .await;
        let out = match r {
            Ok(()) => {
                let (exports, base, pdf2) =
                    (self.paths.exports.clone(), base.to_string(), pdf.clone());
                blocking(move || wa_jobs::place_unique(&pdf2, &exports, &base, "pdf")).await
            }
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_dir_all(&work);
        out
    }

    /// `copy_messages_text`: WhatsApp-style lines for the clipboard.
    pub async fn copy_messages_text(
        &self,
        chat_jid: String,
        message_ids: Vec<String>,
    ) -> Result<String> {
        require("chat", &chat_jid)?;
        if message_ids.is_empty() {
            return Err(AppError::invalid("Select at least one message"));
        }
        if message_ids.len() > COPY_MAX {
            return Err(AppError::invalid("Too many messages selected (max 1000)"));
        }
        let db = self.db.clone();
        let msgs = blocking(move || db.messages_by_ids(&chat_jid, &message_ids)).await?;
        if msgs.is_empty() {
            return Err(AppError::not_found("Message not found"));
        }
        if msgs.len() == 1 && !msgs[0].deleted && msgs[0].media.is_none() {
            // A single text message copies as plain text, like WhatsApp.
            return Ok(msgs[0].content.clone());
        }
        Ok(whatsapp_lines(&msgs).trim_end().to_string())
    }
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{n}", std::process::id())
}

fn unique_tmp(p: &Path) -> PathBuf {
    p.with_file_name(format!(
        "{}-{}",
        uuid_like(),
        p.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_and_names() {
        assert_eq!(
            html_escape("<script>alert('x')</script> & \"q\""),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;q&quot;"
        );
        assert_eq!(safe_name("ML/CSE: 3C*"), "ML_CSE_ 3C_");
        assert_eq!(safe_name("../.."), "_");
        assert_eq!(safe_name(""), "chat");
        assert_eq!(safe_name("नमस्ते 🙏"), "नमस्ते 🙏");
    }
}
