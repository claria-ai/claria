use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::state::SharedState;

/// Dispatch Artifact JSON-protocol requests.
pub async fn dispatch(target_suffix: &str, _body: Value, state: SharedState) -> Response {
    match target_suffix {
        "ListCustomerAgreements" => list_customer_agreements(state).await,
        _ => (
            StatusCode::BAD_REQUEST,
            json!({"__type": "InvalidAction", "message": format!("Unknown Artifact action: {target_suffix}")}).to_string(),
        ).into_response(),
    }
}

async fn list_customer_agreements(state: SharedState) -> Response {
    let st = state.read().await;

    let agreements = if st.baa_accepted {
        vec![json!({
            "name": "AWS Business Associate Addendum (BAA)",
            "state": "ACTIVE",
            "effectiveStart": "2026-01-15T00:00:00Z",
        })]
    } else {
        vec![]
    };

    (
        StatusCode::OK,
        [("content-type", "application/x-amz-json-1.1")],
        json!({ "customerAgreements": agreements }).to_string(),
    )
        .into_response()
}
