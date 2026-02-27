use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use claria_core::models::snippet::TextSnippet;
use claria_core::s3_keys;
use claria_storage::objects;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn list_snippets(
    State(state): State<AppState>,
) -> Result<Json<Vec<TextSnippet>>, ApiError> {
    let keys = objects::list_objects(&state.s3, &state.bucket, "snippets/").await?;

    let mut snippets = Vec::new();
    for key in &keys {
        let output = objects::get_object(&state.s3, &state.bucket, key).await?;
        let snippet: TextSnippet = serde_json::from_slice(&output.body)?;
        snippets.push(snippet);
    }

    Ok(Json(snippets))
}

pub async fn get_snippet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TextSnippet>, ApiError> {
    let key = s3_keys::snippet(id);
    let output = objects::get_object(&state.s3, &state.bucket, &key).await?;
    let snippet: TextSnippet = serde_json::from_slice(&output.body)?;
    Ok(Json(snippet))
}

pub async fn create_snippet(
    State(state): State<AppState>,
    Json(snippet): Json<TextSnippet>,
) -> Result<Json<TextSnippet>, ApiError> {
    let key = s3_keys::snippet(snippet.id);
    let body = serde_json::to_vec(&snippet)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(snippet))
}

pub async fn update_snippet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut snippet): Json<TextSnippet>,
) -> Result<Json<TextSnippet>, ApiError> {
    snippet.id = id;
    let key = s3_keys::snippet(id);
    let body = serde_json::to_vec(&snippet)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(snippet))
}

pub async fn delete_snippet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let key = s3_keys::snippet(id);
    objects::delete_object(&state.s3, &state.bucket, &key).await?;
    Ok(Json(()))
}
