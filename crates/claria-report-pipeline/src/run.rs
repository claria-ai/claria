//! The durable drafting run behind whole-report generation: creating it,
//! releasing it when a turn fails, healing one whose completion write was lost,
//! re-entering the turn loop to finish an interrupted one, and the two ways a
//! run that will never be picked back up ends — finalized from what it landed,
//! or abandoned.

use std::collections::HashSet;

use aws_sdk_s3::Client as S3Client;
use claria_core::models::{
    report::{AuthorshipKind, ReportContent, ReportWorkspace, SectionAuthorship},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, PlanEntry, RunInstruction, RunPlan,
        RunSection, RunSectionState, SectionIntent,
    },
};
use claria_report_store::{
    LoadedRun, LoadedWorkspace, MAX_INSTRUCTION_CHARACTERS, create_draft_run, list_draft_runs,
    load_draft_run, load_for_report, mark_template_current, save_draft_run, save_loaded,
};
use uuid::Uuid;

use crate::{
    FullReportGenerationOutcome, ReportPipelineError,
    full_draft_context::DraftTurnKind,
    tools::{merge_undrafted_sections, written_sections},
    turn::{FullReportRequest, execute_full_draft_turn, validate_model_choice},
};

/// The plan the un-gated whole-report command runs on: one `Draft` entry per
/// section the report already has, in document order, auto-approved.
///
/// It is nobody's decision — no model produced it and no clinician saw it —
/// which is exactly what the tool loop checks before defending a plan row
/// against the writer. It exists so the run's plan is the single authority on
/// which sections the finisher demands a decision about, on the path that has
/// no planning pass in front of it.
pub(crate) fn synthetic_plan(
    workspace: &ReportWorkspace,
    model_id: &str,
    now: jiff::Timestamp,
) -> RunPlan {
    RunPlan {
        model_id: model_id.to_string(),
        entries: workspace
            .draft
            .content
            .sections
            .iter()
            .map(|section| PlanEntry {
                section_id: section.id,
                heading: section.heading.clone(),
                intent: SectionIntent::Draft,
                required: true,
                scope: String::new(),
                evidence: Vec::new(),
                instruction: None,
            })
            .collect(),
        user_edited: false,
        approved_at: Some(now),
        plan_warnings: Vec::new(),
        created_at: now,
    }
}

fn new_draft_run(
    workspace: &ReportWorkspace,
    model_id: &str,
    guidance: &str,
    now: jiff::Timestamp,
    plan: Option<RunPlan>,
    status: DraftRunStatus,
) -> DraftRun {
    DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id: Uuid::new_v4(),
        report_id: workspace.report_id,
        client_id: workspace.client_id,
        base_revision: workspace.draft.revision,
        status,
        plan,
        title: None,
        sections: workspace
            .draft
            .content
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| RunSection {
                section_id: section.id,
                heading: section.heading.clone(),
                position: u32::try_from(index).unwrap_or(u32::MAX),
                state: RunSectionState::Pending,
                blocks: Vec::new(),
                citations: Vec::new(),
                attempts: 0,
                error: None,
                updated_at: now,
            })
            .collect(),
        instructions: if guidance.is_empty() {
            Vec::new()
        } else {
            vec![RunInstruction {
                text: guidance.to_string(),
                added_at: now,
            }]
        },
        writer_model_id: model_id.to_string(),
        finalized_revision: None,
        partial: false,
        created_at: now,
        updated_at: now,
    }
}

/// Create a run and hand the workspace to it. The pointer is saved before the
/// first model call, so a second window that opens mid-run is refused rather
/// than cutting a revision underneath it.
///
/// `plan` and `status` are what separate the two ways a run begins: the
/// un-gated command opens one already `Drafting` on a synthetic plan, while
/// the planning pass opens one `Planning` with no plan at all and fills it in.
pub(crate) async fn create_run_for_workspace(
    s3: &S3Client,
    bucket: &str,
    loaded: &mut LoadedWorkspace,
    model_id: &str,
    guidance: &str,
    plan: Option<RunPlan>,
    status: DraftRunStatus,
) -> Result<LoadedRun, ReportPipelineError> {
    let now = jiff::Timestamp::now();
    let run = create_draft_run(
        s3,
        bucket,
        new_draft_run(&loaded.workspace, model_id, guidance, now, plan, status),
    )
    .await?;
    loaded.workspace.active_run_id = Some(run.run.run_id);
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, loaded).await?;
    Ok(run)
}

/// Give the workspace back after a failed or aborted whole-report turn.
///
/// Sections that landed stay durable in the run object for later salvage; the
/// workspace pointer is cleared unconditionally so retrying, editing, and
/// saving all keep working exactly as they did before runs existed. The
/// workspace is re-read rather than saved from the caller's copy: a failed turn
/// may have mutated that copy in memory, and none of it may reach S3.
pub(crate) async fn release_failed_run(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run: &mut LoadedRun,
) {
    let now = jiff::Timestamp::now();
    run.run.demote_interrupted_sections(now);
    run.run.status = DraftRunStatus::Failed;
    if let Err(error) = save_draft_run(s3, bucket, run).await {
        // The turn already failed for its own reason, which the caller is
        // about to surface. Losing the status stamp only costs the run its
        // "resumable" label, so it must not replace that reason.
        tracing::warn!(
            run_id = %run.run.run_id,
            error = %error,
            "could not stamp the failed drafting run"
        );
    }
    if let Err(error) = clear_active_run(s3, bucket, client_id, report_id, run.run.run_id).await {
        tracing::warn!(
            run_id = %run.run.run_id,
            error = %error,
            "could not release the workspace from its drafting run"
        );
    }
}

/// Park a run the reader stopped, leaving it exactly where they stopped it.
///
/// The one difference from [`release_failed_run`] is the one that matters:
/// the workspace keeps pointing at this run. A stopped run is the resumable
/// state the writing surface hydrates from, and the guards that refuse
/// competing edits while a run is active are what stop the report moving out
/// from under the sections it already wrote. Resuming, finalizing from what
/// it landed, or abandoning it are the three ways the pointer is released.
///
/// Sections that landed need no write of their own — each was made durable
/// before the model was told it had succeeded.
pub(crate) async fn park_stopped_run(s3: &S3Client, bucket: &str, run: &mut LoadedRun) {
    let now = jiff::Timestamp::now();
    run.run.demote_interrupted_sections(now);
    run.run.status = DraftRunStatus::Stopped;
    if let Err(error) = save_draft_run(s3, bucket, run).await {
        // The reader's Stop already took effect — the model is no longer
        // running. Losing the status stamp costs the run its "resumable"
        // label, which the next load heals from the workspace pointer.
        tracing::warn!(
            run_id = %run.run.run_id,
            error = %error,
            "could not stamp the stopped drafting run"
        );
    }
}

/// Re-read the workspace and, if it still points at `run_id`, release it.
/// Returns the workspace as it now stands either way.
async fn clear_active_run(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
) -> Result<LoadedWorkspace, ReportPipelineError> {
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    if loaded.workspace.active_run_id != Some(run_id) {
        return Ok(loaded);
    }
    loaded.workspace.active_run_id = None;
    loaded.workspace.updated_at = jiff::Timestamp::now();
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded)
}

/// Stamp a run whose revision cut succeeded but whose own completion write was
/// lost — the workspace is saved first, so that gap is the one crash window the
/// two-object protocol leaves open. The evidence is entirely in the workspace:
/// it has moved past the revision the run built on and no longer points at it.
pub(crate) fn heal_run(run: &mut DraftRun, workspace: &ReportWorkspace) -> bool {
    let unfinished = run.finalized_revision.is_none()
        && matches!(
            run.status,
            DraftRunStatus::Drafting | DraftRunStatus::Stopped
        );
    if !unfinished
        || workspace.active_run_id == Some(run.run_id)
        || workspace.draft.revision <= run.base_revision
    {
        return false;
    }
    run.status = DraftRunStatus::Completed;
    run.finalized_revision = Some(workspace.draft.revision);
    true
}

/// Stamp every section this run authored with the run that wrote it.
///
/// Skipped and untouched sections keep whatever stamp they already carried:
/// the run did not write them, so claiming it did would misattribute the
/// template's own text.
pub(crate) fn stamp_run_authorship(
    content: &mut ReportContent,
    run: &DraftRun,
    revision: u64,
    now: jiff::Timestamp,
) {
    for section in &mut content.sections {
        let drafted = run.sections.iter().any(|candidate| {
            candidate.section_id == section.id && candidate.state == RunSectionState::Drafted
        });
        if drafted {
            section.authorship = Some(SectionAuthorship {
                kind: AuthorshipKind::ModelGenerated,
                revision,
                model_id: Some(run.writer_model_id.clone()),
                run_id: Some(run.run_id),
                updated_at: now,
            });
        }
    }
}

/// Finish an interrupted whole-report run without losing the sections that
/// already landed.
///
/// The model conversation starts fresh, over the same frozen record corpus,
/// template structure, and plan a first attempt sends — so the cached prefix
/// the interrupted attempt paid for is still there to read. What changes is
/// the kick-off block below the checkpoints: it carries the run's durable
/// per-section state, the instructions typed at the resume, and a template
/// copy for every section the plan wants rewritten. The finisher only demands
/// decisions for sections the run has not already decided, and every
/// already-drafted section keeps its staged blocks unless the writer writes
/// over them.
// The client/report/run identity plus the model and the request is the same
// explicit orchestration boundary the other entry points keep.
#[allow(clippy::too_many_arguments)]
pub async fn resume_draft_run(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullReportGenerationOutcome, ReportPipelineError> {
    let guidance = request.guidance.trim();
    if guidance.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportPipelineError::InvalidInput(format!(
            "The full-draft guidance exceeds {MAX_INSTRUCTION_CHARACTERS} characters."
        )));
    }
    validate_model_choice(model_id)?;

    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut run = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;
    if heal_run(&mut run.run, &loaded.workspace) {
        save_draft_run(s3, bucket, &mut run).await?;
    }
    if !matches!(
        run.run.status,
        DraftRunStatus::Failed | DraftRunStatus::Stopped
    ) {
        return Err(ReportPipelineError::InvalidInput(
            "Only an interrupted whole-report draft can be picked back up.".to_string(),
        ));
    }
    if loaded.workspace.draft.revision != run.run.base_revision {
        return Err(ReportPipelineError::InvalidInput(
            "The report has changed since this whole-report draft was interrupted, so its saved sections no longer fit. Start a new whole-report draft."
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
    run.run.demote_interrupted_sections(now);
    if !guidance.is_empty() {
        run.run.instructions.push(RunInstruction {
            text: guidance.to_string(),
            added_at: now,
        });
    }
    run.run.status = DraftRunStatus::Drafting;
    save_draft_run(s3, bucket, &mut run).await?;
    loaded.workspace.active_run_id = Some(run_id);
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;

    execute_full_draft_turn(
        sdk_config,
        s3,
        bucket,
        loaded,
        model_id,
        request,
        run,
        DraftTurnKind::Resume,
    )
    .await
}

/// The drafting run a Writing session should hydrate from, if any.
///
/// The workspace's own pointer wins whenever it is set: that run owns the
/// session right now, so nothing else is a candidate. Otherwise the newest
/// interrupted run counts — and only while the report still stands exactly
/// where the run left it. A run whose `base_revision` has moved on can be
/// neither resumed nor finalized, so offering it would buy the user a refusal
/// instead of a choice.
pub async fn load_resumable_draft_run(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<Option<DraftRun>, ReportPipelineError> {
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    if let Some(run_id) = loaded.workspace.active_run_id {
        let run = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;
        return Ok(Some(run.run));
    }
    let revision = loaded.workspace.draft.revision;
    Ok(list_draft_runs(s3, bucket, client_id, report_id)
        .await?
        .into_iter()
        .find(|run| {
            matches!(run.status, DraftRunStatus::Stopped | DraftRunStatus::Failed)
                && run.base_revision == revision
        }))
}

/// What [`finalize_partial_draft`] changed, so the caller can record it
/// without re-deriving the counts from the saved workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct PartialDraftFinalization {
    pub workspace: ReportWorkspace,
    pub drafted_sections: u32,
    pub skipped_sections: u32,
    pub revision: u64,
}

/// Cut a revision from what an interrupted run actually landed, instead of
/// picking it back up.
///
/// Every section the run never finished becomes a skipped placeholder — the
/// same shape a deliberate `skip_full_draft_section` produces, template copy
/// and all — so the document is whole, the preview greys out what was not
/// written, and the export elides it. The run is stamped `partial` against the
/// revision it produced. No model is called: this is store reads, the finish
/// cut's own assembly, and two writes.
pub async fn finalize_partial_draft(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
) -> Result<PartialDraftFinalization, ReportPipelineError> {
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut run = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;
    ensure_finalizable(&run.run, &loaded.workspace, run_id)?;

    let now = jiff::Timestamp::now();
    run.run.demote_interrupted_sections(now);
    let drafted = written_sections(&run.run);
    if drafted.is_empty() {
        return Err(ReportPipelineError::InvalidInput(
            "This whole-report draft has no finished sections to keep. Pick it back up, or discard it."
                .to_string(),
        ));
    }
    // Everything the run left undecided is deferred, not lost: the model never
    // replaced that text, and a skipped placeholder is how the report says so.
    let mut skipped = HashSet::new();
    let mut kept = HashSet::new();
    for section in &mut run.run.sections {
        match section.state {
            RunSectionState::Pending | RunSectionState::Failed => {
                section.state = RunSectionState::Skipped;
                section.blocks.clear();
                section.updated_at = now;
                skipped.insert(section.section_id);
            }
            RunSectionState::Skipped => {
                skipped.insert(section.section_id);
            }
            RunSectionState::Kept => {
                kept.insert(section.section_id);
            }
            RunSectionState::Drafted | RunSectionState::Drafting | RunSectionState::Flagged => {}
        }
    }

    let content = ReportContent {
        title: run
            .run
            .title
            .clone()
            .unwrap_or_else(|| loaded.workspace.draft.content.title.clone()),
        sections: merge_undrafted_sections(
            &drafted,
            &skipped,
            &kept,
            &loaded.workspace.draft.content.sections,
        ),
    };
    let expected_revision = loaded.workspace.draft.revision;
    loaded.workspace.draft = loaded
        .workspace
        .draft
        .replace_content(expected_revision, content, now)
        .map_err(|error| ReportPipelineError::InvalidRun(error.to_string()))?;
    let revision = loaded.workspace.draft.revision;
    stamp_run_authorship(&mut loaded.workspace.draft.content, &run.run, revision, now);
    loaded.workspace.active_run_id = None;
    loaded.workspace.session.last_agent_revision = Some(revision);
    mark_template_current(&mut loaded.workspace);
    loaded.workspace.updated_at = now;
    // Workspace first, run second — the same ordering the finish cut uses, so
    // a lost run write heals from the workspace rather than losing a revision.
    save_loaded(s3, bucket, &mut loaded).await?;

    run.run.status = DraftRunStatus::Completed;
    run.run.finalized_revision = Some(revision);
    run.run.partial = true;
    if let Err(error) = save_draft_run(s3, bucket, &mut run).await {
        tracing::warn!(
            run_id = %run_id,
            error = %error,
            "the drafting run was finalized but could not record its own finish"
        );
    }

    Ok(PartialDraftFinalization {
        drafted_sections: u32::try_from(drafted.len()).unwrap_or(u32::MAX),
        skipped_sections: u32::try_from(skipped.len()).unwrap_or(u32::MAX),
        revision,
        workspace: loaded.workspace,
    })
}

fn ensure_finalizable(
    run: &DraftRun,
    workspace: &ReportWorkspace,
    run_id: Uuid,
) -> Result<(), ReportPipelineError> {
    if !matches!(run.status, DraftRunStatus::Failed | DraftRunStatus::Stopped) {
        return Err(ReportPipelineError::InvalidInput(
            "Only an interrupted whole-report draft can be finalized from what it wrote."
                .to_string(),
        ));
    }
    if workspace.draft.revision != run.base_revision {
        return Err(ReportPipelineError::InvalidInput(
            "The report has changed since this whole-report draft was interrupted, so its saved sections no longer fit. Start a new whole-report draft."
                .to_string(),
        ));
    }
    if workspace.session.pending_proposal.is_some() {
        return Err(ReportPipelineError::InvalidInput(
            "Accept or reject the pending proposal before starting another writer action."
                .to_string(),
        ));
    }
    if workspace
        .active_run_id
        .is_some_and(|active| active != run_id)
    {
        return Err(ReportPipelineError::InvalidInput(
            claria_report_store::ACTIVE_RUN_MESSAGE.to_string(),
        ));
    }
    Ok(())
}

/// Give up on a drafting run without touching the report.
///
/// The sections it landed stay in the run object as history — abandoning is
/// about releasing the session, not erasing what was written — and the
/// workspace pointer is cleared only if it still names this run.
pub async fn abandon_draft_run(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
) -> Result<ReportWorkspace, ReportPipelineError> {
    let mut run = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;
    if run.run.status == DraftRunStatus::Completed {
        return Err(ReportPipelineError::InvalidInput(
            "This whole-report draft already produced a saved revision, so it cannot be discarded. Revert the report instead."
                .to_string(),
        ));
    }
    run.run.status = DraftRunStatus::Abandoned;
    save_draft_run(s3, bucket, &mut run).await?;
    let loaded = clear_active_run(s3, bucket, client_id, report_id, run_id).await?;
    Ok(loaded.workspace)
}
