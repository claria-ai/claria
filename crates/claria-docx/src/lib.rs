//! Deterministic local DOCX rendering for accepted report drafts.
//!
//! This crate has no filesystem or AWS access. It converts one structured
//! accepted draft into OOXML bytes; the desktop controller decides where the
//! user saves those bytes.

mod error;
mod render;

pub use error::DocxError;
pub use render::render_report;
