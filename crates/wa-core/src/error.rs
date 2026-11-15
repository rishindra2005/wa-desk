use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Broad category of a failure. The UI switches on this, so keep it stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorKind {
    NotFound,
    /// Bridge unreachable or WhatsApp disconnected.
    BridgeDown,
    /// Bridge rejected our token.
    Auth,
    /// Path/scope/permission violation.
    Forbidden,
    /// SQLite busy/locked or resource contention.
    Busy,
    Timeout,
    /// Bad input from the caller.
    Invalid,
    /// An external tool (ffmpeg, soffice, rsync, gs, phone) failed.
    External,
    Cancelled,
    Internal,
}

/// The one error type that crosses crate and IPC boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, thiserror::Error)]
#[error("{kind:?}: {message}")]
#[ts(export)]
pub struct AppError {
    pub kind: ErrorKind,
    /// Short, user-presentable message.
    pub message: String,
    /// Optional technical detail (already redacted; never contains the bridge token).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

impl AppError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: None,
        }
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, m)
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, m)
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Forbidden, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, m)
    }
    pub fn external(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::External, m)
    }
    pub fn busy(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Busy, m)
    }
    pub fn timeout(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Timeout, m)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "cancelled")
    }
    pub fn bridge_down(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::BridgeDown, m)
    }
    pub fn auth(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Auth, m)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let kind = match e.kind() {
            K::NotFound => ErrorKind::NotFound,
            K::PermissionDenied => ErrorKind::Forbidden,
            K::TimedOut => ErrorKind::Timeout,
            K::InvalidInput | K::InvalidData => ErrorKind::Invalid,
            _ => ErrorKind::Internal,
        };
        AppError::new(kind, "I/O error").with_detail(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::invalid("JSON error").with_detail(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_snake_case_and_omits_empty_detail() {
        let e = AppError::bridge_down("bridge offline");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "bridge_down");
        assert!(v.get("detail").is_none());
        let e = e.with_detail("conn refused");
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"], "conn refused");
    }

    #[test]
    fn io_errors_map_to_kinds() {
        let e: AppError = std::io::Error::from(std::io::ErrorKind::NotFound).into();
        assert_eq!(e.kind, ErrorKind::NotFound);
        let e: AppError = std::io::Error::from(std::io::ErrorKind::PermissionDenied).into();
        assert_eq!(e.kind, ErrorKind::Forbidden);
    }
}
