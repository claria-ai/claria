use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use claria_core::models::assessment::Assessment;
use claria_core::s3_keys;
use claria_storage::objects;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn list_assessments(
    State(state): State<AppState>,
) -> Result<Json<Vec<Assessment>>, ApiError> {
    let keys = objects::list_objects(&state.s3, &state.bucket, "assessments/").await?;

    let mut assessments = Vec::new();
    for key in &keys {
        let output = objects::get_object(&state.s3, &state.bucket, key).await?;
        let assessment: Assessment = serde_json::from_slice(&output.body)?;
        assessments.push(assessment);
    }

    Ok(Json(assessments))
}

pub async fn get_assessment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Assessment>, ApiError> {
    let key = s3_keys::assessment(id);
    let output = objects::get_object(&state.s3, &state.bucket, &key).await?;
    let assessment: Assessment = serde_json::from_slice(&output.body)?;
    Ok(Json(assessment))
}

pub async fn create_assessment(
    State(state): State<AppState>,
    Json(assessment): Json<Assessment>,
) -> Result<Json<Assessment>, ApiError> {
    let key = s3_keys::assessment(assessment.id);
    let body = serde_json::to_vec(&assessment)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(assessment))
}

pub async fn update_assessment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut assessment): Json<Assessment>,
) -> Result<Json<Assessment>, ApiError> {
    assessment.id = id;
    let key = s3_keys::assessment(id);
    let body = serde_json::to_vec(&assessment)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(assessment))
}

pub async fn delete_assessment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let key = s3_keys::assessment(id);
    objects::delete_object(&state.s3, &state.bucket, &key).await?;
    Ok(Json(()))
}
