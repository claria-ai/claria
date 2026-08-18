//! The planning pass: deciding what each section must assert, and what in the
//! records says so, before a word of the report is written.
//!
//! A plan is produced by forced tool calls to a smaller model, then checked
//! by code. The model is trusted to read a corpus and propose scope; it is
//! not trusted to have covered every section, to have kept the section IDs it
//! was given, or to have named a record that exists. Those are decidable
//! questions, so the host decides them:
//!
//! - **Coverage and identity are hard.** One row per section the call was
//!   given, no invented IDs, no duplicates. Failing that costs one repair
//!   round — the diagnostic goes back verbatim and the tool is forced again —
//!   and then the pass fails. A plan missing a section would silently delete
//!   it from the document.
//! - **Evidence is soft.** A filename that names no record, or a draft row
//!   left with nothing backing it, lands as a warning on the run. The
//!   clinician is about to read this plan at the gate; refusing to show it to
//!   them because one filename was mistyped helps nobody.
//!
//! A fresh plan is asked for [`PLAN_BATCH_SECTIONS`] sections at a time, in
//! template order, one sequential call per batch. Every batch reads the same
//! cached prefix and carries only its own question; a batch that validates is
//! decided, announced, and never asked for again, and the pass checks the
//! stop signal before opening the next call. Nothing is persisted until the
//! last batch lands, so a pass that fails halfway leaves the report as it
//! found it.
//!
//! Resuming works the same way over a different question — what to do with
//! the sections that already landed — and only when there is a question to
//! ask. It stays one call, because its question is the run's whole state
//! table. A resume with no new instructions has a decidable answer, so it
//! skips the model entirely.

use std::collections::{HashMap, HashSet};

use aws_sdk_s3::Client as S3Client;
use claria_bedrock::{
    analysis::{
        self, AnalysisInputBudget, PlanAction, PlanEvidence, ResumeDecision, SectionPlanRow,
        StructuredCallOutput, StructuredCallRequest,
    },
    converse::{StopSignal, StreamBounds},
    error::BedrockError,
    retry::{MAX_ATTEMPTS, with_throttle_retry_observed},
};
use claria_core::models::{
    report::{
        ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole, ReportToolResultStatus,
        ReportWorkspace,
    },
    report_run::{
        DraftRun, DraftRunStatus, EvidenceRef, MAX_PLAN_EVIDENCE, MAX_PLAN_SCOPE_CHARACTERS,
        MAX_PLAN_WARNINGS, PlanEntry, PlanEntryEdit, RunPlan, RunSection, RunSectionState,
        SectionIntent,
    },
    turn_usage::TurnUsage,
};
use claria_report_store::{
    LoadedRun, MAX_INSTRUCTION_CHARACTERS, load_draft_run, load_for_report, save_draft_run,
    save_loaded,
};
use uuid::Uuid;

use crate::{
    FullReportGenerationOutcome, ReportPipelineError,
    full_draft_context::{DraftTurnKind, planner_template_context, section_state_table},
    parallel_draft::execute_parallel_draft_turn,
    prompts::planner_system_prompt,
    record_context::{
        BudgetRole, FullRecordContext, load_full_record_context, load_record_inventory,
    },
    run::{create_run_for_workspace, release_failed_run},
    turn::{FullReportRequest, ReportTurnProgress, ensure_ready_for_turn, validate_model_choice},
};

/// Output-token ceiling for one planning call, and the reserve subtracted
/// from the planner's context window to form its input budget — enforced, not
/// aspirational. A plan row is an outline entry: a section ID, a sentence or
/// three of scope, and a handful of filenames with one line each on why they
/// matter. A call is asked for [`PLAN_BATCH_SECTIONS`] of them, which is a
/// couple of thousand tokens in practice and, at every schema ceiling at
/// once, still a third inside this — the `const` assertion below is what
/// holds the two numbers together. Whatever is left goes back to the input
/// budget the corpus is read under. A batch that reaches the ceiling is a
/// typed failure rather than a document outline silently missing rows.
pub const PLAN_OUTPUT_TOKEN_RESERVE: u32 = 16_384;

/// Stamped as the plan's author when a resume is decided by code rather than
/// by a model. Not a Bedrock model ID and deliberately not shaped like one,
/// so a cost or provenance reader cannot mistake it for a call that happened.
pub const DETERMINISTIC_PLAN_MODEL_ID: &str = "claria-deterministic-resume";

/// Repair rounds granted to one planning call whose answer failed host
/// validation. One: the diagnostic names the exact rows that were wrong, so a
/// model that cannot use that is not going to use a second copy of it either,
/// and every round is a full re-read of the corpus.
const PLAN_REPAIR_ROUNDS: u32 = 1;

/// Sections one planning call is asked to plan.
///
/// A fresh plan is a sequence of these rather than one call for the whole
/// document. The single call was the pipeline's longest-running request by
/// far — one forced tool emitting every row of a thirty-section report — and
/// its length is what put it under the stream's inter-frame watchdog with
/// nothing to show the reader and nothing to salvage. Eight rows is a couple
/// of thousand tokens of realistic output: short enough to land well inside
/// the watchdog, long enough that the per-call overhead (a fresh instruction
/// and a cached-prefix read) stays small against it. It also makes the pass
/// checkpointed — every batch that validates is decided, and the reader is
/// told so — and gives Stop somewhere to land between calls.
pub const PLAN_BATCH_SECTIONS: usize = 8;

/// Characters of JSON scaffolding around one plan row's values: the four key
/// names and their quotes, colons and commas, the row's own braces, and the
/// same again for each evidence object it carries. Rounded well up — it is a
/// term in a ceiling check, so overstating it is the safe direction.
const PLAN_ROW_JSON_OVERHEAD_CHARS: usize = 160;

/// The characters-per-token ratio output is sized at, matching the estimate
/// the input budget uses. Tool-call JSON is denser than prose, so a ceiling
/// derived at this ratio is conservative.
const OUTPUT_CHARS_PER_TOKEN: usize = 4;

/// The largest one plan row's JSON can be, derived from the schema's own
/// ceilings rather than guessed: a 36-character section UUID, the longest
/// action word, a scope at its limit, and
/// [`analysis::MAX_PLANNER_EVIDENCE`] evidence objects each holding a
/// filename and a relevance line at theirs.
const WORST_CASE_PLAN_ROW_CHARS: usize = PLAN_ROW_JSON_OVERHEAD_CHARS
    + SECTION_ID_CHARACTERS
    + LONGEST_PLAN_ACTION_CHARACTERS
    + MAX_PLAN_SCOPE_CHARACTERS
    + analysis::MAX_PLANNER_EVIDENCE
        * (analysis::MAX_EVIDENCE_FILENAME_CHARACTERS + analysis::MAX_PLANNER_RELEVANCE_CHARACTERS);

/// A section UUID's canonical text length, which the schema pins exactly.
const SECTION_ID_CHARACTERS: usize = 36;

/// `"draft"` — the longer of the two action words.
const LONGEST_PLAN_ACTION_CHARACTERS: usize = 5;

// The batch size and the output ceiling are the same constraint stated twice,
// so they are checked against each other rather than maintained apart: a
// batch whose worst case does not fit would be a reserve that is a lie, and
// truncated forced-tool JSON is a failed pass rather than a short plan.
const _: () = assert!(
    PLAN_BATCH_SECTIONS * WORST_CASE_PLAN_ROW_CHARS
        <= PLAN_OUTPUT_TOKEN_RESERVE as usize * OUTPUT_CHARS_PER_TOKEN,
    "a worst-case batch of plan rows must fit the planner's output ceiling"
);

/// What one planning pass produced, and what it cost.
///
/// The usage rides out with the run so the command layer can put the planner
/// call's tokens and dollars in the audit event. Nothing in it is PHI.
#[derive(Debug, Clone)]
pub struct DraftPlanOutcome {
    pub run: DraftRun,
    pub usage: Option<TurnUsage>,
    pub converse_calls: u32,
}

/// The two models a planning pass has to fit inside.
#[derive(Debug, Clone, Copy)]
pub struct PlanModels<'a> {
    pub planner_model_id: &'a str,
    /// The model expected to write the sections. Provisional at plan time —
    /// the gate's Start button is what fixes it — but the record corpus has
    /// to fit both windows, and this is the one that will hold the corpus for
    /// a hundred calls rather than one.
    pub writer_model_id: &'a str,
}

/// A planning request: the clinician's guidance, plus the optional channels a
/// live pass reports and cancels through.
#[derive(Clone, Copy)]
pub struct DraftPlanRequest<'a> {
    pub instructions: &'a str,
    progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
    stop: Option<&'a StopSignal>,
    stream_bounds: StreamBounds,
}

impl<'a> DraftPlanRequest<'a> {
    pub fn new(instructions: &'a str) -> Self {
        Self {
            instructions,
            progress: None,
            stop: None,
            stream_bounds: StreamBounds::analysis(),
        }
    }

    /// How long each planning call may stay silent. Without this the pass
    /// runs at the analysis family's compiled-in waits.
    pub fn with_stream_bounds(mut self, stream_bounds: StreamBounds) -> Self {
        self.stream_bounds = stream_bounds;
        self
    }

    pub fn with_progress(
        mut self,
        progress: &'a (dyn Fn(ReportTurnProgress) + Send + Sync),
    ) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Fire this signal to abandon the pass mid-call. Without one the pass
    /// runs to completion.
    pub fn with_stop(mut self, stop: &'a StopSignal) -> Self {
        self.stop = Some(stop);
        self
    }

    fn emit_progress(self, event: ReportTurnProgress) {
        if let Some(progress) = self.progress {
            progress(event);
        }
    }
}

/// Plan a whole-report draft and leave it waiting at the gate.
///
/// The run is created before the model is called and the workspace points at
/// it for the whole gate window, so a second window cannot start a competing
/// draft or edit the report out from under a plan somebody is reviewing. A
/// pass that fails releases that pointer and marks the run failed, exactly as
/// the drafting executor does — the alternative is a report locked behind a
/// run that never got a plan.
// Client, report, and the expected revision are the same explicit
// orchestration boundary every other entry point in this crate keeps.
#[allow(clippy::too_many_arguments)]
pub async fn generate_draft_plan(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    models: PlanModels<'_>,
    request: DraftPlanRequest<'_>,
) -> Result<DraftPlanOutcome, ReportPipelineError> {
    let instructions = request.instructions.trim();
    validate_instructions(instructions)?;
    validate_model_choice(models.planner_model_id)?;
    validate_model_choice(models.writer_model_id)?;

    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    ensure_ready_for_turn(&loaded.workspace, expected_revision)?;
    if loaded.workspace.draft.content.sections.is_empty() {
        return Err(ReportPipelineError::InvalidInput(
            "This report has no sections to plan. Apply a template, or write a section, before planning a whole-report draft."
                .to_string(),
        ));
    }

    let mut run = create_run_for_workspace(
        s3,
        bucket,
        &mut loaded,
        models.writer_model_id,
        instructions,
        None,
        DraftRunStatus::Planning,
    )
    .await?;

    let outcome = plan_fresh_run(
        sdk_config,
        s3,
        bucket,
        &loaded.workspace,
        &mut run,
        models,
        request,
    )
    .await;
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            release_failed_run(s3, bucket, client_id, report_id, &mut run).await;
            Err(error)
        }
    }
}

async fn plan_fresh_run(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    workspace: &ReportWorkspace,
    run: &mut LoadedRun,
    models: PlanModels<'_>,
    request: DraftPlanRequest<'_>,
) -> Result<DraftPlanOutcome, ReportPipelineError> {
    let corpus = prepare_analysis_corpus(s3, bucket, workspace.client_id, models, request).await?;
    let system = analysis_system_blocks(workspace, &corpus)?;

    let expected: Vec<(Uuid, &str)> = workspace
        .draft
        .content
        .sections
        .iter()
        .map(|section| (section.id, section.heading.as_str()))
        .collect();
    let total = section_total(expected.len());
    let batches: Vec<&[(Uuid, &str)]> = expected.chunks(PLAN_BATCH_SECTIONS).collect();
    let batch_count = batches.len();

    // One pass, one budget, one row counter, however many calls the document
    // takes: the verified `CountTokens` belongs to the request shape, which
    // every batch shares, and the counter's denominator is the whole document.
    let pass = AnalysisPass::new(sdk_config, models.planner_model_id, &system, request, total)?;
    let mut tally = PassTally::default();
    // One accumulator for the whole plan: a filename the corpus does not have
    // is the same warning whichever batch names it, and the gate shows a
    // bounded, deduplicated list.
    let mut warnings = Warnings::default();
    let mut entries: Vec<PlanEntry> = Vec::with_capacity(expected.len());

    for (index, batch) in batches.iter().enumerate() {
        // Between batches is where a Stop lands: the call in flight is
        // interrupted by the stream loop, and this is what stops the pass
        // opening the next billed one.
        if pass.stop.is_stopped() {
            return Err(map_bedrock_error(BedrockError::Stopped, PLANNER_LABELS));
        }
        let batch_len = section_total(batch.len());
        let planned = pass
            .run_call(
                fresh_plan_instruction(
                    request.instructions.trim(),
                    batch,
                    index.saturating_add(1),
                    batch_count,
                ),
                analysis::SUBMIT_SECTION_PLAN_TOOL,
                batch_len,
                &mut tally,
                |input| {
                    let rows = analysis::decode_section_plan(input).map_err(bedrock_diagnostic)?;
                    validate_section_plan(&rows.rows, batch, &corpus, &mut warnings)
                },
            )
            .await?;
        let first = section_total(index * PLAN_BATCH_SECTIONS).saturating_add(1);
        entries.extend(planned);
        pass.advance_rows(batch_len);
        request.emit_progress(ReportTurnProgress::PlanBatchPlanned {
            first,
            last: first.saturating_add(batch_len).saturating_sub(1),
            total,
        });
    }

    let now = jiff::Timestamp::now();
    run.run.plan = Some(RunPlan {
        model_id: models.planner_model_id.to_string(),
        entries,
        user_edited: false,
        synthetic: false,
        approved_at: None,
        plan_warnings: warnings.into_vec(),
        created_at: now,
    });
    run.run.status = DraftRunStatus::AwaitingApproval;
    save_draft_run(s3, bucket, run).await?;

    Ok(DraftPlanOutcome {
        run: run.run.clone(),
        usage: tally.usage,
        converse_calls: tally.converse_calls,
    })
}

/// Apply the clinician's gate edits to a plan waiting for approval.
///
/// Editing is what the gate is for, so this is deliberately permissive about
/// what may change and strict about what the result has to be: a scope inside
/// the domain's ceiling, and evidence naming records this client actually
/// has. It never adds or removes a row — the plan's shape is the document's.
pub async fn update_draft_plan(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
    edits: &[PlanEntryEdit],
) -> Result<DraftRun, ReportPipelineError> {
    let mut run = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;
    if !matches!(
        run.run.status,
        DraftRunStatus::AwaitingApproval | DraftRunStatus::Stopped
    ) {
        return Err(ReportPipelineError::InvalidInput(
            "This drafting run is not waiting for plan approval, so its plan can no longer be edited."
                .to_string(),
        ));
    }
    if run.run.plan.is_none() {
        return Err(ReportPipelineError::InvalidInput(
            "This drafting run has no plan to edit yet.".to_string(),
        ));
    }
    let inventory = load_record_inventory(s3, bucket, client_id)
        .await
        .map_err(|source| ReportPipelineError::storage("listing records for the plan", source))?;
    let known_files: HashSet<&str> = inventory
        .iter()
        .map(|entry| entry.filename.as_str())
        .collect();

    let plan = run.run.plan.as_mut().ok_or_else(|| {
        ReportPipelineError::InvalidInput("This drafting run has no plan to edit yet.".to_string())
    })?;
    let mut edited_intents: Vec<(Uuid, SectionIntent)> = Vec::new();
    for edit in edits {
        let entry = plan
            .entries
            .iter_mut()
            .find(|entry| entry.section_id == edit.section_id)
            .ok_or_else(|| {
                ReportPipelineError::InvalidInput(format!(
                    "Section {} is not part of this plan.",
                    edit.section_id
                ))
            })?;
        if let Some(intent) = edit.intent {
            entry.intent = intent;
            edited_intents.push((edit.section_id, intent));
        }
        if let Some(scope) = &edit.scope {
            if scope.chars().count() > MAX_PLAN_SCOPE_CHARACTERS {
                return Err(ReportPipelineError::InvalidInput(format!(
                    "A section's scope exceeds {MAX_PLAN_SCOPE_CHARACTERS} characters."
                )));
            }
            entry.scope = scope.clone();
        }
        if let Some(evidence) = &edit.evidence {
            if evidence.len() > MAX_PLAN_EVIDENCE {
                return Err(ReportPipelineError::InvalidInput(format!(
                    "A section may reference at most {MAX_PLAN_EVIDENCE} records."
                )));
            }
            for reference in evidence {
                if !known_files.contains(reference.filename.as_str()) {
                    return Err(ReportPipelineError::InvalidInput(format!(
                        "This client has no record named {}.",
                        reference.filename
                    )));
                }
            }
            entry.evidence = evidence.clone();
        }
        if let Some(instruction) = &edit.instruction {
            entry.instruction = (!instruction.trim().is_empty()).then(|| instruction.clone());
        }
    }
    plan.user_edited = true;

    // An interrupted run is decided by its section states: picking it back up
    // re-derives the plan from them, so a directive changed at the resume gate
    // has to land on the section or the resume pass would recompute it away.
    // A fresh gate needs none of this — nothing has been written yet, and
    // `start_draft_run` seeds the sections from the whole approved plan.
    if matches!(
        run.run.status,
        DraftRunStatus::Stopped | DraftRunStatus::Failed
    ) {
        let now = jiff::Timestamp::now();
        for (section_id, intent) in edited_intents {
            if let Some(section) = run
                .run
                .sections
                .iter_mut()
                .find(|section| section.section_id == section_id)
            {
                apply_intent_to_section(section, intent, now);
            }
        }
    }

    save_draft_run(s3, bucket, &mut run).await?;
    Ok(run.run.clone())
}

/// Approve a plan and draft the report it describes.
///
/// The run already exists and already owns the workspace — it has since the
/// plan pass created it — so this stamps the approval, fixes the writing
/// model, seeds the decisions the plan already made, and hands the run to the
/// same executor the un-gated command uses.
// Same explicit orchestration boundary as every other entry point here.
#[allow(clippy::too_many_arguments)]
pub async fn start_draft_run(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullReportGenerationOutcome, ReportPipelineError> {
    validate_model_choice(model_id)?;
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut run = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;

    if run.run.status != DraftRunStatus::AwaitingApproval || run.run.plan.is_none() {
        return Err(ReportPipelineError::InvalidInput(
            "This drafting run is not waiting for plan approval.".to_string(),
        ));
    }
    if loaded.workspace.draft.revision != run.run.base_revision {
        return Err(ReportPipelineError::InvalidInput(
            "The report changed after this plan was made, so the plan no longer fits it. Plan the draft again."
                .to_string(),
        ));
    }
    if loaded.workspace.session.pending_proposal.is_some() {
        return Err(ReportPipelineError::InvalidInput(
            "Accept or reject the pending proposal before starting another writer action."
                .to_string(),
        ));
    }
    if loaded
        .workspace
        .active_run_id
        .is_some_and(|active| active != run_id)
    {
        return Err(ReportPipelineError::InvalidInput(
            claria_report_store::ACTIVE_RUN_MESSAGE.to_string(),
        ));
    }

    let now = jiff::Timestamp::now();
    if let Some(plan) = run.run.plan.as_mut() {
        plan.approved_at = Some(now);
    }
    run.run.writer_model_id = model_id.to_string();
    run.run.status = DraftRunStatus::Drafting;
    apply_plan_to_sections(&mut run.run, now);
    save_draft_run(s3, bucket, &mut run).await?;
    loaded.workspace.active_run_id = Some(run_id);
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;

    // A run that reached this function has an approved plan by definition, so
    // it always drafts in parallel. The serial conversation stays for the
    // un-gated command, whose synthetic 1:1 plan is nobody's decision and
    // states nothing that would make its sections independent.
    execute_parallel_draft_turn(
        sdk_config,
        s3,
        bucket,
        loaded,
        model_id,
        request,
        run,
        DraftTurnKind::Fresh,
    )
    .await
}

/// Re-plan an interrupted run and then finish it.
///
/// New instructions are a question only a model can answer — they may change
/// what a section that already landed should say — so they get a planning
/// call over the run's durable state. No new instructions is a question with
/// a decidable answer, so it skips the model: keep what was drafted, skip
/// what was skipped, draft the rest.
// Same explicit orchestration boundary as every other entry point here.
#[allow(clippy::too_many_arguments)]
pub async fn resume_planned_draft_run(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
    planner_model_id: &str,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullReportGenerationOutcome, ReportPipelineError> {
    validate_model_choice(model_id)?;
    validate_model_choice(planner_model_id)?;
    plan_draft_resume(
        sdk_config,
        s3,
        bucket,
        client_id,
        report_id,
        run_id,
        PlanModels {
            planner_model_id,
            writer_model_id: model_id,
        },
        DraftPlanRequest::new(request.guidance)
            .with_stream_bounds(request.limits.runtime().analysis_stream_bounds()),
    )
    .await?;
    crate::run::resume_draft_run(
        sdk_config, s3, bucket, client_id, report_id, run_id, model_id, request,
    )
    .await
}

/// Decide what a resumed run should do with each of its sections, and write
/// that plan onto the run.
///
/// Left public so a gated resume can show the new plan before executing it:
/// the run stays interrupted and unapproved afterwards, exactly as it was.
// Same explicit orchestration boundary as every other entry point here.
#[allow(clippy::too_many_arguments)]
pub async fn plan_draft_resume(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
    models: PlanModels<'_>,
    request: DraftPlanRequest<'_>,
) -> Result<DraftPlanOutcome, ReportPipelineError> {
    let instructions = request.instructions.trim();
    validate_instructions(instructions)?;
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut run = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;
    if !matches!(
        run.run.status,
        DraftRunStatus::Failed | DraftRunStatus::Stopped
    ) {
        return Err(ReportPipelineError::InvalidInput(
            "Only an interrupted whole-report draft can be re-planned.".to_string(),
        ));
    }

    let now = jiff::Timestamp::now();
    // A resume plan is only as decided as the plan it rebuilds from: it
    // carries that plan's evidence forward row by row, so a synthetic source
    // yields a synthetic resume.
    let synthetic = run.run.plan.as_ref().is_some_and(|plan| plan.synthetic);
    if instructions.is_empty() {
        let entries = deterministic_resume_entries(&run.run);
        run.run.plan = Some(RunPlan {
            model_id: DETERMINISTIC_PLAN_MODEL_ID.to_string(),
            entries,
            user_edited: false,
            synthetic,
            approved_at: Some(now),
            plan_warnings: Vec::new(),
            created_at: now,
        });
        apply_plan_to_sections(&mut run.run, now);
        save_draft_run(s3, bucket, &mut run).await?;
        return Ok(DraftPlanOutcome {
            run: run.run.clone(),
            usage: None,
            converse_calls: 0,
        });
    }

    let corpus = prepare_analysis_corpus(s3, bucket, client_id, models, request).await?;
    let system = analysis_system_blocks(&loaded.workspace, &corpus)?;
    let instruction = resume_plan_instruction(&run.run, instructions);

    // Deliberately one call over every section, where a fresh plan is
    // batched. A resume asks what to do with sections that already have
    // durable state, and that state is one table the model reads as a whole:
    // splitting it would hand each call a slice of a picture whose point is
    // that it is complete. It is also the smaller question — a decision word
    // and a line of reasoning per row — so it never had the fresh plan's
    // length problem.
    let total = section_total(run.run.sections.len());
    let pass = AnalysisPass::new(sdk_config, models.planner_model_id, &system, request, total)?;
    let mut tally = PassTally::default();
    let mut warnings = Warnings::default();
    let entries = pass
        .run_call(
            instruction,
            analysis::SUBMIT_RESUME_PLAN_TOOL,
            total,
            &mut tally,
            |input| {
                let rows = analysis::decode_resume_plan(input).map_err(bedrock_diagnostic)?;
                validate_resume_plan(&rows.rows, &run.run, &corpus, &mut warnings)
            },
        )
        .await?;

    run.run.plan = Some(RunPlan {
        model_id: models.planner_model_id.to_string(),
        entries,
        user_edited: false,
        synthetic: false,
        approved_at: Some(now),
        plan_warnings: warnings.into_vec(),
        created_at: now,
    });
    apply_plan_to_sections(&mut run.run, now);
    save_draft_run(s3, bucket, &mut run).await?;
    Ok(DraftPlanOutcome {
        run: run.run.clone(),
        usage: tally.usage,
        converse_calls: tally.converse_calls,
    })
}

// ── The forced call, and its one repair round ───────────────────────────────

/// What a planning pass has spent so far, across every call it has made.
#[derive(Debug, Default)]
struct PassTally {
    usage: Option<TurnUsage>,
    converse_calls: u32,
}

/// The key every plan row opens with, in both plan tools' schemas.
const SECTION_ID_KEY: &str = "\"section_id\"";

/// How many rows this pass is owed, as a progress denominator. The ceiling
/// on a document's sections is far below `u32`, so saturating is unreachable
/// rather than a policy.
fn section_total(sections: usize) -> u32 {
    u32::try_from(sections).unwrap_or(u32::MAX)
}

/// Counts finished plan rows in the partial tool-input JSON, so a call that
/// runs for minutes can say how far along it is.
///
/// Deliberately not a parser. It looks for one key and reads nothing else out
/// of the buffer — a partial plan carries the same untrusted scope text the
/// whole one does, and the whole one is what gets validated. The count is a
/// number for a progress line and nothing else depends on it.
#[derive(Debug, Default)]
struct PlanRowCounter {
    /// Bytes since the last key, so one split across two fragments is still
    /// seen exactly once. Bounded by the length of a single row.
    carry: String,
    /// Rows seen in the attempt now streaming.
    seen: u32,
    /// Rows earlier batches already had accepted. The bar is against the
    /// whole document, so a batch counts on from where its predecessor
    /// stopped rather than restarting at one.
    base: u32,
    /// Highest count already announced. A throttle retry or a repair round
    /// starts the JSON again from nothing, and the reader must not watch the
    /// count walk backwards.
    announced: u32,
}

impl PlanRowCounter {
    /// Begin a fresh attempt. What has already been announced stands, and so
    /// does what earlier batches settled.
    fn restart(&mut self) {
        self.carry.clear();
        self.seen = 0;
    }

    /// Retire a batch that validated: its rows are decided, so the next
    /// batch's counting starts above them.
    fn advance_base(&mut self, batch_len: u32) {
        self.base = self.base.saturating_add(batch_len);
    }

    /// Absorb one fragment of partial JSON, returning the new row count when
    /// it has moved past everything announced so far.
    ///
    /// `batch_len` is how many rows the call now streaming was asked for and
    /// `total` how many the document has: a planner that answers with more
    /// rows than it was given is a validation failure, not a progress line
    /// that counts past its own denominator.
    fn absorb(&mut self, fragment: &str, batch_len: u32, total: u32) -> Option<u32> {
        self.carry.push_str(fragment);
        let mut consumed = 0;
        while let Some(at) = self.carry[consumed..].find(SECTION_ID_KEY) {
            consumed += at + SECTION_ID_KEY.len();
            self.seen = self.seen.saturating_add(1);
        }
        self.carry.drain(..consumed);
        let seen = self
            .base
            .saturating_add(self.seen.min(batch_len))
            .min(total);
        (seen > self.announced).then(|| {
            self.announced = seen;
            seen
        })
    }
}

/// One planning pass, and everything its calls share.
///
/// A fresh plan is several calls over one request shape: the same system
/// blocks, the same tool set, the same cache plan, and one verified input
/// count. Holding them here is what keeps that true — a budget built inside
/// the batch loop would silently pay for a `CountTokens` per batch, and a
/// counter built there would restart the reader's number at every checkpoint.
struct AnalysisPass<'a> {
    sdk_config: &'a aws_config::SdkConfig,
    planner_model_id: &'a str,
    system: &'a [String],
    tools: analysis::AnalysisToolConfiguration,
    cache_plan: claria_bedrock::converse::CachePlan,
    /// Behind a lock so each retry attempt can re-borrow it: the verified
    /// count belongs to the request, not the attempt, so a re-sent request
    /// must not pay for a second `CountTokens`.
    budget: tokio::sync::Mutex<AnalysisInputBudget>,
    /// Rows counted off the partial answer, behind a lock because the stream
    /// hands fragments to a shared callback rather than to this task.
    counter: std::sync::Mutex<PlanRowCounter>,
    stop: &'a StopSignal,
    request: DraftPlanRequest<'a>,
    /// Rows the whole pass is owed: the progress denominator, known before
    /// the first call.
    section_total: u32,
}

impl<'a> AnalysisPass<'a> {
    fn new(
        sdk_config: &'a aws_config::SdkConfig,
        planner_model_id: &'a str,
        system: &'a [String],
        request: DraftPlanRequest<'a>,
        section_total: u32,
    ) -> Result<Self, ReportPipelineError> {
        Ok(Self {
            sdk_config,
            planner_model_id,
            system,
            tools: analysis::analysis_tool_configuration()
                .map_err(|error| map_bedrock_error(error, PLANNER_LABELS))?,
            cache_plan: claria_bedrock::converse::CachePlan::analysis(
                claria_core::model_id::ModelCapabilities::for_id(planner_model_id),
            ),
            budget: tokio::sync::Mutex::new(AnalysisInputBudget::new(
                planner_model_id,
                "report_plan",
                PLAN_OUTPUT_TOKEN_RESERVE,
            )),
            counter: std::sync::Mutex::new(PlanRowCounter::default()),
            // A caller that supplied no signal gets one nobody can fire.
            stop: match request.stop {
                Some(stop) => stop,
                None => crate::turn::never_stopped(),
            },
            request,
            section_total,
        })
    }

    /// Retire the rows a batch settled, so the next one counts on from them.
    fn advance_rows(&self, batch_len: u32) {
        if let Ok(mut counter) = self.counter.lock() {
            counter.advance_base(batch_len);
        }
    }

    /// Force one analysis tool, validate the answer, and grant exactly
    /// [`PLAN_REPAIR_ROUNDS`] correction before giving up.
    ///
    /// The correction is an error `tool_result` carrying the validator's own
    /// words. Those words name section IDs, row indexes, and JSON fields —
    /// never record or draft text — so sending them back costs nothing and is
    /// the difference between a model that fixes one row and a model that
    /// re-guesses the whole plan.
    ///
    /// A repair round is not the same thing as a retry. Rounds answer a plan
    /// the host refused; retries answer a call that never landed — a throttle,
    /// or a stream that stalled or was severed — and re-send the identical
    /// request without spending a round or a second `CountTokens`.
    ///
    /// The conversation starts empty every time. A batch that carried its
    /// predecessors' transcripts would grow a messages tier that nothing
    /// caches, for an answer that is about sections the earlier calls were
    /// never shown.
    async fn run_call<T>(
        &self,
        instruction: String,
        forced_tool: &'static str,
        batch_len: u32,
        tally: &mut PassTally,
        mut validate: impl FnMut(&serde_json::Value) -> Result<T, String>,
    ) -> Result<T, ReportPipelineError> {
        let mut messages = vec![ReportProtocolMessage {
            role: ReportProtocolRole::User,
            content: vec![ReportProtocolBlock::Text { text: instruction }],
            created_at: jiff::Timestamp::now(),
        }];

        let count_row = |fragment: &str| {
            let Ok(mut counter) = self.counter.lock() else {
                return;
            };
            if let Some(planned) = counter.absorb(fragment, batch_len, self.section_total) {
                self.request
                    .emit_progress(ReportTurnProgress::PlanRowPlanned {
                        planned,
                        total: self.section_total,
                    });
            }
        };

        let mut rejection: Option<String> = None;
        for round in 0..=PLAN_REPAIR_ROUNDS {
            tally.converse_calls = tally.converse_calls.saturating_add(1);
            let call_number = tally.converse_calls;
            let on_retry = |attempt, delay| {
                self.request.emit_progress(ReportTurnProgress::retrying(
                    call_number,
                    attempt,
                    MAX_ATTEMPTS,
                    delay,
                ));
            };
            let output: StructuredCallOutput =
                with_throttle_retry_observed("report_plan", Some(&on_retry), || {
                    let messages = &messages;
                    let count_row = &count_row;
                    if let Ok(mut counter) = self.counter.lock() {
                        counter.restart();
                    }
                    // Announced per attempt rather than per round: a re-sent
                    // request is a new stream whose row count starts over, and
                    // saying it started is what retires the retry line.
                    self.request
                        .emit_progress(ReportTurnProgress::ModelCallStarted { call_number });
                    async move {
                        analysis::converse_structured(
                            self.sdk_config,
                            StructuredCallRequest {
                                model_id: self.planner_model_id,
                                system: self.system,
                                messages,
                                tools: &self.tools,
                                forced_tool,
                                max_tokens: PLAN_OUTPUT_TOKEN_RESERVE,
                                cache_plan: self.cache_plan.clone(),
                                stream_bounds: self.request.stream_bounds,
                                stop: self.stop,
                                on_partial_tool_input: Some(count_row),
                                operation: "report_plan",
                            },
                            &mut *self.budget.lock().await,
                        )
                        .await
                    }
                })
                .await
                .map_err(|error| map_bedrock_error(error, PLANNER_LABELS))?;
            tally.usage = merge_optional_usage(tally.usage.take(), output.usage.clone());

            match validate(&output.input) {
                Ok(value) => return Ok(value),
                Err(diagnostic) => {
                    if round == PLAN_REPAIR_ROUNDS {
                        rejection = Some(diagnostic);
                        break;
                    }
                    messages.push(ReportProtocolMessage {
                        role: ReportProtocolRole::Assistant,
                        content: vec![ReportProtocolBlock::ToolUse {
                            tool_use_id: output.tool_use_id.clone(),
                            name: forced_tool.to_string(),
                            input: output.input.clone(),
                        }],
                        created_at: jiff::Timestamp::now(),
                    });
                    messages.push(ReportProtocolMessage {
                        role: ReportProtocolRole::User,
                        content: vec![ReportProtocolBlock::ToolResult {
                            tool_use_id: output.tool_use_id,
                            status: ReportToolResultStatus::Error,
                            content: serde_json::json!({"error": {
                                "code": "invalid_plan",
                                "message": diagnostic,
                            }}),
                        }],
                        created_at: jiff::Timestamp::now(),
                    });
                    rejection = None;
                }
            }
        }

        let diagnostic = rejection.unwrap_or_else(|| "the plan could not be validated".to_string());
        let planner_model_id = self.planner_model_id;
        let converse_calls = tally.converse_calls;
        tracing::error!(
            planner_model_id,
            forced_tool,
            converse_calls,
            diagnostic,
            "the planner could not produce a valid plan"
        );
        Err(ReportPipelineError::InvalidInput(format!(
            "The planning model could not produce a usable plan for this report, even after a correction. Nothing was saved. Details: {diagnostic}"
        )))
    }
}

/// How one analysis role names itself in the sentences a clinician reads when
/// its call fails. Every message below is the same message with these three
/// nouns substituted, which is why there is one mapper rather than one per
/// role: a new failure mode has to be worded once.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AnalysisRoleLabels {
    /// "planning pass", "review pass".
    pub(crate) pass: &'static str,
    /// "planning model", "reviewing model".
    pub(crate) model: &'static str,
    /// "plan", "review".
    pub(crate) artifact: &'static str,
}

pub(crate) const PLANNER_LABELS: AnalysisRoleLabels = AnalysisRoleLabels {
    pass: "planning pass",
    model: "planning model",
    artifact: "plan",
};

pub(crate) fn map_bedrock_error(
    error: BedrockError,
    labels: AnalysisRoleLabels,
) -> ReportPipelineError {
    let AnalysisRoleLabels {
        pass,
        model,
        artifact,
    } = labels;
    match &error {
        BedrockError::Stopped => ReportPipelineError::InvalidInput(format!(
            "The {pass} was stopped before it finished. Nothing was saved."
        )),
        BedrockError::ResponseTruncated { .. } => ReportPipelineError::InvalidInput(format!(
            "The {artifact} was cut off at the {model}'s response limit, so it could not be used. Try a smaller report, or choose a {model} with a larger response limit."
        )),
        BedrockError::ContextBudgetExceeded { .. } => ReportPipelineError::InvalidInput(format!(
            "This client's records exceed the {model}'s context window. Remove or split oversized records, or choose a {model} with a larger context window."
        )),
        BedrockError::UnsupportedModel(_) => ReportPipelineError::InvalidInput(format!(
            "The selected {model} is not verified for report tools. Choose a current Claude model in Preferences."
        )),
        // Every attempt went out and none came back complete — the stream
        // never started, went silent, or was severed, or the request got no
        // response at all. The wait, not the request, is the actionable
        // fact. Must precede the catch-all, which would otherwise report a
        // service that answered nothing as one Claria could not reach.
        _ if error.is_interrupted_before_completion() => {
            ReportPipelineError::InvalidInput(format!(
                "Bedrock appears to be having a hard time: the {pass} got no response, even after retries. Nothing was saved. Try again later."
            ))
        }
        _ => {
            // Structural, PHI-free: service codes, request IDs, protocol
            // shapes. The user-facing sentence below deliberately carries none
            // of it.
            tracing::error!(error = %error, pass, "the analysis call failed");
            ReportPipelineError::InvalidInput(format!(
                "Claria could not reach the {model}. Nothing was saved. The Claria Console log holds the underlying error."
            ))
        }
    }
}

/// A decode failure's serde diagnostic, verbatim.
pub(crate) fn bedrock_diagnostic(error: BedrockError) -> String {
    match error {
        BedrockError::SchemaViolation(message) => message,
        other => other.to_string(),
    }
}

pub(crate) fn merge_optional_usage(
    total: Option<TurnUsage>,
    call: Option<TurnUsage>,
) -> Option<TurnUsage> {
    match (total, call) {
        (None, call) => call,
        (Some(total), None) => Some(total),
        (Some(mut total), Some(call)) => {
            total.input_tokens = total.input_tokens.saturating_add(call.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(call.output_tokens);
            total.cache_read_input_tokens = total
                .cache_read_input_tokens
                .saturating_add(call.cache_read_input_tokens);
            total.cache_write_input_tokens = total
                .cache_write_input_tokens
                .saturating_add(call.cache_write_input_tokens);
            total.cost_usd += call.cost_usd;
            if total.pricing_version != call.pricing_version {
                total.pricing_version = 0;
            }
            Some(total)
        }
    }
}

// ── Request assembly ────────────────────────────────────────────────────────

/// Build the record corpus every analysis call in this pass reads.
async fn prepare_analysis_corpus(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    models: PlanModels<'_>,
    request: DraftPlanRequest<'_>,
) -> Result<FullRecordContext, ReportPipelineError> {
    let inventory = load_record_inventory(s3, bucket, client_id)
        .await
        .map_err(|source| ReportPipelineError::storage("listing records for the plan", source))?;
    let corpus = load_full_record_context(
        s3,
        bucket,
        &[
            BudgetRole::planner(models.planner_model_id),
            // The writer's own ceiling is a per-request setting the plan pass
            // never sees; the default is what sizes this corpus, exactly as
            // it did before that ceiling became configurable.
            BudgetRole::writer(
                models.writer_model_id,
                claria_bedrock::report::DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE,
            ),
        ],
        &inventory,
    )
    .await?;
    request.emit_progress(ReportTurnProgress::RecordContextPrepared {
        included_files: corpus.summary.included_files,
        unavailable_files: corpus.summary.unavailable_files,
        total_characters: corpus.summary.total_characters,
    });
    Ok(corpus)
}

/// The analysis family's system blocks: the fixed planner policy, then the
/// two untrusted blocks the cache point sits above.
///
/// Untrusted data after the instructions, inside named delimiters, with the
/// data-not-instructions rule stated in the policy above it — the same
/// posture the writer uses, in a different transport tier.
///
/// The template block here carries structure without prose. Every analysis
/// role — fresh plan, resume plan, review sweep — reads these same blocks and
/// none of them reads a template body, so leaving the prose out is one
/// smaller shared prefix rather than a saving one role takes at the others'
/// expense. The writer's own block still carries it.
pub(crate) fn analysis_system_blocks(
    workspace: &ReportWorkspace,
    corpus: &FullRecordContext,
) -> Result<Vec<String>, ReportPipelineError> {
    let template = planner_template_context(workspace, &workspace.draft.content)
        .map_err(ReportPipelineError::InvalidInput)?;
    Ok(vec![
        planner_system_prompt(),
        corpus.prompt.clone(),
        template,
    ])
}

/// One batch's question: which sections this call answers for, and what a row
/// about one has to say.
///
/// The section IDs are listed here rather than left implicit in the template
/// block because the template block is the cached prefix — identical in every
/// batch — and the batch is the only thing that differs. Naming them by ID and
/// by ID alone also keeps the question small: the headings are already above,
/// and repeating untrusted text below the cache point buys nothing.
fn fresh_plan_instruction(
    guidance: &str,
    batch: &[(Uuid, &str)],
    batch_number: usize,
    batch_count: usize,
) -> String {
    let mut text = format!(
        "Plan this report before it is written. The report is being planned in \
         {batch_count} batches and this is batch {batch_number} of {batch_count}.\n\n\
         Plan these sections, and only these:\n"
    );
    for (section_id, _) in batch {
        text.push_str(&format!("- {section_id}\n"));
    }
    text.push_str(
        "\nRequirements:\n\
         1. Return exactly one row per section ID listed above, in the order listed. Copy each \
            section_id exactly; never invent, merge, reorder, or omit one. Return no row for any \
            other section of this report — the sections not listed above belong to the other \
            batches, and a row for one of them fails this call.\n\
         2. Assign a scope to EVERY section you were given: what that section must assert about \
            this client, in terms the writer can act on. One to three specific sentences.\n\
         3. Cite evidence as filenames copied exactly out of <untrusted_record_context>, each \
            with one line on why that record matters to that section. The host checks every \
            filename against the corpus, so an approximated name is dropped. Do not copy record \
            text into the plan — the writer reads the records in full when it drafts.\n\
         4. Give every section you mark \"draft\" at least one evidence record. A section the \
            records cannot support belongs on a \"skip\" row whose scope says what is missing.\n\
         5. Mark a row \"skip\" only when the records cannot support the section or the guidance \
            below defers it — never to shorten the job. A skipped section keeps its heading and \
            its place in the document and is left out of the export.\n\n",
    );
    if guidance.is_empty() {
        text.push_str("No additional user guidance was supplied.\n");
    } else {
        text.push_str(&format!("Additional user guidance:\n{guidance}\n"));
    }
    text.push_str("\nCall submit_section_plan exactly once with this batch's rows.\n");
    text
}

fn resume_plan_instruction(run: &DraftRun, instructions: &str) -> String {
    let mut text = String::from(
        "This drafting run was interrupted and is being picked back up. Plan what happens to \
         each of its sections now.\n\n\
         Requirements:\n\
         1. Return exactly one row per section in the state table below, in the order it lists \
            them. Copy each section_id exactly; never invent, merge, or omit a section.\n\
         2. \"keep\" is valid only for a section whose state is drafted: it leaves that section \
            exactly as it stands.\n\
         3. \"rewrite\" and \"draft\" both need a scope saying what the section must assert, and \
            \"rewrite\" also needs evidence — it is replacing text that already exists, so it has \
            to say what the replacement rests on.\n\
         4. Name evidence by filename, copied exactly out of <untrusted_record_context>, with \
            one line on why that record matters; a name the corpus does not list is dropped. Do \
            not copy record text into the plan.\n\
         5. Honour the updated instructions below. They are why this run is being re-planned: a \
            section that already landed may need rewriting to satisfy them.\n\n",
    );
    text.push_str(&format!("Run so far: {}\n", run_op_summary(run)));
    text.push_str(&format!(
        "\nSection state:\n{}\n",
        section_state_table(&run.sections)
    ));
    if let Some(original) = run.instructions.first() {
        text.push_str(&format!(
            "\nThe instruction this run started from:\n{}\n",
            original.text
        ));
    }
    text.push_str(&format!("\nUpdated instructions:\n{instructions}\n"));
    text.push_str("\nCall submit_resume_plan exactly once with the complete plan.\n");
    text
}

/// Counts, never content: what the interrupted run got through.
fn run_op_summary(run: &DraftRun) -> String {
    let count = |state: RunSectionState| {
        run.sections
            .iter()
            .filter(|section| section.state == state)
            .count()
    };
    format!(
        "{} sections in total — {} drafted, {} skipped, {} failed, {} never started.",
        run.sections.len(),
        count(RunSectionState::Drafted),
        count(RunSectionState::Skipped),
        count(RunSectionState::Failed),
        count(RunSectionState::Pending) + count(RunSectionState::Drafting)
    )
}

// ── Host validation ─────────────────────────────────────────────────────────

/// Check one batch of a fresh plan against the sections that batch was given.
///
/// Coverage and identity are the hard part and produce an `Err` the repair
/// round hands back — and the diagnostic is per-batch, so a model that
/// answered for the whole document is told it returned sections it was not
/// given rather than being congratulated on a superset. Evidence is checked
/// too, but a filename that names no record is a warning on the plan the
/// clinician is about to read, not a reason to refuse to show it to them.
///
/// The warnings accumulator is the caller's because a plan has one warning
/// list however many batches wrote it.
fn validate_section_plan(
    rows: &[SectionPlanRow],
    expected: &[(Uuid, &str)],
    corpus: &FullRecordContext,
    warnings: &mut Warnings,
) -> Result<Vec<PlanEntry>, String> {
    let by_id = index_rows(rows.iter().map(|row| row.section_id.as_str()), expected)?;

    let mut entries = Vec::with_capacity(expected.len());
    for (section_id, heading) in expected {
        let row = &rows[by_id[section_id]];
        let intent = match row.action {
            PlanAction::Draft => SectionIntent::Draft,
            PlanAction::Skip => SectionIntent::Skip,
        };
        let evidence = resolve_evidence(&row.evidence, corpus, warnings);
        if intent == SectionIntent::Draft && evidence.is_empty() {
            warnings.push(format!("no_resolved_evidence:{section_id}"));
        }
        entries.push(PlanEntry {
            section_id: *section_id,
            heading: (*heading).to_string(),
            intent,
            // Every row here answers for a section the template supplied, so
            // the completion gate may demand content for all of them.
            required: true,
            scope: truncate_characters(row.scope.trim(), MAX_PLAN_SCOPE_CHARACTERS),
            evidence,
            instruction: None,
        });
    }
    Ok(entries)
}

/// Check a resume plan against the run it is resuming.
fn validate_resume_plan(
    rows: &[analysis::ResumePlanRow],
    run: &DraftRun,
    corpus: &FullRecordContext,
    warnings: &mut Warnings,
) -> Result<Vec<PlanEntry>, String> {
    let mut ordered: Vec<&RunSection> = run.sections.iter().collect();
    ordered.sort_by_key(|section| section.position);
    let expected: Vec<(Uuid, &str)> = ordered
        .iter()
        .map(|section| (section.section_id, section.heading.as_str()))
        .collect();
    let by_id = index_rows(rows.iter().map(|row| row.section_id.as_str()), &expected)?;

    let states: HashMap<Uuid, RunSectionState> = run
        .sections
        .iter()
        .map(|section| (section.section_id, section.state))
        .collect();
    let previous: HashMap<Uuid, &PlanEntry> = run
        .plan
        .iter()
        .flat_map(|plan| plan.entries.iter())
        .map(|entry| (entry.section_id, entry))
        .collect();

    let mut entries = Vec::with_capacity(expected.len());
    for (section_id, heading) in &expected {
        let row = &rows[by_id[section_id]];
        let intent = match row.decision {
            ResumeDecision::Keep => SectionIntent::Keep,
            ResumeDecision::Rewrite => SectionIntent::Rewrite,
            ResumeDecision::Draft => SectionIntent::Draft,
            ResumeDecision::Skip => SectionIntent::Skip,
        };
        if row.decision == ResumeDecision::Keep
            && states.get(section_id) != Some(&RunSectionState::Drafted)
        {
            return Err(format!(
                "section {section_id} was given decision \"keep\", but its state is not \"drafted\"; only an already-drafted section can be kept"
            ));
        }
        let scope = row
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|scope| !scope.is_empty());
        if matches!(
            row.decision,
            ResumeDecision::Rewrite | ResumeDecision::Draft
        ) && scope.is_none()
        {
            return Err(format!(
                "section {section_id} was given decision \"{}\" without a scope; rewrite and draft rows must say what the section asserts",
                if row.decision == ResumeDecision::Rewrite {
                    "rewrite"
                } else {
                    "draft"
                }
            ));
        }
        let supplied = row.evidence.as_deref().unwrap_or_default();
        if row.decision == ResumeDecision::Rewrite && supplied.is_empty() {
            return Err(format!(
                "section {section_id} was given decision \"rewrite\" without evidence; replacing text that already exists must name the records the replacement rests on"
            ));
        }
        let mut evidence = resolve_evidence(supplied, corpus, warnings);
        if evidence.is_empty()
            && let Some(entry) = previous.get(section_id)
        {
            // The earlier plan's evidence is still true about the records;
            // dropping it because this round did not restate it would lose
            // the writer its grounding for a section it still has to write.
            evidence = entry.evidence.clone();
        }
        if matches!(intent, SectionIntent::Draft | SectionIntent::Rewrite) && evidence.is_empty() {
            warnings.push(format!("no_resolved_evidence:{section_id}"));
        }
        let scope = scope
            .map(|scope| truncate_characters(scope, MAX_PLAN_SCOPE_CHARACTERS))
            .or_else(|| previous.get(section_id).map(|entry| entry.scope.clone()))
            .unwrap_or_default();
        entries.push(PlanEntry {
            section_id: *section_id,
            heading: (*heading).to_string(),
            intent,
            required: previous.get(section_id).is_none_or(|entry| entry.required),
            scope,
            evidence,
            instruction: previous
                .get(section_id)
                .and_then(|entry| entry.instruction.clone()),
        });
    }
    Ok(entries)
}

/// Map each expected section ID to the row that answers for it, refusing an
/// unparseable ID, an invented one, a duplicate, or a missing one.
///
/// The diagnostic names the offending IDs because that is the whole value of
/// the repair round: "coverage was wrong" tells the model nothing it can act
/// on. Section UUIDs are host identifiers, not content.
pub(crate) fn index_rows<'a>(
    supplied: impl Iterator<Item = &'a str>,
    expected: &[(Uuid, &str)],
) -> Result<HashMap<Uuid, usize>, String> {
    let known: HashSet<Uuid> = expected.iter().map(|(id, _)| *id).collect();
    let mut by_id: HashMap<Uuid, usize> = HashMap::with_capacity(expected.len());
    for (index, raw) in supplied.enumerate() {
        let Ok(section_id) = raw.parse::<Uuid>() else {
            return Err(format!(
                "row {index} carries section_id \"{raw}\", which is not a 36-character UUID; copy the ID exactly from the supplied structure"
            ));
        };
        if !known.contains(&section_id) {
            return Err(format!(
                "row {index} carries section_id {section_id}, which is not one of the sections you were given; copy the IDs exactly and do not invent sections"
            ));
        }
        if by_id.insert(section_id, index).is_some() {
            return Err(format!(
                "section {section_id} appears in more than one row; return exactly one row per section"
            ));
        }
    }
    let mut missing: Vec<String> = expected
        .iter()
        .filter(|(id, _)| !by_id.contains_key(id))
        .map(|(id, _)| id.to_string())
        .collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(format!(
            "{} of the {} sections you were given have no row: {}. Return exactly one row per section, in the order they were listed.",
            missing.len(),
            expected.len(),
            missing.join(", ")
        ));
    }
    Ok(by_id)
}

/// Keep the evidence that names a file in the pinned corpus, and record a
/// warning for each one that does not.
fn resolve_evidence(
    supplied: &[PlanEvidence],
    corpus: &FullRecordContext,
    warnings: &mut Warnings,
) -> Vec<EvidenceRef> {
    supplied
        .iter()
        .take(MAX_PLAN_EVIDENCE)
        .filter_map(|evidence| {
            if !corpus
                .citable_filenames
                .iter()
                .any(|filename| filename == &evidence.filename)
            {
                warnings.push(format!("unknown_evidence_file:{}", evidence.filename));
                return None;
            }
            Some(EvidenceRef {
                filename: evidence.filename.clone(),
                note: evidence
                    .relevance
                    .as_deref()
                    .map(str::trim)
                    .filter(|note| !note.is_empty())
                    .map(|note| {
                        truncate_characters(
                            note,
                            claria_core::models::report_run::MAX_EVIDENCE_RELEVANCE_CHARACTERS,
                        )
                    }),
            })
        })
        .collect()
}

/// The plan's warning list: deduplicated, ordered, and bounded, so one badly
/// quoted file cannot produce a hundred identical lines at the gate.
#[derive(Default)]
pub(crate) struct Warnings(Vec<String>);

impl Warnings {
    pub(crate) fn push(&mut self, warning: String) {
        if self.0.len() < MAX_PLAN_WARNINGS && !self.0.contains(&warning) {
            self.0.push(warning);
        }
    }

    pub(crate) fn into_vec(self) -> Vec<String> {
        self.0
    }
}

pub(crate) fn truncate_characters(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

// ── Deterministic paths ─────────────────────────────────────────────────────

/// What a resume does when nobody asked for anything different: keep what
/// landed, respect what was deliberately skipped, and write the rest. No
/// model call, because there is no judgement here to buy.
fn deterministic_resume_entries(run: &DraftRun) -> Vec<PlanEntry> {
    let previous: HashMap<Uuid, &PlanEntry> = run
        .plan
        .iter()
        .flat_map(|plan| plan.entries.iter())
        .map(|entry| (entry.section_id, entry))
        .collect();
    let mut ordered: Vec<&RunSection> = run.sections.iter().collect();
    ordered.sort_by_key(|section| section.position);
    ordered
        .into_iter()
        .map(|section| {
            let intent = match section.state {
                RunSectionState::Drafted | RunSectionState::Flagged | RunSectionState::Kept => {
                    SectionIntent::Keep
                }
                RunSectionState::Skipped => SectionIntent::Skip,
                RunSectionState::Pending | RunSectionState::Drafting | RunSectionState::Failed => {
                    SectionIntent::Draft
                }
            };
            let earlier = previous.get(&section.section_id);
            PlanEntry {
                section_id: section.section_id,
                heading: section.heading.clone(),
                intent,
                required: earlier.is_none_or(|entry| entry.required),
                scope: earlier.map(|entry| entry.scope.clone()).unwrap_or_default(),
                evidence: earlier
                    .map(|entry| entry.evidence.clone())
                    .unwrap_or_default(),
                instruction: earlier.and_then(|entry| entry.instruction.clone()),
            }
        })
        .collect()
}

/// Seed the run's section states from the decisions its plan already made.
///
/// A `skip` row needs no model call to carry out, so the section is skipped
/// here and the writer is never asked about it. A `draft` or `rewrite` row
/// over a section that already landed clears it back to pending — rewriting
/// means replacing, and leaving the staged blocks in place would make the
/// finisher treat the section as already decided. `keep` deliberately touches
/// nothing.
fn apply_plan_to_sections(run: &mut DraftRun, now: jiff::Timestamp) {
    let Some(plan) = run.plan.clone() else {
        return;
    };
    for entry in &plan.entries {
        let Some(section) = run
            .sections
            .iter_mut()
            .find(|section| section.section_id == entry.section_id)
        else {
            continue;
        };
        apply_intent_to_section(section, entry.intent, now);
    }
}

/// Carry one plan row's decision onto the section it names.
///
/// `keep` deliberately touches nothing, and a directive a section is already
/// in is not a write — re-applying a plan must not restamp `updated_at`.
fn apply_intent_to_section(section: &mut RunSection, intent: SectionIntent, now: jiff::Timestamp) {
    match intent {
        SectionIntent::Skip if section.state != RunSectionState::Skipped => {
            section.state = RunSectionState::Skipped;
            section.blocks.clear();
            section.citations.clear();
            section.error = None;
            section.updated_at = now;
        }
        SectionIntent::Draft | SectionIntent::Rewrite
            if section.state != RunSectionState::Pending =>
        {
            section.state = RunSectionState::Pending;
            section.blocks.clear();
            section.citations.clear();
            section.error = None;
            section.updated_at = now;
        }
        _ => {}
    }
}

fn validate_instructions(instructions: &str) -> Result<(), ReportPipelineError> {
    if instructions.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportPipelineError::InvalidInput(format!(
            "The planning guidance exceeds {MAX_INSTRUCTION_CHARACTERS} characters."
        )));
    }
    Ok(())
}
