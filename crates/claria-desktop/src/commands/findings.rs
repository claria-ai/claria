//! Review findings — listing them and resolving one.

use claria_core::models::findings::{FindingAction, ReportFindings};
use tauri::State;

pub use claria_desktop::report_authoring::ReportFindingResolution;

use super::{CommandContext, parse_uuid, run};
use crate::state::DesktopState;

/// The durable audit action one resolution records.
const fn audit_action(action: FindingAction) -> &'static str {
    match action {
        FindingAction::ApplyStyle => "finding_applied",
        FindingAction::UndoStyle => "finding_undone",
        FindingAction::Dismiss => "finding_dismissed",
    }
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
