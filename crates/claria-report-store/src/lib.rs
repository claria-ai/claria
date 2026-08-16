//! Durable writer state for report authoring.
//!
//! This crate owns everything the Writing surface keeps in S3: workspace
//! objects and their optimistic-concurrency protocol, immutable revisions,
//! resumable drafting runs, attempt and per-call usage receipts, and the
//! global writer prompt and
//! template libraries. It knows nothing about Bedrock — records are built by
//! the caller and handed here fully formed — so client-record lifecycle code
//! can restore report workspaces without pulling in an LLM client.

mod attempts;
mod error;
pub mod prompt_library;
mod revisions;
mod runs;
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
pub use runs::{LoadedRun, create_draft_run, list_draft_runs, load_draft_run, save_draft_run};
pub use workspace::{
    LoadedWorkspace, apply_report_template, apply_report_template_for_report,
    delete_report_workspace_for_client, ensure_no_active_run, ensure_revision,
    find_report_workspace, list_report_workspaces, load_export_snapshot, load_for_report,
    load_or_create, load_report_workspace, load_report_workspace_by_id, mark_template_current,
    record_report_export, rename_report_session, resolve_report_proposal,
    resolve_report_proposal_for_report, restore_report_workspace_for_client, save_loaded,
    save_report_draft, save_report_draft_for_report, start_report_workspace,
    start_report_workspace_with_id, store_report_template_source, suggested_docx_filename,
};

/// Ceiling for one writer instruction or whole-report guidance message. The
/// drafting-run model bounds its own instructions with it, so it is owned
/// there and re-exported here for the instruction box and the saved library
/// prompts that prefill it.
pub use claria_core::models::report_run::MAX_INSTRUCTION_CHARACTERS;

/// Retained proposal accept/reject receipts per writing session.
pub(crate) const MAX_RESOLUTIONS: usize = 100;

pub const REPORT_CONFLICT_MESSAGE: &str =
    "The report changed on another computer. Reload it before continuing.";

/// Refusal shown for every report mutation attempted while a whole-report
/// drafting run owns the session.
pub const ACTIVE_RUN_MESSAGE: &str = "A whole-report draft is currently running for this session. \
Wait for it to finish, or stop it, before changing the report.";

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
