//! Driving the writer pipeline headlessly.
//!
//! These are the desktop's `plan.rs` command bodies with the Tauri state, the
//! IPC progress channel, and the audit writes taken out, and a
//! [`ProgressRecorder`](crate::progress::ProgressRecorder) put in their place.
//! Model IDs arrive already resolved so a test can drive the pipeline without
//! reaching Bedrock's model-listing API.

use claria::{CompletionReport, FullReportGenerationOutcome};
use claria_core::models::{report::ReportWorkspace, report_run::DraftRun, turn_usage::TurnUsage};
use eyre::{Context, Result, eyre};
use uuid::Uuid;

use crate::{EvalContext, cost::RunCost, progress::ProgressRecorder};

/// One client and how many record files it has.
#[derive(Debug, Clone)]
pub struct ClientListing {
    pub client_id: Uuid,
    pub name: String,
    pub record_count: usize,
    pub workspace_count: usize,
}

/// Everything a `plan` pass produced.
#[derive(Debug)]
pub struct PlanReport {
    pub run: DraftRun,
    pub usage: Option<TurnUsage>,
    pub converse_calls: u32,
    pub cost: RunCost,
}

/// Everything a `run` produced: the plan, then the draft, then the gate.
#[derive(Debug)]
pub struct DraftReport {
    pub plan: PlanReport,
    pub outcome: FullReportGenerationOutcome,
    pub completion: CompletionReport,
    /// Plan and draft costs summed.
    pub cost: RunCost,
}

/// Every client in the bucket with its record and Writing-session counts.
///
/// Reads only — no Bedrock, so it costs no attempt.
pub async fn list_clients(context: &EvalContext) -> Result<Vec<ClientListing>> {
    let cache = claria_records::RecordCache::new();
    let summaries = claria_records::list_client_summaries(&context.s3, &context.bucket, &cache)
        .await
        .wrap_err("could not list clients")?;

    let mut listings = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let client_id: Uuid = summary
            .id
            .parse()
            .wrap_err_with(|| format!("client key {} is not a UUID", summary.id))?;
        let records = claria_records::record_inventory(&context.s3, &context.bucket, client_id)
            .await
            .wrap_err("could not list a client's records")?;
        let workspaces =
            claria_report_store::list_report_workspaces(&context.s3, &context.bucket, client_id)
                .await
                .wrap_err("could not list a client's Writing sessions")?;
        listings.push(ClientListing {
            client_id,
            name: summary.name,
            record_count: records.len(),
            workspace_count: workspaces.len(),
        });
    }
    listings.sort_by_key(|listing| std::cmp::Reverse(listing.record_count));
    Ok(listings)
}

/// Which report a pass runs against.
#[derive(Debug, Clone, Copy)]
pub enum WorkspaceChoice {
    /// Reuse the client's most recently updated Writing session that has
    /// sections in it.
    Existing,
    /// Start a fresh Writing session and apply this managed writer template
    /// to it.
    FreshFromTemplate(Uuid),
}

/// Resolve the report a pass will plan against.
pub async fn prepare_workspace(
    context: &EvalContext,
    client_id: Uuid,
    choice: WorkspaceChoice,
) -> Result<ReportWorkspace> {
    match choice {
        WorkspaceChoice::Existing => {
            let mut workspaces = claria_report_store::list_report_workspaces(
                &context.s3,
                &context.bucket,
                client_id,
            )
            .await
            .wrap_err("could not list the client's Writing sessions")?;
            workspaces.retain(|workspace| !workspace.draft.content.sections.is_empty());
            workspaces.sort_by_key(|workspace| workspace.updated_at);
            workspaces.pop().ok_or_else(|| {
                eyre!(
                    "client {client_id} has no Writing session with sections in it. \
                     Pass --template <uuid> to start one from a managed writer template."
                )
            })
        }
        WorkspaceChoice::FreshFromTemplate(template_id) => {
            apply_template(context, client_id, template_id).await
        }
    }
}

/// Start a fresh Writing session and apply a managed writer template to it.
///
/// The same three steps the desktop takes: parse the managed DOCX, keep the
/// redacted source alongside the workspace so export can preserve Word
/// formatting, then apply the parsed content as revision 1.
async fn apply_template(
    context: &EvalContext,
    client_id: Uuid,
    template_id: Uuid,
) -> Result<ReportWorkspace> {
    let (metadata, bytes) = claria_report_store::template_library::load_docx_with_metadata(
        &context.s3,
        &context.bucket,
        template_id,
        claria_docx::MAX_TEMPLATE_DOCX_BYTES,
    )
    .await
    .wrap_err("could not load the writer template")?;
    let (imported, source) = tokio::task::spawn_blocking(move || {
        claria_docx::import_template(&bytes).map(|imported| (imported, bytes))
    })
    .await
    .map_err(|_| eyre!("the writer template could not be inspected safely"))?
    .wrap_err("could not parse the writer template")?;

    let workspace = claria_report_store::start_report_workspace_with_id(
        &context.s3,
        &context.bucket,
        client_id,
        Uuid::new_v4(),
    )
    .await
    .wrap_err("could not start a Writing session")?;

    claria_report_store::store_report_template_source(
        &context.s3,
        &context.bucket,
        client_id,
        &imported.source_sha256,
        source,
    )
    .await
    .wrap_err("could not store the template source")?;

    claria_report_store::apply_report_template_for_report(
        &context.s3,
        &context.bucket,
        client_id,
        workspace.report_id,
        workspace.draft.revision,
        claria_report_store::ReportTemplateApplication {
            content: imported.content,
            source_sha256: imported.source_sha256,
            writer_template_id: template_id,
            writer_template_name: metadata.name,
            warnings: imported.warnings,
        },
    )
    .await
    .wrap_err("could not apply the writer template")
}

/// Run one planning pass and leave the plan at the gate.
///
/// Consumes one governor attempt; the caller claims it before calling.
pub async fn plan(
    context: &EvalContext,
    workspace: &ReportWorkspace,
    planner_model_id: &str,
    writer_model_id: &str,
    instructions: &str,
    progress: &ProgressRecorder,
) -> Result<PlanReport> {
    let sink = {
        let progress = progress.clone();
        move |event| progress.record(event)
    };
    let outcome = claria::generate_draft_plan(
        &context.sdk_config,
        &context.s3,
        &context.bucket,
        workspace.client_id,
        workspace.report_id,
        workspace.draft.revision,
        claria::PlanModels {
            planner_model_id,
            writer_model_id,
        },
        claria::DraftPlanRequest::new(instructions).with_progress(&sink),
    )
    .await
    .wrap_err("the planning pass failed")?;

    let cost = crate::cost::total(outcome.usage.iter());
    Ok(PlanReport {
        run: outcome.run,
        usage: outcome.usage,
        converse_calls: outcome.converse_calls,
        cost,
    })
}

/// Approve a plan waiting at the gate and draft the report it describes, then
/// ask the completion gate what is still missing.
///
/// "Auto-approve" is the whole gate step: the plan is taken exactly as the
/// planner produced it, with no `update_draft_plan` edit in between.
pub async fn draft(
    context: &EvalContext,
    workspace: &ReportWorkspace,
    run_id: Uuid,
    writer_model_id: &str,
    plan_report: PlanReport,
    progress: &ProgressRecorder,
) -> Result<DraftReport> {
    let sink = {
        let progress = progress.clone();
        move |event| progress.record(event)
    };
    let cache = claria::ReportPromptCache::new();
    let prompt_body = load_full_draft_prompt(context).await?;
    let limits =
        claria::ReportTurnLimits::default().scaled_for_plan(plan_report.run.sections.len());

    let outcome = claria::start_draft_run(
        &context.sdk_config,
        &context.s3,
        &context.bucket,
        workspace.client_id,
        workspace.report_id,
        run_id,
        writer_model_id,
        claria::FullReportRequest::new("")
            .with_limits(limits)
            .with_progress(&sink)
            .with_prompt_cache(&cache)
            .with_system_prompt_body(&prompt_body),
    )
    .await
    .wrap_err("the drafting run failed")?;

    let completion = claria::evaluate_report_completion(
        &context.s3,
        &context.bucket,
        workspace.client_id,
        workspace.report_id,
    )
    .await
    .wrap_err("the completion gate could not be evaluated")?;

    let mut cost = plan_report.cost;
    cost.add(&outcome.attempt.usage);
    Ok(DraftReport {
        plan: plan_report,
        outcome,
        completion,
        cost,
    })
}

/// The clinician's edited whole-report prompt body, or the shipped default.
async fn load_full_draft_prompt(context: &EvalContext) -> Result<String> {
    match claria_storage::objects::get_object(
        &context.s3,
        &context.bucket,
        claria_core::s3_keys::FULL_REPORT_SYSTEM_PROMPT,
    )
    .await
    {
        Ok(output) => String::from_utf8(output.body)
            .wrap_err("the saved whole-report prompt is not valid UTF-8"),
        Err(claria_storage::error::StorageError::NotFound { .. }) => {
            Ok(claria::FULL_REPORT_SYSTEM_PROMPT_BODY.to_string())
        }
        Err(error) => Err(error).wrap_err("could not read the saved whole-report prompt"),
    }
}
