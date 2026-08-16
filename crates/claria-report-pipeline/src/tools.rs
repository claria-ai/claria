//! Host-side execution of the report tool protocol: bounded record reads,
//! proposal staging, and the durable full-draft run.

use std::collections::{HashMap, HashSet};

use aws_sdk_s3::Client as S3Client;
use claria_bedrock::{
    error::BedrockError,
    report::{
        self, FinishFullDraftRequest, MarkSectionFailedRequest, ProposeReportChangesRequest,
        RecordCitationRequest, ReportBlockRequest, ReportProposalOperationRequest, ReportToolCall,
        ReportToolRequest, SetFullDraftTitleRequest, SkipFullDraftSectionRequest,
        WriteFullDraftSectionRequest,
    },
};
use claria_core::models::{
    report::{
        ReportBlock, ReportContent, ReportOperation, ReportProposal, ReportSection,
        ReportToolResultStatus, ReportWorkspace, validate_report_content, validate_report_summary,
    },
    report_run::{
        DraftRun, MAX_CITATION_QUOTE_CHARACTERS, MAX_SECTION_CITATIONS,
        MAX_SECTION_ERROR_CHARACTERS, MIN_CITATION_QUOTE_CHARACTERS, PlanEntry, RecordCitation,
        RunSection, RunSectionState, SectionIntent,
    },
};
use claria_report_store::{LoadedRun, ReportFailureCode, save_draft_run};
use claria_storage::error::StorageError;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    DEFAULT_READ_LIMIT, MAX_LIST_RESULT_BYTES, MAX_LISTED_FILES, MAX_READ_CHARACTERS_PER_TURN,
    MAX_READ_LIMIT, record_context::RecordInventoryEntry, turn::TurnRunFailure,
};

pub(crate) struct ExecutedTool {
    pub(crate) status: ReportToolResultStatus,
    pub(crate) content: serde_json::Value,
}

/// The whole-report draft in flight, backed by its durable run object.
///
/// Every accepted title, section write, and skip is written through to S3
/// before its success `tool_result` is handed back, so the conversation never
/// believes more than durable truth. Only the assembled finish result is held
/// in memory: it is consumed by the revision cut moments later, and a run that
/// dies between the two resumes from the sections that already landed.
struct FullDraftRun<'a> {
    run: &'a mut LoadedRun,
    finalized: Option<FinalizedFullDraft>,
    /// Consecutive rejected `write_full_draft_section` calls per section. A
    /// section that keeps failing validation is bounded rather than allowed to
    /// burn the run's whole call budget re-attempting the same mistake; a
    /// successful write clears its count.
    consecutive_write_failures: HashMap<Uuid, u32>,
    /// Sections already told once that a skip contradicts an approved plan.
    /// The corrective error is one strike, not a wall.
    plan_conflicts_raised: HashSet<Uuid>,
}

/// Rejected writes tolerated for one section before the host fails it and
/// tells the writer to move on. Two attempts is a repair round; a third
/// identical failure is a section the writer cannot land, and spending the
/// remaining call budget on it costs every section after it.
const MAX_SECTION_WRITE_ATTEMPTS: u32 = 3;

pub(crate) struct FinalizedFullDraft {
    pub(crate) content: ReportContent,
    pub(crate) summary: String,
}

pub(crate) struct ToolExecutionContext<'a> {
    s3: &'a S3Client,
    bucket: &'a str,
    inventory: &'a [RecordInventoryEntry],
    workspace: &'a ReportWorkspace,
    model_id: &'a str,
    read_characters: u32,
    read_cache: HashMap<String, String>,
    pub(crate) staged_proposal: Option<ReportProposal>,
    full_draft: Option<FullDraftRun<'a>>,
    /// Filenames the run's record snapshot actually carries, so a citation can
    /// be checked against what the model was given rather than against the
    /// client's whole bucket.
    citable_filenames: &'a [String],
}

impl<'a> ToolExecutionContext<'a> {
    // The tool loop's collaborators are all borrowed for the life of the turn;
    // bundling them into a struct just to pass one argument would hide which
    // of them the full-draft path needs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        s3: &'a S3Client,
        bucket: &'a str,
        inventory: &'a [RecordInventoryEntry],
        workspace: &'a ReportWorkspace,
        model_id: &'a str,
        full_draft_run: Option<&'a mut LoadedRun>,
        citable_filenames: &'a [String],
    ) -> Self {
        Self {
            s3,
            bucket,
            inventory,
            workspace,
            model_id,
            read_characters: 0,
            read_cache: HashMap::new(),
            staged_proposal: None,
            full_draft: full_draft_run.map(|run| FullDraftRun {
                run,
                finalized: None,
                consecutive_write_failures: HashMap::new(),
                plan_conflicts_raised: HashSet::new(),
            }),
            citable_filenames,
        }
    }

    pub(crate) fn full_draft_is_finalized(&self) -> bool {
        self.full_draft
            .as_ref()
            .is_some_and(|draft| draft.finalized.is_some())
    }

    /// Planned sections the run has not yet decided, in document order, as
    /// `(id, heading)` — what the writer still owes before it may finish.
    pub(crate) fn undecided_plan_sections(&self) -> Vec<(Uuid, String)> {
        let Some(draft) = self.full_draft.as_ref() else {
            return Vec::new();
        };
        let run = &draft.run.run;
        let required = required_section_ids(run);
        let mut undecided: Vec<&RunSection> = run
            .sections
            .iter()
            .filter(|section| required.contains(&section.section_id))
            .filter(|section| !is_decided(section.state))
            .collect();
        undecided.sort_by_key(|section| section.position);
        undecided
            .into_iter()
            .map(|section| (section.section_id, section.heading.clone()))
            .collect()
    }

    pub(crate) fn take_finalized_full_draft(&mut self) -> Option<FinalizedFullDraft> {
        self.full_draft.as_mut()?.finalized.take()
    }

    pub(crate) async fn execute(
        &mut self,
        call: &ReportToolCall,
    ) -> Result<ExecutedTool, TurnRunFailure> {
        let request = match report::decode_tool_request(call) {
            Ok(request) => request,
            Err(error) => {
                // Carry the decode diagnostic verbatim: these messages
                // describe JSON structure (field names, types, positions),
                // never report or record content, and a generic sentence
                // wastes the model's repair round.
                let diagnostic = match &error {
                    BedrockError::SchemaViolation(message) => message.clone(),
                    other => other.to_string(),
                };
                return Ok(tool_error(
                    "invalid_tool_input",
                    &format!("The tool input did not match the configured schema: {diagnostic}"),
                ));
            }
        };

        match request {
            ReportToolRequest::ListRecordFiles(_) => {
                if self.full_draft.is_some() {
                    return Ok(tool_error(
                        "tool_not_available",
                        "The complete readable-record snapshot is already in context; list_record_files is unavailable in full-draft mode.",
                    ));
                }
                Ok(list_record_files_result(self.inventory))
            }
            ReportToolRequest::ReadRecordFile(request) => {
                if self.full_draft.is_some() {
                    return Ok(tool_error(
                        "tool_not_available",
                        "The complete readable-record snapshot is already in context; read_record_file is unavailable in full-draft mode.",
                    ));
                }
                self.read_record_file(request.filename, request.offset, request.limit)
                    .await
            }
            ReportToolRequest::ProposeReportChanges(request) => {
                if self.full_draft.is_some() {
                    return Ok(tool_error(
                        "tool_not_available",
                        "Full-draft mode saves one atomic working revision; do not stage a reviewable proposal.",
                    ));
                }
                if self.staged_proposal.is_some() {
                    return Ok(tool_error(
                        "proposal_already_pending",
                        "A valid proposal is already pending in this turn.",
                    ));
                }
                match stage_proposal(self.workspace, self.model_id, call, request) {
                    Ok(proposal) => {
                        let result = ExecutedTool {
                            status: ReportToolResultStatus::Success,
                            content: serde_json::json!({
                                "status": "pending_user_acceptance",
                                "proposal_id": proposal.id.to_string(),
                                "base_revision": proposal.base_revision
                            }),
                        };
                        self.staged_proposal = Some(proposal);
                        Ok(result)
                    }
                    Err(reason) => Ok(tool_error(
                        "invalid_proposal",
                        &format!(
                            "The proposed operations were not valid for the accepted report: {reason}"
                        ),
                    )),
                }
            }
            ReportToolRequest::SetFullDraftTitle(request) => {
                self.set_full_draft_title(request).await
            }
            ReportToolRequest::WriteFullDraftSection(request) => {
                self.write_full_draft_section(request).await
            }
            ReportToolRequest::SkipFullDraftSection(request) => {
                self.skip_full_draft_section(request).await
            }
            ReportToolRequest::MarkSectionFailed(request) => {
                self.mark_section_failed(request).await
            }
            ReportToolRequest::FinishFullDraft(request) => Ok(self.finish_full_draft(request)),
        }
    }

    /// Write the run object back before the caller believes the mutation it
    /// just made. A failure here fails the turn: the durable state that stands
    /// is whatever the previous successful save left behind.
    async fn commit_run(&mut self) -> Result<(), TurnRunFailure> {
        let Some(draft) = self.full_draft.as_mut() else {
            return Ok(());
        };
        save_draft_run(self.s3, self.bucket, draft.run)
            .await
            .map_err(|error| {
                TurnRunFailure::new(
                    ReportFailureCode::RecordStorage,
                    "Claria could not save the drafting run to managed storage, so the turn was stopped. Sections saved before this point were kept.",
                )
                .with_internal(error)
            })
    }

    async fn set_full_draft_title(
        &mut self,
        request: SetFullDraftTitleRequest,
    ) -> Result<ExecutedTool, TurnRunFailure> {
        let Some(draft) = self.full_draft.as_mut() else {
            return Ok(tool_error(
                "tool_not_available",
                "set_full_draft_title is available only during explicit full-draft generation.",
            ));
        };
        if draft.finalized.is_some() {
            return Ok(tool_error(
                "full_draft_already_finalized",
                "The complete draft is already finalized; do not modify it again.",
            ));
        }
        let proposed = ReportContent {
            title: request.title.clone(),
            sections: written_sections(&draft.run.run),
        };
        if let Err(error) = validate_report_content(&proposed) {
            return Ok(tool_error(
                "invalid_full_draft_title",
                &format!("The full-draft title was invalid: {error}"),
            ));
        }
        draft.run.run.title = Some(request.title);
        self.commit_run().await?;
        Ok(ExecutedTool {
            status: ReportToolResultStatus::Success,
            content: serde_json::json!({"status": "title_staged"}),
        })
    }

    async fn write_full_draft_section(
        &mut self,
        request: WriteFullDraftSectionRequest,
    ) -> Result<ExecutedTool, TurnRunFailure> {
        let Some(draft) = self.full_draft.as_ref() else {
            return Ok(tool_error(
                "tool_not_available",
                "write_full_draft_section is available only during explicit full-draft generation.",
            ));
        };
        if draft.finalized.is_some() {
            return Ok(tool_error(
                "full_draft_already_finalized",
                "The complete draft is already finalized; do not modify it again.",
            ));
        }
        // Resolving the ID comes first and on its own: a failure here names no
        // section the host knows, so it cannot count toward that section's
        // attempt bound.
        let run = &draft.run.run;
        let (section_id, attributable) = match &request.section_id {
            Some(value) => {
                let Ok(id) = value.parse::<Uuid>() else {
                    return Ok(tool_error(
                        "invalid_section_id",
                        "section_id must be a copied 36-character UUID or null for a new section.",
                    ));
                };
                if id.is_nil() {
                    return Ok(tool_error(
                        "invalid_section_id",
                        "section_id cannot be the nil UUID.",
                    ));
                }
                let known = run.sections.iter().any(|section| section.section_id == id);
                if !known {
                    return Ok(tool_error(
                        "invented_section_id",
                        "The section_id was not supplied by the host or returned by an earlier section write. Copy an existing ID exactly, or pass null for a new section.",
                    ));
                }
                (id, true)
            }
            None => (Uuid::new_v4(), false),
        };

        let prepared = match self.prepare_section_write(section_id, request) {
            Ok(prepared) => prepared,
            Err(rejection) if !attributable => {
                return Ok(tool_error(rejection.code, &rejection.message));
            }
            Err(rejection) => return self.record_write_rejection(section_id, rejection).await,
        };

        let supplied_order: Vec<Uuid> = self
            .workspace
            .draft
            .content
            .sections
            .iter()
            .map(|section| section.id)
            .collect();
        let now = jiff::Timestamp::now();
        let Some(draft) = self.full_draft.as_mut() else {
            return Ok(tool_error(
                "tool_not_available",
                "write_full_draft_section is available only during explicit full-draft generation.",
            ));
        };
        draft.consecutive_write_failures.remove(&section_id);
        let run = &mut draft.run.run;
        match run
            .sections
            .iter_mut()
            .find(|section| section.section_id == section_id)
        {
            Some(section) => {
                section.heading = prepared.heading.clone();
                section.blocks = prepared.blocks;
                section.citations = prepared.citations;
                // A write is the stronger decision: it overrides an earlier
                // skip, exactly as the in-memory candidate did.
                section.state = RunSectionState::Drafted;
                section.attempts = section.attempts.saturating_add(1);
                section.error = None;
                section.updated_at = now;
            }
            None => {
                // A section the writer invented. It joins the run's section
                // universe, and the plan gains the matching row that keeps
                // "one entry per section" true.
                run.sections.push(RunSection {
                    section_id,
                    heading: prepared.heading.clone(),
                    position: 0,
                    state: RunSectionState::Drafted,
                    blocks: prepared.blocks,
                    citations: prepared.citations,
                    attempts: 1,
                    error: None,
                    updated_at: now,
                });
                if let Some(plan) = run.plan.as_mut() {
                    plan.entries.push(PlanEntry {
                        section_id,
                        heading: prepared.heading,
                        intent: SectionIntent::Draft,
                        // The host never asked for this section, so it is not
                        // one the finisher demands a decision about — and a
                        // skip of it is still an invented ID.
                        required: false,
                        scope: String::new(),
                        evidence: Vec::new(),
                        instruction: None,
                    });
                }
            }
        }
        renumber_positions(run, &prepared.written_order, &supplied_order);
        self.commit_run().await?;
        Ok(ExecutedTool {
            status: ReportToolResultStatus::Success,
            content: serde_json::json!({
                "status": "section_staged",
                "section_id": section_id.to_string(),
                "position": prepared.position,
                "block_count": prepared.block_count,
                "citation_count": prepared.citation_count,
                "section_count": prepared.section_count
            }),
        })
    }

    /// Validate one section write against the candidate it would produce,
    /// without touching the run. Read-only so the attempt bound below can
    /// decide what to do with a rejection before anything is committed.
    fn prepare_section_write(
        &self,
        section_id: Uuid,
        request: WriteFullDraftSectionRequest,
    ) -> Result<PreparedSectionWrite, WriteRejection> {
        let Some(draft) = self.full_draft.as_ref() else {
            return Err(WriteRejection::new(
                "tool_not_available",
                "write_full_draft_section is available only during explicit full-draft generation.",
            ));
        };
        let run = &draft.run.run;
        let position = usize::try_from(request.position).map_err(|_| {
            WriteRejection::new(
                "invalid_section_position",
                "The section position is too large.",
            )
        })?;
        let mut sections = written_sections(run);
        if let Some(existing) = sections.iter().position(|section| section.id == section_id) {
            sections.remove(existing);
        }
        if position > sections.len() {
            return Err(WriteRejection::new(
                "invalid_section_position",
                format!(
                    "Section position {position} is outside 0..={}; write sections in final order.",
                    sections.len()
                ),
            ));
        }
        let citations = self.validate_citations(request.citations.unwrap_or_default())?;
        let block_count = request.blocks.len();
        let heading = request.heading;
        let blocks: Vec<ReportBlock> = request.blocks.into_iter().map(block_from_request).collect();
        sections.insert(
            position,
            ReportSection {
                id: section_id,
                heading: heading.clone(),
                blocks: blocks.clone(),
                skipped: false,
                template_blocks: None,
                authorship: None,
            },
        );
        let proposed = ReportContent {
            title: run
                .title
                .clone()
                .unwrap_or_else(|| self.workspace.draft.content.title.clone()),
            sections: sections.clone(),
        };
        validate_report_content(&proposed).map_err(|error| {
            WriteRejection::new(
                "invalid_full_draft_section",
                format!("The full-draft section was invalid: {error}"),
            )
        })?;
        Ok(PreparedSectionWrite {
            position,
            block_count,
            citation_count: citations.len(),
            section_count: sections.len(),
            written_order: sections.iter().map(|section| section.id).collect(),
            heading,
            blocks,
            citations,
        })
    }

    /// Check each quote names a file the run actually put in front of the
    /// model, and is long enough to be resolvable later. The quote itself is
    /// deliberately NOT matched against the record text here: that is the
    /// completion gate's job, and doing it per write would re-read the corpus
    /// on every section.
    fn validate_citations(
        &self,
        citations: Vec<RecordCitationRequest>,
    ) -> Result<Vec<RecordCitation>, WriteRejection> {
        if citations.len() > MAX_SECTION_CITATIONS {
            return Err(WriteRejection::new(
                "too_many_citations",
                format!("A section may cite at most {MAX_SECTION_CITATIONS} record quotes."),
            ));
        }
        citations
            .into_iter()
            .map(|citation| {
                if !self
                    .citable_filenames
                    .iter()
                    .any(|filename| filename == &citation.filename)
                {
                    return Err(WriteRejection::new(
                        "unknown_citation_file",
                        "A citation named a file that is not in this run's record snapshot. Copy filenames exactly from the record context; unreadable records cannot be cited.",
                    ));
                }
                let quote_characters = citation.quote.chars().count();
                if !(MIN_CITATION_QUOTE_CHARACTERS..=MAX_CITATION_QUOTE_CHARACTERS)
                    .contains(&quote_characters)
                {
                    return Err(WriteRejection::new(
                        "invalid_citation_quote",
                        format!(
                            "A citation quote was {quote_characters} Unicode characters; quotes must be {MIN_CITATION_QUOTE_CHARACTERS}–{MAX_CITATION_QUOTE_CHARACTERS}."
                        ),
                    ));
                }
                Ok(RecordCitation {
                    filename: citation.filename,
                    quote: citation.quote,
                })
            })
            .collect()
    }

    /// Count one rejected write against its section, and fail the section
    /// once it has used its attempts — so the run continues with the sections
    /// after it instead of spending the rest of its call budget here.
    async fn record_write_rejection(
        &mut self,
        section_id: Uuid,
        rejection: WriteRejection,
    ) -> Result<ExecutedTool, TurnRunFailure> {
        let Some(draft) = self.full_draft.as_mut() else {
            return Ok(tool_error(rejection.code, &rejection.message));
        };
        let strikes = draft
            .consecutive_write_failures
            .entry(section_id)
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        if *strikes < MAX_SECTION_WRITE_ATTEMPTS {
            return Ok(tool_error(rejection.code, &rejection.message));
        }
        // The diagnostic is the tool error's own code and message: structural
        // JSON/validator wording, never report or record text.
        let diagnostic = truncate_characters(
            &format!("{}: {}", rejection.code, rejection.message),
            MAX_SECTION_ERROR_CHARACTERS,
        );
        let now = jiff::Timestamp::now();
        if let Some(section) = draft
            .run
            .run
            .sections
            .iter_mut()
            .find(|section| section.section_id == section_id)
        {
            section.state = RunSectionState::Failed;
            section.blocks.clear();
            section.attempts = section.attempts.saturating_add(1);
            section.error = Some(diagnostic.clone());
            section.updated_at = now;
        }
        self.commit_run().await?;
        Ok(tool_error(
            "section_attempts_exhausted",
            &format!(
                "This section was rejected {MAX_SECTION_WRITE_ATTEMPTS} times and is now marked failed; continue with the next planned section. Last diagnostic — {diagnostic}"
            ),
        ))
    }

    async fn mark_section_failed(
        &mut self,
        request: MarkSectionFailedRequest,
    ) -> Result<ExecutedTool, TurnRunFailure> {
        let Some(draft) = self.full_draft.as_mut() else {
            return Ok(tool_error(
                "tool_not_available",
                "mark_section_failed is available only during explicit full-draft generation.",
            ));
        };
        if draft.finalized.is_some() {
            return Ok(tool_error(
                "full_draft_already_finalized",
                "The complete draft is already finalized; do not modify it again.",
            ));
        }
        let Ok(section_id) = request.section_id.parse::<Uuid>() else {
            return Ok(tool_error(
                "invalid_section_id",
                "section_id must be a copied 36-character UUID.",
            ));
        };
        let reason = request.reason.trim().to_string();
        if reason.is_empty() || reason.chars().count() > MAX_SECTION_ERROR_CHARACTERS {
            return Ok(tool_error(
                "invalid_failure_reason",
                &format!("reason must be 1–{MAX_SECTION_ERROR_CHARACTERS} characters."),
            ));
        }
        let now = jiff::Timestamp::now();
        let Some(section) = draft
            .run
            .run
            .sections
            .iter_mut()
            .find(|section| section.section_id == section_id)
        else {
            return Ok(tool_error(
                "invented_section_id",
                "Only a planned section can be marked failed. Copy its ID exactly from plan_context.",
            ));
        };
        section.state = RunSectionState::Failed;
        // A failed section carries no staged content: the assembly falls back
        // to the base revision, and the run validator refuses blocks outside
        // the drafted state.
        section.blocks.clear();
        section.attempts = section.attempts.saturating_add(1);
        section.error = Some(reason);
        section.updated_at = now;
        self.commit_run().await?;
        Ok(ExecutedTool {
            status: ReportToolResultStatus::Success,
            content: serde_json::json!({
                "status": "section_marked_failed",
                "section_id": section_id.to_string()
            }),
        })
    }

    async fn skip_full_draft_section(
        &mut self,
        request: SkipFullDraftSectionRequest,
    ) -> Result<ExecutedTool, TurnRunFailure> {
        let Some(draft) = self.full_draft.as_mut() else {
            return Ok(tool_error(
                "tool_not_available",
                "skip_full_draft_section is available only during explicit full-draft generation.",
            ));
        };
        if draft.finalized.is_some() {
            return Ok(tool_error(
                "full_draft_already_finalized",
                "The complete draft is already finalized; do not modify it again.",
            ));
        }
        let Ok(section_id) = request.section_id.parse::<Uuid>() else {
            return Ok(tool_error(
                "invalid_section_id",
                "section_id must be a copied 36-character UUID.",
            ));
        };
        let required = required_section_ids(&draft.run.run);
        let index = draft
            .run
            .run
            .sections
            .iter()
            .position(|section| section.section_id == section_id)
            .filter(|_| required.contains(&section_id));
        let Some(index) = index else {
            return Ok(tool_error(
                "invented_section_id",
                "Only a supplied template/report section can be skipped. Copy its ID exactly from the untrusted context; sections you added yourself should simply not be written.",
            ));
        };
        // A skip that contradicts a plan a human approved gets exactly one
        // corrective round. The plan row goes back verbatim so the model can
        // see what it is arguing with; if it still wants the skip, the skip
        // stands and the divergence is recorded rather than fought over.
        let conflict = draft
            .run
            .run
            .plan
            .as_ref()
            .filter(|plan| plan.user_edited)
            .and_then(|plan| {
                plan.entries
                    .iter()
                    .find(|entry| entry.section_id == section_id)
            })
            .filter(|entry| entry.intent == SectionIntent::Draft)
            .cloned();
        let diverged = match conflict {
            Some(entry) if draft.plan_conflicts_raised.insert(section_id) => {
                return Ok(ExecutedTool {
                    status: ReportToolResultStatus::Error,
                    content: serde_json::json!({
                        "error": {
                            "code": "plan_conflict",
                            "message": "The approved plan asks for this section to be drafted, not skipped. Draft it, or call skip_full_draft_section again to record a deliberate divergence.",
                            "plan_entry": {
                                "section_id": entry.section_id.to_string(),
                                "heading": entry.heading,
                                "intent": entry.intent,
                                "required": entry.required,
                                "scope": entry.scope,
                                "instruction": entry.instruction
                            }
                        }
                    }),
                });
            }
            Some(_) => true,
            None => false,
        };
        let section = &mut draft.run.run.sections[index];
        if section.state == RunSectionState::Drafted {
            return Ok(tool_error(
                "section_already_written",
                "This section was already written in this draft; a skip cannot override a write. The written section stands.",
            ));
        }
        section.state = RunSectionState::Skipped;
        if diverged {
            section.error = Some("skip diverged from the approved plan".to_string());
        }
        section.updated_at = jiff::Timestamp::now();
        self.commit_run().await?;
        Ok(ExecutedTool {
            status: ReportToolResultStatus::Success,
            content: serde_json::json!({
                "status": "section_skipped",
                "section_id": section_id.to_string()
            }),
        })
    }

    fn finish_full_draft(&mut self, request: FinishFullDraftRequest) -> ExecutedTool {
        if let Err(error) = validate_report_summary(&request.summary) {
            return tool_error(
                "invalid_full_draft_summary",
                &format!("The full-draft summary was invalid: {error}"),
            );
        }
        let Some(draft) = self.full_draft.as_ref() else {
            return tool_error(
                "tool_not_available",
                "finish_full_draft is available only during explicit full-draft generation.",
            );
        };
        if draft.finalized.is_some() {
            return tool_error(
                "full_draft_already_finalized",
                "The complete draft is already finalized.",
            );
        }
        let run = &draft.run.run;
        let Some(title) = run.title.clone() else {
            return tool_error(
                "full_draft_incomplete",
                "Call set_full_draft_title before finishing the complete draft.",
            );
        };
        let written = written_sections(run);
        if written.is_empty() {
            return tool_error(
                "full_draft_incomplete",
                "Write at least one complete report section before finishing.",
            );
        }
        let skipped_section_ids = sections_in_state(run, RunSectionState::Skipped);
        let failed_section_ids = sections_in_state(run, RunSectionState::Failed);
        let drafted_section_ids = sections_in_state(run, RunSectionState::Drafted);
        // Failed joins drafted and skipped as a decision. A section the writer
        // genuinely could not draft must not hold the whole run hostage — the
        // run completes with the failure visible instead.
        let mut missing = required_section_ids(run)
            .difference(&drafted_section_ids)
            .filter(|id| !skipped_section_ids.contains(id) && !failed_section_ids.contains(id))
            .copied()
            .collect::<Vec<_>>();
        missing.sort();
        if !missing.is_empty() {
            return tool_error(
                "full_draft_incomplete",
                &format!(
                    "Write, explicitly skip, or mark failed every planned section before finishing. Undecided section IDs: {}",
                    missing
                        .iter()
                        .map(Uuid::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        let sections = merge_undrafted_sections(
            &written,
            &skipped_section_ids,
            &failed_section_ids,
            &self.workspace.draft.content.sections,
        );
        let content = ReportContent { title, sections };
        if let Err(error) = validate_report_content(&content) {
            return tool_error(
                "invalid_full_draft",
                &format!("The complete draft was invalid: {error}"),
            );
        }
        let section_count = content.sections.len();
        let skipped_section_count = skipped_section_ids.len();
        let failed_section_count = failed_section_ids.len();
        let base_revision = self.workspace.draft.revision;
        if let Some(draft) = self.full_draft.as_mut() {
            draft.finalized = Some(FinalizedFullDraft {
                content,
                summary: request.summary,
            });
        }
        ExecutedTool {
            status: ReportToolResultStatus::Success,
            content: serde_json::json!({
                "status": "full_draft_finalized",
                "section_count": section_count,
                "skipped_section_count": skipped_section_count,
                "failed_section_count": failed_section_count,
                "base_revision": base_revision
            }),
        }
    }

    async fn read_record_file(
        &mut self,
        filename: String,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<ExecutedTool, TurnRunFailure> {
        if filename.is_empty() || filename.chars().count() > 1_024 {
            return Ok(tool_error(
                "invalid_filename",
                "Filename length is invalid.",
            ));
        }
        let Some(entry) = self
            .inventory
            .iter()
            .find(|entry| entry.filename == filename)
        else {
            return Ok(tool_error(
                "file_not_in_inventory",
                "The filename is not in this client's safe record inventory.",
            ));
        };
        let read_key = &entry.read_key;
        if entry.source_bytes > claria_core::record_text::MAX_RECORD_TEXT_BYTES {
            return Ok(tool_error(
                "record_too_large",
                "The readable record exceeds Claria's 2 MiB source limit.",
            ));
        }
        let requested_limit = limit.unwrap_or(DEFAULT_READ_LIMIT);
        if requested_limit == 0 || requested_limit > MAX_READ_LIMIT {
            return Ok(tool_error(
                "invalid_read_limit",
                "Read limit must be between 1 and 12000 characters.",
            ));
        }
        let remaining = MAX_READ_CHARACTERS_PER_TURN.saturating_sub(self.read_characters);
        if remaining == 0 {
            return Ok(tool_error(
                "turn_read_limit_reached",
                "This turn has reached the 48000-character record read limit.",
            ));
        }
        let effective_limit = requested_limit.min(remaining);
        let offset = offset.unwrap_or(0);

        if !self.read_cache.contains_key(read_key) {
            let output = match claria_storage::objects::get_object_bounded(
                self.s3,
                self.bucket,
                read_key,
                claria_core::record_text::MAX_RECORD_TEXT_BYTES,
            )
            .await
            {
                Ok(output) => output,
                Err(StorageError::ObjectTooLarge { .. }) => {
                    return Ok(tool_error(
                        "record_too_large",
                        "The readable record exceeds Claria's 2 MiB source limit.",
                    ));
                }
                Err(error) => {
                    return Err(TurnRunFailure::new(
                        ReportFailureCode::RecordStorage,
                        "Claria could not read the approved record text from managed storage. Retry the turn.",
                    )
                    .with_internal(error));
                }
            };
            let Some(text) = claria_core::record_text::decode_record_text(&output.body) else {
                return Ok(tool_error(
                    "record_not_text",
                    "The record is not printable UTF-8 text and has no readable extraction.",
                ));
            };
            self.read_cache.insert(read_key.clone(), text.to_string());
        }
        let Some(text) = self.read_cache.get(read_key) else {
            return Err(TurnRunFailure::new(
                ReportFailureCode::RecordStorage,
                "Claria could not cache the approved record text. Retry the turn.",
            ));
        };
        let total_characters_usize = text.chars().count();
        let total_characters = match u32::try_from(total_characters_usize) {
            Ok(value) => value,
            Err(_) => {
                return Ok(tool_error(
                    "record_too_large",
                    "The record text is too large for bounded character reads.",
                ));
            }
        };
        if offset > total_characters {
            return Ok(tool_error(
                "offset_out_of_range",
                "Read offset is beyond the end of the record text.",
            ));
        }
        let selected: String = text
            .chars()
            .skip(offset as usize)
            .take(effective_limit as usize)
            .collect();
        let returned_characters = selected.chars().count() as u32;
        self.read_characters = self.read_characters.saturating_add(returned_characters);
        let end = offset.saturating_add(returned_characters);
        let next_offset = (end < total_characters).then_some(end);
        let sha256 = Sha256::digest(selected.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        Ok(ExecutedTool {
            status: ReportToolResultStatus::Success,
            content: serde_json::json!({
                "filename": filename,
                "text": selected,
                "offset": offset,
                "returned_characters": returned_characters,
                "total_characters": total_characters,
                "next_offset": next_offset,
                "sha256": sha256
            }),
        })
    }
}

/// Sections the host demands an explicit written-or-skipped decision about.
///
/// The plan is the authority: with today's synthetic 1:1 plan this is exactly
/// the set of sections the workspace supplied, and a section the writer
/// invented mid-run carries `required: false` so skipping it is still an
/// invented ID rather than a legal deferral.
fn required_section_ids(run: &DraftRun) -> HashSet<Uuid> {
    match &run.plan {
        Some(plan) => plan
            .entries
            .iter()
            .filter(|entry| entry.required)
            .map(|entry| entry.section_id)
            .collect(),
        None => run
            .sections
            .iter()
            .map(|section| section.section_id)
            .collect(),
    }
}

fn sections_in_state(run: &DraftRun, state: RunSectionState) -> HashSet<Uuid> {
    run.sections
        .iter()
        .filter(|section| section.state == state)
        .map(|section| section.section_id)
        .collect()
}

/// The staged draft as the writer ordered it.
///
/// A drafted section's `position` is its index in the sequence of writes, so
/// sorting the drafted rows by it reproduces the ordered candidate the
/// in-memory staging used to keep — which is what makes the assembled
/// document byte-identical to a pre-run draft over the same tool calls.
fn written_sections(run: &DraftRun) -> Vec<ReportSection> {
    let mut drafted = run
        .sections
        .iter()
        .filter(|section| section.state == RunSectionState::Drafted)
        .collect::<Vec<_>>();
    drafted.sort_by_key(|section| section.position);
    drafted
        .into_iter()
        .map(|section| ReportSection {
            id: section.section_id,
            heading: section.heading.clone(),
            blocks: section.blocks.clone(),
            skipped: false,
            template_blocks: None,
            authorship: None,
        })
        .collect()
}

/// Re-lay the run's positions after a write: the drafted sections take
/// `0..written.len()` in the order they were written, and everything still
/// undecided follows in the order the workspace supplied it. Positions have to
/// stay unique across the run, so the two groups cannot simply overlap.
fn renumber_positions(run: &mut DraftRun, written: &[Uuid], supplied_order: &[Uuid]) {
    let run_ids: Vec<Uuid> = run
        .sections
        .iter()
        .map(|section| section.section_id)
        .collect();
    let mut positions: HashMap<Uuid, u32> = HashMap::with_capacity(run_ids.len());
    let mut next = 0_u32;
    // Written order first, then supplied order, then anything left over in the
    // run's own order — every section of the run lands exactly once.
    for id in written
        .iter()
        .chain(supplied_order.iter())
        .chain(run_ids.iter())
    {
        if run_ids.contains(id) && !positions.contains_key(id) {
            positions.insert(*id, next);
            next = next.saturating_add(1);
        }
    }
    for section in &mut run.sections {
        if let Some(position) = positions.get(&section.section_id) {
            section.position = *position;
        }
    }
}

/// Merge the sections the run did not draft back into the written ones.
///
/// Written sections keep the model's chosen order. Each undrafted supplied
/// section re-enters positioned after the nearest earlier supplied section
/// already present in the result — so consecutive gaps keep their supplied
/// relative order, and one with no earlier surviving neighbor lands at the
/// front.
///
/// What it re-enters as differs by decision. A **skipped** section becomes an
/// empty `skipped` placeholder carrying its supplied heading, ID, template
/// copy, and authorship: the authored body stays empty and the template copy
/// is what the greyed-out preview renders. A **failed** section is copied from
/// the base revision verbatim instead — the writer could not replace that
/// text, so leaving it alone is the honest outcome, and a base section that
/// was already an empty placeholder stays exactly that.
fn merge_undrafted_sections(
    written: &[ReportSection],
    skipped_section_ids: &HashSet<Uuid>,
    failed_section_ids: &HashSet<Uuid>,
    supplied: &[ReportSection],
) -> Vec<ReportSection> {
    let mut sections = written.to_vec();
    for (supplied_index, supplied_section) in supplied.iter().enumerate() {
        let skipped = skipped_section_ids.contains(&supplied_section.id);
        if !skipped && !failed_section_ids.contains(&supplied_section.id) {
            continue;
        }
        let insert_at = supplied[..supplied_index]
            .iter()
            .rev()
            .find_map(|prior| {
                sections
                    .iter()
                    .position(|section| section.id == prior.id)
                    .map(|position| position + 1)
            })
            .unwrap_or(0);
        let section = if skipped {
            ReportSection {
                id: supplied_section.id,
                heading: supplied_section.heading.clone(),
                blocks: Vec::new(),
                skipped: true,
                template_blocks: supplied_section.template_blocks.clone(),
                authorship: supplied_section.authorship.clone(),
            }
        } else {
            supplied_section.clone()
        };
        sections.insert(insert_at, section);
    }
    sections
}

/// A section write the host refused, carrying the diagnostic the model gets
/// back — structural wording only, so it is safe to persist on the run.
struct WriteRejection {
    code: &'static str,
    message: String,
}

impl WriteRejection {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// A validated section write, ready to be applied to the run.
struct PreparedSectionWrite {
    heading: String,
    blocks: Vec<ReportBlock>,
    citations: Vec<RecordCitation>,
    position: usize,
    block_count: usize,
    citation_count: usize,
    section_count: usize,
    written_order: Vec<Uuid>,
}

/// Whether the run has an answer for a section, in the sense the finisher
/// demands: written, deliberately deferred, or genuinely failed.
fn is_decided(state: RunSectionState) -> bool {
    matches!(
        state,
        RunSectionState::Drafted
            | RunSectionState::Skipped
            | RunSectionState::Failed
            | RunSectionState::Kept
    )
}

/// Cut a diagnostic to `limit` Unicode characters so it fits the run's
/// PHI-free error field without splitting a character.
fn truncate_characters(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect()
}

fn list_record_files_result(inventory: &[RecordInventoryEntry]) -> ExecutedTool {
    let mut files: Vec<serde_json::Value> = inventory
        .iter()
        .take(MAX_LISTED_FILES)
        .map(|entry| {
            serde_json::json!({
                "filename": entry.filename,
                "readable": entry.source_bytes <= claria_core::record_text::MAX_RECORD_TEXT_BYTES,
                "source_too_large": entry.source_bytes > claria_core::record_text::MAX_RECORD_TEXT_BYTES
            })
        })
        .collect();

    let mut truncated = inventory.len() > files.len();
    while serde_json::to_vec(&serde_json::json!({
        "files": files,
        "truncated": truncated
    }))
    .map_or(usize::MAX, |bytes| bytes.len())
        > MAX_LIST_RESULT_BYTES
    {
        if files.pop().is_none() {
            break;
        }
        truncated = true;
    }
    ExecutedTool {
        status: ReportToolResultStatus::Success,
        content: serde_json::json!({"files": files, "truncated": truncated}),
    }
}

fn stage_proposal(
    workspace: &ReportWorkspace,
    model_id: &str,
    call: &ReportToolCall,
    request: ProposeReportChangesRequest,
) -> Result<ReportProposal, String> {
    validate_report_summary(&request.summary).map_err(|error| error.to_string())?;
    let operations = request
        .operations
        .into_iter()
        .map(operation_from_request)
        .collect::<Result<Vec<_>, _>>()?;
    let proposed_content = workspace
        .draft
        .preview(&operations)
        .map_err(|error| error.to_string())?;
    Ok(ReportProposal {
        id: Uuid::new_v4(),
        report_id: workspace.report_id,
        base_revision: workspace.draft.revision,
        model_id: model_id.to_string(),
        summary: request.summary,
        operations,
        proposed_content,
        tool_use_id: call.tool_use_id.clone(),
        created_at: jiff::Timestamp::now(),
    })
}

fn operation_from_request(
    operation: ReportProposalOperationRequest,
) -> Result<ReportOperation, String> {
    match operation {
        ReportProposalOperationRequest::SetTitle { title } => {
            Ok(ReportOperation::SetTitle { title })
        }
        ReportProposalOperationRequest::AddSection {
            position,
            heading,
            blocks,
        } => Ok(ReportOperation::AddSection {
            position,
            section: ReportSection {
                id: Uuid::new_v4(),
                heading,
                blocks: blocks.into_iter().map(block_from_request).collect(),
                skipped: false,
                template_blocks: None,
                authorship: None,
            },
        }),
        ReportProposalOperationRequest::ReplaceSection {
            section_id,
            heading,
            blocks,
        } => Ok(ReportOperation::ReplaceSection {
            section_id: section_id
                .parse()
                .map_err(|_| format!("Invalid section ID: {section_id}"))?,
            heading,
            blocks: blocks.into_iter().map(block_from_request).collect(),
        }),
        ReportProposalOperationRequest::RemoveSection { section_id } => {
            Ok(ReportOperation::RemoveSection {
                section_id: section_id
                    .parse()
                    .map_err(|_| format!("Invalid section ID: {section_id}"))?,
            })
        }
    }
}

fn block_from_request(block: ReportBlockRequest) -> ReportBlock {
    match block {
        ReportBlockRequest::Paragraph { text } => ReportBlock::Paragraph { text },
        ReportBlockRequest::BulletList { items } => ReportBlock::BulletList { items },
        ReportBlockRequest::Table {
            rows,
            has_header,
            column_widths,
        } => ReportBlock::Table {
            rows,
            has_header,
            column_widths,
        },
    }
}

fn tool_error(code: &str, message: &str) -> ExecutedTool {
    ExecutedTool {
        status: ReportToolResultStatus::Error,
        content: serde_json::json!({"error": {"code": code, "message": message}}),
    }
}
