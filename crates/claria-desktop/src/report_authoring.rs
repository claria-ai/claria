//! Tauri-facing report-authoring view models.
//!
//! Workflow policy and AWS orchestration live in `claria`.
//! The report domain types (`ReportDraft`, `ReportContent`, blocks,
//! operations, exports, resolutions) derive `specta::Type` in `claria-core`
//! and cross the IPC boundary as-is; this module keeps only the views that
//! genuinely differ from their domain type — flattening, redaction, derived
//! timelines — each carrying a comment saying why it earns its life.

use std::collections::{HashMap, HashSet};

use claria_core::models::{
    findings::ReportFindings,
    report::{
        ReportBlock, ReportContent, ReportDraft, ReportExport, ReportExportStatus, ReportOperation,
        ReportProposal, ReportProposalDecision as CoreProposalDecision, ReportProposalResolution,
        ReportProtocolBlock, ReportProtocolRole, ReportSection, ReportTemplateImport,
        ReportTemplateWarning, ReportTemplateWarningCode, ReportToolResultStatus, ReportWorkspace,
        report_template_placeholder_count,
    },
    report_run::{
        DraftRun, DraftRunStatus, EvidenceRef, PlanEntry as DraftPlanEntry, RecordCitation,
        RunInstruction, RunRecordSnapshot, RunSection, RunSectionRecords, RunSectionState,
        SectionIntent,
    },
    turn_usage::TurnUsage,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

/// Earns its life over `ReportWorkspace`: flattens the session container and
/// replaces raw turns with derived timeline/context views.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportWorkspaceView {
    pub schema_version: u32,
    pub report_id: String,
    pub client_id: String,
    pub session_name: String,
    pub draft: ReportDraft,
    pub turns: Vec<ReportAuthoringTurnView>,
    pub pending_proposal: Option<ReportProposalView>,
    pub resolutions: Vec<ReportProposalResolution>,
    pub last_agent_revision: Option<u64>,
    pub last_export: Option<ReportExport>,
    pub template_import: Option<ReportTemplateImportView>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct EditorHistoryEntry {
    pub report_id: String,
    pub name: String,
    pub title: String,
    pub revision: u64,
    pub turn_count: u32,
    pub updated_at: String,
    pub last_export: Option<ReportExport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportRevisionView {
    pub revision: u64,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportTemplateImportView {
    pub writer_template_id: Option<String>,
    pub writer_template_name: Option<String>,
    pub imported_revision: u64,
    pub imported_at: String,
    pub warnings: Vec<ReportTemplateWarningView>,
    pub reviewed_revision: Option<u64>,
    pub review_required: bool,
    pub placeholder_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportTemplateWarningView {
    pub code: String,
    pub message: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportTemplatePreview {
    pub import_id: String,
    pub content: ReportContent,
    pub warnings: Vec<ReportTemplateWarningView>,
    pub stats: ReportTemplateStatsView,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportTemplateStatsView {
    pub sections: u32,
    pub paragraphs: u32,
    pub bullet_lists: u32,
    pub tables: u32,
    pub table_cells: u32,
    pub placeholder_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WriterTemplateView {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub uploaded_at: String,
    pub use_count: u64,
}

pub fn writer_template_view(
    template: claria_report_store::template_library::WriterTemplateSummary,
) -> WriterTemplateView {
    WriterTemplateView {
        id: template.metadata.id.to_string(),
        name: template.metadata.name,
        size: template.metadata.size,
        uploaded_at: template.metadata.uploaded_at.to_string(),
        use_count: template.use_count,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportAuthoringTurnView {
    pub id: String,
    pub model_id: String,
    pub timeline: Vec<ReportTimelineItemView>,
    pub usage: TurnUsage,
    pub usage_complete: bool,
    pub converse_calls: u32,
    pub tool_uses: u32,
    /// Records preloaded outside the model's record-reading tools. Only the
    /// filename and availability cross IPC; source hashes/counts stay in the
    /// durable workspace and record text is never persisted there.
    pub context_files: Vec<ReportContextFileView>,
    pub context_reads: Vec<ReportContextReadView>,
    pub created_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportContextFileView {
    pub filename: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportContextReadView {
    pub filename: String,
    pub offset: u64,
    pub returned_characters: u64,
    pub total_characters: Option<u64>,
    pub read_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportTurnProgressView {
    RecordContextPrepared {
        included_files: u32,
        unavailable_files: u32,
        total_characters: u64,
    },
    ModelCallStarted {
        call_number: u32,
    },
    /// The last announced call never landed and the identical request is
    /// going out again. `attempt` is the one about to be made, so the first
    /// retry reports 2 of `max_attempts`.
    ModelCallRetrying {
        call_number: u32,
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
    },
    ToolStarted {
        name: String,
        context: Option<String>,
    },
    ToolFinished {
        name: String,
        context: Option<String>,
        status: ReportToolActivityStatus,
    },
    /// Another plan row has been counted off the answer the planner is still
    /// streaming. Display only — the plan is validated whole when the call
    /// returns — so the count may restart if the call is re-sent.
    PlanRowPlanned {
        planned: u32,
        total: u32,
    },
    /// One batch of the plan is decided and will not be asked for again.
    /// `first` and `last` are one-based and inclusive, against the document's
    /// whole section count.
    PlanBatchPlanned {
        first: u32,
        last: u32,
        total: u32,
    },
    /// The drafting run's plan is settled, before the first model call: the
    /// honest denominator for the progress that follows.
    PlanReady {
        section_count: u32,
    },
    SectionStarted {
        section_id: String,
        index: u32,
        total: u32,
    },
    /// A section landed durably, carrying its content so the preview can
    /// render it without a refetch. The section needs no redaction here: it is
    /// the host-validated staged content the finish cut copies verbatim into
    /// the workspace this same command returns.
    SectionCompleted {
        section_id: String,
        section: ReportSection,
        drafted: u32,
        total: u32,
    },
    SectionSkipped {
        section_id: String,
        drafted: u32,
        total: u32,
    },
    SectionFailed {
        section_id: String,
        message: String,
        drafted: u32,
        total: u32,
    },
    TitleSet {
        title: String,
    },
    ReviewPassStarted {
        property: String,
        index: u32,
        total: u32,
    },
    ReviewPassCompleted {
        property: String,
        findings: u32,
        completed: u32,
        total: u32,
    },
}

impl From<claria::ReportTurnProgress> for ReportTurnProgressView {
    fn from(value: claria::ReportTurnProgress) -> Self {
        match value {
            claria::ReportTurnProgress::RecordContextPrepared {
                included_files,
                unavailable_files,
                total_characters,
            } => Self::RecordContextPrepared {
                included_files,
                unavailable_files,
                total_characters,
            },
            claria::ReportTurnProgress::ModelCallStarted { call_number } => {
                Self::ModelCallStarted { call_number }
            }
            claria::ReportTurnProgress::ModelCallRetrying {
                call_number,
                attempt,
                max_attempts,
                delay_ms,
            } => Self::ModelCallRetrying {
                call_number,
                attempt,
                max_attempts,
                delay_ms,
            },
            claria::ReportTurnProgress::ToolStarted { name, context } => {
                Self::ToolStarted { name, context }
            }
            claria::ReportTurnProgress::ToolFinished {
                name,
                context,
                status,
            } => Self::ToolFinished {
                name,
                context,
                status: match status {
                    ReportToolResultStatus::Success => ReportToolActivityStatus::Succeeded,
                    ReportToolResultStatus::Error => ReportToolActivityStatus::Failed,
                },
            },
            claria::ReportTurnProgress::PlanRowPlanned { planned, total } => {
                Self::PlanRowPlanned { planned, total }
            }
            claria::ReportTurnProgress::PlanBatchPlanned { first, last, total } => {
                Self::PlanBatchPlanned { first, last, total }
            }
            claria::ReportTurnProgress::PlanReady { section_count } => {
                Self::PlanReady { section_count }
            }
            claria::ReportTurnProgress::SectionStarted {
                section_id,
                index,
                total,
            } => Self::SectionStarted {
                section_id,
                index,
                total,
            },
            claria::ReportTurnProgress::SectionCompleted {
                section_id,
                section,
                drafted,
                total,
            } => Self::SectionCompleted {
                section_id,
                section,
                drafted,
                total,
            },
            claria::ReportTurnProgress::SectionSkipped {
                section_id,
                drafted,
                total,
            } => Self::SectionSkipped {
                section_id,
                drafted,
                total,
            },
            claria::ReportTurnProgress::SectionFailed {
                section_id,
                message,
                drafted,
                total,
            } => Self::SectionFailed {
                section_id,
                message,
                drafted,
                total,
            },
            claria::ReportTurnProgress::TitleSet { title } => Self::TitleSet { title },
            claria::ReportTurnProgress::ReviewPassStarted {
                property,
                index,
                total,
            } => Self::ReviewPassStarted {
                property,
                index,
                total,
            },
            claria::ReportTurnProgress::ReviewPassCompleted {
                property,
                findings,
                completed,
                total,
            } => Self::ReviewPassCompleted {
                property,
                findings,
                completed,
                total,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportTimelineItemView {
    Message {
        role: ReportTimelineRole,
        text: String,
        created_at: String,
    },
    ToolActivity {
        name: String,
        summary: String,
        status: ReportToolActivityStatus,
        /// Pretty-printed Bedrock toolUse block exactly as reconstructed from
        /// the model response. Intended for the collapsed nerd view.
        invocation_json: String,
        /// Pretty-printed correlated toolResult block. Record excerpts remain
        /// absent because persisted tool results are sanitized upstream.
        result_json: Option<String>,
        created_at: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReportTimelineRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReportToolActivityStatus {
    Requested,
    Succeeded,
    Failed,
}

/// Earns its life over `ReportProposal`: omits `tool_use_id`, the internal
/// Bedrock correlation handle the frontend has no business seeing.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportProposalView {
    pub id: String,
    pub report_id: String,
    pub base_revision: u64,
    pub model_id: String,
    pub summary: String,
    pub operations: Vec<ReportOperation>,
    pub proposed_content: ReportContent,
    pub created_at: String,
}

/// Earns its life next to `ReportProposalDecision`: this is the imperative
/// request wire shape (`accept`/`reject`), not the stored past-tense
/// resolution (`accepted`/`rejected`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportProposalChoice {
    Accept,
    Reject,
}

impl From<ReportProposalChoice> for CoreProposalDecision {
    fn from(value: ReportProposalChoice) -> Self {
        match value {
            ReportProposalChoice::Accept => Self::Accepted,
            ReportProposalChoice::Reject => Self::Rejected,
        }
    }
}

/// Earns its life: resolving a finding changes both the report and the
/// findings that describe it, and the caller needs the pair to stay in step.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportFindingResolution {
    pub workspace: ReportWorkspaceView,
    pub findings: ReportFindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportDraftEdit {
    pub title: String,
    pub sections: Vec<ReportSectionEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportBlockReferenceInput {
    pub section_id: String,
    pub block_index: u32,
}

impl ReportBlockReferenceInput {
    pub fn into_domain(self) -> Result<claria::ReportBlockReference, String> {
        Ok(claria::ReportBlockReference {
            section_id: self
                .section_id
                .parse::<Uuid>()
                .map_err(|_| format!("Invalid referenced section ID: {}", self.section_id))?,
            block_index: self.block_index,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportSectionEdit {
    pub id: Option<String>,
    pub heading: String,
    pub blocks: Vec<ReportBlock>,
    #[serde(default)]
    pub skipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportTurnResponse {
    pub workspace: ReportWorkspaceView,
    pub turn_id: String,
    pub attempt_id: String,
    pub assistant_text: String,
    pub usage: TurnUsage,
    pub usage_complete: bool,
    pub converse_calls: u32,
    pub tool_uses: u32,
    pub proposal_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct FullReportGenerationResponse {
    pub workspace: ReportWorkspaceView,
    pub turn_id: String,
    pub attempt_id: String,
    pub assistant_text: String,
    pub usage: TurnUsage,
    pub usage_complete: bool,
    pub converse_calls: u32,
    pub tool_uses: u32,
    pub included_record_files: u32,
    pub unavailable_record_files: u32,
    pub record_characters: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReportExportResult {
    pub exported: bool,
    pub report_id: String,
    pub revision: u64,
    pub status: ReportExportStatus,
    pub attempted_at: String,
    pub status_persisted: bool,
    /// Whether the export carried the imported template's formatting.
    #[serde(default)]
    pub template_applied: bool,
    /// Why the template's formatting was reduced or unavailable, when the
    /// workspace expected one — never silently degraded.
    #[serde(default)]
    pub template_warning: Option<TemplateExportWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TemplateExportWarning {
    /// The stored template source no longer exists (imports predating the
    /// template shelf never retained it); the export used generated
    /// formatting.
    TemplateMissing,
    /// The template body could not be walked (e.g. content controls); the
    /// export used generated body formatting inside the template package.
    TemplateBodyFallback,
}

pub fn content_from_edit(edit: ReportDraftEdit) -> Result<ReportContent, String> {
    let mut seen = HashSet::new();
    let sections = edit
        .sections
        .into_iter()
        .map(|section| {
            let id = match section.id {
                Some(id) => id
                    .parse::<Uuid>()
                    .map_err(|_| format!("Invalid report section ID: {id}"))?,
                None => Uuid::new_v4(),
            };
            if !seen.insert(id) {
                return Err(format!("Duplicate report section ID: {id}"));
            }
            // Hand-writing content into a deferred section un-defers it,
            // even if the frontend forgot to clear the flag.
            let skipped = section.skipped && section.blocks.is_empty();
            // The editor never sees a section's template copy or authorship
            // stamp, so an edit cannot carry them: the store re-attaches the
            // template copy by section ID when it saves.
            Ok(ReportSection {
                id,
                heading: section.heading,
                blocks: section.blocks,
                skipped,
                template_blocks: None,
                template_directives: Vec::new(),
                authorship: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ReportContent {
        title: edit.title,
        sections,
    })
}

pub fn turn_response_view(outcome: claria::ReportTurnOutcome) -> ReportTurnResponse {
    ReportTurnResponse {
        workspace: workspace_view(&outcome.workspace),
        turn_id: outcome.turn_id.to_string(),
        attempt_id: outcome.attempt.attempt_id.to_string(),
        assistant_text: outcome.assistant_text,
        usage: outcome.attempt.usage,
        usage_complete: outcome.attempt.usage_complete,
        converse_calls: outcome.attempt.converse_calls,
        tool_uses: outcome.attempt.tool_uses,
        proposal_id: outcome.proposal_id.map(|id| id.to_string()),
    }
}

pub fn full_report_response_view(
    outcome: claria::FullReportGenerationOutcome,
) -> FullReportGenerationResponse {
    FullReportGenerationResponse {
        workspace: workspace_view(&outcome.workspace),
        turn_id: outcome.turn_id.to_string(),
        attempt_id: outcome.attempt.attempt_id.to_string(),
        assistant_text: outcome.assistant_text,
        usage: outcome.attempt.usage,
        usage_complete: outcome.attempt.usage_complete,
        converse_calls: outcome.attempt.converse_calls,
        tool_uses: outcome.attempt.tool_uses,
        included_record_files: outcome.record_context.included_files,
        unavailable_record_files: outcome.record_context.unavailable_files,
        record_characters: outcome.record_context.total_characters,
    }
}

pub fn workspace_view(workspace: &ReportWorkspace) -> ReportWorkspaceView {
    ReportWorkspaceView {
        schema_version: workspace.schema_version,
        report_id: workspace.report_id.to_string(),
        client_id: workspace.client_id.to_string(),
        session_name: workspace.session_name.clone(),
        draft: workspace.draft.clone(),
        turns: workspace.session.turns.iter().map(turn_view).collect(),
        pending_proposal: workspace
            .session
            .pending_proposal
            .as_ref()
            .map(proposal_view),
        resolutions: workspace.session.resolutions.clone(),
        last_agent_revision: workspace.session.last_agent_revision,
        last_export: workspace.session.last_export.clone(),
        template_import: workspace
            .template_import
            .as_ref()
            .map(|template| template_import_view(template, workspace)),
        created_at: workspace.created_at.to_string(),
        updated_at: workspace.updated_at.to_string(),
    }
}

pub fn template_preview_view(
    import_id: Uuid,
    imported: &claria_docx::ImportedTemplate,
) -> ReportTemplatePreview {
    ReportTemplatePreview {
        import_id: import_id.to_string(),
        content: imported.content.clone(),
        warnings: imported
            .warnings
            .iter()
            .map(template_warning_view)
            .collect(),
        stats: ReportTemplateStatsView {
            sections: imported.stats.sections,
            paragraphs: imported.stats.paragraphs,
            bullet_lists: imported.stats.bullet_lists,
            tables: imported.stats.tables,
            table_cells: imported.stats.table_cells,
            placeholder_count: imported.stats.placeholder_count,
        },
    }
}

fn template_import_view(
    template: &ReportTemplateImport,
    workspace: &ReportWorkspace,
) -> ReportTemplateImportView {
    ReportTemplateImportView {
        writer_template_id: template.writer_template_id.map(|id| id.to_string()),
        writer_template_name: template.writer_template_name.clone(),
        imported_revision: template.imported_revision,
        imported_at: template.imported_at.to_string(),
        warnings: template
            .warnings
            .iter()
            .map(template_warning_view)
            .collect(),
        reviewed_revision: template.reviewed_revision,
        review_required: template.reviewed_revision != Some(workspace.draft.revision),
        placeholder_count: report_template_placeholder_count(&workspace.draft.content),
    }
}

fn template_warning_view(warning: &ReportTemplateWarning) -> ReportTemplateWarningView {
    ReportTemplateWarningView {
        code: template_warning_code(warning.code).to_string(),
        message: template_warning_message(warning.code).to_string(),
        count: warning.count,
    }
}

fn template_warning_code(code: ReportTemplateWarningCode) -> &'static str {
    match code {
        ReportTemplateWarningCode::CommentsOmitted => "comments_omitted",
        ReportTemplateWarningCode::ExternalLinksRemoved => "external_links_removed",
        ReportTemplateWarningCode::FootnotesEndnotesOmitted => "footnotes_endnotes_omitted",
        ReportTemplateWarningCode::HeadersFootersOmitted => "headers_footers_omitted",
        ReportTemplateWarningCode::HeadingLevelsFlattened => "heading_levels_flattened",
        ReportTemplateWarningCode::ImagesOmitted => "images_omitted",
        ReportTemplateWarningCode::IrregularTablesOmitted => "irregular_tables_omitted",
        ReportTemplateWarningCode::MergedTablesOmitted => "merged_tables_omitted",
        ReportTemplateWarningCode::MissingTitle => "missing_title",
        ReportTemplateWarningCode::NestedTablesOmitted => "nested_tables_omitted",
        ReportTemplateWarningCode::NumberedListsImportedAsBullets => {
            "numbered_lists_imported_as_bullets"
        }
        ReportTemplateWarningCode::SectionsInferredFromFormatting => {
            "sections_inferred_from_formatting"
        }
        ReportTemplateWarningCode::TextBoxesOmitted => "text_boxes_omitted",
        ReportTemplateWarningCode::TrackedChangesResolved => "tracked_changes_resolved",
        ReportTemplateWarningCode::UnsupportedElementsOmitted => "unsupported_elements_omitted",
    }
}

fn template_warning_message(code: ReportTemplateWarningCode) -> &'static str {
    match code {
        ReportTemplateWarningCode::CommentsOmitted => "Word comments were omitted.",
        ReportTemplateWarningCode::ExternalLinksRemoved => {
            "External link targets were removed; visible link text was retained."
        }
        ReportTemplateWarningCode::FootnotesEndnotesOmitted => {
            "Footnotes or endnotes were omitted."
        }
        ReportTemplateWarningCode::HeadersFootersOmitted => "Headers or footers were omitted.",
        ReportTemplateWarningCode::HeadingLevelsFlattened => {
            "Nested Word heading levels were flattened into report sections."
        }
        ReportTemplateWarningCode::ImagesOmitted => "Images were omitted.",
        ReportTemplateWarningCode::IrregularTablesOmitted => {
            "Tables outside the supported row and column limits were omitted."
        }
        ReportTemplateWarningCode::MergedTablesOmitted => "Tables with merged cells were omitted.",
        ReportTemplateWarningCode::MissingTitle => {
            "No Word Title style was found; a placeholder report title was added."
        }
        ReportTemplateWarningCode::NestedTablesOmitted => "Nested tables were omitted.",
        ReportTemplateWarningCode::NumberedListsImportedAsBullets => {
            "Word list paragraphs were imported as bullet lists."
        }
        ReportTemplateWarningCode::SectionsInferredFromFormatting => {
            "No Word heading styles were applied, so sections were split at the bold and capitalized lines that look like headings. Check the section list; applying Heading styles in Word makes the split exact."
        }
        ReportTemplateWarningCode::TextBoxesOmitted => "Text-box content was omitted.",
        ReportTemplateWarningCode::TrackedChangesResolved => {
            "Visible inserted text was retained and deleted tracked text was omitted."
        }
        ReportTemplateWarningCode::UnsupportedElementsOmitted => {
            "Unsupported Word elements were omitted."
        }
    }
}

fn proposal_view(proposal: &ReportProposal) -> ReportProposalView {
    ReportProposalView {
        id: proposal.id.to_string(),
        report_id: proposal.report_id.to_string(),
        base_revision: proposal.base_revision,
        model_id: proposal.model_id.clone(),
        summary: proposal.summary.clone(),
        operations: proposal.operations.clone(),
        proposed_content: proposal.proposed_content.clone(),
        created_at: proposal.created_at.to_string(),
    }
}

pub fn editor_history_entry(workspace: &ReportWorkspace) -> EditorHistoryEntry {
    EditorHistoryEntry {
        report_id: workspace.report_id.to_string(),
        name: workspace.session_name.clone(),
        title: workspace.draft.content.title.clone(),
        revision: workspace.draft.revision,
        turn_count: u32::try_from(workspace.session.turns.len()).unwrap_or(u32::MAX),
        updated_at: workspace.updated_at.to_string(),
        last_export: workspace.session.last_export.clone(),
    }
}

fn turn_view(turn: &claria_core::models::report::ReportAuthoringTurn) -> ReportAuthoringTurnView {
    let mut results: HashMap<&str, (&ReportToolResultStatus, &serde_json::Value)> = HashMap::new();
    for message in &turn.messages {
        for block in &message.content {
            if let ReportProtocolBlock::ToolResult {
                tool_use_id,
                status,
                content,
            } = block
            {
                results.insert(tool_use_id, (status, content));
            }
        }
    }

    let mut timeline = Vec::new();
    for message in &turn.messages {
        for block in &message.content {
            match block {
                ReportProtocolBlock::Text { text } => {
                    timeline.push(ReportTimelineItemView::Message {
                        role: match message.role {
                            ReportProtocolRole::User => ReportTimelineRole::User,
                            ReportProtocolRole::Assistant => ReportTimelineRole::Assistant,
                        },
                        text: text.clone(),
                        created_at: message.created_at.to_string(),
                    });
                }
                ReportProtocolBlock::ToolUse {
                    tool_use_id,
                    name,
                    input,
                } => {
                    let result = results.get(tool_use_id.as_str());
                    let status = match result.map(|(status, _)| *status) {
                        Some(ReportToolResultStatus::Success) => {
                            ReportToolActivityStatus::Succeeded
                        }
                        Some(ReportToolResultStatus::Error) => ReportToolActivityStatus::Failed,
                        None => ReportToolActivityStatus::Requested,
                    };
                    let invocation_json = pretty_json(&serde_json::json!({
                        "toolUse": {
                            "toolUseId": tool_use_id,
                            "name": name,
                            "input": input
                        }
                    }));
                    let result_json = result.map(|(result_status, content)| {
                        pretty_json(&serde_json::json!({
                            "toolResult": {
                                "toolUseId": tool_use_id,
                                "status": match result_status {
                                    ReportToolResultStatus::Success => "success",
                                    ReportToolResultStatus::Error => "error",
                                },
                                "content": [{"json": content}]
                            }
                        }))
                    });
                    timeline.push(ReportTimelineItemView::ToolActivity {
                        name: name.clone(),
                        summary: tool_activity_summary(
                            name,
                            input,
                            result.map(|(_, value)| *value),
                            status,
                        ),
                        status,
                        invocation_json,
                        result_json,
                        created_at: message.created_at.to_string(),
                    });
                }
                ReportProtocolBlock::ReasoningText { .. }
                | ReportProtocolBlock::ReasoningRedacted { .. }
                | ReportProtocolBlock::ToolResult { .. } => {}
            }
        }
    }

    let mut context_reads = Vec::new();
    for message in &turn.messages {
        for block in &message.content {
            let ReportProtocolBlock::ToolUse {
                tool_use_id,
                name,
                input,
            } = block
            else {
                continue;
            };
            if name != claria_bedrock::report::READ_RECORD_FILE_TOOL {
                continue;
            }
            let Some((status, result)) = results.get(tool_use_id.as_str()) else {
                continue;
            };
            if **status != ReportToolResultStatus::Success {
                continue;
            }
            context_reads.push(ReportContextReadView {
                filename: input
                    .get("filename")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("record file")
                    .to_string(),
                offset: result
                    .get("offset")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                returned_characters: result
                    .get("returned_characters")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                total_characters: result
                    .get("total_characters")
                    .and_then(serde_json::Value::as_u64),
                read_at: message.created_at.to_string(),
            });
        }
    }

    ReportAuthoringTurnView {
        id: turn.id.to_string(),
        model_id: turn.model_id.clone(),
        timeline,
        usage: turn.usage.clone(),
        usage_complete: turn.usage_complete,
        converse_calls: turn.converse_calls,
        tool_uses: turn.tool_uses,
        context_files: turn
            .record_context_files
            .iter()
            .map(|file| ReportContextFileView {
                filename: file.filename().to_string(),
                available: file.is_available(),
            })
            .collect(),
        context_reads,
        created_at: turn.created_at.to_string(),
        completed_at: turn.completed_at.to_string(),
    }
}

fn pretty_json(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn tool_activity_summary(
    name: &str,
    input: &serde_json::Value,
    result: Option<&serde_json::Value>,
    status: ReportToolActivityStatus,
) -> String {
    if matches!(status, ReportToolActivityStatus::Failed) {
        return result
            .and_then(|value| value.pointer("/error/message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("The tool failed safely.")
            .to_string();
    }

    match name {
        claria_bedrock::report::LIST_RECORD_FILES_TOOL => {
            let count = result
                .and_then(|value| {
                    value
                        .get("file_count")
                        .and_then(serde_json::Value::as_u64)
                        .or_else(|| {
                            value
                                .get("files")
                                .and_then(serde_json::Value::as_array)
                                .map(|files| files.len() as u64)
                        })
                })
                .unwrap_or(0);
            format!("Listed {count} record files")
        }
        claria_bedrock::report::READ_RECORD_FILE_TOOL => {
            let filename = input
                .get("filename")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("record file");
            let offset = result
                .and_then(|value| value.get("offset"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_else(|| {
                    input
                        .get("offset")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0)
                });
            let returned = result
                .and_then(|value| value.get("returned_characters"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            format!(
                "Read {filename}, characters {offset}–{}",
                offset.saturating_add(returned)
            )
        }
        claria_bedrock::report::PROPOSE_REPORT_CHANGES_TOOL => {
            "Staged report changes for approval".to_string()
        }
        _ => format!("Requested {name}"),
    }
}

// ── Drafting-run history ────────────────────────────────────────────────────

/// One report's drafting runs, newest first, as the Draft run tab reads them.
///
/// Earns its life over `Vec<DraftRun>` on two counts. It **drops the staged
/// blocks**: every drafted section carries a copy of its own prose, so a
/// history of five runs would ship the document five times over the bridge to
/// render a summary that never shows a word of it — the pane renders the
/// report itself from the workspace. And it **joins each section to its plan
/// row**, because every reader of this data wants the pair (what was decided,
/// what happened) and doing the join once here is cheaper and less
/// error-prone than doing it in the pane.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DraftRunHistoryView {
    pub runs: Vec<DraftRunHistoryEntry>,
    /// The run that currently owns the session, if one does. It is the only
    /// entry the writer will refuse edits behind.
    pub active_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DraftRunHistoryEntry {
    pub run_id: String,
    pub status: DraftRunStatus,
    /// This run holds the session right now.
    pub active: bool,
    pub base_revision: u64,
    /// The revision the finish cut produced. Present only on a completed run,
    /// and what lets the pane say which run wrote the document on screen.
    pub finalized_revision: Option<u64>,
    /// Finalized from a stopped state: what never landed was skipped.
    pub partial: bool,
    pub title: Option<String>,
    pub writer_model_id: String,
    /// The planning model. Absent on a run that never got a plan.
    pub planner_model_id: Option<String>,
    /// Nobody decided this plan — it was manufactured 1:1 from the sections
    /// the report already had.
    pub plan_synthetic: bool,
    /// The clinician changed the plan at the gate before approving it.
    pub plan_user_edited: bool,
    pub plan_approved_at: Option<String>,
    /// PHI-free `code:detail` strings about what the host could not confirm.
    pub plan_warnings: Vec<String>,
    /// The guidance the run was given, oldest first: what was typed at the
    /// plan step, plus anything added at a resume.
    pub instructions: Vec<RunInstruction>,
    /// The record corpus the run was built from. `None` on a run recorded
    /// before Claria captured one.
    pub record_snapshot: Option<RunRecordSnapshot>,
    pub sections: Vec<DraftRunHistorySection>,
    pub counts: DraftRunSectionCounts,
    pub created_at: String,
    pub updated_at: String,
}

/// One section of one run: the plan's decision and the run's outcome, in one
/// row.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DraftRunHistorySection {
    pub section_id: String,
    pub heading: String,
    pub position: u32,
    pub state: RunSectionState,
    /// What the plan decided for this section. `None` when the run has no
    /// plan row for it — an invented section on a run without a plan.
    pub intent: Option<SectionIntent>,
    /// The report cannot be complete while this section is empty.
    pub required: bool,
    /// What the section had to cover, in the planner's words.
    pub scope: String,
    pub evidence: Vec<EvidenceRef>,
    /// Per-section steering that overrode the run-wide instructions.
    pub instruction: Option<String>,
    /// The records that were in the model call that wrote this section.
    pub records: Option<RunSectionRecords>,
    pub citations: Vec<RecordCitation>,
    /// Blocks the run staged for this section. The prose itself is the
    /// report; this is how much of it the run wrote.
    pub block_count: u32,
    pub characters: u64,
    pub attempts: u32,
    /// PHI-free note about why the section failed, or why a skip diverged
    /// from the approved plan.
    pub error: Option<String>,
    pub updated_at: String,
}

/// How one run's sections came out, so the pane's progress bar and its
/// summary line read the same number.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct DraftRunSectionCounts {
    pub total: u32,
    pub drafted: u32,
    pub skipped: u32,
    pub kept: u32,
    pub failed: u32,
    /// Sections still undecided: pending, or left mid-flight by an
    /// interruption.
    pub undecided: u32,
    /// Sections in any terminal state — the numerator of the progress bar.
    pub decided: u32,
}

pub fn draft_run_history_view(history: &claria::DraftRunHistory) -> DraftRunHistoryView {
    DraftRunHistoryView {
        runs: history
            .runs
            .iter()
            .map(|run| draft_run_history_entry(run, history.active_run_id))
            .collect(),
        active_run_id: history.active_run_id.map(|id| id.to_string()),
    }
}

fn draft_run_history_entry(run: &DraftRun, active_run_id: Option<Uuid>) -> DraftRunHistoryEntry {
    let plan_entries: HashMap<Uuid, &DraftPlanEntry> = run
        .plan
        .iter()
        .flat_map(|plan| plan.entries.iter())
        .map(|entry| (entry.section_id, entry))
        .collect();
    let mut sections: Vec<&RunSection> = run.sections.iter().collect();
    sections.sort_by_key(|section| section.position);

    let mut counts = DraftRunSectionCounts {
        total: u32::try_from(sections.len()).unwrap_or(u32::MAX),
        drafted: 0,
        skipped: 0,
        kept: 0,
        failed: 0,
        undecided: 0,
        decided: 0,
    };
    for section in &sections {
        match section.state {
            RunSectionState::Drafted | RunSectionState::Flagged => counts.drafted += 1,
            RunSectionState::Skipped => counts.skipped += 1,
            RunSectionState::Kept => counts.kept += 1,
            RunSectionState::Failed => counts.failed += 1,
            // `Drafting` is transient and never true across a load: a section
            // stored in it belongs to a run whose process died mid-section,
            // which leaves it exactly as undecided as a pending one.
            RunSectionState::Pending | RunSectionState::Drafting => counts.undecided += 1,
        }
    }
    counts.decided = counts.total.saturating_sub(counts.undecided);

    DraftRunHistoryEntry {
        run_id: run.run_id.to_string(),
        status: run.status,
        active: active_run_id == Some(run.run_id),
        base_revision: run.base_revision,
        finalized_revision: run.finalized_revision,
        partial: run.partial,
        title: run.title.clone(),
        writer_model_id: run.writer_model_id.clone(),
        planner_model_id: run.plan.as_ref().map(|plan| plan.model_id.clone()),
        plan_synthetic: run.plan.as_ref().is_some_and(|plan| plan.synthetic),
        plan_user_edited: run.plan.as_ref().is_some_and(|plan| plan.user_edited),
        plan_approved_at: run
            .plan
            .as_ref()
            .and_then(|plan| plan.approved_at)
            .map(|at| at.to_string()),
        plan_warnings: run
            .plan
            .as_ref()
            .map(|plan| plan.plan_warnings.clone())
            .unwrap_or_default(),
        instructions: run.instructions.clone(),
        record_snapshot: run.record_snapshot.clone(),
        sections: sections
            .into_iter()
            .map(|section| {
                let entry = plan_entries.get(&section.section_id);
                DraftRunHistorySection {
                    section_id: section.section_id.to_string(),
                    heading: section.heading.clone(),
                    position: section.position,
                    state: section.state,
                    intent: entry.map(|entry| entry.intent),
                    required: entry.is_some_and(|entry| entry.required),
                    scope: entry.map(|entry| entry.scope.clone()).unwrap_or_default(),
                    evidence: entry
                        .map(|entry| entry.evidence.clone())
                        .unwrap_or_default(),
                    instruction: entry.and_then(|entry| entry.instruction.clone()),
                    records: section.records.clone(),
                    citations: section.citations.clone(),
                    block_count: u32::try_from(section.blocks.len()).unwrap_or(u32::MAX),
                    characters: section.blocks.iter().map(block_characters).sum(),
                    attempts: section.attempts,
                    error: section.error.clone(),
                    updated_at: section.updated_at.to_string(),
                }
            })
            .collect(),
        counts,
        created_at: run.created_at.to_string(),
        updated_at: run.updated_at.to_string(),
    }
}

/// How much text one staged block holds. Counts only — the block itself never
/// crosses in a history entry.
fn block_characters(block: &ReportBlock) -> u64 {
    let characters = |text: &String| u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
    match block {
        ReportBlock::Paragraph { text } => characters(text),
        ReportBlock::BulletList { items } => items.iter().map(characters).sum(),
        ReportBlock::Table { rows, .. } => rows.iter().flatten().map(characters).sum(),
    }
}
