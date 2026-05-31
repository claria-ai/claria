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

    // Bedrock control plane (real wire URIs).
    match (method, path) {
        (&Method::GET, "/foundation-models") => list_foundation_models(state).await,
        (&Method::GET, "/inference-profiles") => list_inference_profiles(state).await,
        (&Method::GET, p) if p.starts_with("/foundation-model-availability/") => {
            let model_id = decode_model_id(p.trim_start_matches("/foundation-model-availability/"));
            get_model_availability(&model_id, state).await
        }
        (&Method::GET, p) if p.starts_with("/list-foundation-model-agreement-offers/") => {
            let model_id =
                decode_model_id(p.trim_start_matches("/list-foundation-model-agreement-offers/"));
            list_agreement_offers(&model_id).await
        }
        (&Method::POST, "/create-foundation-model-agreement") => {
            create_model_agreement(body, state).await
        }
        (&Method::POST, "/delete-foundation-model-agreement") => {
            delete_model_agreement(body, state).await
        }
        (&Method::GET, "/use-case-for-model-access") => get_use_case_form(state).await,
        (&Method::POST, "/use-case-for-model-access") => put_use_case_form(body, state).await,
        _ => (
            StatusCode::NOT_FOUND,
            json!({"message": format!("Unknown Bedrock path: {path}")}).to_string(),
        )
            .into_response(),
    }
}

/// Path labels percent-encode the `:` in model ids (e.g. `...-v1:0`).
fn decode_model_id(raw: &str) -> String {
    raw.replace("%3A", ":").replace("%3a", ":")
}

fn json_response(value: Value) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        value.to_string(),
    )
        .into_response()
}

fn json_status(status: StatusCode, value: Value) -> Response {
    (
        status,
        [("content-type", "application/json")],
        value.to_string(),
    )
        .into_response()
}

/// Error body shaped so the AWS SDK surfaces the type in the message, used for
/// the "use-case form not submitted" case.
fn ftu_not_filled_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        [
            ("content-type", "application/json"),
            ("x-amzn-errortype", "ResourceNotFoundException"),
        ],
        json!({
            "message": "FTUFormNotFilled: Model use case details have not been submitted for this account"
        })
        .to_string(),
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

async fn get_model_availability(model_id: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let status = st
        .model_agreement_status
        .get(model_id)
        .map(String::as_str)
        .unwrap_or("NOT_AVAILABLE");

    // The agreement axis mirrors the lifecycle; entitlement flips to AVAILABLE
    // only once the agreement is EXECUTED (the authoritative "can invoke" gate).
    let (agreement_status, entitlement) = match status {
        "AVAILABLE" => ("AVAILABLE", "NOT_AVAILABLE"),
        "PENDING" => ("PENDING", "NOT_AVAILABLE"),
        "EXECUTED" => ("NOT_AVAILABLE", "AVAILABLE"),
        "ERROR" => ("ERROR", "NOT_AVAILABLE"),
        _ => ("NOT_AVAILABLE", "NOT_AVAILABLE"),
    };

    let mut agreement = json!({ "status": agreement_status });
    if agreement_status == "ERROR" {
        agreement["errorMessage"] = json!("the marketplace subscription failed to provision");
    }

    json_response(json!({
        "modelId": model_id,
        "agreementAvailability": agreement,
        "authorizationStatus": "AUTHORIZED",
        "entitlementAvailability": entitlement,
        "regionAvailability": "AVAILABLE",
    }))
}

async fn list_agreement_offers(model_id: &str) -> Response {
    json_response(json!({
        "offers": [{
            "offerId": format!("offer-{model_id}"),
            "offerToken": format!("mock-offer-token-{model_id}"),
            "termDetails": {
                "usageBasedPricingTerm": {
                    "rateCard": [
                        {
                            "dimension": "InputTokens",
                            "description": "Input tokens",
                            "price": "0.000015",
                            "unit": "1K tokens"
                        },
                        {
                            "dimension": "OutputTokens",
                            "description": "Output tokens",
                            "price": "0.000075",
                            "unit": "1K tokens"
                        }
                    ]
                },
                "legalTerm": {
                    "url": format!("https://aws.amazon.com/marketplace/eula/{model_id}")
                },
                "supportTerm": {
                    "refundPolicyDescription": "No refunds for usage-based charges."
                },
                "validityTerm": {
                    "agreementDuration": "P1Y"
                }
            }
        }]
    }))
}

async fn create_model_agreement(body: Value, state: SharedState) -> Response {
    let model_id = body["modelId"].as_str().unwrap_or("").to_string();
    let mut st = state.write().await;

    // Anthropic requires the FTU form before any agreement can be created.
    if st.ftu_form.is_none() {
        return ftu_not_filled_response();
    }

    // Agreement creation is async: the subscription starts PENDING and is
    // promoted to EXECUTED out of band (see the /mock/bedrock/promote endpoint).
    st.model_agreement_status
        .insert(model_id.clone(), "PENDING".to_string());

    json_status(StatusCode::ACCEPTED, json!({ "modelId": model_id }))
}

async fn delete_model_agreement(body: Value, state: SharedState) -> Response {
    let model_id = body["modelId"].as_str().unwrap_or("");
    let mut st = state.write().await;
    st.model_agreement_status
        .insert(model_id.to_string(), "NOT_AVAILABLE".to_string());
    json_response(json!({}))
}

async fn get_use_case_form(state: SharedState) -> Response {
    let st = state.read().await;
    match &st.ftu_form {
        Some(form_data) => json_response(json!({ "formData": form_data })),
        None => ftu_not_filled_response(),
    }
}

async fn put_use_case_form(body: Value, state: SharedState) -> Response {
    let form_data = body["formData"].as_str().unwrap_or("").to_string();
    let mut st = state.write().await;
    st.ftu_form = Some(form_data);
    json_status(StatusCode::CREATED, json!({}))
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
