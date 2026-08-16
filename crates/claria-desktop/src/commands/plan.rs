//! Planning a whole-report draft, editing the plan, and drafting against it.
//!
//! These four commands are the gated path through the drafting pipeline:
//! plan, edit, start, and — for a run that was interrupted — resume. They sit
//! beside `generate_full_report`, which still runs the un-gated path for the
//! writer surface that has not moved to the gate yet.

use claria_core::models::report_run::{DraftRun, PlanEntryEdit, SectionIntent};
use tauri::State;

use claria_desktop::report_authoring::{FullReportGenerationResponse, ReportTurnProgressView};

use super::{CommandContext, CommandError, merge_details, parse_uuid, run, usage_audit_details};
use crate::state::DesktopState;

/// Resolve the model for one supporting role against what this account can
/// actually reach.
///
/// The discovered list is the same one the model picker is built from, so an
/// override the picker offered is an override this honours, and one the
/// account has since lost falls through to the derived default instead of
/// failing the command with a Bedrock error.
pub(crate) async fn role_model_id(
    ctx: &CommandContext,
    override_id: Option<&str>,
    writer_model_id: &str,
) -> Result<String, CommandError> {
    let discovered: Vec<String> = claria_bedrock::chat::list_chat_models(&ctx.sdk_config)
        .await?
        .into_iter()
        .map(|model| model.model_id)
        .collect();
    Ok(
        claria_core::model_id::resolve_role_model(override_id, &discovered, writer_model_id)
            .to_string(),
    )
}

/// Plan a whole-report draft and leave it at the gate.
///
/// Guarded exactly like a writer turn — no pending proposal, the revision the
/// caller expects, no run already in flight — and the run it creates holds the
/// report for the whole gate window, so nothing can edit the report out from
/// under a plan being reviewed. A plan pass that fails releases that hold.
#[tauri::command]
#[specta::specta]
pub async fn generate_draft_plan(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
    instructions: String,
    on_progress: tauri::ipc::Channel<ReportTurnProgressView>,
) -> Result<DraftRun, String> {
    run("generate_draft_plan", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        // The writing model is only fixed when the clinician presses Start,
        // but the record corpus has to fit its window too, so the pass sizes
        // against their saved preference and the run carries it until then.
        let planner_override = ctx.cfg.draft_pipeline.planner_model_id.clone();
        let preferred = ctx.cfg.preferred_model_id.clone().unwrap_or_default();
        let planner_model_id = role_model_id(&ctx, planner_override.as_deref(), &preferred).await?;
        let writer_model_id = if preferred.is_empty() {
            planner_model_id.clone()
        } else {
            preferred
        };
        let progress = |event: claria_report_pipeline::ReportTurnProgress| {
            let _ = on_progress.send(event.into());
        };
        let outcome = claria_report_pipeline::generate_draft_plan(
            &ctx.sdk_config,
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            expected_revision,
            claria_report_pipeline::PlanModels {
                planner_model_id: &planner_model_id,
                writer_model_id: &writer_model_id,
            },
            claria_report_pipeline::DraftPlanRequest::new(&instructions).with_progress(&progress),
        )
        .await?;
        ctx.record_audit(
            ctx.audit_event("draft_plan_generated", "report", report_id.to_string())
                .with_details(plan_audit_details(&planner_model_id, &outcome)),
        )
        .await;
        Ok(outcome.run)
    })
    .await
}

/// Apply the clinician's edits to a plan waiting at the gate.
#[tauri::command]
#[specta::specta]
pub async fn update_draft_plan(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    run_id: String,
    edits: Vec<PlanEntryEdit>,
) -> Result<DraftRun, String> {
    run("update_draft_plan", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let run_id = parse_uuid(&run_id)?;
        let updated = claria_report_pipeline::update_draft_plan(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            run_id,
            &edits,
        )
        .await?;
        ctx.record_audit(
            ctx.audit_event("draft_plan_edited", "report", report_id.to_string())
                .with_details(serde_json::json!({
                    "client_id": client_id.to_string(),
                    "run_id": run_id.to_string(),
                    "edited_sections": edits.len(),
                    "section_count": updated.sections.len(),
                })),
        )
        .await;
        Ok(updated)
    })
    .await
}

/// Approve the plan and draft the report it describes.
#[tauri::command]
#[specta::specta]
// Tauri command parameters are the typed IPC contract; report identity, the
// model, and the progress channel legitimately take this past clippy's ceiling.
#[allow(clippy::too_many_arguments)]
pub async fn start_draft_run(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    run_id: String,
    model_id: String,
    on_progress: tauri::ipc::Channel<ReportTurnProgressView>,
) -> Result<FullReportGenerationResponse, String> {
    run("start_draft_run", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let run_id = parse_uuid(&run_id)?;
        let limits = ctx.cfg.report_authoring.limits()?;
        let progress = |event: claria_report_pipeline::ReportTurnProgress| {
            let _ = on_progress.send(event.into());
        };
        let prompt_body =
            super::prompts::load_prompt(&ctx.s3, &ctx.bucket, "report-full-draft").await?;
        let outcome = claria_report_pipeline::start_draft_run(
            &ctx.sdk_config,
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            run_id,
            &model_id,
            claria_report_pipeline::FullReportRequest::new("")
                .with_limits(limits)
                .with_progress(&progress)
                .with_prompt_cache(&state.report_prompt_cache)
                .with_system_prompt_body(&prompt_body)
                .with_model_tuning(super::model_tuning_for(&ctx.cfg, &model_id)),
        )
        .await?;
        let attempt = outcome.attempt.clone();
        let response = claria_desktop::report_authoring::full_report_response_view(outcome);
        let mut details = usage_audit_details(&attempt.model_id, Some(&attempt.usage), None);
        merge_details(
            &mut details,
            serde_json::json!({
                "client_id": client_id.to_string(),
                "run_id": run_id.to_string(),
                "writer_model_id": model_id,
                "revision": response.workspace.draft.revision,
                "section_count": response.workspace.draft.content.sections.len(),
                "converse_calls": attempt.converse_calls,
            }),
        );
        ctx.record_audit(
            ctx.audit_event("draft_run_started", "report", report_id.to_string())
                .with_details(details),
        )
        .await;
        Ok(response)
    })
    .await
}

/// Pick an interrupted drafting run back up.
///
/// New instructions get a planning call over the run's durable state — they
/// may change what an already-drafted section should say — and no new
/// instructions is decided in code: keep what landed, draft the rest.
///
/// This resumes directly whatever the plan-gate preference says. The gate is
/// a frontend decision: showing the re-plan before executing it means calling
/// the planning pass on its own first, which the pane does, and there is no
/// second command for "resume without re-planning".
#[tauri::command]
#[specta::specta]
// Tauri command parameters are the typed IPC contract; report identity, the
// model, and the progress channel legitimately take this past clippy's ceiling.
#[allow(clippy::too_many_arguments)]
pub async fn resume_draft_run(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    run_id: String,
    updated_instructions: Option<String>,
    model_id: String,
    on_progress: tauri::ipc::Channel<ReportTurnProgressView>,
) -> Result<FullReportGenerationResponse, String> {
    run("resume_draft_run", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let run_id = parse_uuid(&run_id)?;
        let limits = ctx.cfg.report_authoring.limits()?;
        let instructions = updated_instructions.unwrap_or_default();
        let planner_override = ctx.cfg.draft_pipeline.planner_model_id.clone();
        let planner_model_id = role_model_id(&ctx, planner_override.as_deref(), &model_id).await?;
        let progress = |event: claria_report_pipeline::ReportTurnProgress| {
            let _ = on_progress.send(event.into());
        };
        let prompt_body =
            super::prompts::load_prompt(&ctx.s3, &ctx.bucket, "report-full-draft").await?;
        let outcome = claria_report_pipeline::resume_planned_draft_run(
            &ctx.sdk_config,
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            run_id,
            &planner_model_id,
            &model_id,
            claria_report_pipeline::FullReportRequest::new(&instructions)
                .with_limits(limits)
                .with_progress(&progress)
                .with_prompt_cache(&state.report_prompt_cache)
                .with_system_prompt_body(&prompt_body)
                .with_model_tuning(super::model_tuning_for(&ctx.cfg, &model_id)),
        )
        .await?;
        let attempt = outcome.attempt.clone();
        let response = claria_desktop::report_authoring::full_report_response_view(outcome);
        let mut details = usage_audit_details(&attempt.model_id, Some(&attempt.usage), None);
        merge_details(
            &mut details,
            serde_json::json!({
                "client_id": client_id.to_string(),
                "run_id": run_id.to_string(),
                "writer_model_id": model_id,
                "planner_model_id": planner_model_id,
                "re_planned": !instructions.trim().is_empty(),
                "revision": response.workspace.draft.revision,
                "converse_calls": attempt.converse_calls,
            }),
        );
        ctx.record_audit(
            ctx.audit_event("draft_run_resumed", "report", report_id.to_string())
                .with_details(details),
        )
        .await;
        Ok(response)
    })
    .await
}

/// Counts and model IDs for the planning audit event — never a scope, a
/// heading, or an evidence quote.
fn plan_audit_details(
    planner_model_id: &str,
    outcome: &claria_report_pipeline::DraftPlanOutcome,
) -> serde_json::Value {
    let plan = outcome.run.plan.as_ref();
    let intent_count = |wanted: SectionIntent| {
        plan.map_or(0, |plan| {
            plan.entries
                .iter()
                .filter(|entry| entry.intent == wanted)
                .count()
        })
    };
    let mut details = usage_audit_details(planner_model_id, outcome.usage.as_ref(), None);
    merge_details(
        &mut details,
        serde_json::json!({
            "client_id": outcome.run.client_id.to_string(),
            "run_id": outcome.run.run_id.to_string(),
            "planner_model_id": planner_model_id,
            "section_count": outcome.run.sections.len(),
            "planned_draft_sections": intent_count(SectionIntent::Draft),
            "planned_skip_sections": intent_count(SectionIntent::Skip),
            "plan_warnings": plan.map_or(0, |plan| plan.plan_warnings.len()),
            "converse_calls": outcome.converse_calls,
        }),
    );
    details
}
