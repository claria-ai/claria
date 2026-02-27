use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use claria_core::models::transaction::BedrockTransaction;
use claria_core::s3_keys;
use claria_storage::objects;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn list_transactions(
    State(state): State<AppState>,
) -> Result<Json<Vec<BedrockTransaction>>, ApiError> {
    // Transactions are stored per-report at reports/{id}/transaction.json.
    // List all report keys and collect transactions.
    let keys = objects::list_objects(&state.s3, &state.bucket, "reports/").await?;

    let mut transactions = Vec::new();
    for key in &keys {
        if key.ends_with("/transaction.json")
            && let Ok(output) = objects::get_object(&state.s3, &state.bucket, key).await
            && let Ok(txn) = serde_json::from_slice::<BedrockTransaction>(&output.body)
        {
            transactions.push(txn);
        }
    }

    Ok(Json(transactions))
}

pub async fn get_transaction(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<BedrockTransaction>, ApiError> {
    let key = s3_keys::report_transaction(id);
    let output = objects::get_object(&state.s3, &state.bucket, &key).await?;
    let txn: BedrockTransaction = serde_json::from_slice(&output.body)?;
    Ok(Json(txn))
}
