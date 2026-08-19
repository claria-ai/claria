//! Review findings — producing them, listing them, and resolving one.

use claria_core::models::findings::{FindingAction, ReportFindings};
use tauri::State;

use claria_storage::audit::{Action, actions};

pub use claria_desktop::report_authoring::ReportFindingResolution;
use claria_desktop::report_authoring::ReportTurnProgressView;

use super::{CommandContext, merge_details, parse_uuid, run, usage_audit_details};
use crate::state::DesktopState;

/// The durable audit action one resolution records.
const fn audit_action(action: FindingAction) -> Action {
    match action {
        FindingAction::ApplyStyle => actions::REPORT_FINDING_APPLY,
        FindingAction::UndoStyle => actions::REPORT_FINDING_UNDO,
        FindingAction::Dismiss => actions::REPORT_FINDING_DISMISS,
    }
}

/// Review one accepted revision for every property and save what comes back.
///
/// One request per property runs in parallel against the reviewing model, so
/// the command holds its future for the length of the slowest branch. A
/// property whose branch fails leaves no coverage row: the returned findings
/// say which properties were actually read, and the audit event names the
/// ones that were not.
#[tauri::command]
#[specta::specta]
pub async fn run_review_sweeps(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    revision: u64,
    on_progress: tauri::ipc::Channel<ReportTurnProgressView>,
) -> Result<ReportFindings, String> {
    run("run_review_sweeps", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let reviewer_override = ctx.cfg.draft_pipeline.reviewer_model_id.clone();
        let preferred = ctx.cfg.preferred_model_id.clone().unwrap_or_default();
        let reviewer_model_id =
            super::plan::role_model_id(&ctx, reviewer_override.as_deref(), &preferred).await?;
        let progress = |event: claria_report_pipeline::ReportTurnProgress| {
            let _ = on_progress.send(event.into());
        };
        let outcome = claria_report_pipeline::run_review_sweeps(
            &ctx.sdk_config,
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            revision,
            &reviewer_model_id,
            claria_report_pipeline::ReviewSweepRequest::new()
                .with_progress(&progress)
                .with_stream_bounds(ctx.cfg.report_authoring.runtime().analysis_stream_bounds()),
        )
        .await?;
        let mut details = usage_audit_details(&reviewer_model_id, outcome.usage.as_ref(), None);
        merge_details(
            &mut details,
            serde_json::json!({
                "client_id": client_id.to_string(),
                "reviewer_model_id": reviewer_model_id,
                "revision": revision,
                "findings_by_property": outcome.findings_by_property(),
                "failed_properties": outcome.failed_properties(),
                "converse_calls": outcome.converse_calls,
            }),
        );
        ctx.record_audit(
            ctx.audit_event(
                actions::REPORT_REVIEW_SWEEP,
                "report",
                report_id.to_string(),
            )
            .with_details(details),
        )
        .await;
        Ok(outcome.findings)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_report_findings(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
) -> Result<ReportFindings, String> {
    run("list_report_findings", async {
        let ctx = CommandContext::new(&state).await?;
        Ok(claria_report_store::list_report_findings(
            &ctx.s3,
            &ctx.bucket,
            parse_uuid(&client_id)?,
            parse_uuid(&report_id)?,
        )
        .await?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_report_finding(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    finding_id: String,
    action: FindingAction,
) -> Result<ReportFindingResolution, String> {
    run("resolve_report_finding", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let finding_id = parse_uuid(&finding_id)?;
        let outcome = claria_report_store::resolve_finding(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            parse_uuid(&report_id)?,
            finding_id,
            action,
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&outcome.workspace);
        ctx.record_audit(
            ctx.audit_event(audit_action(action), "report", workspace.report_id.clone())
                .with_details(serde_json::json!({
                    "client_id": client_id.to_string(),
                    "finding_id": finding_id.to_string(),
                    "revision": workspace.draft.revision,
                })),
        )
        .await;
        Ok(ReportFindingResolution {
            workspace,
            findings: outcome.findings,
        })
    })
    .await
}
