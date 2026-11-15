//! Rule engine for incoming WhatsApp messages with loop and rate protection.
//!
//! [`Engine::process`] runs enabled rules on new rows. Guards: master switch,
//! own/reaction/deleted/status messages, watermark and 10-minute age limit,
//! explicit opt-in for groups (replies there only when @mentioned), a
//! per-rule-per-chat cooldown and a global hourly send cap.

mod engine;
pub mod rules;

pub use engine::{Clock, Engine, MAX_AGE_SECS, WATERMARK_KEY};
pub use rules::{render, validate_rule};
