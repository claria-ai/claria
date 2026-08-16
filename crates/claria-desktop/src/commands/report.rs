//! Writing — the separate opt-in report workflow commands.

use tauri::State;

pub use claria_desktop::report_authoring::{
    EditorHistoryEntry, FullReportGenerationResponse, ReportBlockReferenceInput, ReportDraftEdit,
    ReportExportResult, ReportProposalChoice, ReportRevisionView, ReportTurnProgressView,
    ReportTurnResponse, ReportWorkspaceView, TemplateExportWarning,
};

use claria_core::models::report::{ReportDraft, ReportExportStatus};

use super::{CommandContext, merge_details, parse_uuid, run, usage_audit_details};
use crate::state::DesktopState;

#[tauri::command]
#[specta::specta]
pub async fn start_report_workspace(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
) -> Result<ReportWorkspaceView, String> {
    run("start_report_workspace", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let workspace = claria_report_store::start_report_workspace_with_id(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
        )
        .await?;
        Ok(claria_desktop::report_authoring::workspace_view(&workspace))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn load_report_workspace(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
) -> Result<ReportWorkspaceView, String> {
    run("load_report_workspace", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let workspace = claria_report_store::load_report_workspace_by_id(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            parse_uuid(&report_id)?,
        )
        .await?;
        Ok(claria_desktop::report_authoring::workspace_view(&workspace))
    })
    .await
}

/// Return every persisted Writing session for the Record screen's Editor
/// History folder without creating a new workspace.
#[tauri::command]
#[specta::specta]
pub async fn list_editor_history(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<EditorHistoryEntry>, String> {
    run("list_editor_history", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        Ok(
            claria_report_store::list_report_workspaces(&ctx.s3, &ctx.bucket, client_id)
                .await?
                .iter()
                .filter(|workspace| {
                    workspace.draft.revision > 0
                        || !workspace.session.turns.is_empty()
                        || workspace.template_import.is_some()
                })
                .map(claria_desktop::report_authoring::editor_history_entry)
                .collect(),
        )
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_report_session(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    name: String,
) -> Result<ReportWorkspaceView, String> {
    run("rename_report_session", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let workspace = claria_report_store::rename_report_session(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            &name,
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

        ctx.record_audit(
            ctx.audit_event(
                "report_session_renamed",
                "report",
                workspace.report_id.clone(),
            )
            .with_details(serde_json::json!({ "client_id": client_id.to_string() })),
        )
        .await;

        Ok(workspace)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_report_revisions(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
) -> Result<Vec<ReportRevisionView>, String> {
    run("list_report_revisions", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let revisions = claria_report_store::list_report_revisions(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            &state.revision_cache,
        )
        .await?;
        Ok(revisions
            .into_iter()
            .map(|revision| ReportRevisionView {
                revision: revision.revision,
                title: revision.title,
                updated_at: revision.updated_at.to_string(),
            })
            .collect())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn load_report_revision(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    revision: u64,
) -> Result<ReportDraft, String> {
    run("load_report_revision", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let draft = claria_report_store::load_report_revision(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            revision,
            &state.revision_cache,
        )
        .await?;
        Ok(draft)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn revert_report_revision(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
    revision: u64,
) -> Result<ReportWorkspaceView, String> {
    run("revert_report_revision", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let workspace = claria_report_store::revert_report_revision(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            expected_revision,
            revision,
            &state.revision_cache,
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

        ctx.record_audit(
            ctx.audit_event(
                "report_revision_restored",
                "report",
                workspace.report_id.clone(),
            )
            .with_details(serde_json::json!({
                "client_id": client_id.to_string(),
                "source_revision": revision,
                "new_revision": workspace.draft.revision
            })),
        )
        .await;

        Ok(workspace)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn save_report_draft(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
    draft: ReportDraftEdit,
) -> Result<ReportWorkspaceView, String> {
    run("save_report_draft", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let content = claria_desktop::report_authoring::content_from_edit(draft)?;
        let workspace = claria_report_store::save_report_draft_for_report(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            expected_revision,
            content,
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

        ctx.record_audit(
            ctx.audit_event("report_draft_saved", "report", workspace.report_id.clone())
                .with_details(serde_json::json!({
                    "client_id": client_id.to_string(),
                    "report_id": workspace.report_id,
                    "revision": workspace.draft.revision,
                    "section_count": workspace.draft.content.sections.len()
                })),
        )
        .await;

        Ok(workspace)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn discard_queued_report_edits(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
) -> Result<ReportWorkspaceView, String> {
    run("discard_queued_report_edits", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let workspace = claria_report_store::discard_queued_report_edits(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            expected_revision,
            &state.revision_cache,
        )
        .await?;
        state.report_prompt_cache.invalidate(report_id);
        let workspace = claria_desktop::report_authoring::workspace_view(&workspace);
        ctx.record_audit(
            ctx.audit_event(
                "report_queued_edits_discarded",
                "report",
                workspace.report_id.clone(),
            )
            .with_details(serde_json::json!({
                "client_id": client_id.to_string(),
                "revision": workspace.draft.revision,
            })),
        )
        .await;
        Ok(workspace)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn generate_full_report(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
    model_id: String,
    guidance: String,
    on_progress: tauri::ipc::Channel<ReportTurnProgressView>,
) -> Result<FullReportGenerationResponse, String> {
    run("generate_full_report", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let limits = ctx.cfg.report_authoring.limits()?;
        tracing::info!(
            client_id = %client_id,
            report_id = %report_id,
            expected_revision,
            model_id,
            "whole-report generation requested"
        );
        let progress = |event: claria_report_pipeline::ReportTurnProgress| {
            let _ = on_progress.send(event.into());
        };
        let prompt_body =
            super::prompts::load_prompt(&ctx.s3, &ctx.bucket, "report-full-draft").await?;
        let result = claria_report_pipeline::generate_full_report_for_report(
            &ctx.sdk_config,
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            expected_revision,
            &model_id,
            claria_report_pipeline::FullReportRequest::new(&guidance)
                .with_limits(limits)
                .with_progress(&progress)
                .with_prompt_cache(&state.report_prompt_cache)
                .with_system_prompt_body(&prompt_body)
                .with_model_tuning(super::model_tuning_for(&ctx.cfg, &model_id)),
        )
        .await;

        match result {
            Ok(outcome) => {
                let attempt = outcome.attempt.clone();
                let record_context = outcome.record_context.clone();
                let response = claria_desktop::report_authoring::full_report_response_view(outcome);
                let mut audit_details =
                    usage_audit_details(&attempt.model_id, Some(&attempt.usage), None);
                merge_details(
                    &mut audit_details,
                    serde_json::json!({
                        "status": "succeeded",
                        "client_id": attempt.client_id.to_string(),
                        "report_id": attempt.report_id.to_string(),
                        "attempt_id": attempt.attempt_id.to_string(),
                        "turn_id": response.turn_id,
                        "revision": response.workspace.draft.revision,
                        "section_count": response.workspace.draft.content.sections.len(),
                        "skipped_section_count": response
                            .workspace
                            .draft
                            .content
                            .sections
                            .iter()
                            .filter(|section| section.skipped)
                            .count(),
                        "converse_calls": attempt.converse_calls,
                        "tool_uses": attempt.tool_uses,
                        "usage_complete": attempt.usage_complete,
                        "included_record_files": record_context.included_files,
                        "unavailable_record_files": record_context.unavailable_files,
                        "record_characters": record_context.total_characters,
                    }),
                );
                tracing::info!(
                    client_id = %attempt.client_id,
                    report_id = %attempt.report_id,
                    revision = response.workspace.draft.revision,
                    included_record_files = record_context.included_files,
                    unavailable_record_files = record_context.unavailable_files,
                    converse_calls = attempt.converse_calls,
                    "whole-report generation completed"
                );
                ctx.record_audit(
                    ctx.audit_event(
                        "report_full_draft_generated",
                        "report",
                        attempt.report_id.to_string(),
                    )
                    .with_details(audit_details),
                )
                .await;
                Ok(response)
            }
            Err(error) => {
                let attempt = error.attempt().cloned();
                let resource_id = attempt.as_ref().map_or_else(
                    || report_id.to_string(),
                    |value| value.report_id.to_string(),
                );
                let mut audit_details = usage_audit_details(
                    &model_id,
                    attempt.as_ref().map(|value| &value.usage),
                    None,
                );
                merge_details(
                    &mut audit_details,
                    serde_json::json!({
                        "status": "failed",
                        "client_id": client_id.to_string(),
                        "report_id": attempt.as_ref().map_or_else(
                            || report_id.to_string(),
                            |value| value.report_id.to_string(),
                        ),
                        "attempt_id": attempt.as_ref().map(|value| value.attempt_id.to_string()),
                        "failure_code": error.failure_code(),
                        "converse_calls": attempt.as_ref().map_or(0, |value| value.converse_calls),
                        "tool_uses": attempt.as_ref().map_or(0, |value| value.tool_uses),
                        "usage_complete": attempt.as_ref().is_some_and(|value| value.usage_complete),
                    }),
                );
                ctx.record_audit(
                    ctx.audit_event("report_full_draft_failed", "report", resource_id)
                        .with_details(audit_details),
                )
                .await;
                Err(error.into())
            }
        }
    })
    .await
}

#[tauri::command]
#[specta::specta]
// Tauri command parameters are the typed IPC contract; report identity and
// the progress channel legitimately take this one past clippy's ceiling.
#[allow(clippy::too_many_arguments)]
pub async fn send_report_message(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
    model_id: String,
    instruction: String,
    references: Vec<ReportBlockReferenceInput>,
    on_progress: tauri::ipc::Channel<ReportTurnProgressView>,
) -> Result<ReportTurnResponse, String> {
    run("send_report_message", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let references = references
            .into_iter()
            .map(ReportBlockReferenceInput::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let limits = ctx.cfg.report_authoring.limits()?;
        let progress = |event: claria_report_pipeline::ReportTurnProgress| {
            let _ = on_progress.send(event.into());
        };
        let prompt_body =
            super::prompts::load_prompt(&ctx.s3, &ctx.bucket, "report-system").await?;
        let result = claria_report_pipeline::send_report_message_for_report(
            &ctx.sdk_config,
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            expected_revision,
            &model_id,
            claria_report_pipeline::ReportMessageRequest::new(&instruction)
                .with_references(&references)
                .with_limits(limits)
                .with_progress(&progress)
                .with_prompt_cache(&state.report_prompt_cache)
                .with_system_prompt_body(&prompt_body)
                .with_model_tuning(super::model_tuning_for(&ctx.cfg, &model_id)),
        )
        .await;

        match result {
            Ok(outcome) => {
                let attempt = outcome.attempt.clone();
                let response = claria_desktop::report_authoring::turn_response_view(outcome);
                // Usage fields come from the shared helper; `usage_complete`
                // is then overridden with the attempt's own aggregate flag
                // (per-call omissions can leave a partial sum).
                let mut audit_details =
                    usage_audit_details(&attempt.model_id, Some(&attempt.usage), None);
                merge_details(
                    &mut audit_details,
                    serde_json::json!({
                        "status": "succeeded",
                        "client_id": attempt.client_id.to_string(),
                        "report_id": attempt.report_id.to_string(),
                        "attempt_id": attempt.attempt_id.to_string(),
                        "turn_id": response.turn_id,
                        "proposal_id": response.proposal_id,
                        "revision": response.workspace.draft.revision,
                        "converse_calls": attempt.converse_calls,
                        "tool_uses": attempt.tool_uses,
                        "usage_complete": attempt.usage_complete,
                    }),
                );
                ctx.record_audit(
                    ctx.audit_event(
                        "report_tool_turn_succeeded",
                        "report",
                        attempt.report_id.to_string(),
                    )
                    .with_details(audit_details),
                )
                .await;
                Ok(response)
            }
            Err(error) => {
                let attempt = error.attempt().cloned();
                let resource_id = attempt.as_ref().map_or_else(
                    || report_id.to_string(),
                    |value| value.report_id.to_string(),
                );
                let mut audit_details = usage_audit_details(
                    &model_id,
                    attempt.as_ref().map(|value| &value.usage),
                    None,
                );
                merge_details(
                    &mut audit_details,
                    serde_json::json!({
                        "status": "failed",
                        "client_id": client_id.to_string(),
                        "report_id": attempt.as_ref().map_or_else(
                            || report_id.to_string(),
                            |value| value.report_id.to_string(),
                        ),
                        "attempt_id": attempt.as_ref().map(|value| value.attempt_id.to_string()),
                        "failure_code": error.failure_code(),
                        "converse_calls": attempt.as_ref().map_or(0, |value| value.converse_calls),
                        "tool_uses": attempt.as_ref().map_or(0, |value| value.tool_uses),
                        "usage_complete": attempt.as_ref().is_some_and(|value| value.usage_complete),
                    }),
                );
                ctx.record_audit(
                    ctx.audit_event("report_tool_turn_failed", "report", resource_id)
                        .with_details(audit_details),
                )
                .await;
                Err(error.into())
            }
        }
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_report_proposal(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    proposal_id: String,
    decision: ReportProposalChoice,
) -> Result<ReportWorkspaceView, String> {
    run("resolve_report_proposal", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let proposal_id = parse_uuid(&proposal_id)?;
        let action = match decision {
            ReportProposalChoice::Accept => "report_proposal_accepted",
            ReportProposalChoice::Reject => "report_proposal_rejected",
        };
        let workspace = claria_report_store::resolve_report_proposal_for_report(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            proposal_id,
            decision.into(),
        )
        .await?;
        let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

        ctx.record_audit(
            ctx.audit_event(action, "report", workspace.report_id.clone())
                .with_details(serde_json::json!({
                    "client_id": client_id.to_string(),
                    "report_id": workspace.report_id,
                    "proposal_id": proposal_id.to_string(),
                    "resulting_revision": workspace.draft.revision,
                    "section_count": workspace.draft.content.sections.len()
                })),
        )
        .await;

        Ok(workspace)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn export_report_docx(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
) -> Result<ReportExportResult, String> {
    run("export_report_docx", async {
        let ctx = CommandContext::new(&state).await?;
        let client_id = parse_uuid(&client_id)?;
        let report_id = parse_uuid(&report_id)?;
        let snapshot = claria_report_store::load_export_snapshot(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            expected_revision,
        )
        .await?;
        let (bytes, template_applied, template_warning) =
            if let Some(template) = snapshot.template_source.as_deref() {
                let (bytes, fidelity) =
                    claria_docx::render_report_with_template(template, &snapshot.draft)?;
                let warning = (fidelity == claria_docx::TemplateRenderFidelity::PlainBodyFallback)
                    .then_some(TemplateExportWarning::TemplateBodyFallback);
                if warning.is_some() {
                    tracing::warn!(
                        report_id = %report_id,
                        ?fidelity,
                        "template export fell back to generated body formatting"
                    );
                }
                (bytes, warning.is_none(), warning)
            } else if snapshot.template_missing {
                tracing::warn!(
                    report_id = %report_id,
                    "report template source is missing; exporting without template formatting"
                );
                (
                    claria_docx::render_report(&snapshot.draft)?,
                    false,
                    Some(TemplateExportWarning::TemplateMissing),
                )
            } else {
                (claria_docx::render_report(&snapshot.draft)?, false, None)
            };
        let draft = snapshot.draft;
        let filename = claria_report_store::suggested_docx_filename(&draft.content.title);
        // Use the asynchronous dialog implementation. In particular, macOS must
        // schedule NSSavePanel work on the main thread; opening the synchronous
        // dialog after async S3 work can otherwise return as canceled repeatedly.
        let selected = rfd::AsyncFileDialog::new()
            .set_title("Export report to Word")
            .set_file_name(filename)
            .add_filter("Word documents", &["docx"])
            .save_file()
            .await;
        let Some(selected) = selected else {
            let attempted_at = jiff::Timestamp::now();
            let status_persisted = claria_report_store::record_report_export(
                &ctx.s3,
                &ctx.bucket,
                client_id,
                report_id,
                draft.revision,
                claria_core::models::report::ReportExportStatus::Canceled,
            )
            .await
            .is_ok();
            return Ok(ReportExportResult {
                exported: false,
                report_id: report_id.to_string(),
                revision: draft.revision,
                status: ReportExportStatus::Canceled,
                attempted_at: attempted_at.to_string(),
                status_persisted,
                template_applied,
                template_warning,
            });
        };
        let mut path = selected.path().to_path_buf();
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("docx"))
        {
            path.set_extension("docx");
        }
        // The selected local path is intentionally never logged or audited.
        if let Err(error) = claria_desktop::local_export::write_private_atomic(&path, &bytes) {
            let _ = claria_report_store::record_report_export(
                &ctx.s3,
                &ctx.bucket,
                client_id,
                report_id,
                draft.revision,
                claria_core::models::report::ReportExportStatus::Failed,
            )
            .await;
            return Err(error.to_string().into());
        }
        let attempted_at = jiff::Timestamp::now();
        let status_persisted = claria_report_store::record_report_export(
            &ctx.s3,
            &ctx.bucket,
            client_id,
            report_id,
            draft.revision,
            claria_core::models::report::ReportExportStatus::Exported,
        )
        .await
        .is_ok();

        ctx.record_audit(
            ctx.audit_event("report_docx_exported", "report", report_id.to_string())
                .with_details(serde_json::json!({
                    "client_id": client_id.to_string(),
                    "report_id": report_id.to_string(),
                    "revision": draft.revision,
                    "section_count": draft.content.sections.len(),
                    "destination": "local_unmanaged_storage"
                })),
        )
        .await;

        Ok(ReportExportResult {
            exported: true,
            report_id: report_id.to_string(),
            revision: draft.revision,
            status: ReportExportStatus::Exported,
            attempted_at: attempted_at.to_string(),
            status_persisted,
            template_applied,
            template_warning,
        })
    })
    .await
}
