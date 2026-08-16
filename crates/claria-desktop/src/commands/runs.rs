//! Drafting-run lifecycle: hydrating an interrupted run, cutting a revision
//! from what it landed, and discarding it.
//!
//! `DraftRun` crosses IPC as itself — it derives `specta::Type` in
//! `claria-core` and a mirror view would differ from it in nothing.

use tauri::State;

use claria_core::models::report_run::DraftRun;
use claria_desktop::report_authoring::ReportWorkspaceView;

use super::{CommandContext, parse_uuid, run};
use crate::state::DesktopState;

/// The drafting run the Writing surface should reattach to, or `null` when
/// there is nothing resumable for this report.
#[tauri::command]
#[specta::specta]
pub async fn load_draft_run(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
) -> Result<Option<DraftRun>, String> {
    run("load_draft_run", async {
        let ctx = CommandContext::new(&state).await?;
        Ok(claria_report_pipeline::load_resumable_draft_run(
            &ctx.s3,
            &ctx.bucket,
            parse_uuid(&client_id)?,
            parse_uuid(&report_id)?,
        )
        .await?)
    })
    .await
}

/// Keep what an interrupted run wrote: undone sections become skipped
/// placeholders and the result is saved as a new revision.
#[tauri::command]
#[specta::specta]
pub async fn finalize_partial_draft(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    run_id: String,
) -> Result<ReportWorkspaceView, String> {
    run("finalize_partial_draft", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let run_id = parse_uuid(&run_id)?;
        let outcome = claria_report_pipeline::finalize_partial_draft(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            parse_uuid(&report_id)?,
            run_id,
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&outcome.workspace);
        ctx.record_audit(
            ctx.audit_event(
                "draft_run_finalized_partial",
                "report",
                workspace.report_id.clone(),
            )
            .with_details(serde_json::json!({
                "client_id": client_id.to_string(),
                "run_id": run_id.to_string(),
                "drafted_section_count": outcome.drafted_sections,
                "skipped_section_count": outcome.skipped_sections,
                "revision": outcome.revision,
            })),
        )
        .await;
        Ok(workspace)
    })
    .await
}

/// Discard a drafting run. The report is untouched; the sections the run
/// landed stay in the run object as history.
#[tauri::command]
#[specta::specta]
pub async fn abandon_draft_run(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    run_id: String,
) -> Result<ReportWorkspaceView, String> {
    run("abandon_draft_run", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let run_id = parse_uuid(&run_id)?;
        let workspace = claria_report_pipeline::abandon_draft_run(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            parse_uuid(&report_id)?,
            run_id,
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&workspace);
        ctx.record_audit(
            ctx.audit_event("draft_run_abandoned", "report", workspace.report_id.clone())
                .with_details(serde_json::json!({
                    "client_id": client_id.to_string(),
                    "run_id": run_id.to_string(),
                })),
        )
        .await;
        Ok(workspace)
    })
    .await
}
