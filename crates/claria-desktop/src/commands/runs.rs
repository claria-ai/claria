//! Drafting-run lifecycle: hydrating an interrupted run, cutting a revision
//! from what it landed, and discarding it.
//!
//! `DraftRun` crosses IPC as itself — it derives `specta::Type` in
//! `claria-core` and a mirror view would differ from it in nothing.

use tauri::State;

use claria_storage::audit::actions;

use claria_core::models::report_run::DraftRun;
use claria_desktop::report_authoring::{
    DraftRunHistoryView, ReportWorkspaceView, draft_run_history_view,
};

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
        Ok(claria::load_resumable_draft_run(
            &ctx.s3,
            &ctx.bucket,
            parse_uuid(&client_id)?,
            parse_uuid(&report_id)?,
        )
        .await?)
    })
    .await
}

/// Every drafting run this report has recorded, newest first.
///
/// The read-only counterpart to `load_draft_run`, which answers only "what can
/// be picked back up" and therefore never returns a completed run. This is what
/// the Draft run tab reads: a run that finished is the one that wrote the
/// document on screen, and it is the history the tab exists to show.
#[tauri::command]
#[specta::specta]
pub async fn load_draft_run_history(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
) -> Result<DraftRunHistoryView, String> {
    run("load_draft_run_history", async {
        let ctx = CommandContext::new(&state).await?;
        let history = claria::load_draft_run_history(
            &ctx.s3,
            &ctx.bucket,
            parse_uuid(&client_id)?,
            parse_uuid(&report_id)?,
        )
        .await?;
        Ok(draft_run_history_view(&history))
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
        let outcome = claria::finalize_partial_draft(
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
                actions::REPORT_DRAFT_RUN_FINALIZE_PARTIAL,
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
        let workspace = claria::abandon_draft_run(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            parse_uuid(&report_id)?,
            run_id,
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&workspace);
        ctx.record_audit(
            ctx.audit_event(
                actions::REPORT_DRAFT_RUN_ABANDON,
                "report",
                workspace.report_id.clone(),
            )
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
