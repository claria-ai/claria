//! Durable writer state for report authoring.
//!
//! This crate owns everything the Writing surface keeps in S3: workspace
//! objects and their optimistic-concurrency protocol, immutable revisions,
//! attempt and per-call usage receipts, and the global writer prompt and
//! template libraries. It knows nothing about Bedrock — records are built by
//! the caller and handed here fully formed — so client-record lifecycle code
//! can restore report workspaces without pulling in an LLM client.

mod attempts;
mod error;
pub mod prompt_library;
mod revisions;
pub mod template_library;
mod workspace;

use claria_core::models::report::{ReportContent, ReportDraft, ReportTemplateWarning};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use attempts::{
    ATTEMPT_SCHEMA_VERSION, ReportAttemptMetadata, ReportAttemptStatus, ReportCallUsageRecord,
    persist_attempt, persist_call_usage,
};
pub use error::{ReportFailureCode, ReportStoreError};
pub use revisions::{
    RevisionCache, discard_queued_report_edits, list_report_revisions, load_report_revision,
    revert_report_revision,
};
pub use workspace::{
    LoadedWorkspace, apply_report_template, apply_report_template_for_report,
    delete_report_workspace_for_client, ensure_revision, find_report_workspace,
    list_report_workspaces, load_export_snapshot, load_for_report, load_or_create,
    load_report_workspace, load_report_workspace_by_id, mark_template_current,
    record_report_export, rename_report_session, resolve_report_proposal,
    resolve_report_proposal_for_report, restore_report_workspace_for_client, save_loaded,
    save_report_draft, save_report_draft_for_report, start_report_workspace,
    start_report_workspace_with_id, store_report_template_source, suggested_docx_filename,
};

/// Ceiling for one writer instruction or whole-report guidance message.
/// Public because saved library prompts prefill the instruction box, so
/// their body ceiling is this ceiling.
pub const MAX_INSTRUCTION_CHARACTERS: usize = 20_000;

/// Retained proposal accept/reject receipts per writing session.
pub(crate) const MAX_RESOLUTIONS: usize = 100;

pub const REPORT_CONFLICT_MESSAGE: &str =
    "The report changed on another computer. Reload it before continuing.";

#[derive(Debug, Clone, PartialEq)]
pub struct ReportExportSnapshot {
    pub draft: ReportDraft,
    pub template_source: Option<Vec<u8>>,
    /// The workspace references a template whose stored source object no
    /// longer exists (imports predating v0.22 never retained it). The caller
    /// must surface this instead of silently exporting without formatting.
    pub template_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportRevisionSummary {
    pub revision: u64,
    pub title: String,
    pub updated_at: jiff::Timestamp,
}

#[derive(Debug, Clone)]
pub struct ReportTemplateApplication {
    pub content: ReportContent,
    pub source_sha256: String,
    pub writer_template_id: Uuid,
    pub writer_template_name: String,
    pub warnings: Vec<ReportTemplateWarning>,
}
