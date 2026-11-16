//! Shared contract for wa-desk: domain types (exported to TypeScript via ts-rs),
//! the single error type, timestamp parsing, bridge store-path rules, runtime
//! configuration and the traits that decouple crates from each other.
//!
//! The shared contract; every other crate depends on it. Update `docs/CONTRACT.md` when changing it.

pub mod config;
pub mod error;
pub mod paths;
pub mod time;
pub mod traits;
pub mod types;

pub use error::{AppError, ErrorKind, Result};
