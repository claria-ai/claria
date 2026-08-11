//! Tauri-facing report-authoring view models.
//!
//! Workflow policy and AWS orchestration live in `claria-report-authoring`.
//! The report domain types (`ReportDraft`, `ReportContent`, blocks,
//! operations, exports, resolutions) derive `specta::Type` in `claria-core`
//! and cross the IPC boundary as-is; this module keeps only the views that
//! genuinely differ from their domain type — flattening, redaction, derived
//! timelines — each carrying a comment saying why it earns its life.

use std::collections::{HashMap, HashSet};

use claria_core::models::{
    report::{
        ReportBlock, ReportContent, ReportDraft, ReportExport, ReportExportStatus, ReportOperation,
        ReportProposal, ReportProposalDecision as CoreProposalDecision, ReportProposalResolution,
        ReportProtocolBlock, ReportProtocolRole, ReportSection, ReportTemplateImport,
        ReportTemplateWarning, ReportTemplateWarningCode, ReportToolResultStatus, ReportWorkspace,
        report_template_placeholder_count,
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
    template: claria_report_authoring::writer_templates::WriterTemplateSummary,
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
    pub context_reads: Vec<ReportContextReadView>,
    pub created_at: String,
    pub completed_at: String,
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
    ToolStarted {
        name: String,
        context: Option<String>,
    },
    ToolFinished {
        name: String,
        context: Option<String>,
        status: ReportToolActivityStatus,
    },
}

impl From<claria_report_authoring::ReportTurnProgress> for ReportTurnProgressView {
    fn from(value: claria_report_authoring::ReportTurnProgress) -> Self {
        match value {
            claria_report_authoring::ReportTurnProgress::RecordContextPrepared {
                included_files,
                unavailable_files,
                total_characters,
            } => Self::RecordContextPrepared {
                included_files,
                unavailable_files,
                total_characters,
            },
            claria_report_authoring::ReportTurnProgress::ModelCallStarted { call_number } => {
                Self::ModelCallStarted { call_number }
            }
            claria_report_authoring::ReportTurnProgress::ToolStarted { name, context } => {
                Self::ToolStarted { name, context }
            }
            claria_report_authoring::ReportTurnProgress::ToolFinished {
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
    pub fn into_domain(self) -> Result<claria_report_authoring::ReportBlockReference, String> {
        Ok(claria_report_authoring::ReportBlockReference {
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
            Ok(ReportSection {
                id,
                heading: section.heading,
                blocks: section.blocks,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ReportContent {
        title: edit.title,
        sections,
    })
}

pub fn turn_response_view(
    outcome: claria_report_authoring::ReportTurnOutcome,
) -> ReportTurnResponse {
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
    outcome: claria_report_authoring::FullReportGenerationOutcome,
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
