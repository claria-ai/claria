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

pub const REPORT_WORKSPACE_SCHEMA_VERSION: u32 = 1;
pub const MAX_REPORT_SECTIONS: usize = 100;
pub const MAX_SECTION_BLOCKS: usize = 200;
pub const MAX_REPORT_TEXT_CHARACTERS: usize = 500_000;
pub const MAX_PROPOSAL_OPERATIONS: usize = 25;
pub const MAX_REPORT_TURNS: usize = 20;
pub const MAX_REPORT_PROTOCOL_BYTES: usize = 512 * 1024;

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
    pub draft: ReportDraft,
    pub session: ReportSession,
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
    Paragraph { text: String },
    BulletList { items: Vec<String> },
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
        self.draft.validate()?;
        self.session.validate(self)?;
        Ok(())
    }

    /// Add a complete turn and prune only complete, oldest turns to the
    /// retention limits. The newest turn is retained even if it alone exceeds
    /// the byte ceiling, so tool-use/result correlation is never split.
    pub fn push_turn(&mut self, turn: ReportAuthoringTurn) -> Result<(), CoreError> {
        validate_turn(&turn)?;
        self.session.turns.push(turn);
        while self.session.turns.len() > MAX_REPORT_TURNS {
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
    let workspace: ReportWorkspace = serde_json::from_slice(bytes)?;
    workspace.validate()?;
    Ok(workspace)
}

pub fn validate_report_content(content: &ReportContent) -> Result<(), CoreError> {
    validate_content(content)
}

pub fn validate_report_summary(summary: &str) -> Result<(), CoreError> {
    validate_nonempty_text("proposal summary", summary, MAX_PROPOSAL_SUMMARY_CHARACTERS)
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
    for section in &content.sections {
        validate_section(section)?;
        if !ids.insert(section.id) {
            return Err(invalid(format!("duplicate section ID {}", section.id)));
        }
        total_characters += section.heading.chars().count();
        for block in &section.blocks {
            total_characters += match block {
                ReportBlock::Paragraph { text } => text.chars().count(),
                ReportBlock::BulletList { items } => {
                    items.iter().map(|item| item.chars().count()).sum()
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
