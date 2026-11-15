//! HTTP client for the Go WhatsApp bridge plus outbox staging.
//!
//! - [`BridgeClient`] implements [`wa_core::traits::BridgeApi`] over the
//!   bridge's loopback REST API (bearer token + strict Host header).
//! - [`outbox`] copies user-chosen files into the only directory the bridge
//!   accepts `media_path` from.
//!
//! The bridge token is never logged, never put in an error and never leaves
//! this crate.

mod client;
pub mod outbox;
mod redact;

pub use client::{BridgeClient, Timeouts, qr_svg_data_url};
