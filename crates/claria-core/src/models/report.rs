//! Structured, versioned report-authoring workspaces.
//!
//! A workspace keeps the accepted report draft separate from the model's
//! proposal. Model output is never applied directly: callers stage a
//! [`ReportProposal`], show its exact candidate to the user, and only then use
//! [`ReportDraft::accept`] to recompute and persist the accepted content.

use std::collections::HashSet;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::CoreError, models::turn_usage::TurnUsage};

pub const REPORT_WORKSPACE_SCHEMA_VERSION: u32 = 3;
pub const MAX_REPORT_SECTIONS: usize = 100;
pub const MAX_SECTION_BLOCKS: usize = 200;
pub const MAX_TABLE_ROWS: usize = 200;
pub const MAX_TABLE_COLUMNS: usize = 20;
pub const MAX_TABLE_CELL_CHARACTERS: usize = 5_000;
pub const MAX_REPORT_TABLE_CELLS: usize = 20_000;
pub const TABLE_WIDTH_BASIS_POINTS: u32 = 10_000;
pub const MAX_REPORT_TEXT_CHARACTERS: usize = 500_000;
pub const MAX_PROPOSAL_OPERATIONS: usize = 25;
pub const MAX_REPORT_TURNS: usize = 200;
pub const MAX_REPORT_PROTOCOL_BYTES: usize = 512 * 1024;
pub const MAX_REPORT_SESSION_NAME_CHARACTERS: usize = 120;

const MAX_TITLE_CHARACTERS: usize = 200;
const MAX_HEADING_CHARACTERS: usize = 200;
const MAX_PARAGRAPH_CHARACTERS: usize = 20_000;
const MAX_BULLET_ITEMS: usize = 100;
const MAX_BULLET_ITEM_CHARACTERS: usize = 2_000;
const MAX_PROPOSAL_SUMMARY_CHARACTERS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportWorkspace {
    pub schema_version: u32,
    pub report_id: Uuid,
    pub client_id: Uuid,
    #[serde(default = "default_report_session_name")]
    pub session_name: String,
    pub draft: ReportDraft,
    pub session: ReportSession,
    /// Provenance and review state for the most recently imported DOCX
    /// template. The source's bytes and local path are never copied into the
    /// client workspace; managed template sources live under `writer_templates/`.
    #[serde(default)]
    pub template_import: Option<ReportTemplateImport>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportDraft {
    pub revision: u64,
    pub content: ReportContent,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_applied_proposal_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportContent {
    pub title: String,
    pub sections: Vec<ReportSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportSection {
    pub id: Uuid,
    pub heading: String,
    pub blocks: Vec<ReportBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportBlock {
    Paragraph {
        text: String,
    },
    BulletList {
        items: Vec<String>,
    },
    /// A rectangular, plain-text table. The first row is rendered as a
    /// semantic header when `has_header` is true. Optional widths are basis
    /// points whose sum is exactly 10,000 (100%).
    Table {
        rows: Vec<Vec<String>>,
        has_header: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column_widths: Option<Vec<u16>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportTemplateImport {
    /// SHA-256 of the selected DOCX bytes. This ties the import to the file
    /// inspected locally without persisting its name, path, or package bytes.
    pub source_sha256: String,
    pub imported_revision: u64,
    pub imported_at: Timestamp,
    pub warnings: Vec<ReportTemplateWarning>,
    /// Export is allowed only when this equals the current accepted revision.
    #[serde(default)]
    pub reviewed_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportTemplateWarning {
    pub code: ReportTemplateWarningCode,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReportTemplateWarningCode {
    CommentsOmitted,
    ExternalLinksRemoved,
    FootnotesEndnotesOmitted,
    HeadersFootersOmitted,
    HeadingLevelsFlattened,
    ImagesOmitted,
    IrregularTablesOmitted,
    MergedTablesOmitted,
    MissingTitle,
    NestedTablesOmitted,
    NumberedListsImportedAsBullets,
    TextBoxesOmitted,
    TrackedChangesResolved,
    UnsupportedElementsOmitted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ReportSession {
    pub turns: Vec<ReportAuthoringTurn>,
    pub pending_proposal: Option<ReportProposal>,
    pub resolutions: Vec<ReportProposalResolution>,
    /// Revision of the accepted report included in the most recent completed
    /// assistant turn. A newer draft revision means user edits are queued for
    /// the assistant's next message.
    #[serde(default)]
    pub last_agent_revision: Option<u64>,
    /// Most recent attempt to export this writing session to Word.
    #[serde(default)]
    pub last_export: Option<ReportExport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportExport {
    pub revision: u64,
    pub status: ReportExportStatus,
    pub attempted_at: Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportExportStatus {
    Exported,
    Canceled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportAuthoringTurn {
    pub id: Uuid,
    pub model_id: String,
    pub messages: Vec<ReportProtocolMessage>,
    pub usage: TurnUsage,
    /// False when one or more successful Converse responses omitted usage.
    pub usage_complete: bool,
    pub converse_calls: u32,
    pub tool_uses: u32,
    pub created_at: Timestamp,
    pub completed_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportProtocolMessage {
    pub role: ReportProtocolRole,
    pub content: Vec<ReportProtocolBlock>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportProtocolRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportProtocolBlock {
    Text {
        text: String,
    },
    /// Opaque model reasoning retained only long enough to round-trip an
    /// immediate tool-result call. The authoring controller removes it before
    /// workspace persistence.
    ReasoningText {
        text: String,
        signature: Option<String>,
    },
    /// Provider-redacted reasoning bytes, also transient and never persisted.
    ReasoningRedacted {
        data: Vec<u8>,
    },
    ToolUse {
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        status: ReportToolResultStatus,
        content: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportToolResultStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportProposal {
    pub id: Uuid,
    pub report_id: Uuid,
    pub base_revision: u64,
    pub model_id: String,
    pub summary: String,
    pub operations: Vec<ReportOperation>,
    pub proposed_content: ReportContent,
    pub tool_use_id: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportOperation {
    SetTitle {
        title: String,
    },
    AddSection {
        position: u32,
        section: ReportSection,
    },
    ReplaceSection {
        section_id: Uuid,
        heading: String,
        blocks: Vec<ReportBlock>,
    },
    RemoveSection {
        section_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportProposalResolution {
    pub proposal_id: Uuid,
    pub decision: ReportProposalDecision,
    pub resulting_revision: u64,
    pub resolved_at: Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportProposalDecision {
    Accepted,
    Rejected,
}

impl ReportWorkspace {
    pub fn new(client_id: Uuid, now: Timestamp) -> Self {
        Self {
            schema_version: REPORT_WORKSPACE_SCHEMA_VERSION,
            report_id: Uuid::new_v4(),
            client_id,
            session_name: default_report_session_name(),
            draft: ReportDraft {
                revision: 0,
                content: ReportContent {
                    title: "Untitled report".to_string(),
                    sections: Vec::new(),
                },
                created_at: now,
                updated_at: now,
                last_applied_proposal_id: None,
            },
            session: ReportSession::default(),
            template_import: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version > REPORT_WORKSPACE_SCHEMA_VERSION {
            return Err(invalid(format!(
                "workspace schema version {} is newer than supported version {}",
                self.schema_version, REPORT_WORKSPACE_SCHEMA_VERSION
            )));
        }
        if self.schema_version != REPORT_WORKSPACE_SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported workspace schema version {}",
                self.schema_version
            )));
        }
        if self.report_id.is_nil() || self.client_id.is_nil() {
            return Err(invalid("workspace IDs must not be nil"));
        }
        if self.created_at > self.updated_at {
            return Err(invalid("workspace updated_at precedes created_at"));
        }
        validate_nonempty_text(
            "writer session name",
            &self.session_name,
            MAX_REPORT_SESSION_NAME_CHARACTERS,
        )?;
        self.draft.validate()?;
        self.session.validate(self)?;
        if let Some(template) = &self.template_import {
            validate_template_import(template, self)?;
        }
        Ok(())
    }

    pub fn rename_session(&mut self, name: &str, now: Timestamp) -> Result<(), CoreError> {
        let name = name.trim();
        validate_nonempty_text(
            "writer session name",
            name,
            MAX_REPORT_SESSION_NAME_CHARACTERS,
        )?;
        self.session_name = name.to_string();
        self.updated_at = now;
        Ok(())
    }

    /// Add a complete turn using the default retention limit.
    pub fn push_turn(&mut self, turn: ReportAuthoringTurn) -> Result<(), CoreError> {
        self.push_turn_with_limit(turn, MAX_REPORT_TURNS)
    }

    /// Add a complete turn and prune only complete, oldest turns to the
    /// configured retention limit. The newest turn is retained even if it
    /// alone exceeds the byte ceiling, so tool-use/result correlation is never
    /// split.
    pub fn push_turn_with_limit(
        &mut self,
        turn: ReportAuthoringTurn,
        max_retained_turns: usize,
    ) -> Result<(), CoreError> {
        if max_retained_turns == 0 || max_retained_turns > MAX_REPORT_TURNS {
            return Err(invalid(format!(
                "report turn retention must be between 1 and {MAX_REPORT_TURNS}"
            )));
        }
        validate_turn(&turn)?;
        self.session.turns.push(turn);
        self.prune_turns(max_retained_turns)
    }

    /// Prune complete oldest turns to a configured count and the protocol byte
    /// ceiling. This can be applied before a model call when the user lowers
    /// their retention preference.
    pub fn prune_turns(&mut self, max_retained_turns: usize) -> Result<(), CoreError> {
        if max_retained_turns == 0 || max_retained_turns > MAX_REPORT_TURNS {
            return Err(invalid(format!(
                "report turn retention must be between 1 and {MAX_REPORT_TURNS}"
            )));
        }
        while self.session.turns.len() > max_retained_turns {
            self.session.turns.remove(0);
        }
        while self.session.turns.len() > 1
            && protocol_history_bytes(&self.session.turns)? > MAX_REPORT_PROTOCOL_BYTES
        {
            self.session.turns.remove(0);
        }
        Ok(())
    }
}

impl ReportDraft {
    pub fn preview(&self, operations: &[ReportOperation]) -> Result<ReportContent, CoreError> {
        if operations.is_empty() {
            return Err(invalid("a proposal must contain at least one operation"));
        }
        if operations.len() > MAX_PROPOSAL_OPERATIONS {
            return Err(invalid(format!(
                "a proposal may contain at most {MAX_PROPOSAL_OPERATIONS} operations"
            )));
        }

        let mut candidate = self.content.clone();
        for operation in operations {
            match operation {
                ReportOperation::SetTitle { title } => {
                    validate_nonempty_text("title", title, MAX_TITLE_CHARACTERS)?;
                    candidate.title.clone_from(title);
                }
                ReportOperation::AddSection { position, section } => {
                    validate_section(section)?;
                    let position = usize::try_from(*position)
                        .map_err(|_| invalid("section position is too large"))?;
                    if position > candidate.sections.len() {
                        return Err(invalid(format!(
                            "section position {position} is outside 0..={}",
                            candidate.sections.len()
                        )));
                    }
                    if candidate.sections.iter().any(|item| item.id == section.id) {
                        return Err(invalid(format!("section ID {} already exists", section.id)));
                    }
                    candidate.sections.insert(position, section.clone());
                }
                ReportOperation::ReplaceSection {
                    section_id,
                    heading,
                    blocks,
                } => {
                    validate_nonempty_text("section heading", heading, MAX_HEADING_CHARACTERS)?;
                    validate_blocks(blocks)?;
                    let section = candidate
                        .sections
                        .iter_mut()
                        .find(|section| section.id == *section_id)
                        .ok_or_else(|| invalid(format!("section {section_id} does not exist")))?;
                    section.heading.clone_from(heading);
                    section.blocks.clone_from(blocks);
                }
                ReportOperation::RemoveSection { section_id } => {
                    let index = candidate
                        .sections
                        .iter()
                        .position(|section| section.id == *section_id)
                        .ok_or_else(|| invalid(format!("section {section_id} does not exist")))?;
                    candidate.sections.remove(index);
                }
            }
        }

        validate_content(&candidate)?;
        if candidate == self.content {
            return Err(invalid("proposal does not change the accepted report"));
        }
        Ok(candidate)
    }

    pub fn accept(
        &self,
        proposal: &ReportProposal,
        accepted_at: Timestamp,
    ) -> Result<ReportDraft, CoreError> {
        if proposal.base_revision != self.revision {
            return Err(invalid(format!(
                "proposal is based on revision {}, but accepted revision is {}",
                proposal.base_revision, self.revision
            )));
        }
        validate_proposal(proposal, None)?;
        let recomputed = self.preview(&proposal.operations)?;
        if recomputed != proposal.proposed_content {
            return Err(invalid("proposal candidate does not match its operations"));
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("report revision overflow"))?;
        Ok(Self {
            revision,
            content: recomputed,
            created_at: self.created_at,
            updated_at: accepted_at,
            last_applied_proposal_id: Some(proposal.id),
        })
    }

    pub fn replace_content(
        &self,
        expected_revision: u64,
        content: ReportContent,
        saved_at: Timestamp,
    ) -> Result<ReportDraft, CoreError> {
        if expected_revision != self.revision {
            return Err(invalid(format!(
                "expected revision {expected_revision}, but accepted revision is {}",
                self.revision
            )));
        }
        validate_content(&content)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("report revision overflow"))?;
        Ok(Self {
            revision,
            content,
            created_at: self.created_at,
            updated_at: saved_at,
            last_applied_proposal_id: self.last_applied_proposal_id,
        })
    }

    fn validate(&self) -> Result<(), CoreError> {
        if self.created_at > self.updated_at {
            return Err(invalid("draft updated_at precedes created_at"));
        }
        validate_content(&self.content)
    }
}

impl ReportSession {
    fn validate(&self, workspace: &ReportWorkspace) -> Result<(), CoreError> {
        if self.turns.len() > MAX_REPORT_TURNS {
            return Err(invalid(format!(
                "session contains more than {MAX_REPORT_TURNS} turns"
            )));
        }
        if self.turns.len() > 1 && protocol_history_bytes(&self.turns)? > MAX_REPORT_PROTOCOL_BYTES
        {
            return Err(invalid(format!(
                "session protocol history exceeds {MAX_REPORT_PROTOCOL_BYTES} bytes"
            )));
        }
        for turn in &self.turns {
            validate_turn(turn)?;
        }
        if self
            .last_agent_revision
            .is_some_and(|revision| revision > workspace.draft.revision)
        {
            return Err(invalid(
                "last agent revision is newer than the accepted report",
            ));
        }
        if let Some(export) = &self.last_export {
            if export.revision > workspace.draft.revision {
                return Err(invalid(
                    "last export revision is newer than the accepted report",
                ));
            }
            if export.attempted_at < workspace.created_at {
                return Err(invalid("last export predates the writing session"));
            }
        }
        if let Some(proposal) = &self.pending_proposal {
            validate_proposal(proposal, Some(workspace))?;
            if proposal.base_revision != workspace.draft.revision {
                return Err(invalid("pending proposal has a stale base revision"));
            }
            let candidate = workspace.draft.preview(&proposal.operations)?;
            if candidate != proposal.proposed_content {
                return Err(invalid(
                    "pending proposal candidate does not match its operations",
                ));
            }
        }
        Ok(())
    }
}

/// Decode and validate a persisted workspace. Validation is deliberately part
/// of decoding so future schema versions and corrupted candidates are never
/// presented as accepted state.
pub fn decode_report_workspace(bytes: &[u8]) -> Result<ReportWorkspace, CoreError> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    let schema_version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64);
    // Version 1 predates DOCX-template provenance and structured table blocks.
    if schema_version == Some(1) && value.get("template_import").is_none() {
        value["template_import"] = serde_json::Value::Null;
    }
    // Versions 1 and 2 predate user-facing writer session names.
    if schema_version.is_some_and(|version| version <= 2) {
        value["session_name"] = serde_json::json!(default_report_session_name());
        value["schema_version"] = serde_json::json!(REPORT_WORKSPACE_SCHEMA_VERSION);
    }
    let workspace: ReportWorkspace = serde_json::from_value(value)?;
    workspace.validate()?;
    Ok(workspace)
}

fn default_report_session_name() -> String {
    "Writer Session (1)".to_string()
}

pub fn validate_report_content(content: &ReportContent) -> Result<(), CoreError> {
    validate_content(content)
}

pub fn validate_report_summary(summary: &str) -> Result<(), CoreError> {
    validate_nonempty_text("proposal summary", summary, MAX_PROPOSAL_SUMMARY_CHARACTERS)
}

/// Count common unresolved template markers without retaining or logging any
/// report text. This is a review hint, not a claim that all carryover can be
/// detected automatically.
pub fn report_template_placeholder_count(content: &ReportContent) -> u32 {
    let mut count = placeholder_count(&content.title);
    for section in &content.sections {
        count = count.saturating_add(placeholder_count(&section.heading));
        for block in &section.blocks {
            match block {
                ReportBlock::Paragraph { text } => {
                    count = count.saturating_add(placeholder_count(text));
                }
                ReportBlock::BulletList { items } => {
                    for item in items {
                        count = count.saturating_add(placeholder_count(item));
                    }
                }
                ReportBlock::Table { rows, .. } => {
                    for cell in rows.iter().flatten() {
                        count = count.saturating_add(placeholder_count(cell));
                    }
                }
            }
        }
    }
    count
}

/// Validate Bedrock message ordering and exact tool-use/result correlation.
///
/// A complete protocol starts with a user message, alternates roles, assigns
/// every tool-use ID once, and follows each assistant tool-use message with
/// exactly one user result for every requested ID.
pub fn validate_report_protocol(messages: &[ReportProtocolMessage]) -> Result<(), CoreError> {
    validate_protocol(messages, true)
}

/// Validate a Converse request history, which must end in a user message but
/// otherwise follows the same role and tool-correlation rules.
pub fn validate_report_conversation(messages: &[ReportProtocolMessage]) -> Result<(), CoreError> {
    validate_protocol(messages, false)?;
    if messages.last().map(|message| message.role) != Some(ReportProtocolRole::User) {
        return Err(invalid("a Converse request must end with a user message"));
    }
    Ok(())
}

fn validate_content(content: &ReportContent) -> Result<(), CoreError> {
    validate_nonempty_text("title", &content.title, MAX_TITLE_CHARACTERS)?;
    if content.sections.len() > MAX_REPORT_SECTIONS {
        return Err(invalid(format!(
            "report may contain at most {MAX_REPORT_SECTIONS} sections"
        )));
    }

    let mut ids = HashSet::with_capacity(content.sections.len());
    let mut total_characters = content.title.chars().count();
    let mut total_table_cells = 0_usize;
    for section in &content.sections {
        validate_section(section)?;
        if !ids.insert(section.id) {
            return Err(invalid(format!("duplicate section ID {}", section.id)));
        }
        total_characters += section.heading.chars().count();
        for block in &section.blocks {
            if let ReportBlock::Table { rows, .. } = block {
                total_table_cells = total_table_cells.saturating_add(
                    rows.iter()
                        .map(Vec::len)
                        .fold(0_usize, usize::saturating_add),
                );
                if total_table_cells > MAX_REPORT_TABLE_CELLS {
                    return Err(invalid(format!(
                        "report tables may contain at most {MAX_REPORT_TABLE_CELLS} cells"
                    )));
                }
            }
            total_characters += match block {
                ReportBlock::Paragraph { text } => text.chars().count(),
                ReportBlock::BulletList { items } => {
                    items.iter().map(|item| item.chars().count()).sum()
                }
                ReportBlock::Table { rows, .. } => {
                    rows.iter().flatten().map(|cell| cell.chars().count()).sum()
                }
            };
        }
        if total_characters > MAX_REPORT_TEXT_CHARACTERS {
            return Err(invalid(format!(
                "report text exceeds {MAX_REPORT_TEXT_CHARACTERS} characters"
            )));
        }
    }
    Ok(())
}

fn validate_section(section: &ReportSection) -> Result<(), CoreError> {
    if section.id.is_nil() {
        return Err(invalid("section ID must not be nil"));
    }
    validate_nonempty_text("section heading", &section.heading, MAX_HEADING_CHARACTERS)?;
    validate_blocks(&section.blocks)
}

fn validate_blocks(blocks: &[ReportBlock]) -> Result<(), CoreError> {
    if blocks.len() > MAX_SECTION_BLOCKS {
        return Err(invalid(format!(
            "a section may contain at most {MAX_SECTION_BLOCKS} blocks"
        )));
    }
    for block in blocks {
        match block {
            ReportBlock::Paragraph { text } => {
                validate_nonempty_text("paragraph", text, MAX_PARAGRAPH_CHARACTERS)?;
            }
            ReportBlock::BulletList { items } => {
                if items.is_empty() || items.len() > MAX_BULLET_ITEMS {
                    return Err(invalid(format!(
                        "a bullet list must contain 1 to {MAX_BULLET_ITEMS} items"
                    )));
                }
                for item in items {
                    validate_nonempty_text("bullet item", item, MAX_BULLET_ITEM_CHARACTERS)?;
                }
            }
            ReportBlock::Table {
                rows,
                column_widths,
                ..
            } => validate_table(rows, column_widths.as_deref())?,
        }
    }
    Ok(())
}

fn validate_table(rows: &[Vec<String>], column_widths: Option<&[u16]>) -> Result<(), CoreError> {
    if rows.is_empty() || rows.len() > MAX_TABLE_ROWS {
        return Err(invalid(format!(
            "a table must contain 1 to {MAX_TABLE_ROWS} rows"
        )));
    }
    let columns = rows[0].len();
    if columns == 0 || columns > MAX_TABLE_COLUMNS {
        return Err(invalid(format!(
            "a table must contain 1 to {MAX_TABLE_COLUMNS} columns"
        )));
    }
    let mut any_text = false;
    for row in rows {
        if row.len() != columns {
            return Err(invalid(
                "every table row must contain the same number of cells",
            ));
        }
        for cell in row {
            validate_xml_text("table cell", cell, MAX_TABLE_CELL_CHARACTERS)?;
            any_text |= !cell.trim().is_empty();
        }
    }
    if !any_text {
        return Err(invalid("a table must contain at least one nonempty cell"));
    }

    if let Some(widths) = column_widths {
        if widths.len() != columns || widths.contains(&0) {
            return Err(invalid(
                "table column widths must contain one positive width per column",
            ));
        }
        let total: u32 = widths.iter().map(|width| u32::from(*width)).sum();
        if total != TABLE_WIDTH_BASIS_POINTS {
            return Err(invalid(format!(
                "table column widths must sum to {TABLE_WIDTH_BASIS_POINTS}"
            )));
        }
    }
    Ok(())
}

fn validate_template_import(
    template: &ReportTemplateImport,
    workspace: &ReportWorkspace,
) -> Result<(), CoreError> {
    if template.source_sha256.len() != 64
        || !template
            .source_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid("template source hash must be a SHA-256 hex digest"));
    }
    if template.imported_revision == 0 || template.imported_revision > workspace.draft.revision {
        return Err(invalid(
            "template import revision is outside the accepted report history",
        ));
    }
    if template.imported_at < workspace.created_at || template.imported_at > workspace.updated_at {
        return Err(invalid(
            "template import timestamp is outside the workspace history",
        ));
    }
    if template
        .reviewed_revision
        .is_some_and(|revision| revision != workspace.draft.revision)
    {
        return Err(invalid(
            "template review must apply to the current accepted revision",
        ));
    }
    if template.warnings.len() > 32 {
        return Err(invalid(
            "template import contains too many warning categories",
        ));
    }
    let mut codes = HashSet::with_capacity(template.warnings.len());
    for warning in &template.warnings {
        if warning.count == 0 || !codes.insert(warning.code) {
            return Err(invalid(
                "template warnings must have positive counts and unique codes",
            ));
        }
    }
    Ok(())
}

fn validate_nonempty_text(
    label: &str,
    value: &str,
    max_characters: usize,
) -> Result<(), CoreError> {
    if value.trim().is_empty() {
        return Err(invalid(format!("{label} must not be empty")));
    }
    validate_xml_text(label, value, max_characters)
}

fn validate_xml_text(label: &str, value: &str, max_characters: usize) -> Result<(), CoreError> {
    let characters = value.chars().count();
    if characters > max_characters {
        return Err(invalid(format!(
            "{label} exceeds {max_characters} characters"
        )));
    }
    if value.chars().any(|character| {
        !matches!(
            character,
            '\u{0009}' | '\u{000A}' | '\u{000D}' | '\u{0020}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
        )
    }) {
        return Err(invalid(format!(
            "{label} contains a character that cannot be represented in Word XML"
        )));
    }
    Ok(())
}

fn placeholder_count(text: &str) -> u32 {
    let lowercase = text.to_lowercase();
    ["{{", "<<", "[client", "[name", "[date", "_____"]
        .iter()
        .map(|marker| u32::try_from(lowercase.matches(marker).count()).unwrap_or(u32::MAX))
        .fold(0_u32, u32::saturating_add)
}

fn validate_proposal(
    proposal: &ReportProposal,
    workspace: Option<&ReportWorkspace>,
) -> Result<(), CoreError> {
    if proposal.id.is_nil() || proposal.report_id.is_nil() {
        return Err(invalid("proposal IDs must not be nil"));
    }
    if proposal.model_id.trim().is_empty() {
        return Err(invalid("proposal model ID must not be empty"));
    }
    if proposal.tool_use_id.trim().is_empty() {
        return Err(invalid("proposal tool-use ID must not be empty"));
    }
    validate_report_summary(&proposal.summary)?;
    if proposal.operations.is_empty() || proposal.operations.len() > MAX_PROPOSAL_OPERATIONS {
        return Err(invalid(format!(
            "a proposal must contain 1 to {MAX_PROPOSAL_OPERATIONS} operations"
        )));
    }
    validate_content(&proposal.proposed_content)?;
    if let Some(workspace) = workspace
        && proposal.report_id != workspace.report_id
    {
        return Err(invalid("proposal belongs to another report"));
    }
    Ok(())
}

fn validate_turn(turn: &ReportAuthoringTurn) -> Result<(), CoreError> {
    if turn.id.is_nil() {
        return Err(invalid("turn ID must not be nil"));
    }
    if turn.model_id.trim().is_empty() {
        return Err(invalid("turn model ID must not be empty"));
    }
    if turn.messages.is_empty() {
        return Err(invalid("turn must contain protocol messages"));
    }
    if turn.converse_calls == 0 {
        return Err(invalid("turn must contain at least one Converse call"));
    }
    if turn.created_at > turn.completed_at {
        return Err(invalid("turn completed_at precedes created_at"));
    }
    if turn.messages.iter().any(|message| {
        message.content.iter().any(|block| {
            matches!(
                block,
                ReportProtocolBlock::ReasoningText { .. }
                    | ReportProtocolBlock::ReasoningRedacted { .. }
            )
        })
    }) {
        return Err(invalid("persisted turns must not retain model reasoning"));
    }
    validate_protocol(&turn.messages, true)?;

    let counted_tool_uses = turn
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter(|block| matches!(block, ReportProtocolBlock::ToolUse { .. }))
        .count();
    if usize::try_from(turn.tool_uses).ok() != Some(counted_tool_uses) {
        return Err(invalid("turn tool-use count does not match its protocol"));
    }
    Ok(())
}

fn validate_protocol(
    messages: &[ReportProtocolMessage],
    require_terminal_assistant: bool,
) -> Result<(), CoreError> {
    if messages.is_empty() {
        return Err(invalid("protocol messages must not be empty"));
    }

    let mut all_tool_ids = HashSet::new();
    let mut expected_results: Option<HashSet<&str>> = None;
    for (index, message) in messages.iter().enumerate() {
        let expected_role = if index % 2 == 0 {
            ReportProtocolRole::User
        } else {
            ReportProtocolRole::Assistant
        };
        if message.role != expected_role {
            return Err(invalid(
                "protocol roles must alternate from user to assistant",
            ));
        }
        if message.content.is_empty() {
            return Err(invalid("protocol message content must not be empty"));
        }

        let mut result_ids = HashSet::new();
        let mut use_ids = HashSet::new();
        for block in &message.content {
            match block {
                ReportProtocolBlock::Text { text } => {
                    if text.is_empty() {
                        return Err(invalid("protocol text block must not be empty"));
                    }
                    if expected_results.is_some() {
                        return Err(invalid(
                            "a correlated tool-result message must contain only results",
                        ));
                    }
                }
                ReportProtocolBlock::ReasoningText { text, .. } => {
                    if message.role != ReportProtocolRole::Assistant {
                        return Err(invalid("reasoning blocks must have the assistant role"));
                    }
                    if text.is_empty() {
                        return Err(invalid("reasoning text must not be empty"));
                    }
                }
                ReportProtocolBlock::ReasoningRedacted { data } => {
                    if message.role != ReportProtocolRole::Assistant {
                        return Err(invalid("reasoning blocks must have the assistant role"));
                    }
                    if data.is_empty() {
                        return Err(invalid("redacted reasoning must not be empty"));
                    }
                }
                ReportProtocolBlock::ToolUse {
                    tool_use_id, name, ..
                } => {
                    if message.role != ReportProtocolRole::Assistant {
                        return Err(invalid("tool uses must have the assistant role"));
                    }
                    if tool_use_id.trim().is_empty() || name.trim().is_empty() {
                        return Err(invalid("tool-use ID and name must not be empty"));
                    }
                    if !all_tool_ids.insert(tool_use_id.as_str())
                        || !use_ids.insert(tool_use_id.as_str())
                    {
                        return Err(invalid("tool-use IDs must be unique across a turn"));
                    }
                }
                ReportProtocolBlock::ToolResult { tool_use_id, .. } => {
                    if message.role != ReportProtocolRole::User {
                        return Err(invalid("tool results must have the user role"));
                    }
                    if tool_use_id.trim().is_empty() {
                        return Err(invalid("tool-result ID must not be empty"));
                    }
                    if !result_ids.insert(tool_use_id.as_str()) {
                        return Err(invalid("a tool use must have exactly one result"));
                    }
                }
            }
        }

        if let Some(expected) = expected_results.take() {
            if result_ids != expected {
                return Err(invalid(
                    "tool results must correlate exactly with the preceding tool uses",
                ));
            }
            if !use_ids.is_empty() {
                return Err(invalid("a user tool-result message cannot request tools"));
            }
        } else if !result_ids.is_empty() {
            return Err(invalid("tool results have no preceding tool uses"));
        }

        if !use_ids.is_empty() {
            expected_results = Some(use_ids);
        }
    }

    if expected_results.is_some() {
        return Err(invalid("protocol ends with unresolved tool uses"));
    }
    if require_terminal_assistant
        && messages.last().map(|message| message.role) != Some(ReportProtocolRole::Assistant)
    {
        return Err(invalid(
            "complete protocol must end with an assistant message",
        ));
    }
    Ok(())
}

fn protocol_history_bytes(turns: &[ReportAuthoringTurn]) -> Result<usize, CoreError> {
    Ok(serde_json::to_vec(turns)?.len())
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidReport(message.into())
}
