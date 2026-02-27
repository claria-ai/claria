use axum::Json;
use serde::{Deserialize, Serialize};

use claria_bedrock::tokens::get_pricing;
use claria_core::models::cost::CostEstimate;
use claria_core::models::token_count::TokenCount;

use crate::error::ApiError;

#[derive(Deserialize)]
pub struct CostEstimateRequest {
    pub model_id: String,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
}

#[derive(Serialize)]
pub struct CostEstimateResponse {
    pub estimate: CostEstimate,
}

/// Estimate the cost of a Bedrock call before executing it.
pub async fn estimate_cost(
    Json(req): Json<CostEstimateRequest>,
) -> Result<Json<CostEstimateResponse>, ApiError> {
    let pricing = get_pricing(&req.model_id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown model: {}", req.model_id)))?;

    let tokens = TokenCount {
        input: req.estimated_input_tokens,
        output: req.estimated_output_tokens,
    };

    let estimated_cost_usd = pricing.estimate_cost(tokens);

    Ok(Json(CostEstimateResponse {
        estimate: CostEstimate {
            model_id: req.model_id,
            estimated_tokens: tokens,
            estimated_cost_usd,
        },
    }))
}
