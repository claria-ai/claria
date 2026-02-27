use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use claria_core::models::template::Template;
use claria_core::s3_keys;
use claria_storage::objects;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn list_templates(
    State(state): State<AppState>,
) -> Result<Json<Vec<Template>>, ApiError> {
    let keys = objects::list_objects(&state.s3, &state.bucket, "templates/").await?;

    let mut templates = Vec::new();
    for key in &keys {
        let output = objects::get_object(&state.s3, &state.bucket, key).await?;
        let template: Template = serde_json::from_slice(&output.body)?;
        templates.push(template);
    }

    Ok(Json(templates))
}

pub async fn get_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Template>, ApiError> {
    let key = s3_keys::template(id);
    let output = objects::get_object(&state.s3, &state.bucket, &key).await?;
    let template: Template = serde_json::from_slice(&output.body)?;
    Ok(Json(template))
}

pub async fn create_template(
    State(state): State<AppState>,
    Json(template): Json<Template>,
) -> Result<Json<Template>, ApiError> {
    let key = s3_keys::template(template.id);
    let body = serde_json::to_vec(&template)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(template))
}

pub async fn update_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut template): Json<Template>,
) -> Result<Json<Template>, ApiError> {
    template.id = id;
    let key = s3_keys::template(id);
    let body = serde_json::to_vec(&template)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(template))
}

pub async fn delete_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let key = s3_keys::template(id);
    objects::delete_object(&state.s3, &state.bucket, &key).await?;
    Ok(Json(()))
}
