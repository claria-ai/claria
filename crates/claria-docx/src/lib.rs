//! Bounded DOCX import and deterministic rendering for structured reports.
//!
//! This crate has no filesystem or AWS access. It converts untrusted package
//! bytes into a constrained report preview and accepted drafts into OOXML; the
//! desktop controller owns native file selection and local writes.

mod error;
mod import;
mod render;
mod style_catalog;
mod template_render;

pub use error::DocxError;
pub use import::{ImportedTemplate, MAX_TEMPLATE_DOCX_BYTES, TemplateImportStats, import_template};
pub use render::render_report;
pub use template_render::{TemplateRenderFidelity, render_report_with_template};
