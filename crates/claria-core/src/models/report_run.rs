//! Durable, per-section state for one whole-report drafting run.
//!
//! A [`DraftRun`] is the writer's resumable unit of work. Whole-report
//! generation used to stage every section in memory and persist only at the
//! end, so a run that died on its forty-seventh section lost all forty-six.
//! The run object is written back after every landed section instead: the
//! conversation never believes more than durable truth.
//!
//! Three semantics carry most of the weight:
//!
//! - [`RunSectionState::Kept`] means the plan deliberately leaves the
//!   section's base-revision content alone. Every section of the report gets a
//!   [`RunSection`] row, kept and skipped ones included, so `sections` is
//!   always the complete section universe. The finish cut assembles from it
//!   rather than re-deciding which sections exist.
//! - [`RunSectionState::Drafting`] is transient and only ever correct while a
//!   section is in flight. A section found `Drafting` on load belongs to an
//!   interrupted run; the caller demotes it with
//!   [`DraftRun::demote_interrupted_sections`] before resuming.
//! - [`DraftRun::base_revision`] is the accepted revision the run builds on.
//!   The finish cut passes it as the expected revision, so a report edited
//!   underneath a run conflicts instead of silently overwriting.

use std::collections::HashSet;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::CoreError,
    models::report::{
        MAX_HEADING_CHARACTERS, MAX_REPORT_SECTIONS, MAX_TITLE_CHARACTERS, ReportBlock,
        validate_blocks, validate_nonempty_text, validate_xml_text,
    },
};

pub const DRAFT_RUN_SCHEMA_VERSION: u32 = 1;

/// Initial guidance plus every instruction added on resume.
pub const MAX_RUN_INSTRUCTIONS: usize = 20;

/// Ceiling for one writer instruction, wherever it is typed: the whole-report
/// guidance box, a saved library prompt body, or a [`RunInstruction`] added
/// when a stopped run is picked back up.
pub const MAX_INSTRUCTION_CHARACTERS: usize = 20_000;

/// Shortest quote worth checking against a record. Below this a "quote" is a
/// fragment that matches half the corpus, so the completion gate could not
/// tell a real citation from an accident.
pub const MIN_CITATION_QUOTE_CHARACTERS: usize = 10;
pub const MAX_CITATION_QUOTE_CHARACTERS: usize = 300;
pub const MAX_SECTION_CITATIONS: usize = 20;
/// Ceiling for the PHI-free note explaining why a section failed.
pub const MAX_SECTION_ERROR_CHARACTERS: usize = 500;
pub const MAX_PLAN_SCOPE_CHARACTERS: usize = 600;
pub const MAX_PLAN_EVIDENCE: usize = 8;
/// Ceiling for the planner's one-line reason an evidence quote matters. Short
/// on purpose: it is a pointer to the quote, not a second copy of it.
pub const MAX_EVIDENCE_RELEVANCE_CHARACTERS: usize = 200;
/// Ceiling on the PHI-free codes a plan carries about its own weak spots.
/// Bounded so a pathological plan cannot grow the run object without limit.
pub const MAX_PLAN_WARNINGS: usize = 200;

/// Record filenames are client-chosen and can be long; this only stops a
/// pathological value from being persisted.
const MAX_RECORD_FILENAME_CHARACTERS: usize = 1_024;

/// One resumable whole-report drafting run.
///
/// The object is rewritten (conditionally, on its ETag) after every section
/// the writer lands, so an interrupted run resumes from durable state instead
/// of restarting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct DraftRun {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub report_id: Uuid,
    pub client_id: Uuid,
    /// Accepted revision the run builds on. The finish cut uses it as the
    /// expected revision, so concurrent edits to the report conflict rather
    /// than being overwritten by assembled sections.
    pub base_revision: u64,
    pub status: DraftRunStatus,
    pub plan: Option<RunPlan>,
    pub title: Option<String>,
    /// Every section of the report, in no particular order — read
    /// [`RunSection::position`] for document order.
    pub sections: Vec<RunSection>,
    pub instructions: Vec<RunInstruction>,
    pub writer_model_id: String,
    /// Revision produced by the finish cut. Present only once the run is
    /// [`DraftRunStatus::Completed`].
    pub finalized_revision: Option<u64>,
    /// The run was finalized from a stopped state: sections that never landed
    /// were skipped rather than drafted.
    pub partial: bool,
    #[specta(type = String)]
    pub created_at: Timestamp,
    #[specta(type = String)]
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DraftRunStatus {
    Planning,
    AwaitingApproval,
    Drafting,
    Stopped,
    Failed,
    Completed,
    Abandoned,
}

/// User guidance for the run: the instruction it started from, plus anything
/// added when a stopped run was picked back up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RunInstruction {
    pub text: String,
    #[specta(type = String)]
    pub added_at: Timestamp,
}

/// The section plan the user approved before drafting started.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RunPlan {
    /// Model that produced the plan, which is not the model that writes the
    /// sections.
    pub model_id: String,
    pub entries: Vec<PlanEntry>,
    /// The user changed the plan at the gate before approving it.
    pub user_edited: bool,
    /// Nobody decided this plan: it was manufactured from the sections the
    /// report already had, one `Draft` row each, on a path with no planning
    /// pass in front of it. A synthetic plan has never been through evidence
    /// assignment, so the completion gate cannot hold a section against it for
    /// citing nothing. Inherited when a plan is rebuilt from an earlier one,
    /// because a derived plan is no more decided than its source.
    #[serde(default)]
    pub synthetic: bool,
    /// Unset while the plan is still waiting at the gate.
    #[specta(type = Option<String>)]
    pub approved_at: Option<Timestamp>,
    /// What the host could not confirm about the plan the model produced,
    /// as `code:detail` strings — an evidence quote that resolves against no
    /// record, a draft row with nothing left backing it. Codes and filenames
    /// only, never record text, so the gate can show them and the console can
    /// log them.
    ///
    /// A warning never blocks the plan: the user fixes it at the gate.
    #[serde(default)]
    pub plan_warnings: Vec<String>,
    #[specta(type = String)]
    pub created_at: Timestamp,
}

/// One section's worth of gate edits. Absent fields are left exactly as the
/// planner wrote them, so the pane can save the one control the user touched
/// without restating the rest of the row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct PlanEntryEdit {
    pub section_id: Uuid,
    #[serde(default)]
    pub intent: Option<SectionIntent>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub evidence: Option<Vec<EvidenceRef>>,
    /// Per-section steering. An all-whitespace value clears the instruction
    /// rather than storing a blank one.
    #[serde(default)]
    pub instruction: Option<String>,
}

/// What the plan decided for exactly one section. There is one entry per
/// [`RunSection`] and no entry without one.
///
/// Renamed across the IPC boundary: the provisioner exports its own
/// `PlanEntry`, and two types of the same name are a bindings-export panic at
/// startup rather than a compile error.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[specta(rename = "DraftPlanEntry")]
pub struct PlanEntry {
    pub section_id: Uuid,
    pub heading: String,
    pub intent: SectionIntent,
    /// The report cannot be considered complete while this section is empty.
    pub required: bool,
    /// What the section should cover, in the planner's words. May be empty for
    /// a section the plan does not intend to write.
    pub scope: String,
    pub evidence: Vec<EvidenceRef>,
    /// Per-section steering that overrides the run-wide instructions.
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SectionIntent {
    Draft,
    Rewrite,
    /// Leave the section's base-revision content exactly as it is.
    Keep,
    Skip,
}

/// A record the planner expects a section to draw on. Records are referenced
/// by filename; the note is the planner's own reason, never record text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct EvidenceRef {
    pub filename: String,
    pub note: Option<String>,
}

/// The durable state machine for one section of the report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RunSection {
    pub section_id: Uuid,
    pub heading: String,
    /// Zero-based ordering slot, unique across the run. A drafted section's
    /// slot is where the writer put it, so sorting the drafted rows by it
    /// reproduces the document the run is assembling; everything still
    /// undecided trails behind them in the order the report supplied it.
    pub position: u32,
    pub state: RunSectionState,
    /// Staged content that has already survived a write. Nonempty only in
    /// [`RunSectionState::Drafted`]; the finish cut reads it verbatim.
    pub blocks: Vec<ReportBlock>,
    pub citations: Vec<RecordCitation>,
    /// How many times the writer has tried this section.
    pub attempts: u32,
    /// PHI-free note about why the section failed: error codes and counts,
    /// never record or draft content.
    pub error: Option<String>,
    #[specta(type = String)]
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RunSectionState {
    Pending,
    /// Transient: correct only while the writer is mid-section. Any section
    /// still `Drafting` when a run is loaded was interrupted and must be
    /// demoted to `Pending` — see [`DraftRun::demote_interrupted_sections`].
    Drafting,
    Drafted,
    Failed,
    /// Drafted, but a review pass raised a finding against it.
    Flagged,
    Skipped,
    /// The base-revision content stands; the run does not touch this section.
    Kept,
}

/// A quote the writer attributed to a record, used by the completion gate to
/// check that cited text actually exists in the client's records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RecordCitation {
    pub filename: String,
    pub quote: String,
}

impl DraftRun {
    /// Demote every section left in the transient [`RunSectionState::Drafting`]
    /// state back to [`RunSectionState::Pending`], returning how many were
    /// demoted. Callers run this on every load: a persisted `Drafting` section
    /// is by definition one whose write never landed.
    pub fn demote_interrupted_sections(&mut self, now: Timestamp) -> usize {
        let mut demoted = 0;
        for section in &mut self.sections {
            if section.state == RunSectionState::Drafting {
                section.state = RunSectionState::Pending;
                section.updated_at = now;
                demoted += 1;
            }
        }
        demoted
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version > DRAFT_RUN_SCHEMA_VERSION {
            return Err(invalid(format!(
                "drafting run schema version {} is newer than supported version {DRAFT_RUN_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        if self.schema_version != DRAFT_RUN_SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported drafting run schema version {}",
                self.schema_version
            )));
        }
        if self.run_id.is_nil() || self.report_id.is_nil() || self.client_id.is_nil() {
            return Err(invalid("drafting run IDs must not be nil"));
        }
        if self.created_at > self.updated_at {
            return Err(invalid("drafting run updated_at precedes created_at"));
        }
        validate_nonempty_text("writer model ID", &self.writer_model_id, 200)?;
        if let Some(title) = &self.title {
            validate_nonempty_text("drafting run title", title, MAX_TITLE_CHARACTERS)?;
        }

        self.validate_instructions()?;
        let section_ids = self.validate_sections()?;
        if let Some(plan) = &self.plan {
            plan.validate(&section_ids)?;
        }

        if let Some(revision) = self.finalized_revision {
            if self.status != DraftRunStatus::Completed {
                return Err(invalid(
                    "a drafting run with a finalized revision must be completed",
                ));
            }
            if revision <= self.base_revision {
                return Err(invalid(
                    "the finalized revision must be newer than the revision the run built on",
                ));
            }
        } else if self.partial {
            return Err(invalid(
                "a partial drafting run must record the revision it was finalized into",
            ));
        }
        Ok(())
    }

    fn validate_instructions(&self) -> Result<(), CoreError> {
        if self.instructions.len() > MAX_RUN_INSTRUCTIONS {
            return Err(invalid(format!(
                "a drafting run may contain at most {MAX_RUN_INSTRUCTIONS} instructions"
            )));
        }
        for instruction in &self.instructions {
            validate_nonempty_text(
                "drafting run instruction",
                &instruction.text,
                MAX_INSTRUCTION_CHARACTERS,
            )?;
        }
        Ok(())
    }

    fn validate_sections(&self) -> Result<HashSet<Uuid>, CoreError> {
        if self.sections.len() > MAX_REPORT_SECTIONS {
            return Err(invalid(format!(
                "a drafting run may contain at most {MAX_REPORT_SECTIONS} sections"
            )));
        }
        let mut ids = HashSet::with_capacity(self.sections.len());
        let mut positions = HashSet::with_capacity(self.sections.len());
        for section in &self.sections {
            if section.section_id.is_nil() {
                return Err(invalid("section ID must not be nil"));
            }
            if !ids.insert(section.section_id) {
                return Err(invalid(format!(
                    "duplicate section ID {}",
                    section.section_id
                )));
            }
            if !positions.insert(section.position) {
                return Err(invalid(format!(
                    "duplicate section position {}",
                    section.position
                )));
            }
            section.validate()?;
        }
        Ok(ids)
    }
}

impl RunSection {
    fn validate(&self) -> Result<(), CoreError> {
        validate_nonempty_text("section heading", &self.heading, MAX_HEADING_CHARACTERS)?;
        validate_blocks(&self.blocks)?;
        if !self.blocks.is_empty() && self.state != RunSectionState::Drafted {
            return Err(invalid(
                "only a drafted section may carry staged content blocks",
            ));
        }
        if self.citations.len() > MAX_SECTION_CITATIONS {
            return Err(invalid(format!(
                "a section may cite at most {MAX_SECTION_CITATIONS} record quotes"
            )));
        }
        for citation in &self.citations {
            validate_nonempty_text(
                "citation filename",
                &citation.filename,
                MAX_RECORD_FILENAME_CHARACTERS,
            )?;
            validate_nonempty_text(
                "citation quote",
                &citation.quote,
                MAX_CITATION_QUOTE_CHARACTERS,
            )?;
            if citation.quote.chars().count() < MIN_CITATION_QUOTE_CHARACTERS {
                return Err(invalid(format!(
                    "a citation quote must be at least {MIN_CITATION_QUOTE_CHARACTERS} characters"
                )));
            }
        }
        if let Some(error) = &self.error {
            validate_nonempty_text("section error", error, MAX_SECTION_ERROR_CHARACTERS)?;
        }
        Ok(())
    }
}

impl RunPlan {
    fn validate(&self, section_ids: &HashSet<Uuid>) -> Result<(), CoreError> {
        validate_nonempty_text("planner model ID", &self.model_id, 200)?;
        if self
            .approved_at
            .is_some_and(|approved_at| approved_at < self.created_at)
        {
            return Err(invalid("the plan was approved before it was created"));
        }
        if self.entries.len() > MAX_REPORT_SECTIONS {
            return Err(invalid(format!(
                "a plan may contain at most {MAX_REPORT_SECTIONS} entries"
            )));
        }
        if self.plan_warnings.len() > MAX_PLAN_WARNINGS {
            return Err(invalid(format!(
                "a plan may carry at most {MAX_PLAN_WARNINGS} warnings"
            )));
        }
        for warning in &self.plan_warnings {
            validate_nonempty_text(
                "plan warning",
                warning,
                MAX_RECORD_FILENAME_CHARACTERS + MAX_HEADING_CHARACTERS,
            )?;
        }

        let mut planned = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            if !planned.insert(entry.section_id) {
                return Err(invalid(format!(
                    "duplicate plan entry for section {}",
                    entry.section_id
                )));
            }
            if !section_ids.contains(&entry.section_id) {
                return Err(invalid(format!(
                    "plan entry {} does not match a section of the run",
                    entry.section_id
                )));
            }
            entry.validate()?;
        }
        if planned.len() != section_ids.len() {
            return Err(invalid("every section of the run needs one plan entry"));
        }
        Ok(())
    }
}

impl PlanEntry {
    fn validate(&self) -> Result<(), CoreError> {
        validate_nonempty_text("section heading", &self.heading, MAX_HEADING_CHARACTERS)?;
        validate_xml_text("plan scope", &self.scope, MAX_PLAN_SCOPE_CHARACTERS)?;
        if let Some(instruction) = &self.instruction {
            validate_nonempty_text(
                "plan section instruction",
                instruction,
                MAX_INSTRUCTION_CHARACTERS,
            )?;
        }
        if self.evidence.len() > MAX_PLAN_EVIDENCE {
            return Err(invalid(format!(
                "a plan entry may reference at most {MAX_PLAN_EVIDENCE} records"
            )));
        }
        for evidence in &self.evidence {
            validate_nonempty_text(
                "evidence filename",
                &evidence.filename,
                MAX_RECORD_FILENAME_CHARACTERS,
            )?;
            if let Some(note) = &evidence.note {
                validate_nonempty_text("evidence note", note, MAX_EVIDENCE_RELEVANCE_CHARACTERS)?;
            }
        }
        Ok(())
    }
}

/// Decode and validate a persisted drafting run. Validation is part of
/// decoding so a corrupt or future-versioned run is never presented as
/// resumable state.
pub fn decode_draft_run(bytes: &[u8]) -> Result<DraftRun, CoreError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    match value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version) if version == u64::from(DRAFT_RUN_SCHEMA_VERSION) => {}
        Some(version) if version > u64::from(DRAFT_RUN_SCHEMA_VERSION) => {
            return Err(invalid(format!(
                "drafting run schema version {version} is newer than supported version \
                 {DRAFT_RUN_SCHEMA_VERSION}; update Claria to open this run"
            )));
        }
        Some(version) => {
            return Err(invalid(format!(
                "unsupported drafting run schema version {version}"
            )));
        }
        None => return Err(invalid("drafting run is missing its schema version")),
    }
    let run: DraftRun = serde_json::from_value(value)?;
    run.validate()?;
    Ok(run)
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidDraftRun(message.into())
}
