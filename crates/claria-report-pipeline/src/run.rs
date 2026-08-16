//! The durable drafting run behind whole-report generation: creating it,
//! releasing it when a turn fails, healing one whose completion write was lost,
//! and re-entering the turn loop to finish an interrupted one.

use aws_sdk_s3::Client as S3Client;
use claria_core::models::{
    report::{AuthorshipKind, ReportContent, ReportWorkspace, SectionAuthorship},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, PlanEntry, RunInstruction, RunPlan,
        RunSection, RunSectionState, SectionIntent,
    },
};
use claria_report_store::{
    LoadedRun, LoadedWorkspace, MAX_INSTRUCTION_CHARACTERS, create_draft_run, load_draft_run,
    load_for_report, save_draft_run, save_loaded,
};
use uuid::Uuid;

use crate::{
    FullReportGenerationOutcome, ReportPipelineError,
    full_draft_context::DraftTurnKind,
    turn::{FullReportRequest, execute_full_draft_turn, validate_model_choice},
};

/// Stand-in for the planner pass that lands in a later change: one `Draft`
/// entry per section the report already has, in document order.
///
/// It exists so the run's plan is the single authority on which sections the
/// finisher demands a decision about, well before a model produces one.
fn synthetic_plan(
    workspace: &ReportWorkspace,
    model_id: &str,
    now: jiff::Timestamp,
) -> Option<RunPlan> {
    Some(RunPlan {
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
        created_at: now,
    })
}

fn new_draft_run(
    workspace: &ReportWorkspace,
    model_id: &str,
    guidance: &str,
    now: jiff::Timestamp,
) -> DraftRun {
    DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id: Uuid::new_v4(),
        report_id: workspace.report_id,
        client_id: workspace.client_id,
        base_revision: workspace.draft.revision,
        status: DraftRunStatus::Drafting,
        plan: synthetic_plan(workspace, model_id, now),
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

/// Create the run this whole-report turn writes into and hand the workspace to
/// it. The pointer is saved before the first model call, so a second window
/// that opens mid-run is refused rather than cutting a revision underneath it.
pub(crate) async fn start_draft_run(
    s3: &S3Client,
    bucket: &str,
    loaded: &mut LoadedWorkspace,
    model_id: &str,
    guidance: &str,
) -> Result<LoadedRun, ReportPipelineError> {
    let now = jiff::Timestamp::now();
    let run = create_draft_run(
        s3,
        bucket,
        new_draft_run(&loaded.workspace, model_id, guidance, now),
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

async fn clear_active_run(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
) -> Result<(), ReportPipelineError> {
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    if loaded.workspace.active_run_id != Some(run_id) {
        return Ok(());
    }
    loaded.workspace.active_run_id = None;
    loaded.workspace.updated_at = jiff::Timestamp::now();
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(())
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
