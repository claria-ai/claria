//! Native DOCX-template import commands for the Writing workspace.
//!
//! The selected file is parsed locally into a bounded structured candidate.
//! Source bytes, filenames, and paths never enter the webview or S3.

use tauri::State;

use claria_desktop::report_authoring::{ReportTemplatePreview, ReportWorkspaceView};

use crate::{
    commands::{bucket_name, load_sdk_config, record_audit},
    state::{DesktopState, PendingReportTemplate},
};

/// Open a bounded native DOCX picker and return a structured content preview.
/// The parsed candidate remains only in process memory until explicitly
/// applied or discarded.
#[tauri::command]
#[specta::specta]
pub async fn pick_report_template_docx(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Option<ReportTemplatePreview>, String> {
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let selected = rfd::AsyncFileDialog::new()
        .set_title("Import a Word report template")
        .add_filter("Word documents", &["docx"])
        .pick_file()
        .await;
    let Some(selected) = selected else {
        return Ok(None);
    };
    if selected
        .path()
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("docx"))
    {
        return Err(
            "Choose a .docx Word document. Macro-enabled files are not accepted.".to_string(),
        );
    }
    let metadata = tokio::fs::metadata(selected.path())
        .await
        .map_err(|_| "Claria could not inspect the selected Word document.".to_string())?;
    if metadata.len() > claria_docx::MAX_TEMPLATE_DOCX_BYTES {
        return Err("The selected Word document exceeds the 10 MiB template limit.".to_string());
    }
    let bytes = tokio::fs::read(selected.path())
        .await
        .map_err(|_| "Claria could not read the selected Word document.".to_string())?;
    let imported = tokio::task::spawn_blocking(move || claria_docx::import_template(&bytes))
        .await
        .map_err(|_| "Claria could not safely inspect the Word template.".to_string())?
        .map_err(|error| error.to_string())?;
    let import_id = uuid::Uuid::new_v4();
    let preview = claria_desktop::report_authoring::template_preview_view(import_id, &imported);

    let mut pending = state.pending_report_templates.lock().await;
    pending.retain(|_, candidate| candidate.client_id != client_id);
    // Bound abandoned in-memory previews (for example after a webview crash).
    if pending.len() >= 8 {
        pending.clear();
    }
    pending.insert(
        import_id,
        PendingReportTemplate {
            client_id,
            imported,
        },
    );
    Ok(Some(preview))
}

/// Apply an already previewed DOCX candidate as a new accepted revision.
#[tauri::command]
#[specta::specta]
pub async fn apply_report_template(
    state: State<'_, DesktopState>,
    client_id: String,
    expected_revision: u64,
    import_id: String,
) -> Result<ReportWorkspaceView, String> {
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let import_id = import_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let imported = {
        let pending = state.pending_report_templates.lock().await;
        let candidate = pending
            .get(&import_id)
            .ok_or_else(|| "That template preview expired. Choose the DOCX again.".to_string())?;
        if candidate.client_id != client_id {
            return Err("That template preview belongs to another client.".to_string());
        }
        candidate.imported.clone()
    };

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let warning_count = imported.warnings.len();
    let stats = imported.stats.clone();
    let workspace = claria_report_authoring::apply_report_template(
        &s3,
        &bucket,
        client_id,
        expected_revision,
        imported.content,
        imported.source_sha256,
        imported.warnings,
    )
    .await
    .map_err(|error| error.to_string())?;
    state
        .pending_report_templates
        .lock()
        .await
        .remove(&import_id);
    let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "report_template_imported",
            "report",
            workspace.report_id.clone(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({
            "client_id": client_id.to_string(),
            "report_id": workspace.report_id,
            "revision": workspace.draft.revision,
            "warning_category_count": warning_count,
            "section_count": stats.sections,
            "paragraph_count": stats.paragraphs,
            "table_count": stats.tables,
            "table_cell_count": stats.table_cells,
            "placeholder_count": stats.placeholder_count
        })),
    )
    .await;
    Ok(workspace)
}

#[tauri::command]
#[specta::specta]
pub async fn discard_report_template_preview(
    state: State<'_, DesktopState>,
    import_id: String,
) -> Result<(), String> {
    let import_id = import_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    state
        .pending_report_templates
        .lock()
        .await
        .remove(&import_id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn acknowledge_report_template_review(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
) -> Result<ReportWorkspaceView, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let report_id = report_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let workspace = claria_report_authoring::acknowledge_report_template_review(
        &s3,
        &bucket,
        client_id,
        report_id,
        expected_revision,
    )
    .await
    .map_err(|error| error.to_string())?;
    let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "report_template_carryover_reviewed",
            "report",
            workspace.report_id.clone(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({
            "client_id": client_id.to_string(),
            "report_id": workspace.report_id,
            "revision": workspace.draft.revision,
            "placeholder_count": workspace
                .template_import
                .as_ref()
                .map_or(0, |template| template.placeholder_count)
        })),
    )
    .await;
    Ok(workspace)
}
