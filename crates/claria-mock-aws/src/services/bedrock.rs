use axum::{
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};

use crate::state::SharedState;

/// Dispatch Bedrock / Bedrock Runtime REST requests.
pub async fn dispatch(
    method: &Method,
    path: &str,
    body: Value,
    state: SharedState,
) -> Response {
    // Bedrock Runtime: POST /model/{model_id}/converse
    if path.starts_with("/model/") && path.ends_with("/converse") {
        return converse(body, state).await;
    }

    // Bedrock control plane
    match (method, path) {
        (&Method::GET, p) if p.starts_with("/foundation-models") && p.contains("/availability") => {
            get_model_availability(p, state).await
        }
        (&Method::GET, p) if p.starts_with("/foundation-models") && p.contains("/agreement-offers") => {
            list_agreement_offers(p, state).await
        }
        (&Method::GET, "/foundation-models") => list_foundation_models(state).await,
        (&Method::GET, "/inference-profiles") => list_inference_profiles(state).await,
        (&Method::POST, "/custom-model-agreements") => {
            create_model_agreement(body, state).await
        }
        _ => (
            StatusCode::NOT_FOUND,
            json!({"message": format!("Unknown Bedrock path: {path}")}).to_string(),
        )
            .into_response(),
    }
}

fn json_response(value: Value) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        value.to_string(),
    )
        .into_response()
}

async fn list_foundation_models(state: SharedState) -> Response {
    let st = state.read().await;
    let summaries: Vec<Value> = st
        .foundation_models
        .iter()
        .map(|m| {
            json!({
                "modelId": m.model_id,
                "modelName": m.model_name,
                "providerName": m.provider_name,
                "modelLifecycle": {
                    "status": m.model_lifecycle.status,
                },
            })
        })
        .collect();

    json_response(json!({ "modelSummaries": summaries }))
}

async fn list_inference_profiles(state: SharedState) -> Response {
    let st = state.read().await;
    let profiles: Vec<Value> = st
        .inference_profiles
        .iter()
        .map(|p| {
            json!({
                "inferenceProfileId": p.inference_profile_id,
                "inferenceProfileName": p.inference_profile_name,
                "type": p.r#type,
                "status": p.status,
                "models": p.models.iter().map(|m| json!({ "modelArn": m.model_arn })).collect::<Vec<_>>(),
            })
        })
        .collect();

    json_response(json!({ "inferenceProfileSummaries": profiles }))
}

async fn get_model_availability(path: &str, state: SharedState) -> Response {
    // Path: /foundation-models/{model_id}/availability
    let model_id = path
        .strip_prefix("/foundation-models/")
        .and_then(|rest| rest.strip_suffix("/availability"))
        .unwrap_or("");

    let st = state.read().await;
    let agreed = st.model_agreements.contains(model_id);

    json_response(json!({
        "agreementAvailability": {
            "status": if agreed { "AVAILABLE" } else { "NOT_AGREED" },
        }
    }))
}

async fn list_agreement_offers(path: &str, state: SharedState) -> Response {
    // Path: /foundation-models/{model_id}/agreement-offers
    let model_id = path
        .strip_prefix("/foundation-models/")
        .and_then(|rest| rest.strip_suffix("/agreement-offers"))
        .unwrap_or("");

    json_response(json!({
        "offers": [{
            "offerToken": format!("mock-offer-token-{model_id}"),
        }]
    }))
}

async fn create_model_agreement(body: Value, state: SharedState) -> Response {
    let model_id = body["modelId"].as_str().unwrap_or("");
    let mut st = state.write().await;
    st.model_agreements.insert(model_id.to_string());
    json_response(json!({ "agreementId": format!("mock-agreement-{model_id}") }))
}

async fn converse(body: Value, _state: SharedState) -> Response {
    // Return a canned response for any Converse request.
    // If the body contains a document (extraction), return extracted text.
    let has_document = body["messages"]
        .as_array()
        .map(|msgs| {
            msgs.iter().any(|m| {
                m["content"]
                    .as_array()
                    .map(|c| c.iter().any(|block| block.get("document").is_some()))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    let response_text = if has_document {
        "This is the extracted text content from the uploaded document. \
         The document contains clinical assessment data, test scores, \
         and behavioral observations."
            .to_string()
    } else {
        "I understand your question. Based on the available records, \
         I can provide analysis and recommendations. \
         Would you like me to elaborate on any specific aspect?"
            .to_string()
    };

    json_response(json!({
        "output": {
            "message": {
                "role": "assistant",
                "content": [{ "text": response_text }]
            }
        },
        "stopReason": "end_turn",
        "usage": {
            "inputTokens": 150,
            "outputTokens": 50,
        }
    }))
}
