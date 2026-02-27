use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use claria_core::models::anonymize::AnonymizationResult;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AnonymizeRequest {
    pub text: String,
    pub model_id: Option<String>,
}

#[derive(Serialize)]
pub struct AnonymizeResponse {
    pub result: AnonymizationResult,
}

/// Anonymize a document by sending it to Bedrock.
pub async fn anonymize(
    State(state): State<AppState>,
    Json(req): Json<AnonymizeRequest>,
) -> Result<Json<AnonymizeResponse>, ApiError> {
    let model_id = req
        .model_id
        .as_deref()
        .unwrap_or("us.anthropic.claude-sonnet-4-20250514");

    let client =
        claria_bedrock::client::build_client_with_region(&state.cognito_region).await;

    let system_prompt = "Identify all personally identifiable information (PII) in the following document. Replace each PII instance with a consistent placeholder. Return a JSON object with 'anonymized_text' and 'replacements' fields.";

    let result =
        claria_bedrock::transaction::anonymize_document(&client, model_id, system_prompt, &req.text)
            .await?;

    Ok(Json(AnonymizeResponse {
        result: result.output,
    }))
}
