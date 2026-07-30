//! Bedrock Converse integration for the opt-in Writing workflow.
//!
//! This module is deliberately separate from `chat`: every request carries a
//! real Bedrock tool configuration, and every response preserves tool-use
//! blocks for a controller-managed agent loop. It never falls back to the
//! text-only Chat path.

use std::collections::{HashMap, HashSet};

use aws_sdk_bedrockruntime::{
    operation::RequestId,
    types::{
        ContentBlock, ConversationRole, ConverseTokensRequest, CountTokensInput, Message,
        ReasoningContentBlock, ReasoningTextBlock, SystemContentBlock, Tool, ToolConfiguration,
        ToolInputSchema, ToolResultBlock, ToolResultContentBlock, ToolResultStatus,
        ToolSpecification,
    },
};
use aws_smithy_types::{Blob, Document, Number};
use claria_core::models::{
    report::{
        ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole, ReportToolResultStatus,
        validate_report_conversation,
    },
    turn_usage::TurnUsage,
};
use serde::{Deserialize, Serialize};

use crate::{error::BedrockError, tokens};

pub const LIST_RECORD_FILES_TOOL: &str = "list_record_files";
pub const READ_RECORD_FILE_TOOL: &str = "read_record_file";
pub const PROPOSE_REPORT_CHANGES_TOOL: &str = "propose_report_changes";
pub const MAX_TOOL_USES_PER_RESPONSE: usize = 8;
pub const REPORT_OUTPUT_TOKEN_RESERVE: u32 = 8_192;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportConverseOutput {
    pub message: ReportProtocolMessage,
    pub stop_reason: ReportStopReason,
    pub tool_calls: Vec<ReportToolCall>,
    /// Bedrock may omit usage. Callers must preserve that incompleteness
    /// instead of recording a misleading metered zero.
    pub usage: Option<TurnUsage>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportStopReason {
    ContentFiltered,
    EndTurn,
    GuardrailIntervened,
    MalformedModelOutput,
    MalformedToolUse,
    MaxTokens,
    ModelContextWindowExceeded,
    StopSequence,
    ToolUse,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportToolCall {
    pub tool_use_id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "name", content = "input", rename_all = "snake_case")]
pub enum ReportToolRequest {
    ListRecordFiles(ListRecordFilesRequest),
    ReadRecordFile(ReadRecordFileRequest),
    ProposeReportChanges(ProposeReportChangesRequest),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListRecordFilesRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReadRecordFileRequest {
    pub filename: String,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProposeReportChangesRequest {
    pub summary: String,
    pub operations: Vec<ReportProposalOperationRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReportProposalOperationRequest {
    SetTitle {
        title: String,
    },
    AddSection {
        position: u32,
        heading: String,
        blocks: Vec<ReportBlockRequest>,
    },
    ReplaceSection {
        section_id: String,
        heading: String,
        blocks: Vec<ReportBlockRequest>,
    },
    RemoveSection {
        section_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReportBlockRequest {
    Paragraph { text: String },
    BulletList { items: Vec<String> },
}

/// Decode one model tool call into its strict typed request. Unknown names and
/// extra/missing/mistyped fields are returned as schema violations so the
/// caller can send a correlated error tool result instead of aborting the
/// whole turn.
pub fn decode_tool_request(call: &ReportToolCall) -> Result<ReportToolRequest, BedrockError> {
    match call.name.as_str() {
        LIST_RECORD_FILES_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::ListRecordFiles)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        READ_RECORD_FILE_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::ReadRecordFile)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        PROPOSE_REPORT_CHANGES_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::ProposeReportChanges)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        name => Err(BedrockError::SchemaViolation(format!(
            "unknown report tool: {name}"
        ))),
    }
}

/// Send one report-protocol Converse request with all three report tools.
///
/// The caller owns the bounded tool loop and persistence. This function owns
/// the exact Bedrock wire shape, validates safe tool correlation, and returns
/// the response usage priced at this individual call's rates.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(model_id = %model_id, messages = messages.len())
)]
pub async fn converse_report(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ReportProtocolMessage],
) -> Result<ReportConverseOutput, BedrockError> {
    if !model_supports_report_tools(model_id) {
        return Err(BedrockError::UnsupportedModel(format!(
            "{model_id} is not a verified tool-capable Claude model"
        )));
    }
    validate_report_conversation(messages)
        .map_err(|error| BedrockError::SchemaViolation(error.to_string()))?;

    let client = aws_sdk_bedrockruntime::Client::new(config);
    let sdk_messages = messages
        .iter()
        .map(protocol_message_to_sdk)
        .collect::<Result<Vec<_>, _>>()?;
    let tools = report_tool_configuration()?;
    let system = SystemContentBlock::Text(system_prompt.to_string());

    // Count the exact Converse shape, including immutable policy, messages,
    // and tool schemas, immediately before every billed call.
    let token_request = ConverseTokensRequest::builder()
        .set_messages(Some(sdk_messages.clone()))
        .system(system.clone())
        .tool_config(tools.clone())
        .build();
    // Converse uses the cross-region inference profile selected in the UI,
    // while CountTokens requires the underlying foundation model identifier.
    // Keep the exact selected-model tokenizer by removing only a known
    // cross-region scope prefix.
    let counting_model_id = report_counting_model_id(model_id);
    let input_tokens = match count_report_input_tokens(
        &client,
        counting_model_id,
        token_request.clone(),
    )
    .await
    {
        Ok(tokens) => tokens,
        Err(error)
            if matches!(
                &error,
                BedrockError::ReportService { code, .. } if code == "ValidationException"
            ) =>
        {
            // Some newly listed Claude foundations support Converse before
            // CountTokens. Fall back to the newest active Haiku tokenizer so
            // the Writing turn remains usable while retaining the selected
            // model's conservative context-window limit.
            let fallback_model_id = crate::chat::counting_model(config).await?;
            if fallback_model_id == counting_model_id {
                tracing::error!(model_id, error = %error, "report CountTokens fallback unavailable");
                return Err(error);
            }
            tracing::warn!(
                model_id,
                counting_model_id,
                fallback_model_id,
                error = %error,
                "selected report model does not support CountTokens; using Haiku tokenizer"
            );
            count_report_input_tokens(&client, fallback_model_id, token_request)
                .await
                .map_err(|fallback_error| {
                    tracing::error!(
                        model_id,
                        fallback_model_id,
                        error = %fallback_error,
                        "report CountTokens fallback failed"
                    );
                    fallback_error
                })?
        }
        Err(error) => {
            tracing::error!(model_id, error = %error, "report CountTokens call failed");
            return Err(error);
        }
    };
    let input_token_budget = report_input_token_budget(model_id);
    if input_tokens > input_token_budget {
        return Err(BedrockError::ContextBudgetExceeded {
            model_id: model_id.to_string(),
            input_tokens,
            input_token_budget,
        });
    }

    let response = client
        .converse()
        .model_id(model_id)
        .system(system)
        .set_messages(Some(sdk_messages))
        .tool_config(tools)
        .send()
        .await
        .map_err(|error| {
            let status = error
                .raw_response()
                .map(|response| response.status().as_u16());
            let service_error = error.into_service_error();
            let code = service_error
                .meta()
                .code()
                .unwrap_or("UnknownServiceError")
                .to_string();
            let request_id = service_error.meta().request_id().map(ToString::to_string);
            tracing::error!(
                model_id,
                ?status,
                code,
                ?request_id,
                "report Converse call failed"
            );
            BedrockError::ReportService {
                status,
                code,
                request_id,
            }
        })?;

    let output_message = response
        .output()
        .and_then(|output| output.as_message().ok())
        .ok_or_else(|| BedrockError::ResponseParse("no message in report response".to_string()))?;
    if output_message.role() != &ConversationRole::Assistant {
        return Err(BedrockError::ResponseParse(
            "report response message did not have assistant role".to_string(),
        ));
    }

    let mut content = Vec::with_capacity(output_message.content().len());
    let mut tool_calls = Vec::new();
    let mut tool_ids: HashSet<String> = messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|block| match block {
            ReportProtocolBlock::ToolUse { tool_use_id, .. } => Some(tool_use_id.clone()),
            _ => None,
        })
        .collect();
    for block in output_message.content() {
        match block {
            // Bedrock can emit an empty text block next to a valid tool use.
            // It carries no semantic content and cannot be sent back because
            // Converse rejects empty protocol text on the next tool round.
            ContentBlock::Text(text) if text.is_empty() => {}
            ContentBlock::Text(text) => {
                content.push(ReportProtocolBlock::Text { text: text.clone() })
            }
            ContentBlock::ReasoningContent(ReasoningContentBlock::ReasoningText(reasoning)) => {
                content.push(ReportProtocolBlock::ReasoningText {
                    text: reasoning.text().to_string(),
                    signature: reasoning.signature().map(ToString::to_string),
                });
            }
            ContentBlock::ReasoningContent(ReasoningContentBlock::RedactedContent(reasoning)) => {
                content.push(ReportProtocolBlock::ReasoningRedacted {
                    data: reasoning.as_ref().to_vec(),
                });
            }
            ContentBlock::ReasoningContent(_) => {
                return Err(BedrockError::ResponseParse(
                    "report response contained unknown reasoning content".to_string(),
                ));
            }
            ContentBlock::ToolUse(tool) => {
                let tool_use_id = tool.tool_use_id().to_string();
                if tool_use_id.trim().is_empty() {
                    return Err(BedrockError::ResponseParse(
                        "report response contained an empty tool-use ID".to_string(),
                    ));
                }
                if !tool_ids.insert(tool_use_id.clone()) {
                    return Err(BedrockError::ResponseParse(format!(
                        "report response contained duplicate tool-use ID {tool_use_id}"
                    )));
                }
                if tool.name().trim().is_empty() {
                    return Err(BedrockError::ResponseParse(format!(
                        "tool use {tool_use_id} has an empty name"
                    )));
                }
                let input = document_to_json(tool.input())?;
                let call = ReportToolCall {
                    tool_use_id: tool_use_id.clone(),
                    name: tool.name().to_string(),
                    input: input.clone(),
                };
                tool_calls.push(call);
                content.push(ReportProtocolBlock::ToolUse {
                    tool_use_id,
                    name: tool.name().to_string(),
                    input,
                });
            }
            _ => {
                return Err(BedrockError::ResponseParse(
                    "report response contained an unsupported content block".to_string(),
                ));
            }
        }
    }

    if !content.iter().any(|block| {
        matches!(
            block,
            ReportProtocolBlock::Text { .. } | ReportProtocolBlock::ToolUse { .. }
        )
    }) {
        return Err(BedrockError::ResponseParse(
            "report response had no visible text or tool use".to_string(),
        ));
    }
    if tool_calls.len() > MAX_TOOL_USES_PER_RESPONSE {
        return Err(BedrockError::ResponseParse(format!(
            "report response requested {} tools; maximum is {MAX_TOOL_USES_PER_RESPONSE}",
            tool_calls.len()
        )));
    }

    let stop_reason = map_stop_reason(response.stop_reason());
    let has_tools = !tool_calls.is_empty();
    if has_tools != matches!(stop_reason, ReportStopReason::ToolUse) {
        return Err(BedrockError::ResponseParse(format!(
            "inconsistent report stop reason {:?} for {} tool uses",
            stop_reason,
            tool_calls.len()
        )));
    }

    // The AWS JSON deserializer may materialize a default all-zero usage
    // structure when the service omits the block. A successful Converse call
    // necessarily consumes tokens, so preserve that shape as missing rather
    // than recording a misleading metered zero.
    let usage = response.usage().and_then(|usage| {
        let extracted = tokens::extract_turn_usage(usage, model_id);
        (extracted.input_tokens > 0
            || extracted.output_tokens > 0
            || extracted.cache_read_input_tokens > 0
            || extracted.cache_write_input_tokens > 0)
            .then_some(extracted)
    });

    Ok(ReportConverseOutput {
        message: ReportProtocolMessage {
            role: ReportProtocolRole::Assistant,
            content,
            created_at: jiff::Timestamp::now(),
        },
        stop_reason,
        tool_calls,
        usage,
    })
}

/// True only for Claude model IDs whose Bedrock families support Converse
/// tool use. Capability is determined before invocation; a generic AWS
/// `ValidationException` is never guessed to mean "tools unsupported".
pub fn model_supports_report_tools(model_id: &str) -> bool {
    let model_id = model_id.to_ascii_lowercase();
    model_id.contains("anthropic.claude")
        && !model_id.contains("claude-v2")
        && !model_id.contains("claude-instant")
}

/// Conservative context window for the selected Bedrock model/profile.
/// Explicit context variants take precedence; current Claude 3/4 profiles
/// otherwise have a 200k-token window. Unknown IDs get a safe 32k ceiling.
pub fn report_model_context_tokens(model_id: &str) -> u32 {
    let model_id = model_id.to_ascii_lowercase();
    if model_id.contains(":48k") {
        48_000
    } else if model_id.contains(":200k") {
        200_000
    } else if model_id.contains(":1m") || model_id.contains(":1000k") {
        1_000_000
    } else if model_id.contains("anthropic.claude") {
        200_000
    } else {
        32_000
    }
}

pub fn report_input_token_budget(model_id: &str) -> u32 {
    report_model_context_tokens(model_id).saturating_sub(REPORT_OUTPUT_TOKEN_RESERVE)
}

async fn count_report_input_tokens(
    client: &aws_sdk_bedrockruntime::Client,
    model_id: &str,
    request: ConverseTokensRequest,
) -> Result<u32, BedrockError> {
    let response = client
        .count_tokens()
        .model_id(model_id)
        .input(CountTokensInput::Converse(request))
        .send()
        .await
        .map_err(|error| {
            let status = error
                .raw_response()
                .map(|response| response.status().as_u16());
            let service_error = error.into_service_error();
            let code = service_error
                .meta()
                .code()
                .unwrap_or("UnknownServiceError")
                .to_string();
            let request_id = service_error.meta().request_id().map(ToString::to_string);
            BedrockError::ReportService {
                status,
                code,
                request_id,
            }
        })?;
    Ok(u32::try_from(response.input_tokens()).unwrap_or(u32::MAX))
}

fn report_counting_model_id(model_id: &str) -> &str {
    let Some((scope, foundation_model_id)) = model_id.split_once('.') else {
        return model_id;
    };
    if matches!(scope, "us" | "eu" | "apac" | "global") {
        foundation_model_id
    } else {
        model_id
    }
}

fn report_tool_configuration() -> Result<ToolConfiguration, BedrockError> {
    let list_schema = serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    });
    let read_schema = serde_json::json!({
        "type": "object",
        "required": ["filename"],
        "additionalProperties": false,
        "properties": {
            "filename": {"type": "string", "minLength": 1, "maxLength": 1024},
            "offset": {"type": "integer", "minimum": 0},
            "limit": {"type": "integer", "minimum": 1, "maximum": 12000}
        }
    });
    let block_schema = serde_json::json!({
        "oneOf": [
            {
                "type": "object",
                "required": ["kind", "text"],
                "additionalProperties": false,
                "properties": {
                    "kind": {"enum": ["paragraph"]},
                    "text": {"type": "string", "minLength": 1, "maxLength": 20000}
                }
            },
            {
                "type": "object",
                "required": ["kind", "items"],
                "additionalProperties": false,
                "properties": {
                    "kind": {"enum": ["bullet_list"]},
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 100,
                        "items": {"type": "string", "minLength": 1, "maxLength": 2000}
                    }
                }
            }
        ]
    });
    let proposal_schema = serde_json::json!({
        "type": "object",
        "required": ["summary", "operations"],
        "additionalProperties": false,
        "properties": {
            "summary": {"type": "string", "minLength": 1, "maxLength": 500},
            "operations": {
                "type": "array",
                "minItems": 1,
                "maxItems": 25,
                "items": {
                    "oneOf": [
                        {
                            "type": "object",
                            "required": ["kind", "title"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["set_title"]},
                                "title": {"type": "string", "minLength": 1, "maxLength": 200}
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind", "position", "heading", "blocks"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["add_section"]},
                                "position": {"type": "integer", "minimum": 0, "maximum": 100},
                                "heading": {"type": "string", "minLength": 1, "maxLength": 200},
                                "blocks": {"type": "array", "maxItems": 200, "items": block_schema.clone()}
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind", "section_id", "heading", "blocks"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["replace_section"]},
                                "section_id": {"type": "string", "minLength": 36, "maxLength": 36},
                                "heading": {"type": "string", "minLength": 1, "maxLength": 200},
                                "blocks": {"type": "array", "maxItems": 200, "items": block_schema.clone()}
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind", "section_id"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["remove_section"]},
                                "section_id": {"type": "string", "minLength": 36, "maxLength": 36}
                            }
                        }
                    ]
                }
            }
        }
    });

    let tools = vec![
        tool(
            LIST_RECORD_FILES_TOOL,
            "List the current client's record filenames and whether readable text is available.",
            list_schema,
        )?,
        tool(
            READ_RECORD_FILE_TOOL,
            "Read a bounded Unicode-character range from one filename returned by list_record_files.",
            read_schema,
        )?,
        tool(
            PROPOSE_REPORT_CHANGES_TOOL,
            "Stage typed changes for user review. This never saves or applies the report.",
            proposal_schema,
        )?,
    ];

    ToolConfiguration::builder()
        .set_tools(Some(tools))
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}

fn tool(name: &str, description: &str, schema: serde_json::Value) -> Result<Tool, BedrockError> {
    let specification = ToolSpecification::builder()
        .name(name)
        .description(description)
        .input_schema(ToolInputSchema::Json(json_to_document(&schema)?))
        // Deliberately omit `strict`; not every supported Claude profile
        // accepts that newer Bedrock field.
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))?;
    Ok(Tool::ToolSpec(specification))
}

fn protocol_message_to_sdk(message: &ReportProtocolMessage) -> Result<Message, BedrockError> {
    let role = match message.role {
        ReportProtocolRole::User => ConversationRole::User,
        ReportProtocolRole::Assistant => ConversationRole::Assistant,
    };
    let content = message
        .content
        .iter()
        .map(protocol_block_to_sdk)
        .collect::<Result<Vec<_>, _>>()?;
    Message::builder()
        .role(role)
        .set_content(Some(content))
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}

fn protocol_block_to_sdk(block: &ReportProtocolBlock) -> Result<ContentBlock, BedrockError> {
    match block {
        ReportProtocolBlock::Text { text } => Ok(ContentBlock::Text(text.clone())),
        ReportProtocolBlock::ReasoningText { text, signature } => {
            let reasoning = ReasoningTextBlock::builder()
                .text(text)
                .set_signature(signature.clone())
                .build()
                .map_err(|error| BedrockError::Invocation(error.to_string()))?;
            Ok(ContentBlock::ReasoningContent(
                ReasoningContentBlock::ReasoningText(reasoning),
            ))
        }
        ReportProtocolBlock::ReasoningRedacted { data } => Ok(ContentBlock::ReasoningContent(
            ReasoningContentBlock::RedactedContent(Blob::new(data.clone())),
        )),
        ReportProtocolBlock::ToolUse {
            tool_use_id,
            name,
            input,
        } => {
            let tool = aws_sdk_bedrockruntime::types::ToolUseBlock::builder()
                .tool_use_id(tool_use_id)
                .name(name)
                .input(json_to_document(input)?)
                .build()
                .map_err(|error| BedrockError::Invocation(error.to_string()))?;
            Ok(ContentBlock::ToolUse(tool))
        }
        ReportProtocolBlock::ToolResult {
            tool_use_id,
            status,
            content,
        } => {
            let status = match status {
                ReportToolResultStatus::Success => ToolResultStatus::Success,
                ReportToolResultStatus::Error => ToolResultStatus::Error,
            };
            let result = ToolResultBlock::builder()
                .tool_use_id(tool_use_id)
                .content(ToolResultContentBlock::Json(json_to_document(content)?))
                .status(status)
                .build()
                .map_err(|error| BedrockError::Invocation(error.to_string()))?;
            Ok(ContentBlock::ToolResult(result))
        }
    }
}

fn map_stop_reason(reason: &aws_sdk_bedrockruntime::types::StopReason) -> ReportStopReason {
    use aws_sdk_bedrockruntime::types::StopReason;
    match reason {
        StopReason::ContentFiltered => ReportStopReason::ContentFiltered,
        StopReason::EndTurn => ReportStopReason::EndTurn,
        StopReason::GuardrailIntervened => ReportStopReason::GuardrailIntervened,
        StopReason::MalformedModelOutput => ReportStopReason::MalformedModelOutput,
        StopReason::MalformedToolUse => ReportStopReason::MalformedToolUse,
        StopReason::MaxTokens => ReportStopReason::MaxTokens,
        StopReason::ModelContextWindowExceeded => ReportStopReason::ModelContextWindowExceeded,
        StopReason::StopSequence => ReportStopReason::StopSequence,
        StopReason::ToolUse => ReportStopReason::ToolUse,
        _ => ReportStopReason::Unknown,
    }
}

fn json_to_document(value: &serde_json::Value) -> Result<Document, BedrockError> {
    match value {
        serde_json::Value::Null => Ok(Document::Null),
        serde_json::Value::Bool(value) => Ok(Document::Bool(*value)),
        serde_json::Value::String(value) => Ok(Document::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .map(json_to_document)
            .collect::<Result<Vec<_>, _>>()
            .map(Document::Array),
        serde_json::Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), json_to_document(value)?)))
            .collect::<Result<HashMap<_, _>, BedrockError>>()
            .map(Document::Object),
        serde_json::Value::Number(value) => {
            let number = if let Some(value) = value.as_u64() {
                Number::PosInt(value)
            } else if let Some(value) = value.as_i64() {
                Number::NegInt(value)
            } else if let Some(value) = value.as_f64() {
                if !value.is_finite() {
                    return Err(BedrockError::Serialization(serde_json::Error::io(
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "non-finite JSON number",
                        ),
                    )));
                }
                Number::Float(value)
            } else {
                return Err(BedrockError::SchemaViolation(
                    "JSON number could not be represented for Bedrock".to_string(),
                ));
            };
            Ok(Document::Number(number))
        }
    }
}

fn document_to_json(document: &Document) -> Result<serde_json::Value, BedrockError> {
    match document {
        Document::Null => Ok(serde_json::Value::Null),
        Document::Bool(value) => Ok(serde_json::Value::Bool(*value)),
        Document::String(value) => Ok(serde_json::Value::String(value.clone())),
        Document::Array(values) => values
            .iter()
            .map(document_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        Document::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), document_to_json(value)?)))
            .collect::<Result<serde_json::Map<_, _>, BedrockError>>()
            .map(serde_json::Value::Object),
        Document::Number(Number::PosInt(value)) => Ok(serde_json::Value::Number((*value).into())),
        Document::Number(Number::NegInt(value)) => Ok(serde_json::Value::Number((*value).into())),
        Document::Number(Number::Float(value)) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                BedrockError::ResponseParse(
                    "Bedrock document contained a non-finite number".to_string(),
                )
            }),
    }
}
