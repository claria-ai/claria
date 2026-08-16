//! Bedrock Converse integration for the opt-in Writing workflow.
//!
//! This module is deliberately separate from `chat`: every request carries a
//! real Bedrock tool configuration, and every response preserves tool-use
//! blocks for a controller-managed agent loop. It never falls back to the
//! text-only Chat path.

use std::collections::HashSet;

use std::collections::BTreeMap;

use aws_sdk_bedrockruntime::types::{
    ContentBlock, ContentBlockDelta, ContentBlockStart, ConversationRole, ConverseStreamOutput,
    ConverseTokensRequest, InferenceConfiguration, Message, ReasoningContentBlock,
    ReasoningContentBlockDelta, ReasoningTextBlock, StopReason, SystemContentBlock, Tool,
    ToolConfiguration, ToolInputSchema, ToolResultBlock, ToolResultContentBlock, ToolResultStatus,
    ToolSpecification, ToolUseBlock,
};
use aws_smithy_types::Blob;
use claria_core::models::{
    report::{
        MAX_BULLET_ITEM_CHARACTERS, MAX_BULLET_ITEMS, MAX_PARAGRAPH_CHARACTERS,
        MAX_TABLE_CELL_CHARACTERS, MAX_TABLE_COLUMNS, MAX_TABLE_ROWS, ReportProtocolBlock,
        ReportProtocolMessage, ReportProtocolRole, ReportToolResultStatus,
        validate_report_conversation,
    },
    turn_usage::TurnUsage,
};
use serde::{Deserialize, Serialize};

use crate::{converse, error::BedrockError};

pub const LIST_RECORD_FILES_TOOL: &str = "list_record_files";
pub const READ_RECORD_FILE_TOOL: &str = "read_record_file";
pub const PROPOSE_REPORT_CHANGES_TOOL: &str = "propose_report_changes";
pub const SET_FULL_DRAFT_TITLE_TOOL: &str = "set_full_draft_title";
pub const WRITE_FULL_DRAFT_SECTION_TOOL: &str = "write_full_draft_section";
pub const SKIP_FULL_DRAFT_SECTION_TOOL: &str = "skip_full_draft_section";
pub const MARK_SECTION_FAILED_TOOL: &str = "mark_section_failed";
pub const FINISH_FULL_DRAFT_TOOL: &str = "finish_full_draft";
pub const DEFAULT_MAX_TOOL_USES_PER_RESPONSE: usize = 80;
pub const MAX_TOOL_USES_PER_RESPONSE: usize = 100;

/// Output-token reserve for one report Converse call. This is both the
/// `max_tokens` sent on the wire and the amount subtracted from the model's
/// context window to form the input budget — the reserve is enforced, not
/// aspirational. Sized so one response can carry a full replacement of a
/// long clinical section (a maximal 200-block section is ≈25k tokens);
/// the 8k reserve shipped in v0.20–v0.23 forced multi-turn splitting and
/// measurably degraded report flow. Truncation at this ceiling still
/// surfaces as a MaxTokens stop handled by the caller's salvage path.
pub const REPORT_OUTPUT_TOKEN_RESERVE: u32 = 32_768;

/// Proposal wire-schema ceilings mirror the domain validators in
/// `claria-core`, so the tool schema never rejects a proposal the domain
/// would accept. Shrinking these below the domain caps is a known quality
/// regression — the model plans its edits around the advertised schema.
/// Smaller caps do not prevent oversized responses either: those surface as
/// a MaxTokens stop and are handled by the caller's truncation-salvage path.
pub const MAX_PROPOSAL_OPERATIONS: usize = claria_core::models::report::MAX_PROPOSAL_OPERATIONS;
/// Maximum blocks per proposed section — mirrors the domain validator.
pub const MAX_SECTION_BLOCKS: usize = claria_core::models::report::MAX_SECTION_BLOCKS;
/// Citation and failure-note ceilings, mirrored from the drafting-run
/// validators so the advertised schema never rejects what the domain accepts.
pub const MAX_SECTION_CITATIONS: usize = claria_core::models::report_run::MAX_SECTION_CITATIONS;
pub const MIN_CITATION_QUOTE_CHARACTERS: usize =
    claria_core::models::report_run::MIN_CITATION_QUOTE_CHARACTERS;
pub const MAX_CITATION_QUOTE_CHARACTERS: usize =
    claria_core::models::report_run::MAX_CITATION_QUOTE_CHARACTERS;
pub const MAX_SECTION_ERROR_CHARACTERS: usize =
    claria_core::models::report_run::MAX_SECTION_ERROR_CHARACTERS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportConverseOutput {
    pub message: ReportProtocolMessage,
    pub stop_reason: ReportStopReason,
    pub tool_calls: Vec<ReportToolCall>,
    /// Bedrock may omit usage. Callers must preserve that incompleteness
    /// instead of recording a misleading metered zero.
    pub usage: Option<TurnUsage>,
    /// Wall-clock time of the Converse round trip; `None` on records
    /// written before this field existed.
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

impl ReportConverseOutput {
    /// True when the stop reason disagrees with tool presence in a way that
    /// has no meaning of its own — a `tool_use` stop without tool calls, or
    /// tool calls beside a stop that claims the model finished normally.
    /// The loop attempts a corrective round on this before failing.
    ///
    /// `max_tokens` beside complete tool calls is NOT a mismatch: it means
    /// the response was truncated after the calls were emitted, and the loop
    /// executes them so the model can continue. Terminal stops (filtered,
    /// guardrail, malformed, context overflow) keep their own failure
    /// handling regardless of tool presence.
    pub fn stop_tool_mismatch(&self) -> bool {
        match self.stop_reason {
            ReportStopReason::ToolUse => self.tool_calls.is_empty(),
            ReportStopReason::EndTurn
            | ReportStopReason::StopSequence
            | ReportStopReason::Unknown => !self.tool_calls.is_empty(),
            _ => false,
        }
    }
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

impl ReportStopReason {
    /// Stable snake_case label matching the serde representation, for
    /// PHI-free telemetry fields.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContentFiltered => "content_filtered",
            Self::EndTurn => "end_turn",
            Self::GuardrailIntervened => "guardrail_intervened",
            Self::MalformedModelOutput => "malformed_model_output",
            Self::MalformedToolUse => "malformed_tool_use",
            Self::MaxTokens => "max_tokens",
            Self::ModelContextWindowExceeded => "model_context_window_exceeded",
            Self::StopSequence => "stop_sequence",
            Self::ToolUse => "tool_use",
            Self::Unknown => "unknown",
        }
    }
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
    SetFullDraftTitle(SetFullDraftTitleRequest),
    WriteFullDraftSection(WriteFullDraftSectionRequest),
    SkipFullDraftSection(SkipFullDraftSectionRequest),
    MarkSectionFailed(MarkSectionFailedRequest),
    FinishFullDraft(FinishFullDraftRequest),
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
#[serde(deny_unknown_fields)]
pub struct SetFullDraftTitleRequest {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WriteFullDraftSectionRequest {
    /// Copy an existing section ID from the supplied report structure, or use
    /// `null` when adding a genuinely new section.
    #[serde(default)]
    pub section_id: Option<String>,
    pub position: u32,
    pub heading: String,
    pub blocks: Vec<ReportBlockRequest>,
    /// Record quotes the section's claims rest on. Optional, and validated at
    /// commit against the run's record snapshot.
    #[serde(default)]
    pub citations: Option<Vec<RecordCitationRequest>>,
}

/// One record quote the writer attributes to a named record file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordCitationRequest {
    pub filename: String,
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkipFullDraftSectionRequest {
    /// An existing section ID copied from the supplied report structure —
    /// only supplied sections can be deferred, never new ones.
    pub section_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MarkSectionFailedRequest {
    /// A planned section's 36-character UUID, copied exactly.
    pub section_id: String,
    /// Why the section cannot be drafted from the available records.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FinishFullDraftRequest {
    pub summary: String,
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
        SET_FULL_DRAFT_TITLE_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::SetFullDraftTitle)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        WRITE_FULL_DRAFT_SECTION_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::WriteFullDraftSection)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        SKIP_FULL_DRAFT_SECTION_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::SkipFullDraftSection)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        MARK_SECTION_FAILED_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::MarkSectionFailed)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        FINISH_FULL_DRAFT_TOOL => serde_json::from_value(call.input.clone())
            .map(ReportToolRequest::FinishFullDraft)
            .map_err(|error| BedrockError::SchemaViolation(error.to_string())),
        name => Err(BedrockError::SchemaViolation(format!(
            "unknown report tool: {name}"
        ))),
    }
}

/// Per-turn input-token budget for the report loop.
///
/// The first Converse call of a turn runs one real `CountTokens` against the
/// exact request shape; every later call in the same turn estimates the
/// appended messages at ~4 chars/token and re-verifies with a real count
/// only when the estimate comes within ~10% of the budget. Create one per
/// turn and pass it to every [`converse_report_with_tool_limit`] call.
pub struct ReportInputBudget {
    inner: converse::InputTokenBudget,
}

impl ReportInputBudget {
    pub fn new(model_id: &str) -> Self {
        // One budget per writer turn, so this is also the once-per-turn
        // record of the context window the capability table resolved.
        let input_budget_tokens = report_input_token_budget(model_id);
        converse::log_model_budget(
            "report",
            model_id,
            input_budget_tokens,
            REPORT_OUTPUT_TOKEN_RESERVE,
        );
        Self {
            inner: converse::InputTokenBudget::exact(input_budget_tokens),
        }
    }
}

/// Send one report-protocol Converse request with all three report tools using
/// the default per-response tool-use limit and a fresh single-call budget.
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
        &mut ReportInputBudget::new(model_id),
        converse::ModelTuning::default(),
        converse::CachePlan::report_default(claria_core::model_id::ModelCapabilities::for_id(
            model_id,
        )),
        &converse::StopSignal::new(),
    )
    .await
}

/// Send one targeted-edit Converse request with a caller-selected tool-use
/// limit.
///
/// The caller owns the bounded tool loop, persistence, and — via
/// `cache_plan` — where the request's `cachePoint` blocks go. This function
/// owns the exact Bedrock wire shape, validates safe tool correlation, and
/// returns the response usage priced at this individual call's rates.
///
/// `stop` ends the call where it stands; see
/// [`converse_report_with_tool_set`] for what a stopped response costs.
#[allow(clippy::too_many_arguments)]
pub async fn converse_report_with_tool_limit(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ReportProtocolMessage],
    max_tool_uses_per_response: usize,
    budget: &mut ReportInputBudget,
    tuning: converse::ModelTuning,
    cache_plan: converse::CachePlan,
    stop: &converse::StopSignal,
) -> Result<ReportConverseOutput, BedrockError> {
    converse_report_with_tool_set(
        config,
        model_id,
        system_prompt,
        messages,
        max_tool_uses_per_response,
        budget,
        ReportToolSet::TargetedEdit,
        tuning,
        cache_plan,
        stop,
    )
    .await
}

/// Send one full-draft Converse request. The full-draft tool set writes an
/// isolated candidate section by section and finalizes it; it cannot stage a
/// targeted proposal or fetch records because the host supplies the complete
/// readable-record snapshot in the initial context.
#[allow(clippy::too_many_arguments)]
pub async fn converse_full_report_with_tool_limit(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ReportProtocolMessage],
    max_tool_uses_per_response: usize,
    budget: &mut ReportInputBudget,
    tuning: converse::ModelTuning,
    cache_plan: converse::CachePlan,
    stop: &converse::StopSignal,
) -> Result<ReportConverseOutput, BedrockError> {
    converse_report_with_tool_set(
        config,
        model_id,
        system_prompt,
        messages,
        max_tool_uses_per_response,
        budget,
        ReportToolSet::FullDraft,
        tuning,
        cache_plan,
        stop,
    )
    .await
}

#[derive(Debug, Clone, Copy)]
enum ReportToolSet {
    TargetedEdit,
    FullDraft,
}

impl ReportToolSet {
    /// The request-family label this tool set's turns carry — on the shared
    /// usage/cache log line, on the stream watchdog's failures, and in the
    /// error prose a reader is shown — so a console export separates
    /// targeted edits from whole-draft runs without parsing anything else.
    fn operation(self) -> &'static str {
        match self {
            Self::TargetedEdit => "report_targeted",
            Self::FullDraft => "report_full_draft",
        }
    }
}

#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(
        model_id = %model_id,
        messages = messages.len(),
        max_tool_uses_per_response,
        tool_set = ?tool_set
    )
)]
/// Send one report-protocol Converse request and fold its streamed response
/// back into a whole assistant message.
///
/// `stop` is watched alongside every frame — a writer stream can sit silent
/// for minutes while a large prefix prefills, so a Stop that waits for the
/// next frame is not a Stop. Unlike chat, a stopped writer response is
/// discarded WHOLE rather than salvaged: a half-streamed response can hold a
/// tool call whose input JSON is still being assembled, and half a
/// `write_full_draft_section` is not a section. The loop's existing
/// invariant — a call that fails commits nothing, because the protocol only
/// ever grows by complete messages — is what makes throwing it away safe.
#[allow(clippy::too_many_arguments)]
async fn converse_report_with_tool_set(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ReportProtocolMessage],
    max_tool_uses_per_response: usize,
    budget: &mut ReportInputBudget,
    tool_set: ReportToolSet,
    tuning: converse::ModelTuning,
    cache_plan: converse::CachePlan,
    stop: &converse::StopSignal,
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
    let tools = report_tool_configuration(tool_set)?;
    let system = SystemContentBlock::Text(system_prompt.to_string());

    // Budget check: an exact CountTokens of the full Converse shape
    // (policy, messages, tool schemas) on the turn's first call; appended
    // messages are estimated afterward and re-verified only near the budget.
    // Cache points are excluded — they do not change the token count.
    let token_request = ConverseTokensRequest::builder()
        .set_messages(Some(sdk_messages.clone()))
        .system(system.clone())
        .tool_config(tools.clone())
        .build();
    budget
        .inner
        .ensure_within(
            protocol_chars(system_prompt, messages),
            || count_report_tokens(config, &client, model_id, token_request),
            |input_tokens, input_token_budget| BedrockError::ContextBudgetExceeded {
                model_id: model_id.to_string(),
                input_tokens,
                input_token_budget,
            },
        )
        .await?;

    // Prompt caching, exactly where the caller's plan says: each point makes
    // the prefix ahead of it readable from cache on the next call in the
    // loop instead of re-paying full input rates. The plan already resolved
    // model capability, the user's setting, and the tier, so this applies it
    // and decides nothing.
    let cache_ttl = converse::sdk_cache_ttl(cache_plan.ttl);
    let converse_messages = converse::with_cache_points_after(
        sdk_messages,
        &cache_plan.after_blocks,
        cache_ttl.clone(),
    )?;
    let converse_messages = if cache_plan.tail {
        converse::with_tail_cache_point(converse_messages, cache_ttl.clone())?
    } else {
        converse_messages
    };
    let system_blocks = if cache_plan.after_system {
        vec![
            system,
            SystemContentBlock::CachePoint(converse::cache_point(cache_ttl)?),
        ]
    } else {
        vec![system]
    };

    // Streamed, not unary: the writer's output ceiling is far above the
    // point where a non-streaming Converse call risks an HTTP timeout while
    // the model generates. Nothing is forwarded incrementally — the loop
    // needs a whole message before it can execute tool calls — but the
    // connection carries frames throughout instead of idling.
    let operation = tool_set.operation();
    let stream_bounds = converse::StreamBounds::conversational();
    let started = std::time::Instant::now();
    let response = converse::start_converse_stream(
        operation,
        stream_bounds,
        client
            .converse_stream()
            .model_id(model_id)
            .set_system(Some(system_blocks))
            .set_messages(Some(converse_messages))
            .tool_config(tools)
            .inference_config(
                InferenceConfiguration::builder()
                    .max_tokens(REPORT_OUTPUT_TOKEN_RESERVE as i32)
                    .set_temperature(tuning.temperature)
                    .build(),
            )
            .set_additional_model_request_fields(converse::additional_request_fields(tuning)?)
            .send(),
    )
    .await?;

    let mut stream = response.stream;
    let mut collector = ReportStreamCollector::default();
    loop {
        let event = tokio::select! {
            biased;
            () = stop.stopped() => {
                // Dropping the stream closes the connection, so the model
                // stops generating a response nobody is going to read.
                drop(stream);
                tracing::info!(model_id, "report stream stopped by the reader");
                return Err(BedrockError::Stopped);
            }
            event = converse::recv_stream_event(operation, stream_bounds, &mut stream) => event?,
        };
        let Some(event) = event else { break };
        collector.absorb(event);
    }
    let (streamed_content, streamed_stop_reason, streamed_usage) = collector.finish()?;
    let latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));

    let mut content = Vec::with_capacity(streamed_content.len());
    let mut tool_calls = Vec::new();
    let mut tool_ids: HashSet<String> = messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|block| match block {
            ReportProtocolBlock::ToolUse { tool_use_id, .. } => Some(tool_use_id.clone()),
            _ => None,
        })
        .collect();
    for block in &streamed_content {
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

    // A stop reason that disagrees with tool presence (e.g. `end_turn` next
    // to a tool use) is deliberately NOT an error here: the report loop
    // attempts a corrective tool-result round before failing, so the
    // inconsistency must reach it as data via `stop_tool_mismatch`.
    let stop_reason = map_stop_reason(&streamed_stop_reason);

    let usage = converse::optional_usage(
        streamed_usage.as_ref(),
        model_id,
        cache_plan.effective_ttl(),
    );
    converse::log_turn_usage(
        operation,
        model_id,
        usage.as_ref(),
        Some(stop_reason.as_str()),
        latency_ms,
        REPORT_OUTPUT_TOKEN_RESERVE,
    );

    Ok(ReportConverseOutput {
        message: ReportProtocolMessage {
            role: ReportProtocolRole::Assistant,
            content,
            created_at: jiff::Timestamp::now(),
        },
        stop_reason,
        tool_calls,
        usage,
        latency_ms,
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

/// Run one real `CountTokens` for the report request shape.
///
/// Converse uses the cross-region inference profile selected in the UI,
/// while CountTokens requires the underlying foundation model identifier, so
/// only a known scope prefix is stripped. Some newly listed Claude
/// foundations support Converse before CountTokens; those fall back to the
/// newest active Haiku tokenizer while retaining the selected model's
/// conservative context-window limit.
pub(crate) async fn count_report_tokens(
    config: &aws_config::SdkConfig,
    client: &aws_sdk_bedrockruntime::Client,
    model_id: &str,
    token_request: ConverseTokensRequest,
) -> Result<u32, BedrockError> {
    let counting_model_id = claria_core::model_id::strip_scope_prefix(model_id);
    match converse::count_input_tokens(client, counting_model_id, token_request.clone()).await {
        Ok(tokens) => Ok(tokens),
        Err(error)
            if matches!(
                &error,
                BedrockError::Service { code, .. } if code == "ValidationException"
            ) =>
        {
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
            converse::count_input_tokens(client, fallback_model_id, token_request)
                .await
                .map_err(|fallback_error| {
                    tracing::error!(
                        model_id,
                        fallback_model_id,
                        error = %fallback_error,
                        "report CountTokens fallback failed"
                    );
                    fallback_error
                })
        }
        Err(error) => {
            tracing::error!(model_id, error = %error, "report CountTokens call failed");
            Err(error)
        }
    }
}

/// Character measure of the request used for incremental token estimation.
/// Only message/system content counts — the fixed tool schemas are covered
/// by the turn's initial exact count.
pub(crate) fn protocol_chars(system_prompt: &str, messages: &[ReportProtocolMessage]) -> u64 {
    let mut chars = system_prompt.len() as u64;
    for message in messages {
        for block in &message.content {
            chars += match block {
                ReportProtocolBlock::Text { text } => text.len(),
                ReportProtocolBlock::ReasoningText { text, .. } => text.len(),
                ReportProtocolBlock::ReasoningRedacted { data } => data.len(),
                ReportProtocolBlock::ToolUse { name, input, .. } => {
                    name.len() + input.to_string().len()
                }
                ReportProtocolBlock::ToolResult { content, .. } => content.to_string().len(),
            } as u64;
        }
    }
    chars
}

fn report_tool_configuration(tool_set: ReportToolSet) -> Result<ToolConfiguration, BedrockError> {
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
                        "type": "string", "minLength": 1, "maxLength": MAX_PARAGRAPH_CHARACTERS,
                        "description": format!("Plain paragraph text, at most {MAX_PARAGRAPH_CHARACTERS} characters. No markdown markup is rendered.")
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
                        "maxItems": MAX_BULLET_ITEMS,
                        "items": {"type": "string", "minLength": 1, "maxLength": MAX_BULLET_ITEM_CHARACTERS},
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
                        "maxItems": MAX_TABLE_ROWS,
                        "items": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_TABLE_COLUMNS,
                            "items": {"type": "string", "maxLength": MAX_TABLE_CELL_CHARACTERS}
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
                        "maxItems": MAX_TABLE_COLUMNS,
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
                                "heading": heading_schema.clone(),
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
                                "section_id": section_id_schema.clone()
                            }
                        }
                    ]
                }
            }
        }
    });
    let full_title_schema = serde_json::json!({
        "type": "object",
        "required": ["title"],
        "additionalProperties": false,
        "properties": {
            "title": {
                "type": "string", "minLength": 1, "maxLength": 200,
                "description": "The complete title for the generated working draft."
            }
        }
    });
    let full_section_schema = serde_json::json!({
        "type": "object",
        "required": ["section_id", "position", "heading", "blocks"],
        "additionalProperties": false,
        "properties": {
            "section_id": {
                "anyOf": [section_id_schema, {"type": "null"}],
                "description": "Copy an existing 36-character section UUID exactly to fill that template/report section, or null only when adding a genuinely new section."
            },
            "position": {
                "type": "integer", "minimum": 0, "maximum": 100,
                "description": "0-based final position of this section in the complete generated report."
            },
            "heading": heading_schema,
            "blocks": {
                "type": "array", "minItems": 1, "maxItems": MAX_SECTION_BLOCKS,
                "items": block_schema,
                "description": "The complete blocks for this section. Calling the tool again with the returned section_id replaces the earlier staged version."
            },
            "citations": {
                "type": "array",
                "maxItems": MAX_SECTION_CITATIONS,
                "description": format!(
                    "Optional. Up to {MAX_SECTION_CITATIONS} record quotes this section's claims rest on."
                ),
                "items": {
                    "type": "object",
                    "required": ["filename", "quote"],
                    "additionalProperties": false,
                    "properties": {
                        "filename": {
                            "type": "string", "minLength": 1, "maxLength": 1024,
                            "description": "A filename copied exactly from the record snapshot in the untrusted context. Invented names are rejected."
                        },
                        "quote": {
                            "type": "string",
                            "minLength": MIN_CITATION_QUOTE_CHARACTERS,
                            "maxLength": MAX_CITATION_QUOTE_CHARACTERS,
                            "description": format!(
                                "A verbatim span of {MIN_CITATION_QUOTE_CHARACTERS}–{MAX_CITATION_QUOTE_CHARACTERS} Unicode characters copied from that file. Do not paraphrase."
                            )
                        }
                    }
                }
            }
        }
    });
    let mark_section_failed_schema = serde_json::json!({
        "type": "object",
        "required": ["section_id", "reason"],
        "additionalProperties": false,
        "properties": {
            "section_id": {
                "type": "string", "minLength": 36, "maxLength": 36,
                "description": "The 36-character UUID of a planned section, copied exactly from plan_context."
            },
            "reason": {
                "type": "string", "minLength": 1, "maxLength": MAX_SECTION_ERROR_CHARACTERS,
                "description": format!(
                    "Why this section cannot be drafted from the available records, in at most {MAX_SECTION_ERROR_CHARACTERS} characters. Name what is missing; do not quote client content."
                )
            }
        }
    });
    let skip_full_section_schema = serde_json::json!({
        "type": "object",
        "required": ["section_id"],
        "additionalProperties": false,
        "properties": {
            "section_id": {
                "type": "string", "minLength": 36, "maxLength": 36,
                "description": "The 36-character UUID of a supplied template/report section, copied exactly. Only supplied sections can be skipped — never pass an invented ID or the ID of a section you already wrote."
            }
        }
    });
    let finish_full_draft_schema = serde_json::json!({
        "type": "object",
        "required": ["summary"],
        "additionalProperties": false,
        "properties": {
            "summary": {
                "type": "string", "minLength": 1, "maxLength": 500,
                "description": "One or two plain sentences summarizing the completed full draft."
            }
        }
    });

    let tools = match tool_set {
        ReportToolSet::TargetedEdit => vec![
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
                 never claim a change was saved. If a response is cut off at the output-token limit, \
                 completed tool calls still execute and you can continue in the next response.",
                proposal_schema,
            )?,
        ],
        ReportToolSet::FullDraft => vec![
            tool(
                SET_FULL_DRAFT_TITLE_TOOL,
                "Set the complete working draft's title. Call this before finalizing, even when retaining the supplied title.",
                full_title_schema,
            )?,
            tool(
                WRITE_FULL_DRAFT_SECTION_TOOL,
                "Write one complete section into an isolated full-draft candidate. Work through \
                 plan_context in the order its entries are listed and write ONE section per \
                 response, then wait for the tool result before starting the next. Ground the \
                 section in the records that entry's evidence names; its scope is guidance, but its \
                 section_id is authoritative — copy it exactly, and pass null only for a genuinely \
                 new section. position is the 0-based final order. No section is saved to the \
                 working draft until finish_full_draft succeeds.",
                full_section_schema,
            )?,
            tool(
                SKIP_FULL_DRAFT_SECTION_TOOL,
                "Skip only sections the plan marks skip or that the user's guidance defers to a \
                 later pass — never to shorten the job. The section keeps its heading and place in \
                 the document, its body stays empty, and exports omit it until a later edit writes \
                 content into it. Every planned section must be written, skipped, or marked failed \
                 before finish_full_draft; a later write_full_draft_section with the same \
                 section_id overrides the skip.",
                skip_full_section_schema,
            )?,
            tool(
                MARK_SECTION_FAILED_TOOL,
                "Declare that one planned section cannot be drafted from the available records \
                 after a genuine attempt — the records needed for it are missing, unreadable, or \
                 contradict each other beyond what a draft can resolve. The section keeps its \
                 base-revision content and the run finishes with the failure visible to the user. \
                 Never use this to shorten the job, and never in place of skip_full_draft_section \
                 for a section the plan or the user deliberately deferred.",
                mark_section_failed_schema,
            )?,
            tool(
                FINISH_FULL_DRAFT_TOOL,
                "Validate and finalize the isolated full-draft candidate after the title has been \
                 set and every planned section has been written, explicitly skipped, or marked \
                 failed. A successful result authorizes the host to atomically save one new \
                 working-draft revision without a proposal gate.",
                finish_full_draft_schema,
            )?,
        ],
    };

    ToolConfiguration::builder()
        .set_tools(Some(tools))
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}

pub(crate) fn tool(
    name: &str,
    description: &str,
    schema: serde_json::Value,
) -> Result<Tool, BedrockError> {
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

pub(crate) fn protocol_message_to_sdk(
    message: &ReportProtocolMessage,
) -> Result<Message, BedrockError> {
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
                .content(ToolResultContentBlock::Json(converse::json_to_document(
                    content,
                )?))
                .status(status)
                .build()
                .map_err(|error| BedrockError::Invocation(error.to_string()))?;
            Ok(ContentBlock::ToolResult(result))
        }
    }
}

/// One content block being assembled from `ConverseStream` events, keyed by
/// its `contentBlockIndex`. Text, tool input, and reasoning all arrive split
/// across an arbitrary number of deltas, so every field accumulates.
#[derive(Debug, Default)]
struct PartialReportBlock {
    text: String,
    reasoning_text: String,
    reasoning_signature: Option<String>,
    reasoning_redacted: Option<Vec<u8>>,
    tool: Option<PartialToolUse>,
}

#[derive(Debug)]
struct PartialToolUse {
    tool_use_id: String,
    name: String,
    /// Concatenated partial JSON. The service streams tool input as text
    /// fragments that are only valid JSON once joined.
    input: String,
}

/// Folds a `ConverseStream` response back into the same `ContentBlock` list
/// the unary path returns, so the protocol validation below has exactly one
/// implementation regardless of transport.
#[derive(Debug, Default)]
pub struct ReportStreamCollector {
    blocks: BTreeMap<usize, PartialReportBlock>,
    stop_reason: Option<StopReason>,
    usage: Option<aws_sdk_bedrockruntime::types::TokenUsage>,
}

impl ReportStreamCollector {
    /// Absorb one stream event, returning the tool-input fragment it carried
    /// so a caller can tell the reader how far a structured answer has got.
    ///
    /// Tool input rather than text, because that is what the calls on this
    /// path produce: a forced tool's JSON, arriving a few characters at a
    /// time. The fragment is partial JSON and stays that way — it is worth
    /// counting rows off, never parsing, never logging, and never mistaking
    /// for the answer. [`Self::finish`] is still the only thing that
    /// validates.
    pub fn absorb(&mut self, event: ConverseStreamOutput) -> Option<String> {
        match event {
            ConverseStreamOutput::ContentBlockStart(event) => {
                let index = event.content_block_index() as usize;
                if let Some(ContentBlockStart::ToolUse(start)) = event.start() {
                    self.blocks.entry(index).or_default().tool = Some(PartialToolUse {
                        tool_use_id: start.tool_use_id().to_string(),
                        name: start.name().to_string(),
                        input: String::new(),
                    });
                }
                None
            }
            ConverseStreamOutput::ContentBlockDelta(event) => {
                let index = event.content_block_index() as usize;
                let delta = event.delta()?;
                let block = self.blocks.entry(index).or_default();
                match delta {
                    ContentBlockDelta::Text(text) => {
                        block.text.push_str(text);
                        None
                    }
                    ContentBlockDelta::ToolUse(tool) => {
                        // A tool delta before its start event would lose the
                        // ID and name; the service never does that, and the
                        // finalizer rejects a nameless tool if it ever does.
                        let partial = block.tool.as_mut()?;
                        partial.input.push_str(tool.input());
                        (!tool.input().is_empty()).then(|| tool.input().to_string())
                    }
                    ContentBlockDelta::ReasoningContent(reasoning) => {
                        match reasoning {
                            ReasoningContentBlockDelta::Text(text) => {
                                block.reasoning_text.push_str(text)
                            }
                            ReasoningContentBlockDelta::Signature(signature) => {
                                block.reasoning_signature = Some(signature.clone())
                            }
                            ReasoningContentBlockDelta::RedactedContent(data) => {
                                block.reasoning_redacted = Some(data.as_ref().to_vec())
                            }
                            _ => {}
                        }
                        None
                    }
                    _ => None,
                }
            }
            ConverseStreamOutput::MessageStop(event) => {
                self.stop_reason = Some(event.stop_reason().clone());
                None
            }
            ConverseStreamOutput::Metadata(event) => {
                self.usage = event.usage().cloned();
                None
            }
            _ => None,
        }
    }

    /// Rebuild the assistant message in wire order. A missing `messageStop`
    /// is a protocol error rather than a silently-complete turn.
    pub fn finish(
        self,
    ) -> Result<
        (
            Vec<ContentBlock>,
            StopReason,
            Option<aws_sdk_bedrockruntime::types::TokenUsage>,
        ),
        BedrockError,
    > {
        let stop_reason = self.stop_reason.ok_or_else(|| {
            BedrockError::ResponseParse(
                "the report response stream ended without a messageStop event".to_string(),
            )
        })?;

        let mut content = Vec::with_capacity(self.blocks.len());
        for (_, block) in self.blocks {
            if let Some(tool) = block.tool {
                // An empty input stream is a tool called with no arguments.
                let raw = if tool.input.trim().is_empty() {
                    "{}"
                } else {
                    tool.input.as_str()
                };
                let input: serde_json::Value = serde_json::from_str(raw).map_err(|error| {
                    BedrockError::ResponseParse(format!(
                        "tool use {} streamed input that is not valid JSON: {error}",
                        tool.tool_use_id
                    ))
                })?;
                content.push(ContentBlock::ToolUse(
                    ToolUseBlock::builder()
                        .tool_use_id(tool.tool_use_id)
                        .name(tool.name)
                        .input(converse::json_to_document(&input)?)
                        .build()
                        .map_err(|error| BedrockError::ResponseParse(error.to_string()))?,
                ));
                continue;
            }
            if let Some(data) = block.reasoning_redacted {
                content.push(ContentBlock::ReasoningContent(
                    ReasoningContentBlock::RedactedContent(Blob::new(data)),
                ));
                continue;
            }
            if !block.reasoning_text.is_empty() {
                content.push(ContentBlock::ReasoningContent(
                    ReasoningContentBlock::ReasoningText(
                        ReasoningTextBlock::builder()
                            .text(block.reasoning_text)
                            .set_signature(block.reasoning_signature)
                            .build()
                            .map_err(|error| BedrockError::ResponseParse(error.to_string()))?,
                    ),
                ));
                continue;
            }
            content.push(ContentBlock::Text(block.text));
        }

        Ok((content, stop_reason, self.usage))
    }
}

pub(crate) fn map_stop_reason(
    reason: &aws_sdk_bedrockruntime::types::StopReason,
) -> ReportStopReason {
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
