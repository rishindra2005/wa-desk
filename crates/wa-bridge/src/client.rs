//! `BridgeClient`: the only code that talks HTTP to the Go bridge.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use parking_lot::Mutex;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use wa_core::config::Paths;
use wa_core::time::now_epoch;
use wa_core::traits::{
    BridgeApi, BridgeAvatar, BridgeGroup, BridgeNewsletter, BridgeProfile, BridgeSend,
    DownloadOutcome, RefreshStats,
};
use wa_core::types::{BridgeState, ConnectionState};
use wa_core::{AppError, ErrorKind, Result};

use crate::redact::{scrub, truncate};

/// Request timeouts. Defaults: connect 2 s, requests 30 s, downloads 180 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub connect: Duration,
    pub default: Duration,
    pub download: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(2),
            default: Duration::from_secs(30),
            download: Duration::from_secs(180),
        }
    }
}

/// HTTP client for the bridge's REST API. Cheap to share behind an `Arc`.
pub struct BridgeClient {
    http: reqwest::Client,
    base: String,
    token_file: PathBuf,
    token: Mutex<Option<String>>,
    timeouts: Timeouts,
}

impl std::fmt::Debug for BridgeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the cached token.
        f.debug_struct("BridgeClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl BridgeClient {
    /// Client for `paths.bridge_url` authenticating with `paths.token_file`.
    pub fn new(paths: &Paths) -> Result<Self> {
        Self::with_timeouts(&paths.bridge_url, &paths.token_file, Timeouts::default())
    }

    /// Client with explicit base URL, token file and timeouts.
    /// The URL must be loopback (`127.0.0.1`, `localhost` or `[::1]`).
    pub fn with_timeouts(base_url: &str, token_file: &Path, timeouts: Timeouts) -> Result<Self> {
        let base = loopback_base(base_url)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.default)
            .build()
            .map_err(|e| {
                AppError::internal("could not build HTTP client").with_detail(e.to_string())
            })?;
        Ok(Self {
            http,
            base,
            token_file: token_file.to_path_buf(),
            token: Mutex::new(None),
            timeouts,
        })
    }

    /// Base URL in use (scheme://host:port, no trailing slash).
    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// GET /api/health. `BridgeDown` when WhatsApp is disconnected (503).
    pub async fn health(&self) -> Result<Value> {
        self.request(Method::GET, "/api/health", None, self.timeouts.default)
            .await
    }

    fn read_token(&self) -> Result<String> {
        let raw = std::fs::read_to_string(&self.token_file).map_err(|e| {
            AppError::auth("Bridge token file is missing or unreadable")
                .with_detail(format!("{:?}", e.kind()))
        })?;
        let t = raw.trim();
        if t.is_empty() {
            return Err(AppError::auth("Bridge token file is empty"));
        }
        Ok(t.to_string())
    }

    fn token(&self, refresh: bool) -> Result<String> {
        let mut guard = self.token.lock();
        if !refresh && let Some(t) = guard.as_ref() {
            return Ok(t.clone());
        }
        let t = self.read_token()?;
        *guard = Some(t.clone());
        Ok(t)
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value> {
        let mut token = self.token(false)?;
        let mut attempt = 0;
        loop {
            let mut rb = self
                .http
                .request(method.clone(), format!("{}{path}", self.base))
                .bearer_auth(&token)
                .timeout(timeout);
            if let Some(b) = body {
                rb = rb.json(b);
            }
            let resp = rb.send().await.map_err(map_reqwest)?;
            let status = resp.status();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                tracing::debug!(path, "bridge returned 401; re-reading token file");
                attempt += 1;
                token = self.token(true)?;
                continue;
            }
            let text = resp.text().await.map_err(map_reqwest)?;
            tracing::trace!(path, status = status.as_u16(), "bridge response");
            return interpret(status, &text, &token);
        }
    }
}

/// Timeout for metadata reads (profile, group, newsletter, avatar, refresh).
pub const META_TIMEOUT: Duration = Duration::from_secs(10);

/// `?jid=<percent-encoded>` query string.
fn jid_query(path: &str, jid: &str) -> Result<String> {
    let jid = jid.trim();
    if jid.is_empty() || !jid.contains('@') {
        return Err(AppError::invalid("a full JID (user@server) is required"));
    }
    let enc: String = url::form_urlencoded::byte_serialize(jid.as_bytes()).collect();
    Ok(format!("{path}?jid={enc}"))
}

/// Decode a JSON body into a typed struct (lenient: unknown fields ignored).
fn decode<T: serde::de::DeserializeOwned>(v: Value, what: &str) -> Result<T> {
    serde_json::from_value(v).map_err(|e| {
        AppError::external(format!("Unexpected {what} response from the bridge"))
            .with_detail(e.to_string())
    })
}

/// Validate the base URL is loopback and normalise it to `scheme://host:port`.
fn loopback_base(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw.trim())
        .map_err(|_| AppError::invalid("bridge URL is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::invalid("bridge URL must be http(s)"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::invalid("bridge URL must not contain credentials"));
    }
    let host = match url.host() {
        Some(url::Host::Ipv4(ip)) if ip == std::net::Ipv4Addr::LOCALHOST => "127.0.0.1".to_string(),
        Some(url::Host::Ipv6(ip)) if ip == std::net::Ipv6Addr::LOCALHOST => "[::1]".to_string(),
        Some(url::Host::Domain(d)) if d.eq_ignore_ascii_case("localhost") => {
            "localhost".to_string()
        }
        _ => {
            return Err(AppError::forbidden(
                "bridge URL must point at this machine (127.0.0.1, localhost or [::1])",
            ));
        }
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| AppError::invalid("bridge URL needs a port"))?;
    Ok(format!("{}://{host}:{port}", url.scheme()))
}

fn map_reqwest(e: reqwest::Error) -> AppError {
    // reqwest's Display never includes request headers, so the token cannot leak here.
    let detail = e.to_string();
    if e.is_timeout() {
        AppError::timeout("The WhatsApp bridge did not answer in time").with_detail(detail)
    } else if e.is_connect() {
        AppError::bridge_down("The WhatsApp bridge is not running").with_detail(detail)
    } else if e.is_decode() || e.is_body() {
        AppError::external("Could not read the bridge response").with_detail(detail)
    } else {
        AppError::bridge_down("Could not reach the WhatsApp bridge").with_detail(detail)
    }
}

/// Bridge message from a JSON error/success body (`message` or `error`).
fn json_message(v: &Value) -> Option<String> {
    ["message", "error"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

fn interpret(status: StatusCode, text: &str, token: &str) -> Result<Value> {
    let parsed: Option<Value> = if text.trim().is_empty() {
        Some(Value::Null)
    } else {
        serde_json::from_str(text).ok()
    };
    if status.is_success() {
        let v = parsed.ok_or_else(|| {
            AppError::external("Bridge returned an unexpected response")
                .with_detail(scrub(text, Some(token)))
        })?;
        if v.get("success").and_then(Value::as_bool) == Some(false) {
            let m = json_message(&v).unwrap_or_else(|| "Bridge reported a failure".into());
            return Err(AppError::external(scrub(&m, Some(token))));
        }
        return Ok(v);
    }
    let body_msg = match &parsed {
        Some(v) => json_message(v).map(|m| scrub(&m, Some(token))),
        None => None,
    };
    let plain = scrub(text, Some(token));
    let short = body_msg
        .clone()
        .unwrap_or_else(|| truncate(&plain, 200))
        .trim()
        .to_string();
    let with = |default: &str| {
        if short.is_empty() {
            default.to_string()
        } else {
            short.clone()
        }
    };
    let err = match status {
        StatusCode::UNAUTHORIZED => AppError::auth("The bridge rejected wa-desk's access token"),
        StatusCode::FORBIDDEN => AppError::forbidden(with("The bridge refused the request")),
        StatusCode::NOT_FOUND => AppError::not_found(with("Not found on the bridge")),
        StatusCode::SERVICE_UNAVAILABLE => AppError::bridge_down(with("WhatsApp is not connected")),
        StatusCode::CONFLICT => AppError::invalid("That call is no longer ringing"),
        StatusCode::GATEWAY_TIMEOUT | StatusCode::REQUEST_TIMEOUT => {
            AppError::timeout(with("The bridge timed out"))
        }
        s => AppError::new(
            ErrorKind::External,
            if let Some(m) = body_msg.filter(|m| !m.trim().is_empty()) {
                m
            } else if short.is_empty() {
                format!("Bridge error (HTTP {})", s.as_u16())
            } else {
                format!("Bridge error (HTTP {}): {short}", s.as_u16())
            },
        ),
    };
    Err(if plain.is_empty() {
        err
    } else {
        err.with_detail(plain)
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct RawConnection {
    state: String,
    connected: bool,
    paired: bool,
    message: String,
    qr_code: Option<String>,
    qr_expires_at: Option<String>,
    last_error: Option<RawError>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawError {
    code: String,
    message: String,
}

fn parse_connection(v: Value) -> ConnectionState {
    let raw: RawConnection = serde_json::from_value(v).unwrap_or_default();
    let qr_svg = raw
        .qr_code
        .as_deref()
        .filter(|q| !q.is_empty())
        .and_then(qr_svg_data_url);
    let qr_expires_at = raw
        .qr_expires_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp());
    let last_error = raw
        .last_error
        .map(|e| match (e.message.is_empty(), e.code.is_empty()) {
            (false, false) => format!("{} ({})", e.message, e.code),
            (false, true) => e.message,
            _ => e.code,
        });
    ConnectionState {
        state: BridgeState::from_bridge(&raw.state),
        connected: raw.connected,
        paired: raw.paired,
        message: raw.message,
        qr_svg,
        qr_expires_at,
        last_error: last_error.filter(|s| !s.is_empty()),
        checked_at: now_epoch(),
    }
}

fn unreachable_state(e: &AppError) -> ConnectionState {
    let state = match e.kind {
        ErrorKind::BridgeDown | ErrorKind::Timeout => BridgeState::Unreachable,
        _ => BridgeState::Error,
    };
    ConnectionState {
        state,
        connected: false,
        paired: false,
        message: e.message.clone(),
        qr_svg: None,
        qr_expires_at: None,
        last_error: Some(e.message.clone()),
        checked_at: now_epoch(),
    }
}

/// Render pairing-QR text as an SVG `data:` URL (None if it cannot be encoded).
pub fn qr_svg_data_url(data: &str) -> Option<String> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .build();
    let b64 = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    Some(format!("data:image/svg+xml;base64,{b64}"))
}

#[async_trait]
impl BridgeApi for BridgeClient {
    async fn connection(&self) -> ConnectionState {
        match self
            .request(Method::GET, "/api/connection", None, self.timeouts.default)
            .await
        {
            Ok(v) => parse_connection(v),
            Err(e) => unreachable_state(&e),
        }
    }

    async fn reconnect(&self) -> Result<()> {
        self.request(
            Method::POST,
            "/api/connection/reconnect",
            None,
            self.timeouts.default,
        )
        .await
        .map(|_| ())
    }

    async fn send(&self, req: BridgeSend) -> Result<String> {
        if req.recipient.trim().is_empty() {
            return Err(AppError::invalid("recipient is required"));
        }
        let text = req.message.unwrap_or_default();
        if text.is_empty() && req.media_path.is_none() {
            return Err(AppError::invalid("message text or attachment is required"));
        }
        let media = req
            .media_path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let body = json!({
            "recipient": req.recipient,
            "message": text,
            "media_path": media,
            "quoted_message_id": req.quoted_message_id.unwrap_or_default(),
            "quoted_sender_jid": req.quoted_sender_jid.unwrap_or_default(),
            "quoted_content": req.quoted_content.unwrap_or_default(),
            "mentions": req.mentions,
        });
        let v = self
            .request(
                Method::POST,
                "/api/send",
                Some(&body),
                self.timeouts.download,
            )
            .await?;
        Ok(json_message(&v).unwrap_or_else(|| "sent".into()))
    }

    async fn react(
        &self,
        chat_jid: &str,
        message_id: &str,
        emoji: &str,
        from_me: bool,
        sender_jid: Option<&str>,
    ) -> Result<()> {
        let body = json!({
            "recipient": chat_jid,
            "message_id": message_id,
            "emoji": emoji,
            "from_me": from_me,
            "sender_jid": sender_jid.unwrap_or_default(),
        });
        self.request(
            Method::POST,
            "/api/react",
            Some(&body),
            self.timeouts.default,
        )
        .await
        .map(|_| ())
    }

    async fn download(
        &self,
        chat_jid: &str,
        message_id: &str,
        no_retry: bool,
    ) -> Result<DownloadOutcome> {
        let body = json!({"message_id": message_id, "chat_jid": chat_jid, "no_retry": no_retry});
        let v = self
            .request(
                Method::POST,
                "/api/download",
                Some(&body),
                self.timeouts.download,
            )
            .await?;
        let path = v.get("path").and_then(Value::as_str).unwrap_or_default();
        if path.is_empty() {
            return Err(AppError::external("The bridge did not return a file path"));
        }
        let filename = v
            .get("filename")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        Ok(DownloadOutcome {
            path: PathBuf::from(path),
            filename,
        })
    }

    async fn mark_read(
        &self,
        chat_jid: &str,
        message_ids: &[String],
        sender_jid: Option<&str>,
    ) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let mut body = json!({"chat_jid": chat_jid, "message_ids": message_ids});
        if let Some(s) = sender_jid.filter(|s| !s.is_empty()) {
            body["sender_jid"] = json!(s);
        }
        self.request(
            Method::POST,
            "/api/mark-read",
            Some(&body),
            self.timeouts.default,
        )
        .await
        .map(|_| ())
    }

    async fn profile(&self, jid: &str) -> Result<BridgeProfile> {
        self.profile_impl(jid).await
    }

    async fn group(&self, jid: &str) -> Result<BridgeGroup> {
        self.group_impl(jid).await
    }

    async fn newsletter(&self, jid: &str) -> Result<BridgeNewsletter> {
        self.newsletter_impl(jid).await
    }

    async fn avatar(&self, jid: &str, full: bool) -> Result<BridgeAvatar> {
        self.avatar_impl(jid, full).await
    }

    async fn refresh_chats(&self) -> Result<RefreshStats> {
        self.refresh_impl().await
    }

    async fn reject_call(&self, call_id: &str, caller_jid: &str) -> Result<()> {
        if call_id.trim().is_empty() || !caller_jid.contains('@') {
            return Err(AppError::invalid(
                "a call id and the caller's JID are required",
            ));
        }
        let body = json!({"call_id": call_id, "caller_jid": caller_jid});
        self.request(
            Method::POST,
            "/api/call/reject",
            Some(&body),
            self.timeouts.default,
        )
        .await
        .map(|_| ())
    }
}

impl BridgeClient {
    async fn get_meta<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        jid: &str,
        what: &str,
    ) -> Result<T> {
        let v = self
            .request(Method::GET, &jid_query(path, jid)?, None, META_TIMEOUT)
            .await?;
        decode(v, what)
    }

    async fn profile_impl(&self, jid: &str) -> Result<BridgeProfile> {
        self.get_meta("/api/profile", jid, "profile").await
    }

    async fn group_impl(&self, jid: &str) -> Result<BridgeGroup> {
        self.get_meta("/api/group", jid, "group").await
    }

    async fn newsletter_impl(&self, jid: &str) -> Result<BridgeNewsletter> {
        self.get_meta("/api/newsletter", jid, "channel").await
    }

    async fn avatar_impl(&self, jid: &str, full: bool) -> Result<BridgeAvatar> {
        let path = format!(
            "{}&full={}",
            jid_query("/api/avatar", jid)?,
            if full { 1 } else { 0 }
        );
        let v = self.request(Method::GET, &path, None, META_TIMEOUT).await?;
        let p = v.get("path").and_then(Value::as_str).unwrap_or_default();
        if p.is_empty() {
            return Err(AppError::not_found("no picture"));
        }
        Ok(BridgeAvatar {
            path: PathBuf::from(p),
            picture_id: v
                .get("picture_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    async fn refresh_impl(&self) -> Result<RefreshStats> {
        let v = self
            .request(
                Method::POST,
                "/api/chats/refresh",
                None,
                self.timeouts.default,
            )
            .await?;
        decode(v, "refresh")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jid_queries_are_encoded() {
        assert_eq!(
            jid_query("/api/profile", " 9198@s.whatsapp.net ").unwrap(),
            "/api/profile?jid=9198%40s.whatsapp.net"
        );
        assert_eq!(
            jid_query("/api/group", "a&b=c@g.us").unwrap(),
            "/api/group?jid=a%26b%3Dc%40g.us"
        );
        assert!(jid_query("/api/profile", "").is_err());
        assert!(jid_query("/api/profile", "9198").is_err());
    }

    #[test]
    fn accepts_only_loopback() {
        assert_eq!(
            loopback_base("http://127.0.0.1:8080/").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            loopback_base("http://LOCALHOST:9").unwrap(),
            "http://localhost:9"
        );
        assert_eq!(
            loopback_base("http://[::1]:8080").unwrap(),
            "http://[::1]:8080"
        );
        for bad in [
            "http://192.168.1.2:8080",
            "http://127.0.0.2:8080",
            "http://example.com:8080",
            "http://localhost.evil.com:8080",
            "ftp://127.0.0.1:21",
            "http://u:p@127.0.0.1:8080",
            "not a url",
        ] {
            assert!(loopback_base(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn interpret_maps_statuses() {
        let k = |s: u16, body: &str| {
            interpret(StatusCode::from_u16(s).unwrap(), body, "tok")
                .unwrap_err()
                .kind
        };
        assert_eq!(k(401, "Unauthorized"), ErrorKind::Auth);
        assert_eq!(k(403, "Forbidden: host not allowed"), ErrorKind::Forbidden);
        assert_eq!(k(404, "404 page not found"), ErrorKind::NotFound);
        assert_eq!(k(503, "{\"success\":false}"), ErrorKind::BridgeDown);
        assert_eq!(k(500, "boom"), ErrorKind::External);
        assert_eq!(k(400, "Invalid request format"), ErrorKind::External);
        let e =
            interpret(StatusCode::OK, r#"{"success":false,"message":"nope"}"#, "t").unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ErrorKind::External, "nope"));
        assert_eq!(interpret(StatusCode::OK, "", "t").unwrap(), Value::Null);
        assert!(interpret(StatusCode::OK, "<html>", "t").is_err());
    }

    #[test]
    fn json_error_message_is_used() {
        let e = interpret(
            StatusCode::FORBIDDEN,
            r#"{"success":false,"message":"media_path rejected: outside outbox"}"#,
            "t",
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Forbidden);
        assert!(e.message.contains("media_path rejected"), "{e:?}");
        let e = interpret(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"send failed"}"#,
            "t",
        )
        .unwrap_err();
        assert!(e.message.contains("send failed"));
    }

    #[test]
    fn connection_parsing_is_lenient() {
        let s = parse_connection(json!({
            "state": "pairing", "connected": false, "paired": false, "message": "Scan",
            "qrCode": "2@abc,def", "qrExpiresAt": "2026-11-16T10:00:00.123+05:30",
            "updatedAt": "2026-11-16T10:00:00Z", "lastError": {"code": "x", "message": "boom"}
        }));
        assert_eq!(s.state, BridgeState::Pairing);
        assert!(s.qr_svg.unwrap().starts_with("data:image/svg+xml;base64,"));
        assert_eq!(s.qr_expires_at, Some(1_794_803_400));
        assert_eq!(s.last_error.as_deref(), Some("boom (x)"));
        let s = parse_connection(json!({"weird": 1}));
        assert_eq!(s.state, BridgeState::Error);
        assert!(s.qr_svg.is_none() && s.last_error.is_none());
        let s = parse_connection(
            json!({"state":"connected","connected":true,"qrCode":null,"lastError":null}),
        );
        assert!(s.connected && s.qr_svg.is_none());
    }

    #[test]
    fn debug_never_shows_token() {
        let dir = tempfile::tempdir().unwrap();
        let tf = dir.path().join(".bridge-token");
        std::fs::write(&tf, "secret-token-value\n").unwrap();
        let c =
            BridgeClient::with_timeouts("http://127.0.0.1:1", &tf, Timeouts::default()).unwrap();
        assert_eq!(c.token(false).unwrap(), "secret-token-value");
        assert!(!format!("{c:?}").contains("secret"));
    }

    #[test]
    fn token_handles_crlf_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tf = dir.path().join(".bridge-token");
        std::fs::write(&tf, "abc\r\n").unwrap();
        let c =
            BridgeClient::with_timeouts("http://127.0.0.1:1", &tf, Timeouts::default()).unwrap();
        assert_eq!(c.token(true).unwrap(), "abc");
        std::fs::write(&tf, " \n").unwrap();
        assert_eq!(c.token(true).unwrap_err().kind, ErrorKind::Auth);
        std::fs::remove_file(&tf).unwrap();
        assert_eq!(c.token(true).unwrap_err().kind, ErrorKind::Auth);
    }
}
