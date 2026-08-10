//! Bedrock Converse integration for the opt-in Writing workflow.
//!
//! This module is deliberately separate from `chat`: every request carries a
//! real Bedrock tool configuration, and every response preserves tool-use
//! blocks for a controller-managed agent loop. It never falls back to the
//! text-only Chat path.

use std::collections::HashSet;

use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, ConverseTokensRequest, InferenceConfiguration, Message,
    ReasoningContentBlock, ReasoningTextBlock, SystemContentBlock, Tool, ToolConfiguration,
    ToolInputSchema, ToolResultBlock, ToolResultContentBlock, ToolResultStatus, ToolSpecification,
};
use aws_smithy_types::Blob;
use claria_core::models::{
    report::{
        ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole, ReportToolResultStatus,
        validate_report_conversation,
    },
    turn_usage::TurnUsage,
};
use serde::{Deserialize, Serialize};

use crate::{converse, error::BedrockError};

pub const LIST_RECORD_FILES_TOOL: &str = "list_record_files";
pub const READ_RECORD_FILE_TOOL: &str = "read_record_file";
pub const PROPOSE_REPORT_CHANGES_TOOL: &str = "propose_report_changes";
pub const DEFAULT_MAX_TOOL_USES_PER_RESPONSE: usize = 80;
pub const MAX_TOOL_USES_PER_RESPONSE: usize = 100;

/// Output-token reserve for one report Converse call. This is both the
/// `max_tokens` sent on the wire and the amount subtracted from the model's
/// context window to form the input budget — the reserve is enforced, not
/// aspirational.
pub const REPORT_OUTPUT_TOKEN_RESERVE: u32 = 8_192;

/// Proposal schema ceilings, derived from [`REPORT_OUTPUT_TOKEN_RESERVE`]:
/// one response of ≈8k tokens (≈32k characters) must be able to carry a
/// maximal well-formed proposal, so per-call size limits stay small and the
/// tool description tells the model to split large rewrites across turns.
pub const MAX_PROPOSAL_OPERATIONS: u32 = 10;
/// Maximum blocks per proposed section — sized so a full section fits in
/// one [`REPORT_OUTPUT_TOKEN_RESERVE`]-bounded response.
pub const MAX_SECTION_BLOCKS: u32 = 50;

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
    Paragraph {
        text: String,
    },
    BulletList {
        items: Vec<String>,
    },
    Table {
        rows: Vec<Vec<String>>,
        has_header: bool,
        #[serde(default)]
        column_widths: Option<Vec<u16>>,
    },
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

/// Send one report-protocol Converse request with all three report tools using
/// the default per-response tool-use limit.
pub async fn converse_report(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ReportProtocolMessage],
) -> Result<ReportConverseOutput, BedrockError> {
    converse_report_with_tool_limit(
        config,
        model_id,
        system_prompt,
        messages,
        DEFAULT_MAX_TOOL_USES_PER_RESPONSE,
    )
    .await
}

/// Send one report-protocol Converse request with a caller-selected tool-use
/// limit.
///
/// The caller owns the bounded tool loop and persistence. This function owns
/// the exact Bedrock wire shape, validates safe tool correlation, and returns
/// the response usage priced at this individual call's rates.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(
        model_id = %model_id,
        messages = messages.len(),
        max_tool_uses_per_response
    )
)]
pub async fn converse_report_with_tool_limit(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ReportProtocolMessage],
    max_tool_uses_per_response: usize,
) -> Result<ReportConverseOutput, BedrockError> {
    if max_tool_uses_per_response == 0 || max_tool_uses_per_response > MAX_TOOL_USES_PER_RESPONSE {
        return Err(BedrockError::SchemaViolation(format!(
            "report tool-use limit must be between 1 and {MAX_TOOL_USES_PER_RESPONSE}"
        )));
    }
    if !model_supports_report_tools(model_id) {
        return Err(BedrockError::UnsupportedModel(format!(
            "{model_id} is not a verified tool-capable Claude model"
        )));
    }
    validate_report_conversation(messages)
        .map_err(|error| BedrockError::SchemaViolation(error.to_string()))?;

    let client = converse::runtime_client(config);
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
    let counting_model_id = claria_core::model_id::strip_scope_prefix(model_id);
    let input_tokens = match converse::count_input_tokens(
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
                BedrockError::Service { code, .. } if code == "ValidationException"
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
            converse::count_input_tokens(&client, fallback_model_id, token_request)
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
        .inference_config(
            InferenceConfiguration::builder()
                .max_tokens(REPORT_OUTPUT_TOKEN_RESERVE as i32)
                .build(),
        )
        .send()
        .await
        .map_err(|error| converse::classify_error("report Converse", error))?;

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
                let input = converse::document_to_json(tool.input())?;
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
    if tool_calls.len() > max_tool_uses_per_response {
        return Err(BedrockError::ResponseParse(format!(
            "report response requested {} tools; configured maximum is {max_tool_uses_per_response}",
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

    let usage = converse::optional_usage(response.usage(), model_id);

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
/// tool use. Capability is determined before invocation via the central
/// capability table; a generic AWS `ValidationException` is never guessed to
/// mean "tools unsupported".
pub fn model_supports_report_tools(model_id: &str) -> bool {
    claria_core::model_id::ModelCapabilities::for_id(model_id).report_tools
}

/// Conservative context window for the selected Bedrock model/profile, from
/// the central capability table.
pub fn report_model_context_tokens(model_id: &str) -> u32 {
    claria_core::model_id::ModelCapabilities::for_id(model_id).context_window_tokens
}

pub fn report_input_token_budget(model_id: &str) -> u32 {
    report_model_context_tokens(model_id).saturating_sub(REPORT_OUTPUT_TOKEN_RESERVE)
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
            "filename": {
                "type": "string", "minLength": 1, "maxLength": 1024,
                "description": "A filename copied exactly from a list_record_files result. Paths and invented names fail."
            },
            "offset": {
                "type": "integer", "minimum": 0,
                "description": "0-based start position in Unicode characters (not bytes). Defaults to 0. To continue a paginated read, pass the previous result's next_offset."
            },
            "limit": {
                "type": "integer", "minimum": 1, "maximum": 12000,
                "description": "Maximum Unicode characters to return (default 8000, maximum 12000). Reads also draw down the shared 48000-character per-turn budget."
            }
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
                    "text": {
                        "type": "string", "minLength": 1, "maxLength": 20000,
                        "description": "Plain paragraph text, at most 20000 characters. No markdown markup is rendered."
                    }
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
                        "items": {"type": "string", "minLength": 1, "maxLength": 2000},
                        "description": "One plain-text string per bullet, in display order."
                    }
                }
            },
            {
                "type": "object",
                "required": ["kind", "rows", "has_header"],
                "additionalProperties": false,
                "properties": {
                    "kind": {"enum": ["table"]},
                    "rows": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 200,
                        "items": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 20,
                            "items": {"type": "string", "maxLength": 5000}
                        },
                        "description": "Rows in display order; each row is an array of plain-text cells. Every row must have the same number of cells. Leave unknown cells as empty strings rather than inventing values."
                    },
                    "has_header": {
                        "type": "boolean",
                        "description": "true when rows[0] is a header row (rendered bold and shaded on export)."
                    },
                    "column_widths": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 20,
                        "items": {"type": "integer", "minimum": 1, "maximum": 10000},
                        "description": "Optional relative column widths (proportions of total table width, not absolute units); one entry per column. Omit for equal widths."
                    }
                }
            }
        ]
    });
    let section_id_schema = serde_json::json!({
        "type": "string", "minLength": 36, "maxLength": 36,
        "description": "The 36-character section UUID copied exactly from the accepted_report in the untrusted context. Never invent or modify an ID."
    });
    let heading_schema = serde_json::json!({
        "type": "string", "minLength": 1, "maxLength": 200,
        "description": "The section's full heading text."
    });
    let proposal_schema = serde_json::json!({
        "type": "object",
        "required": ["summary", "operations"],
        "additionalProperties": false,
        "properties": {
            "summary": {
                "type": "string", "minLength": 1, "maxLength": 500,
                "description": "One or two plain sentences telling the user what the proposal changes."
            },
            "operations": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_PROPOSAL_OPERATIONS,
                "description": "Operations applied in order against the accepted report shown in the untrusted context.",
                "items": {
                    "oneOf": [
                        {
                            "type": "object",
                            "required": ["kind", "title"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["set_title"]},
                                "title": {
                                    "type": "string", "minLength": 1, "maxLength": 200,
                                    "description": "The new report title."
                                }
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind", "position", "heading", "blocks"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["add_section"]},
                                "position": {
                                    "type": "integer", "minimum": 0, "maximum": 100,
                                    "description": "0-based insertion index in the report's current section list; 0 inserts before the first section, the current section count appends at the end."
                                },
                                "heading": heading_schema.clone(),
                                "blocks": {"type": "array", "maxItems": MAX_SECTION_BLOCKS, "items": block_schema.clone(),
                                    "description": "The new section's content blocks in display order."}
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind", "section_id", "heading", "blocks"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["replace_section"]},
                                "section_id": section_id_schema.clone(),
                                "heading": heading_schema,
                                "blocks": {"type": "array", "maxItems": MAX_SECTION_BLOCKS, "items": block_schema.clone(),
                                    "description": "The complete replacement content. The whole section — heading included — is replaced; unchanged blocks must be restated."}
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind", "section_id"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"enum": ["remove_section"]},
                                "section_id": section_id_schema
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
            "List the current client's record filenames and whether each file fits Claria's bounded \
             text reader. Printable UTF-8 originals are readable regardless of extension; documents \
             and recordings use generated text sidecars. Call this before any read_record_file call \
             — only listed filenames are readable.",
            list_schema,
        )?,
        tool(
            READ_RECORD_FILE_TOOL,
            "Read a bounded range from one filename returned by list_record_files. offset and limit \
             are Unicode characters, not bytes. All reads in a turn share a 48000-character budget; \
             when a result includes next_offset, pass it as offset to continue reading from there. \
             Binary originals without a generated text sidecar are safely rejected.",
            read_schema,
        )?,
        tool(
            PROPOSE_REPORT_CHANGES_TOOL,
            "Stage typed report changes for user review. Copy each section_id exactly from the \
             accepted_report in the untrusted context; positions are 0-based; replace_section \
             replaces the entire section including its heading. At most one call per turn — a \
             second call fails. Nothing is saved or applied until the user accepts the proposal, so \
             never claim a change was saved. Responses are limited to 8192 output tokens: keep one \
             proposal small and split large rewrites across multiple turns.",
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
        .input_schema(ToolInputSchema::Json(converse::json_to_document(&schema)?))
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
                .input(converse::json_to_document(input)?)
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
                .content(ToolResultContentBlock::Json(converse::json_to_document(content)?))
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

