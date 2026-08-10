//! Managed writer-template commands and Writing-session template previews.

use tauri::State;

use claria_desktop::report_authoring::{
    ReportTemplatePreview, ReportWorkspaceView, WriterTemplateView, writer_template_view,
};

use crate::{
    commands::{bucket_name, load_sdk_config, record_audit},
    state::{DesktopState, PendingReportTemplate},
};

#[tauri::command]
#[specta::specta]
pub async fn list_writer_templates(
    state: State<'_, DesktopState>,
) -> Result<Vec<WriterTemplateView>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let templates = claria_report_authoring::writer_templates::list(&s3, &bucket_name(&cfg))
        .await
        .map_err(|error| error.to_string())?;
    Ok(templates.into_iter().map(writer_template_view).collect())
}

/// Pick, validate, and upload a redacted DOCX into the managed template shelf.
/// The local filename and path are never persisted.
#[tauri::command]
#[specta::specta]
pub async fn upload_writer_template(
    state: State<'_, DesktopState>,
) -> Result<Option<WriterTemplateView>, String> {
    let selected = rfd::AsyncFileDialog::new()
        .set_title("Upload a redacted writer template")
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
    let validation_bytes = bytes.clone();
    tokio::task::spawn_blocking(move || claria_docx::import_template(&validation_bytes))
        .await
        .map_err(|_| "Claria could not safely inspect the Word template.".to_string())?
        .map_err(|error| error.to_string())?;

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let templates = claria_report_authoring::writer_templates::list(&s3, &bucket)
        .await
        .map_err(|error| error.to_string())?;
    let ordinal = (1..)
        .find(|ordinal| {
            let candidate = format!("Writer Template ({ordinal})");
            templates
                .iter()
                .all(|template| template.metadata.name != candidate)
        })
        .expect("an unused writer template ordinal exists");
    let id = uuid::Uuid::new_v4();
    let template = claria_report_authoring::writer_templates::create(
        &s3,
        &bucket,
        id,
        &format!("Writer Template ({ordinal})"),
        bytes,
    )
    .await
    .map_err(|error| error.to_string())?;

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "writer_template_uploaded",
            "writer_template",
            id.to_string(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({ "size": template.metadata.size })),
    )
    .await;
    Ok(Some(writer_template_view(template)))
}

#[tauri::command]
#[specta::specta]
pub async fn rename_writer_template(
    state: State<'_, DesktopState>,
    template_id: String,
    name: String,
) -> Result<WriterTemplateView, String> {
    let template_id = template_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let template = claria_report_authoring::writer_templates::rename(
        &s3,
        &bucket_name(&cfg),
        template_id,
        &name,
    )
    .await
    .map_err(|error| error.to_string())?;
    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "writer_template_renamed",
            "writer_template",
            template_id.to_string(),
            cfg.account_id.clone(),
        ),
    )
    .await;
    Ok(writer_template_view(template))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_writer_template(
    state: State<'_, DesktopState>,
    template_id: String,
) -> Result<(), String> {
    let template_id = template_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    claria_report_authoring::writer_templates::delete(&s3, &bucket_name(&cfg), template_id)
        .await
        .map_err(|error| error.to_string())?;
    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "writer_template_deleted",
            "writer_template",
            template_id.to_string(),
            cfg.account_id.clone(),
        ),
    )
    .await;
    Ok(())
}

/// Parse a managed template into an in-memory candidate for direct application.
#[tauri::command]
#[specta::specta]
pub async fn preview_writer_template(
    state: State<'_, DesktopState>,
    client_id: String,
    template_id: String,
) -> Result<ReportTemplatePreview, String> {
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let template_id = template_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let (metadata, bytes) = claria_report_authoring::writer_templates::load_docx_with_metadata(
        &s3,
        &bucket_name(&cfg),
        template_id,
        claria_docx::MAX_TEMPLATE_DOCX_BYTES,
    )
    .await
    .map_err(|error| error.to_string())?;
    let (imported, source_docx) = tokio::task::spawn_blocking(move || {
        claria_docx::import_template(&bytes).map(|imported| (imported, bytes))
    })
    .await
    .map_err(|_| "Claria could not safely inspect the Word template.".to_string())?
    .map_err(|error| error.to_string())?;
    let import_id = uuid::Uuid::new_v4();
    let preview = claria_desktop::report_authoring::template_preview_view(import_id, &imported);

    let mut pending = state.pending_report_templates.lock().await;
    pending.retain(|_, candidate| candidate.client_id != client_id);
    if pending.len() >= 8 {
        pending.clear();
    }
    pending.insert(
        import_id,
        PendingReportTemplate {
            client_id,
            writer_template_id: template_id,
            writer_template_name: metadata.name,
            source_docx,
            imported,
        },
    );
    Ok(preview)
}

/// Apply a parsed managed DOCX candidate as a new accepted revision.
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
    let (imported, writer_template_id, writer_template_name, source_docx) = {
        let pending = state.pending_report_templates.lock().await;
        let candidate = pending.get(&import_id).ok_or_else(|| {
            "That template preview expired. Select the template again.".to_string()
        })?;
        if candidate.client_id != client_id {
            return Err("That template preview belongs to another client.".to_string());
        }
        (
            candidate.imported.clone(),
            candidate.writer_template_id,
            candidate.writer_template_name.clone(),
            candidate.source_docx.clone(),
        )
    };

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let warning_count = imported.warnings.len();
    let stats = imported.stats.clone();
    claria_report_authoring::store_report_template_source(
        &s3,
        &bucket,
        client_id,
        &imported.source_sha256,
        source_docx,
    )
    .await
    .map_err(|error| error.to_string())?;
    let workspace = claria_report_authoring::apply_report_template(
        &s3,
        &bucket,
        client_id,
        expected_revision,
        claria_report_authoring::ReportTemplateApplication {
            content: imported.content,
            source_sha256: imported.source_sha256,
            writer_template_id,
            writer_template_name,
            warnings: imported.warnings,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    state
        .pending_report_templates
        .lock()
        .await
        .remove(&import_id);
    if let Err(error) =
        claria_report_authoring::writer_templates::increment_usage(&s3, &bucket, writer_template_id)
            .await
    {
        tracing::warn!(template_id = %writer_template_id, error = %error, "writer template usage counter reset or could not be saved");
    }
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
            "writer_template_id": writer_template_id.to_string(),
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
