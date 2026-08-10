use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::state::SharedState;

/// Dispatch Cost Explorer JSON-protocol requests.
pub async fn dispatch(target_suffix: &str, body: Value, state: SharedState) -> Response {
    match target_suffix {
        "GetCostAndUsage" => get_cost_and_usage(body, state).await,
        _ => (
            StatusCode::BAD_REQUEST,
            json!({"__type": "InvalidAction", "message": format!("Unknown Cost Explorer action: {target_suffix}")}).to_string(),
        ).into_response(),
    }
}

async fn get_cost_and_usage(_body: Value, state: SharedState) -> Response {
    let st = state.read().await;

    let results: Vec<Value> = st
        .cost_data
        .iter()
        .map(|period| {
            let groups: Vec<Value> = period
                .groups
                .iter()
                .map(|g| {
                    json!({
                        "Keys": [g.key],
                        "Metrics": {
                            "UnblendedCost": {
                                "Amount": g.amount,
                                "Unit": g.unit,
                            }
                        }
                    })
                })
                .collect();

            json!({
                "TimePeriod": {
                    "Start": period.start,
                    "End": period.end,
                },
                "Groups": groups,
            })
        })
        .collect();

    (
        StatusCode::OK,
        [("content-type", "application/x-amz-json-1.1")],
        json!({
            "ResultsByTime": results,
        })
        .to_string(),
    )
        .into_response()
}
