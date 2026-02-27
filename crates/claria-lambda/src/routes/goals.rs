use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use claria_core::models::goal::Goal;
use claria_core::s3_keys;
use claria_storage::objects;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn list_goals(State(state): State<AppState>) -> Result<Json<Vec<Goal>>, ApiError> {
    let keys = objects::list_objects(&state.s3, &state.bucket, "goals/").await?;

    let mut goals = Vec::new();
    for key in &keys {
        let output = objects::get_object(&state.s3, &state.bucket, key).await?;
        let goal: Goal = serde_json::from_slice(&output.body)?;
        goals.push(goal);
    }

    Ok(Json(goals))
}

pub async fn get_goal(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Goal>, ApiError> {
    let key = s3_keys::goal(id);
    let output = objects::get_object(&state.s3, &state.bucket, &key).await?;
    let goal: Goal = serde_json::from_slice(&output.body)?;
    Ok(Json(goal))
}

pub async fn create_goal(
    State(state): State<AppState>,
    Json(goal): Json<Goal>,
) -> Result<Json<Goal>, ApiError> {
    let key = s3_keys::goal(goal.id);
    let body = serde_json::to_vec(&goal)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(goal))
}

pub async fn update_goal(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut goal): Json<Goal>,
) -> Result<Json<Goal>, ApiError> {
    goal.id = id;
    let key = s3_keys::goal(id);
    let body = serde_json::to_vec(&goal)?;
    objects::put_object(&state.s3, &state.bucket, &key, body, Some("application/json")).await?;
    Ok(Json(goal))
}

pub async fn delete_goal(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let key = s3_keys::goal(id);
    objects::delete_object(&state.s3, &state.bucket, &key).await?;
    Ok(Json(()))
}
