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
//! - [`DraftRun::record_snapshot`] and [`RunSection::records`] are the run's
//!   own answer to "what was this written from". The snapshot is captured once
//!   before the first call; each section records whether it read that snapshot
//!   or a restriction of its own. Both are additive and optional, so a run
//!   written by an earlier build still decodes at the same schema version.

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
/// Ceiling for the planner's one-line reason a record matters to a section.
/// Short on purpose: it is a pointer to the record, not a summary of it.
pub const MAX_EVIDENCE_RELEVANCE_CHARACTERS: usize = 200;
/// Ceiling on the PHI-free codes a plan carries about its own weak spots.
/// Bounded so a pathological plan cannot grow the run object without limit.
pub const MAX_PLAN_WARNINGS: usize = 200;

/// Ceiling on a section's user-curated record restriction.
///
/// Deliberately larger than [`MAX_PLAN_EVIDENCE`]: evidence is the planner's
/// shortlist of what a section rests on, while a curated list is the whole
/// world a section's writer is allowed to see, and a clinician restricting a
/// section to "the four BASC protocols and every teacher form" needs room for
/// the second without being pushed into the first's shape. Bounded all the
/// same, because the list is durable state that has to fit in a run object.
pub const MAX_CURATED_RECORDS: usize = 16;

/// Record filenames are client-chosen and can be long; this only stops a
/// pathological value from being persisted.
const MAX_RECORD_FILENAME_CHARACTERS: usize = 1_024;

/// Ceiling on the record corpus one run may record having been built from.
///
/// Generous rather than tight: the corpus preflight already refuses a record
/// set too large to send, so this only stops a pathological listing from
/// growing the run object without limit.
pub const MAX_RUN_SNAPSHOT_FILES: usize = 1_000;

/// Ceiling for the PHI-free note saying why a record could not be read.
pub const MAX_RECORD_UNAVAILABLE_REASON_CHARACTERS: usize = 300;

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
    /// The record corpus this run was built from, captured once before the
    /// first model call.
    ///
    /// `None` on a run recorded before the field existed, and on one that
    /// never reached the point of loading records. It is what makes
    /// [`RunSectionRecords::SharedCorpus`] mean something specific rather
    /// than "everything, presumably".
    #[serde(default)]
    pub record_snapshot: Option<RunRecordSnapshot>,
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
    /// as `code:detail` strings — evidence naming a file the client does not
    /// have, a draft row with nothing left backing it. Codes and filenames
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
    /// The record restriction for this section, as filenames the corpus lists.
    ///
    /// Absent leaves the row's restriction alone; an empty list clears it and
    /// puts the section back on the shared corpus, exactly as an all-whitespace
    /// `instruction` clears the instruction. A non-empty list is validated
    /// against the client's records before it is stored.
    #[serde(default)]
    pub curated_records: Option<Vec<String>>,
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
    /// The records this section's writer may read, when the clinician has
    /// restricted it to a subset. `None` — the default, and what the planner
    /// always produces — means the shared corpus every other section sees.
    ///
    /// The planner never sets this: it is a user decision taken at the plan
    /// gate, which is why the plan tool schema has no field for it. The run
    /// object embeds the plan, so the durable record of what one section's
    /// model call could see survives resume, audit, and export for free.
    ///
    /// `Some` is always a non-empty list of filenames as the record corpus
    /// lists them. An empty restriction would be a section drafted from
    /// nothing, which is a plan mistake rather than a request, so the
    /// validator refuses it.
    #[serde(default)]
    pub curated_records: Option<Vec<String>>,
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
    /// Which records were in the model call that wrote this section.
    ///
    /// `None` for a section no model was ever asked about — pending, kept, or
    /// skipped by the plan — and for a section written before the field
    /// existed. A section a model *did* write always carries one, because the
    /// question "what was this written from" has an answer for it.
    #[serde(default)]
    pub records: Option<RunSectionRecords>,
    #[specta(type = String)]
    pub updated_at: Timestamp,
}

/// The record corpus a drafting run was built from, as the run recorded it.
///
/// Filenames, not text. The run object already carries record filenames on its
/// plan evidence, its curated restrictions, and its citations, so this adds no
/// new class of data to the object — it makes the set the writer actually saw
/// answerable after the fact instead of only inferable from the plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RunRecordSnapshot {
    /// Files whose text reached the shared corpus block, in the byte order the
    /// block listed them — which is the order the model read them in.
    pub files: Vec<RunRecordFile>,
    /// Files the run could see but could not read.
    pub unavailable: Vec<RunUnavailableRecord>,
    /// Characters of record text in the shared corpus block.
    pub total_characters: u64,
    #[specta(type = String)]
    pub captured_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RunRecordFile {
    pub filename: String,
    /// Digest of the text the run was shown, so a record edited afterwards is
    /// distinguishable from the one the writer read.
    pub sha256: String,
    pub characters: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RunUnavailableRecord {
    pub filename: String,
    /// PHI-free reason: an extraction or storage code, never record text.
    pub reason: String,
}

/// What one section's writer was given to read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunSectionRecords {
    /// The run's whole [`DraftRun::record_snapshot`] was in the call.
    SharedCorpus,
    /// The clinician restricted this section, and exactly these files reached
    /// its call. Not the same list as the plan row's `curated_records`: a
    /// curated file the client no longer has never made it into the request,
    /// and this records what was sent rather than what was asked for. Empty
    /// when every curated file had gone missing — a real outcome, and one the
    /// reader should be able to see rather than have normalized away.
    Curated { filenames: Vec<String> },
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
        if let Some(snapshot) = &self.record_snapshot {
            snapshot.validate()?;
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
        if let Some(RunSectionRecords::Curated { filenames }) = &self.records {
            if filenames.len() > MAX_CURATED_RECORDS {
                return Err(invalid(format!(
                    "a section may record at most {MAX_CURATED_RECORDS} curated records"
                )));
            }
            let mut seen = HashSet::with_capacity(filenames.len());
            for filename in filenames {
                validate_nonempty_text(
                    "curated record filename",
                    filename,
                    MAX_RECORD_FILENAME_CHARACTERS,
                )?;
                if !seen.insert(filename.as_str()) {
                    return Err(invalid(format!(
                        "curated record {filename} is recorded twice for the same section"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl RunRecordSnapshot {
    fn validate(&self) -> Result<(), CoreError> {
        if self.files.len() + self.unavailable.len() > MAX_RUN_SNAPSHOT_FILES {
            return Err(invalid(format!(
                "a drafting run may record at most {MAX_RUN_SNAPSHOT_FILES} records in its snapshot"
            )));
        }
        let mut seen = HashSet::with_capacity(self.files.len() + self.unavailable.len());
        for file in &self.files {
            validate_nonempty_text(
                "record snapshot filename",
                &file.filename,
                MAX_RECORD_FILENAME_CHARACTERS,
            )?;
            validate_nonempty_text("record snapshot digest", &file.sha256, 128)?;
            if !seen.insert(file.filename.as_str()) {
                return Err(invalid(format!(
                    "record {} appears twice in the run's snapshot",
                    file.filename
                )));
            }
        }
        for file in &self.unavailable {
            validate_nonempty_text(
                "record snapshot filename",
                &file.filename,
                MAX_RECORD_FILENAME_CHARACTERS,
            )?;
            validate_nonempty_text(
                "record unavailability reason",
                &file.reason,
                MAX_RECORD_UNAVAILABLE_REASON_CHARACTERS,
            )?;
            if !seen.insert(file.filename.as_str()) {
                return Err(invalid(format!(
                    "record {} appears twice in the run's snapshot",
                    file.filename
                )));
            }
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
        self.validate_curated_records()?;
        Ok(())
    }

    /// The restriction is a hard context boundary the run is the audit record
    /// for, so the shapes that cannot mean anything are refused rather than
    /// normalized away: an empty list would be a section drafted from no
    /// records at all, and a repeated filename would make the durable record
    /// disagree with the set the model was actually shown.
    fn validate_curated_records(&self) -> Result<(), CoreError> {
        let Some(curated) = &self.curated_records else {
            return Ok(());
        };
        if curated.is_empty() {
            return Err(invalid(
                "a curated record restriction must name at least one record; leave it unset to \
                 draft the section from every readable record",
            ));
        }
        if curated.len() > MAX_CURATED_RECORDS {
            return Err(invalid(format!(
                "a plan entry may restrict drafting to at most {MAX_CURATED_RECORDS} records"
            )));
        }
        let mut seen = HashSet::with_capacity(curated.len());
        for filename in curated {
            validate_nonempty_text(
                "curated record filename",
                filename,
                MAX_RECORD_FILENAME_CHARACTERS,
            )?;
            if !seen.insert(filename.as_str()) {
                return Err(invalid(format!(
                    "curated record {filename} is named twice for the same section"
                )));
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
