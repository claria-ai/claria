use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use claria_core::models::answer::SchematizedAnswer;
use claria_core::s3_keys;
use claria_export::render::render_template;
use claria_export::styles::DocumentStyles;
use claria_storage::objects;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct ReportSummary {
    pub id: Uuid,
    pub client_name: String,
}

pub async fn list_reports(
    State(state): State<AppState>,
) -> Result<Json<Vec<ReportSummary>>, ApiError> {
    let keys = objects::list_objects(&state.s3, &state.bucket, "reports/").await?;

    let mut seen = std::collections::HashSet::new();
    let mut reports = Vec::new();
    for key in &keys {
        if let Some(id_str) = key
            .strip_prefix("reports/")
            .and_then(|rest| rest.split('/').next())
        {
            if !seen.insert(id_str.to_string()) {
                continue;
            }
            if let Ok(id) = id_str.parse::<Uuid>() {
                let answer_key = s3_keys::report_answer(id);
                if let Ok(output) =
                    objects::get_object(&state.s3, &state.bucket, &answer_key).await
                    && let Ok(answer) = serde_json::from_slice::<SchematizedAnswer>(&output.body)
                {
                    reports.push(ReportSummary {
                        id,
                        client_name: answer.client_name,
                    });
                }
            }
        }
    }

    Ok(Json(reports))
}

pub async fn get_report(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SchematizedAnswer>, ApiError> {
    let key = s3_keys::report_answer(id);
    let output = objects::get_object(&state.s3, &state.bucket, &key).await?;
    let answer: SchematizedAnswer = serde_json::from_slice(&output.body)?;
    Ok(Json(answer))
}

#[derive(Deserialize)]
pub struct ExportRequest {
    pub template_id: Uuid,
    pub format: ExportFormat,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Docx,
    Pdf,
}

/// Export a report to DOCX or PDF.
pub async fn export_report(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ExportRequest>,
) -> Result<Vec<u8>, ApiError> {
    let answer_key = s3_keys::report_answer(id);
    let answer_output = objects::get_object(&state.s3, &state.bucket, &answer_key).await?;
    let answer: SchematizedAnswer = serde_json::from_slice(&answer_output.body)?;

    let template_key = s3_keys::template(req.template_id);
    let template_output = objects::get_object(&state.s3, &state.bucket, &template_key).await?;
    let template_content = String::from_utf8(template_output.body)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let rendered = render_template("report", &template_content, &answer)?;

    let (bytes, s3_dest, content_type) = match req.format {
        ExportFormat::Docx => {
            let styles = DocumentStyles::default();
            let docx_bytes = claria_export::docx::generate_docx(&rendered, &styles)?;
            (
                docx_bytes,
                s3_keys::report_docx(id),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            )
        }
        ExportFormat::Pdf => {
            let pdf_bytes = claria_export::pdf::generate_pdf(&rendered)?;
            (pdf_bytes, s3_keys::report_pdf(id), "application/pdf")
        }
    };

    objects::put_object(
        &state.s3,
        &state.bucket,
        &s3_dest,
        bytes.clone(),
        Some(content_type),
    )
    .await?;

    Ok(bytes)
}
