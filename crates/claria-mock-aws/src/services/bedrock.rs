use axum::{
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::state::SharedState;

/// Dispatch Bedrock / Bedrock Runtime REST requests.
pub async fn dispatch(method: &Method, path: &str, body: Value, state: SharedState) -> Response {
    // Bedrock Runtime operations are POST-only.
    if path.starts_with("/model/") && path.ends_with("/converse") {
        if method != Method::POST {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return converse(path, body, state).await;
    }
    if path.starts_with("/model/") && path.ends_with("/count-tokens") {
        if method != Method::POST {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return count_tokens(path, body, state).await;
    }

    // Bedrock control plane
    match (method, path) {
        (&Method::GET, p) if p.starts_with("/foundation-models") && p.contains("/availability") => {
            get_model_availability(p, state).await
        }
        (&Method::GET, p)
            if p.starts_with("/foundation-models") && p.contains("/agreement-offers") =>
        {
            list_agreement_offers(p, state).await
        }
        (&Method::GET, "/foundation-models") => list_foundation_models(state).await,
        (&Method::GET, "/inference-profiles") => list_inference_profiles(state).await,
        (&Method::POST, "/custom-model-agreements") => create_model_agreement(body, state).await,
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

fn validation_error(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/json")],
        json!({"message": message, "__type": "ValidationException"}).to_string(),
    )
        .into_response()
}

fn validate_report_request(body: &Value) -> Result<(), &'static str> {
    let tool_config = body
        .get("toolConfig")
        .ok_or("toolConfig is required for a report request")?;
    let tools = tool_config
        .get("tools")
        .and_then(Value::as_array)
        .ok_or("toolConfig.tools must be an array")?;
    let mut names = Vec::with_capacity(tools.len());
    for tool in tools {
        let specification = tool
            .get("toolSpec")
            .ok_or("every tool must contain toolSpec")?;
        let name = specification
            .get("name")
            .and_then(Value::as_str)
            .ok_or("every tool must have a name")?;
        let schema = specification
            .pointer("/inputSchema/json")
            .ok_or("every report tool must have a JSON schema")?;
        if schema.get("type").and_then(Value::as_str) != Some("object")
            || schema.get("additionalProperties").and_then(Value::as_bool) != Some(false)
        {
            return Err("every report tool must have a closed object JSON schema");
        }
        let required: std::collections::HashSet<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or("every report tool schema must declare properties")?;
        match name {
            "list_record_files" if !required.is_empty() || !properties.is_empty() => {
                return Err("list_record_files must use the empty object schema");
            }
            "read_record_file"
                if required != std::collections::HashSet::from(["filename"])
                    || !properties.contains_key("filename") =>
            {
                return Err("read_record_file schema must require filename");
            }
            "propose_report_changes"
                if required != std::collections::HashSet::from(["summary", "operations"])
                    || !properties.contains_key("operations") =>
            {
                return Err("propose_report_changes schema must require summary and operations");
            }
            _ => {}
        }
        names.push(name);
    }
    if names
        != [
            "list_record_files",
            "read_record_file",
            "propose_report_changes",
        ]
    {
        return Err("report requests must configure the three report tools in order");
    }

    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("messages must be an array")?;
    if messages.is_empty() {
        return Err("messages must not be empty");
    }

    let mut all_ids = std::collections::HashSet::new();
    let mut expected_results: Option<std::collections::HashSet<&str>> = None;
    for (index, message) in messages.iter().enumerate() {
        let role = message.get("role").and_then(Value::as_str);
        let expected_role = if index % 2 == 0 { "user" } else { "assistant" };
        if role != Some(expected_role) {
            return Err("message roles must alternate starting with user");
        }
        let blocks = message
            .get("content")
            .and_then(Value::as_array)
            .ok_or("message content must be an array")?;
        if blocks.is_empty() {
            return Err("message content must not be empty");
        }

        let mut use_ids = std::collections::HashSet::new();
        let mut result_ids = std::collections::HashSet::new();
        for block in blocks {
            if let Some(tool) = block.get("toolUse") {
                if role != Some("assistant") {
                    return Err("toolUse blocks require assistant role");
                }
                let id = tool
                    .get("toolUseId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or("toolUseId must not be empty")?;
                if !all_ids.insert(id) || !use_ids.insert(id) {
                    return Err("toolUseId must be globally unique");
                }
                if tool.get("name").and_then(Value::as_str).is_none()
                    || !tool.get("input").is_some_and(Value::is_object)
                {
                    return Err("toolUse must contain name and object input");
                }
            } else if let Some(result) = block.get("toolResult") {
                if role != Some("user") {
                    return Err("toolResult blocks require user role");
                }
                let id = result
                    .get("toolUseId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or("toolResult.toolUseId must not be empty")?;
                if !result_ids.insert(id) {
                    return Err("each tool use must have exactly one result");
                }
                if !matches!(
                    result.get("status").and_then(Value::as_str),
                    Some("success" | "error")
                ) || result
                    .get("content")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty)
                {
                    return Err("toolResult must contain status and content");
                }
            } else if block.get("reasoningContent").is_some() {
                if role != Some("assistant") {
                    return Err("reasoningContent blocks require assistant role");
                }
            } else if block.get("text").and_then(Value::as_str).is_none() {
                return Err("unsupported content block");
            } else if expected_results.is_some() {
                return Err("a tool-result message must contain only results");
            }
        }

        if let Some(expected) = expected_results.take() {
            if expected != result_ids {
                return Err("tool results must exactly match preceding tool uses");
            }
        } else if !result_ids.is_empty() {
            return Err("tool result has no preceding tool use");
        }
        if !use_ids.is_empty() {
            expected_results = Some(use_ids);
        }
    }
    if expected_results.is_some() {
        return Err("request contains unresolved tool uses");
    }
    if messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        != Some("user")
    {
        return Err("Converse request must end with user role");
    }
    Ok(())
}

async fn count_tokens(path: &str, body: Value, state: SharedState) -> Response {
    // CountTokens wraps the Converse shape under `input.converse` on the wire.
    let converse = body
        .pointer("/input/converse")
        .or_else(|| body.get("converse"))
        .unwrap_or(&body);
    // Tool-configured (report) counting requests get the full report-shape
    // validation; plain chat counting requests only need messages.
    if converse.get("toolConfig").is_some() {
        if let Err(message) = validate_report_request(converse) {
            return validation_error(message);
        }
    } else if converse
        .get("messages")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return validation_error("messages must be a non-empty array");
    }
    let model_id = path
        .strip_prefix("/model/")
        .and_then(|rest| rest.strip_suffix("/count-tokens"))
        .map(|value| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8_lossy()
                .to_string()
        })
        .unwrap_or_default();
    let mut st = state.write().await;
    st.bedrock_count_token_requests.push(converse.clone());
    st.bedrock_count_token_model_ids.push(model_id.clone());
    if st
        .bedrock_count_tokens_unsupported_models
        .contains(&model_id)
    {
        return validation_error("CountTokens is not supported for this model");
    }
    let estimated = u32::try_from(converse.to_string().chars().count() / 4)
        .unwrap_or(u32::MAX)
        .max(1);
    let input_tokens = st.bedrock_count_tokens_override.unwrap_or(estimated);
    json_response(json!({"inputTokens": input_tokens}))
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

async fn list_agreement_offers(path: &str, _state: SharedState) -> Response {
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

async fn converse(path: &str, body: Value, state: SharedState) -> Response {
    // Tool-configured report calls have their own FIFO script and capture
    // surface. Scripts are never consumed by ordinary Chat or extraction, so
    // the existing text-only path remains deterministic and unchanged.
    if body.get("toolConfig").is_some() {
        if let Err(message) = validate_report_request(&body) {
            return validation_error(message);
        }
        let model_id = path
            .strip_prefix("/model/")
            .and_then(|rest| rest.strip_suffix("/converse"))
            .map(|value| {
                percent_encoding::percent_decode_str(value)
                    .decode_utf8_lossy()
                    .to_string()
            })
            .unwrap_or_default();
        let scripted = {
            let mut st = state.write().await;
            st.bedrock_tool_model_ids.push(model_id);
            st.bedrock_tool_requests.push(body.clone());
            if st.bedrock_tool_responses.is_empty() {
                None
            } else {
                Some(st.bedrock_tool_responses.remove(0))
            }
        };

        return match scripted {
            Some(scripted) => {
                let status = StatusCode::from_u16(scripted.status)
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                (
                    status,
                    [("content-type", "application/json")],
                    scripted.body.to_string(),
                )
                    .into_response()
            }
            None => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                json!({
                    "message": "No scripted tool-configured Converse response",
                    "__type": "InternalServerException"
                })
                .to_string(),
            )
                .into_response(),
        };
    }

    // Plain Converse requests are captured and may be scripted; otherwise
    // the canned response below keeps existing flows deterministic.
    let scripted = {
        let mut st = state.write().await;
        st.bedrock_text_requests.push(body.clone());
        if st.bedrock_text_responses.is_empty() {
            None
        } else {
            Some(st.bedrock_text_responses.remove(0))
        }
    };
    if let Some(scripted) = scripted {
        let status =
            StatusCode::from_u16(scripted.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return (
            status,
            [("content-type", "application/json")],
            scripted.body.to_string(),
        )
            .into_response();
    }

    // Return the existing canned response for ordinary Converse requests.
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
