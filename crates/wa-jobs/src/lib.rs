//! Background jobs: backfill, phone sync, phone-media linker, conversions,
//! thumbnails — plus the cached `thumbnail` / `office_preview` helpers used by
//! the app's `wa-media://` protocol handler.
//!
//! Every job runs under [`JobManager`]: one per kind (conversions: up to 4,
//! LibreOffice serialised), throttled progress events, persistence through
//! `AppStore`, and cancellation that kills external tools' process groups.

pub mod backfill;
pub mod convert;
pub mod linker;
mod manager;
pub mod phone_sync;
mod thumbs;
pub mod tools;
mod util;

pub use manager::{
    JobCtl, JobDeps, JobEvent, JobEventFn, JobFuture, JobManager, MAX_CONVERT, default_phone_root,
};
pub use thumbs::{THUMB_SIZE, office_preview, office_preview_with, thumbnail, thumbnail_with};
pub use tools::{Tool, ToolOutput, Tools};
pub use util::{media_url, place_unique};
