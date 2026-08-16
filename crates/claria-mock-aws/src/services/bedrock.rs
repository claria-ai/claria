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
    if path.starts_with("/model/") && path.ends_with("/converse-stream") {
        if method != Method::POST {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return converse_stream(path, body, state).await;
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

/// Validate one `cachePoint` block the way Bedrock does: the type must be
/// `default`, and the optional `ttl` must be one of the wire values
/// (`5m` / `1h`).
fn validate_cache_point(cache_point: &Value) -> Result<(), &'static str> {
    if cache_point.get("type").and_then(Value::as_str) != Some("default") {
        return Err("cachePoint blocks must carry the default type");
    }
    match cache_point.get("ttl") {
        None => Ok(()),
        Some(ttl) if matches!(ttl.as_str(), Some("5m" | "1h")) => Ok(()),
        Some(_) => Err("cachePoint ttl must be 5m or 1h"),
    }
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
            "set_full_draft_title"
                if required != std::collections::HashSet::from(["title"])
                    || !properties.contains_key("title") =>
            {
                return Err("set_full_draft_title schema must require title");
            }
            "write_full_draft_section"
                if required
                    != std::collections::HashSet::from([
                        "section_id",
                        "position",
                        "heading",
                        "blocks",
                    ])
                    || !properties.contains_key("blocks") =>
            {
                return Err(
                    "write_full_draft_section schema must require section_id, position, heading, and blocks",
                );
            }
            "skip_full_draft_section"
                if required != std::collections::HashSet::from(["section_id"])
                    || !properties.contains_key("section_id") =>
            {
                return Err("skip_full_draft_section schema must require section_id");
            }
            "mark_section_failed"
                if required != std::collections::HashSet::from(["section_id", "reason"])
                    || !properties.contains_key("reason") =>
            {
                return Err("mark_section_failed schema must require section_id and reason");
            }
            "finish_full_draft"
                if required != std::collections::HashSet::from(["summary"])
                    || !properties.contains_key("summary") =>
            {
                return Err("finish_full_draft schema must require summary");
            }
            _ => {}
        }
        names.push(name);
    }
    let targeted_tools = [
        "list_record_files",
        "read_record_file",
        "propose_report_changes",
    ];
    let full_draft_tools = [
        "set_full_draft_title",
        "write_full_draft_section",
        "skip_full_draft_section",
        "mark_section_failed",
        "finish_full_draft",
    ];
    if names != targeted_tools && names != full_draft_tools {
        return Err("report requests must configure one complete report tool set in order");
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
            } else if let Some(cache_point) = block.get("cachePoint") {
                // Prompt-cache markers are valid anywhere in message content.
                validate_cache_point(cache_point)?;
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
    if let Some(forced_tool) = converse
        .pointer("/toolConfig/toolChoice/tool/name")
        .and_then(Value::as_str)
    {
        if let Err(message) = validate_forced_tool_request(converse, forced_tool) {
            return validation_error(message);
        }
    } else if converse.get("toolConfig").is_some() {
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
    // Forced-tool calls (`toolChoice.tool`) — currently the translation flow —
    // are honored separately from the report protocol: the mock responds with
    // a toolUse block for the forced tool, synthesizing a translation
    // envelope when no scripted response is queued.
    if let Some(forced_tool) = body
        .pointer("/toolConfig/toolChoice/tool/name")
        .and_then(Value::as_str)
        .map(ToString::to_string)
    {
        return forced_tool_converse(&forced_tool, body, state).await;
    }

    // Tool-configured report calls have their own FIFO script and capture
    // surface. Scripts are never consumed by ordinary Chat or extraction, so
    // the existing text-only path remains deterministic and unchanged.
    if body.get("toolConfig").is_some() {
        if let Err(message) = validate_report_request(&body) {
            return validation_error(message);
        }
        let (status, payload) = tool_converse_payload(path, "/converse", body, state).await;
        return (
            status,
            [("content-type", "application/json")],
            payload.to_string(),
        )
            .into_response();
    }

    plain_converse(body, state).await
}

/// The registered marker this request's message text contains, if any.
///
/// Longest first, so a marker that is a prefix of another cannot shadow it.
/// Only `text` blocks in `messages` are searched — never the tool schemas,
/// which are identical across the requests a fan-out sends.
async fn matching_marker(body: &Value, state: &SharedState) -> Option<String> {
    let markers: Vec<String> = {
        let st = state.read().await;
        if st.bedrock_tool_responses_by_marker.is_empty() {
            return None;
        }
        st.bedrock_tool_responses_by_marker
            .keys()
            .cloned()
            .collect()
    };
    let text: String = body
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("content"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let mut candidates: Vec<String> = markers
        .into_iter()
        .filter(|marker| text.contains(marker.as_str()))
        .collect();
    candidates.sort_by_key(|marker| std::cmp::Reverse(marker.len()));
    candidates.into_iter().next()
}

/// Pop the next scripted tool-configured Converse payload, capturing the
/// request and its model ID.
///
/// Shared by the unary and streaming endpoints so a test scripts one JSON
/// body and the mock delivers it in whichever shape the caller asked for.
async fn tool_converse_payload(
    path: &str,
    suffix: &str,
    body: Value,
    state: SharedState,
) -> (StatusCode, Value) {
    let model_id = path
        .strip_prefix("/model/")
        .and_then(|rest| rest.strip_suffix(suffix))
        .map(|value| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8_lossy()
                .to_string()
        })
        .unwrap_or_default();
    let scripted = {
        let marker = matching_marker(&body, &state).await;
        let mut st = state.write().await;
        st.bedrock_tool_model_ids.push(model_id);
        st.bedrock_tool_requests.push(body);
        let keyed = marker
            .and_then(|marker| st.bedrock_tool_responses_by_marker.get_mut(&marker))
            .filter(|queue| !queue.is_empty())
            .map(|queue| queue.remove(0));
        match keyed {
            Some(response) => Some(response),
            None if st.bedrock_tool_responses.is_empty() => None,
            None => Some(st.bedrock_tool_responses.remove(0)),
        }
    };

    match scripted {
        Some(scripted) => (
            StatusCode::from_u16(scripted.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            scripted.body,
        ),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({
                "message": "No scripted tool-configured Converse response",
                "__type": "InternalServerException"
            }),
        ),
    }
}

/// Respond to a Converse request whose `toolChoice` forces a specific tool.
///
/// The named tool must exist in the request's tool list. Scripted responses
/// (the tool FIFO) win; otherwise a translation-shaped envelope is
/// synthesized from the request's `segments` payload so SDK tests can drive
/// the real client end to end.
/// The checks Bedrock applies to a `toolChoice.tool` request regardless of
/// transport: the named tool has to be in the request's own tool list, the
/// conversation has to have something in it, and any cache points have to be
/// well formed. Shared by the unary and streaming endpoints so a forced call
/// cannot pass one and fail the other.
fn validate_forced_tool_request(body: &Value, forced_tool: &str) -> Result<(), &'static str> {
    let declared = body
        .pointer("/toolConfig/tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.pointer("/toolSpec/name").and_then(Value::as_str))
                .any(|name| name == forced_tool)
        })
        .unwrap_or(false);
    if !declared {
        return Err("toolChoice names a tool absent from the tool list");
    }
    if body
        .get("messages")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err("messages must be a non-empty array");
    }
    for block in body
        .get("system")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            body.get("messages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|message| message.get("content"))
                .filter_map(Value::as_array)
                .flatten(),
        )
    {
        if let Some(cache_point) = block.get("cachePoint") {
            validate_cache_point(cache_point)?;
        }
    }
    Ok(())
}

async fn forced_tool_converse(forced_tool: &str, body: Value, state: SharedState) -> Response {
    if let Err(message) = validate_forced_tool_request(&body, forced_tool) {
        return validation_error(message);
    }

    let (scripted, segments) = {
        let mut st = state.write().await;
        st.bedrock_tool_requests.push(body.clone());
        let scripted = if st.bedrock_tool_responses.is_empty() {
            None
        } else {
            Some(st.bedrock_tool_responses.remove(0))
        };
        // Pull the `segments` array out of the last user message's text so
        // the default response translates exactly what was requested.
        let segments = body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_array())
            .and_then(|content| {
                content.iter().find_map(|block| {
                    let text = block.get("text").and_then(Value::as_str)?;
                    let json_start = text.find('{')?;
                    serde_json::from_str::<Value>(&text[json_start..]).ok()
                })
            })
            .and_then(|payload| payload.get("segments").cloned());
        (scripted, segments)
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

    let translations: Vec<Value> = segments
        .and_then(|segments| segments.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|segment| {
            json!({
                "index": segment.get("index").cloned().unwrap_or(json!(0)),
                "translation": format!(
                    "[EN] {}",
                    segment.get("source_text").and_then(Value::as_str).unwrap_or("")
                ),
            })
        })
        .collect();

    json_response(json!({
        "output": {
            "message": {
                "role": "assistant",
                "content": [{
                    "toolUse": {
                        "toolUseId": "mock-forced-tool-1",
                        "name": forced_tool,
                        "input": {"translations": translations}
                    }
                }]
            }
        },
        "stopReason": "tool_use",
        "usage": {
            "inputTokens": 120,
            "outputTokens": 40,
        }
    }))
}

async fn plain_converse(body: Value, state: SharedState) -> Response {
    let (status, payload) = plain_converse_payload(body, state).await;
    (
        status,
        [("content-type", "application/json")],
        payload.to_string(),
    )
        .into_response()
}

/// Scripted-or-canned JSON payload for a plain (no `toolConfig`) Converse
/// request — the one behavior shared by the unary and streaming endpoints,
/// which differ only in how the payload goes over the wire.
async fn plain_converse_payload(body: Value, state: SharedState) -> (StatusCode, Value) {
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
        return (status, scripted.body);
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

    (
        StatusCode::OK,
        json!({
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
        }),
    )
}

/// How many characters of assistant text each `contentBlockDelta` carries.
/// Small enough that every canned response streams as several deltas.
const STREAM_DELTA_CHARS: usize = 24;

/// Respond to `ConverseStream` with real AWS event-stream frames.
///
/// Both script FIFOs are shared with the unary endpoint — tool-configured
/// requests draw from the report script, everything else from the plain one
/// — and a 200 payload is decomposed into `messageStart`, per-block
/// start/delta/stop events, `messageStop`, and a trailing `metadata` event.
/// Non-200 scripted responses return as ordinary JSON errors, which the SDK
/// surfaces before streaming begins.
async fn converse_stream(path: &str, body: Value, state: SharedState) -> Response {
    // A forced-tool request is the analysis family (planner, review passes):
    // its tool set is its own, so the report-protocol validation above does
    // not apply, but it draws from the same scripted FIFO and lands in the
    // same capture surface as every other tool-configured request.
    let forced_tool = body
        .pointer("/toolConfig/toolChoice/tool/name")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let (status, payload) = if let Some(forced_tool) = forced_tool {
        if let Err(message) = validate_forced_tool_request(&body, &forced_tool) {
            return validation_error(message);
        }
        tool_converse_payload(path, "/converse-stream", body, state.clone()).await
    } else if body.get("toolConfig").is_some() {
        if let Err(message) = validate_report_request(&body) {
            return validation_error(message);
        }
        tool_converse_payload(path, "/converse-stream", body, state.clone()).await
    } else {
        plain_converse_payload(body, state.clone()).await
    };
    let (stall, drop_body) = {
        let mut state = state.write().await;
        state.bedrock_stream_request_count += 1;
        let past_stall_point = state
            .bedrock_stream_stalls_after
            .is_some_and(|after| state.bedrock_stream_request_count > after);
        if state.bedrock_stream_stalls > 0 {
            state.bedrock_stream_stalls -= 1;
            (true, false)
        } else if past_stall_point {
            (true, false)
        } else if state.bedrock_stream_drops > 0 {
            state.bedrock_stream_drops -= 1;
            (false, true)
        } else {
            (false, false)
        }
    };
    if stall {
        return match stalled_converse_stream() {
            Ok(response) => response,
            Err(error) => encoding_failure(error),
        };
    }
    if drop_body {
        return match dropped_converse_stream() {
            Ok(response) => response,
            Err(error) => encoding_failure(error),
        };
    }

    if status != StatusCode::OK {
        return (
            status,
            [("content-type", "application/json")],
            payload.to_string(),
        )
            .into_response();
    }

    match encode_converse_stream(&payload) {
        Ok(frames) => (
            StatusCode::OK,
            [("content-type", "application/vnd.amazon.eventstream")],
            frames,
        )
            .into_response(),
        Err(error) => encoding_failure(error),
    }
}

fn encoding_failure(error: aws_smithy_eventstream::error::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [("content-type", "application/json")],
        json!({
            "message": format!("failed to encode event stream: {error}"),
            "__type": "InternalServerException"
        })
        .to_string(),
    )
        .into_response()
}

/// A `ConverseStream` response that starts and then dies: the opening frame
/// and one text delta arrive, and the body never completes.
///
/// The request stays open indefinitely, so a caller with no idle bound of
/// its own waits forever. Nothing in the AWS SDK rescues it — the streaming
/// Bedrock operations carry no stalled-stream protection interceptor, and
/// the read timeout covers only the wait for response headers, which
/// already arrived.
fn stalled_converse_stream() -> Result<Response, aws_smithy_eventstream::error::Error> {
    use futures::StreamExt;

    let mut frames = Vec::new();
    write_event(&mut frames, "messageStart", &json!({"role": "assistant"}))?;
    write_event(
        &mut frames,
        "contentBlockDelta",
        &json!({"contentBlockIndex": 0, "delta": {"text": "Beginning of a reply that "}}),
    )?;

    let opening = futures::stream::once(async move {
        Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(frames))
    });
    let never_ends = futures::stream::pending::<Result<bytes::Bytes, std::io::Error>>();
    let body = axum::body::Body::from_stream(opening.chain(never_ends));

    Ok((
        StatusCode::OK,
        [("content-type", "application/vnd.amazon.eventstream")],
        body,
    )
        .into_response())
}

/// A `ConverseStream` response whose connection dies mid-body: the opening
/// frame and one text delta arrive, then the body errors out, which reaches
/// the client as an abruptly severed stream rather than a clean end.
///
/// Unlike [`stalled_converse_stream`], the failure is immediate, so retry
/// paths can be exercised in real time instead of a paused clock — which
/// matters because a paused clock races real network I/O against the SDK's
/// connect-timeout timers.
fn dropped_converse_stream() -> Result<Response, aws_smithy_eventstream::error::Error> {
    use futures::StreamExt;

    let mut frames = Vec::new();
    write_event(&mut frames, "messageStart", &json!({"role": "assistant"}))?;
    write_event(
        &mut frames,
        "contentBlockDelta",
        &json!({"contentBlockIndex": 0, "delta": {"text": "Beginning of a reply that "}}),
    )?;

    let opening = futures::stream::once(async move {
        Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(frames))
    });
    let severed = futures::stream::once(async {
        Err::<bytes::Bytes, std::io::Error>(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "mock connection dropped mid-response",
        ))
    });
    let body = axum::body::Body::from_stream(opening.chain(severed));

    Ok((
        StatusCode::OK,
        [("content-type", "application/vnd.amazon.eventstream")],
        body,
    )
        .into_response())
}

/// Decompose a unary Converse JSON payload into encoded event-stream frames.
///
/// Every content block the report protocol can carry is reproduced in its
/// streaming shape — text and tool-use input arrive split across several
/// deltas, exactly as the service splits them, so a collector that only
/// works on whole blocks fails here rather than in production.
fn encode_converse_stream(
    payload: &Value,
) -> Result<Vec<u8>, aws_smithy_eventstream::error::Error> {
    let stop_reason = payload
        .get("stopReason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn");
    let blocks = payload
        .pointer("/output/message/content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut frames = Vec::new();
    write_event(&mut frames, "messageStart", &json!({"role": "assistant"}))?;
    for (index, block) in blocks.iter().enumerate() {
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            write_chunked_deltas(&mut frames, index, text, |chunk| json!({"text": chunk}))?;
        } else if let Some(tool) = block.get("toolUse") {
            write_event(
                &mut frames,
                "contentBlockStart",
                &json!({
                    "contentBlockIndex": index,
                    "start": {"toolUse": {
                        "toolUseId": tool.get("toolUseId").and_then(Value::as_str).unwrap_or(""),
                        "name": tool.get("name").and_then(Value::as_str).unwrap_or(""),
                    }}
                }),
            )?;
            // The service streams tool input as partial JSON text, never as
            // a structured object; the collector has to concatenate and parse.
            let input = tool.get("input").cloned().unwrap_or_else(|| json!({}));
            write_chunked_deltas(
                &mut frames,
                index,
                &input.to_string(),
                |chunk| json!({"toolUse": {"input": chunk}}),
            )?;
        } else if let Some(reasoning) = block.get("reasoningContent") {
            if let Some(text) = reasoning
                .pointer("/reasoningText/text")
                .and_then(Value::as_str)
            {
                write_chunked_deltas(
                    &mut frames,
                    index,
                    text,
                    |chunk| json!({"reasoningContent": {"text": chunk}}),
                )?;
            }
            if let Some(signature) = reasoning
                .pointer("/reasoningText/signature")
                .and_then(Value::as_str)
            {
                write_event(
                    &mut frames,
                    "contentBlockDelta",
                    &json!({
                        "contentBlockIndex": index,
                        "delta": {"reasoningContent": {"signature": signature}}
                    }),
                )?;
            }
            if let Some(redacted) = reasoning.get("redactedContent").and_then(Value::as_str) {
                write_event(
                    &mut frames,
                    "contentBlockDelta",
                    &json!({
                        "contentBlockIndex": index,
                        "delta": {"reasoningContent": {"redactedContent": redacted}}
                    }),
                )?;
            }
        }
        write_event(
            &mut frames,
            "contentBlockStop",
            &json!({"contentBlockIndex": index}),
        )?;
    }
    write_event(
        &mut frames,
        "messageStop",
        &json!({"stopReason": stop_reason}),
    )?;
    if let Some(usage) = payload.get("usage") {
        write_event(
            &mut frames,
            "metadata",
            &json!({"usage": usage, "metrics": {"latencyMs": 42}}),
        )?;
    }
    Ok(frames)
}

/// Split `text` into [`STREAM_DELTA_CHARS`]-sized `contentBlockDelta` events,
/// wrapping each chunk in the delta shape `build` returns.
fn write_chunked_deltas(
    buffer: &mut Vec<u8>,
    index: usize,
    text: &str,
    build: impl Fn(&str) -> Value,
) -> Result<(), aws_smithy_eventstream::error::Error> {
    let chars: Vec<char> = text.chars().collect();
    for chunk in chars.chunks(STREAM_DELTA_CHARS) {
        let chunk: String = chunk.iter().collect();
        write_event(
            buffer,
            "contentBlockDelta",
            &json!({"contentBlockIndex": index, "delta": build(&chunk)}),
        )?;
    }
    Ok(())
}

/// Frame one event with the standard `:message-type`/`:event-type`/
/// `:content-type` headers and append it to `buffer`.
fn write_event(
    buffer: &mut Vec<u8>,
    event_type: &str,
    payload: &Value,
) -> Result<(), aws_smithy_eventstream::error::Error> {
    use aws_smithy_types::event_stream::{Header, HeaderValue, Message};

    let message = Message::new(bytes::Bytes::from(payload.to_string()))
        .add_header(Header::new(
            ":message-type",
            HeaderValue::String("event".into()),
        ))
        .add_header(Header::new(
            ":event-type",
            HeaderValue::String(event_type.to_string().into()),
        ))
        .add_header(Header::new(
            ":content-type",
            HeaderValue::String("application/json".into()),
        ));
    aws_smithy_eventstream::frame::write_message_to(&message, buffer)
}
