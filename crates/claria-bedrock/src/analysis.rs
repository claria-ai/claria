//! The analysis request family: one forced tool call, one structured answer.
//!
//! Planning a document and reviewing one are the same shape of request. A
//! model is handed the record corpus and the report's structure, asked one
//! question, and made to answer through a tool whose schema is the contract —
//! never free text a host would have to parse structure out of. Everything
//! about that shape lives here: the shared tool configuration, the forced
//! call, and the typed rows that come back.
//!
//! Two properties hold across the whole family and are the reason it is one
//! module rather than one function per role:
//!
//! - **The prefix is shared.** The corpus and the template ride in the
//!   *system* blocks with the cache point after them, so a plan call and a
//!   review call on the same model read the same cached prefix even though
//!   they force different tools — Bedrock invalidates tools, then system,
//!   then messages, and `tool_choice` sits in the messages tier.
//! - **Nothing is tuned.** No adaptive thinking, no effort, no temperature.
//!   These calls have to be reproducible to be cacheable and checkable: the
//!   same corpus must produce the same request bytes, and a plan the host
//!   validates must be the plan a repair round can be compared against.
//!   Every knob is left at the model's default rather than gated per family,
//!   which also means the capability table never has to be consulted here.

use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConverseTokensRequest, InferenceConfiguration, SpecificToolChoice,
    SystemContentBlock, Tool, ToolChoice, ToolConfiguration,
};
use claria_core::models::{
    report::{MAX_REPORT_SECTIONS, ReportProtocolMessage, validate_report_conversation},
    report_run::{
        MAX_CITATION_QUOTE_CHARACTERS, MAX_EVIDENCE_RELEVANCE_CHARACTERS, MAX_PLAN_EVIDENCE,
        MAX_PLAN_SCOPE_CHARACTERS, MIN_CITATION_QUOTE_CHARACTERS,
    },
    turn_usage::TurnUsage,
};
use serde::{Deserialize, Serialize};

use crate::{
    converse::{self, CachePlan, StopSignal},
    error::BedrockError,
    report::{self, ReportStopReason},
};

pub const SUBMIT_SECTION_PLAN_TOOL: &str = "submit_section_plan";
pub const SUBMIT_RESUME_PLAN_TOOL: &str = "submit_resume_plan";

/// Ceiling for the planner's one-line reason behind a resume decision. It is
/// read by the host, never persisted: a free-text rationale about a section
/// can quote the record it came from, and the run object already records the
/// decision itself.
pub const MAX_RESUME_REASON_CHARACTERS: usize = 300;

/// What one forced analysis call produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredCallOutput {
    /// The forced tool's input, exactly as the model submitted it. Decoding
    /// it into a typed row set is the caller's job, because the caller is the
    /// one that can put the serde diagnostic back in front of the model.
    pub input: serde_json::Value,
    /// Bedrock's correlation handle for the call. A caller that rejects the
    /// answer needs it to address the error `tool_result` its repair round
    /// sends back.
    pub tool_use_id: String,
    /// Bedrock may omit usage; absence is preserved rather than metered as
    /// zero.
    pub usage: Option<TurnUsage>,
    /// Always `ToolUse` on success — kept so the caller's usage receipt
    /// records what the wire actually said.
    pub stop_reason: ReportStopReason,
    pub latency_ms: Option<u64>,
}

/// One forced-tool analysis request.
///
/// The system blocks are where the untrusted corpus and template go: they sit
/// above the cache point, they are identical for every role, and they are the
/// expensive part. `messages` carries only the question.
pub struct StructuredCallRequest<'a> {
    pub model_id: &'a str,
    /// System blocks in order. The cache point, when the plan asks for one,
    /// goes after the last of them.
    pub system: &'a [String],
    pub messages: &'a [ReportProtocolMessage],
    /// The whole analysis tool set, shared across roles so the tools tier of
    /// the cache never moves. The forced choice is applied on top.
    pub tools: &'a ToolConfiguration,
    /// The one tool this call must produce. It has to be declared in `tools`.
    pub forced_tool: &'a str,
    pub max_tokens: u32,
    pub cache_plan: CachePlan,
    /// Fired by the reader's Stop button; a default signal never fires.
    pub stop: &'a StopSignal,
    /// Fixed request-family label for the usage log — `"report_plan"`,
    /// `"report_review"`. Never derived from user input.
    pub operation: &'static str,
}

/// Per-call input-token budget for an analysis request.
///
/// One real `CountTokens` against the exact request shape, then character
/// estimates for anything appended (the repair round). The reserve is the
/// caller's, because how much output one of these calls needs depends on what
/// it is being asked for.
pub struct AnalysisInputBudget {
    inner: converse::InputTokenBudget,
    budget_tokens: u32,
}

impl AnalysisInputBudget {
    pub fn new(model_id: &str, operation: &'static str, output_token_reserve: u32) -> Self {
        let budget_tokens = analysis_input_token_budget(model_id, output_token_reserve);
        converse::log_model_budget(operation, model_id, budget_tokens, output_token_reserve);
        Self {
            inner: converse::InputTokenBudget::exact(budget_tokens),
            budget_tokens,
        }
    }

    /// The input budget this analysis call runs under, so a caller sizing the
    /// corpus against it reads the same number the call will enforce.
    pub const fn budget_tokens(&self) -> u32 {
        self.budget_tokens
    }
}

/// Input tokens an analysis call may spend on `model_id`: the capability
/// table's context window less the caller's output reserve. Derived, so a
/// reserve raised in one place cannot leave the budget claiming the old one.
pub fn analysis_input_token_budget(model_id: &str, output_token_reserve: u32) -> u32 {
    claria_core::model_id::ModelCapabilities::for_id(model_id)
        .context_window_tokens
        .saturating_sub(output_token_reserve)
}

/// Send one analysis request and return the forced tool's input.
///
/// Streamed rather than unary for the same reason the writer is: the response
/// is a large structured document and the connection has to carry frames
/// while it is produced. Nothing is forwarded incrementally — a half-written
/// plan is not a plan — but the stream loop awaits the stop signal beside the
/// next frame, so Stop does not have to wait for a model that is thinking.
///
/// The contract is deliberately narrow. Exactly one call to `forced_tool` and
/// a `tool_use` stop is the only success. `max_tokens` is a typed failure
/// rather than a salvage: truncated forced-tool JSON is not repairable and
/// half a plan validated as a whole plan would delete sections. Every other
/// stop reason is a protocol surprise for a call the service was told it had
/// no choice about.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(
        model_id = %request.model_id,
        forced_tool = %request.forced_tool,
        system_blocks = request.system.len(),
        messages = request.messages.len()
    )
)]
pub async fn converse_structured(
    config: &aws_config::SdkConfig,
    request: StructuredCallRequest<'_>,
    budget: &mut AnalysisInputBudget,
) -> Result<StructuredCallOutput, BedrockError> {
    let StructuredCallRequest {
        model_id,
        system,
        messages,
        tools,
        forced_tool,
        max_tokens,
        cache_plan,
        stop,
        operation,
    } = request;

    if !report::model_supports_report_tools(model_id) {
        return Err(BedrockError::UnsupportedModel(format!(
            "{model_id} is not a verified tool-capable Claude model"
        )));
    }
    validate_report_conversation(messages)
        .map_err(|error| BedrockError::SchemaViolation(error.to_string()))?;
    let tools = with_forced_tool(tools, forced_tool)?;

    let client = converse::runtime_client(config);
    let sdk_messages = messages
        .iter()
        .map(report::protocol_message_to_sdk)
        .collect::<Result<Vec<_>, _>>()?;
    let system_blocks: Vec<SystemContentBlock> = system
        .iter()
        .map(|text| SystemContentBlock::Text(text.clone()))
        .collect();

    // One exact count of the whole request shape — policy, corpus, template,
    // question, tool schemas — before the call goes out. Cache points are
    // excluded because they do not change the token count.
    let token_request = ConverseTokensRequest::builder()
        .set_messages(Some(sdk_messages.clone()))
        .set_system(Some(system_blocks.clone()))
        .tool_config(tools.clone())
        .build();
    budget
        .inner
        .ensure_within(
            report::protocol_chars(&system.join("\n"), messages),
            || report::count_report_tokens(config, &client, model_id, token_request),
            |input_tokens, input_token_budget| BedrockError::ContextBudgetExceeded {
                model_id: model_id.to_string(),
                input_tokens,
                input_token_budget,
            },
        )
        .await?;

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
        let mut blocks = system_blocks;
        blocks.push(SystemContentBlock::CachePoint(converse::cache_point(
            cache_ttl,
        )?));
        blocks
    } else {
        system_blocks
    };

    let started = std::time::Instant::now();
    let response = converse::start_converse_stream(
        "analysis ConverseStream",
        client
            .converse_stream()
            .model_id(model_id)
            .set_system(Some(system_blocks))
            .set_messages(Some(converse_messages))
            .tool_config(tools)
            // No tuning fields: see the module docs. The ceiling is still set
            // and still branched on, exactly as every other Converse call must.
            .inference_config(
                InferenceConfiguration::builder()
                    .max_tokens(max_tokens as i32)
                    .build(),
            )
            .send(),
    )
    .await?;

    let mut stream = response.stream;
    let mut collector = report::ReportStreamCollector::default();
    let stopped = loop {
        let event = tokio::select! {
            biased;
            () = stop.stopped() => break true,
            event = converse::recv_stream_event("analysis ConverseStream", &mut stream) => event?,
        };
        let Some(event) = event else { break false };
        collector.absorb(event);
    };
    if stopped {
        // Dropping the stream closes the connection so the model stops
        // producing a structured answer nobody is going to read.
        drop(stream);
        tracing::info!(operation, model_id, "analysis call stopped by the reader");
        return Err(BedrockError::Stopped);
    }

    let (content, wire_stop_reason, wire_usage) = collector.finish()?;
    let latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    let stop_reason = report::map_stop_reason(&wire_stop_reason);
    let usage = converse::optional_usage(wire_usage.as_ref(), model_id, cache_plan.effective_ttl());
    converse::log_turn_usage(
        operation,
        model_id,
        usage.as_ref(),
        Some(stop_reason.as_str()),
        latency_ms,
        max_tokens,
    );

    if stop_reason == ReportStopReason::MaxTokens {
        return Err(BedrockError::ResponseTruncated {
            max_output_tokens: max_tokens,
        });
    }
    if stop_reason != ReportStopReason::ToolUse {
        return Err(BedrockError::ResponseParse(format!(
            "the forced {forced_tool} call stopped for {} instead of tool use",
            stop_reason.as_str()
        )));
    }

    let mut calls = content.iter().filter_map(|block| match block {
        ContentBlock::ToolUse(tool) => Some(tool),
        _ => None,
    });
    let Some(call) = calls.next() else {
        return Err(BedrockError::ResponseParse(format!(
            "the response stopped for tool use without calling {forced_tool}"
        )));
    };
    if calls.next().is_some() {
        return Err(BedrockError::ResponseParse(format!(
            "the forced {forced_tool} call returned more than one tool use"
        )));
    }
    if call.name() != forced_tool {
        return Err(BedrockError::ResponseParse(format!(
            "the forced call returned {} instead of {forced_tool}",
            call.name()
        )));
    }

    let tool_use_id = call.tool_use_id().to_string();
    if tool_use_id.trim().is_empty() {
        return Err(BedrockError::ResponseParse(format!(
            "the forced {forced_tool} call carried an empty tool-use ID"
        )));
    }

    Ok(StructuredCallOutput {
        input: converse::document_to_json(call.input())?,
        tool_use_id,
        usage,
        stop_reason,
        latency_ms,
    })
}

/// Re-issue the shared tool configuration with `forced_tool` selected.
///
/// The tool list is copied verbatim so the tools tier of the prompt cache
/// stays byte-identical across roles; only `toolChoice` differs. A name that
/// is not in the list fails here rather than as a service `ValidationException`
/// with no hint about which caller asked for it.
fn with_forced_tool(
    tools: &ToolConfiguration,
    forced_tool: &str,
) -> Result<ToolConfiguration, BedrockError> {
    let declared = tools.tools().iter().any(|tool| match tool {
        Tool::ToolSpec(specification) => specification.name() == forced_tool,
        _ => false,
    });
    if !declared {
        return Err(BedrockError::SchemaViolation(format!(
            "{forced_tool} is not declared in the analysis tool configuration"
        )));
    }
    let choice = ToolChoice::Tool(
        SpecificToolChoice::builder()
            .name(forced_tool)
            .build()
            .map_err(|error| BedrockError::Invocation(error.to_string()))?,
    );
    ToolConfiguration::builder()
        .set_tools(Some(tools.tools().to_vec()))
        .tool_choice(choice)
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}

// ── Tool schemas ────────────────────────────────────────────────────────────

/// The whole analysis tool set, with no `toolChoice`.
///
/// One configuration for every analysis role, because a differing tool list
/// would move the tools tier of the prompt cache and cost each role the
/// corpus prefix the others paid to write. [`converse_structured`] stamps the
/// per-call `toolChoice` on top of this.
pub fn analysis_tool_configuration() -> Result<ToolConfiguration, BedrockError> {
    ToolConfiguration::builder()
        .set_tools(Some(vec![
            report::tool(
                SUBMIT_SECTION_PLAN_TOOL,
                "Submit the drafting plan for this report. Called exactly once. Return exactly \
                 one row per section listed in the supplied template structure, in the order \
                 that structure lists them — no extra rows, no omitted rows, no invented \
                 section IDs. Every row states what the section must assert and which record \
                 quotes support it; a row that skips a section states why instead.",
                section_plan_schema(),
            )?,
            report::tool(
                SUBMIT_RESUME_PLAN_TOOL,
                "Submit the plan for RESUMING a drafting run that was interrupted. Sections \
                 already drafted are durable and shown to you with their state: decide \"keep\" \
                 to leave one exactly as it stands, \"rewrite\" to replace a drafted or failed \
                 section with a new draft, \"draft\" to write one that was never written, or \
                 \"skip\" to leave one out of this document. Called exactly once, with exactly \
                 one row per section in the supplied state table and no invented section IDs.",
                resume_plan_schema(),
            )?,
        ]))
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}

fn section_id_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string", "minLength": 36, "maxLength": 36,
        "description": "The 36-character section UUID copied exactly from the supplied template structure. Never invent, abbreviate, or modify an ID."
    })
}

fn evidence_schema(required: bool) -> serde_json::Value {
    let description = if required {
        format!(
            "Up to {MAX_PLAN_EVIDENCE} record quotes the section must be grounded in, strongest first."
        )
    } else {
        format!(
            "Up to {MAX_PLAN_EVIDENCE} record quotes the section must be grounded in, strongest first. May be empty only when action is \"skip\"."
        )
    };
    serde_json::json!({
        "type": "array",
        "maxItems": MAX_PLAN_EVIDENCE,
        "description": description,
        "items": {
            "type": "object",
            "required": ["filename", "quote"],
            "additionalProperties": false,
            "properties": {
                "filename": {
                    "type": "string", "minLength": 1, "maxLength": 1024,
                    "description": "A filename copied exactly from the record corpus in the untrusted context. Invented or approximated names are rejected."
                },
                "quote": {
                    "type": "string",
                    "minLength": MIN_CITATION_QUOTE_CHARACTERS,
                    "maxLength": MAX_CITATION_QUOTE_CHARACTERS,
                    "description": format!(
                        "A span of {MIN_CITATION_QUOTE_CHARACTERS}–{MAX_CITATION_QUOTE_CHARACTERS} Unicode characters copied verbatim from that file. The host searches for it in the record text; a paraphrased, corrected, or reflowed quote fails that search and the evidence is dropped."
                    )
                },
                "relevance": {
                    "type": "string", "minLength": 1, "maxLength": MAX_EVIDENCE_RELEVANCE_CHARACTERS,
                    "description": format!(
                        "Optional. Why this quote matters to this section, in at most {MAX_EVIDENCE_RELEVANCE_CHARACTERS} Unicode characters."
                    )
                }
            }
        }
    })
}

fn section_plan_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["rows"],
        "additionalProperties": false,
        "properties": {
            "rows": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_REPORT_SECTIONS,
                "description": "Exactly one row per section in the supplied template structure, in template order.",
                "items": {
                    "type": "object",
                    "required": ["section_id", "action", "scope", "evidence"],
                    "additionalProperties": false,
                    "properties": {
                        "section_id": section_id_schema(),
                        "action": {
                            "enum": ["draft", "skip"],
                            "description": "\"draft\" to write this section in this run, \"skip\" to leave it out of the document."
                        },
                        "scope": {
                            "type": "string", "minLength": 1, "maxLength": MAX_PLAN_SCOPE_CHARACTERS,
                            "description": format!(
                                "What this section must assert, in at most {MAX_PLAN_SCOPE_CHARACTERS} Unicode characters — or, when action is \"skip\", why it is being left out."
                            )
                        },
                        "evidence": evidence_schema(false)
                    }
                }
            }
        }
    })
}

fn resume_plan_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["rows"],
        "additionalProperties": false,
        "properties": {
            "rows": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_REPORT_SECTIONS,
                "description": "Exactly one row per section in the supplied state table, in the order that table lists them.",
                "items": {
                    "type": "object",
                    "required": ["section_id", "decision", "reason"],
                    "additionalProperties": false,
                    "properties": {
                        "section_id": section_id_schema(),
                        "decision": {
                            "enum": ["keep", "rewrite", "draft", "skip"],
                            "description": "\"keep\" leaves an already-drafted section exactly as it stands and is valid only for a section whose state is drafted. \"rewrite\" replaces an existing or failed section with a new draft. \"draft\" writes a section that was never written. \"skip\" leaves the section out of the document."
                        },
                        "reason": {
                            "type": "string", "minLength": 1, "maxLength": MAX_RESUME_REASON_CHARACTERS,
                            "description": format!(
                                "Why this decision, in at most {MAX_RESUME_REASON_CHARACTERS} Unicode characters. Read by the host and not stored; do not put the section's content here."
                            )
                        },
                        "scope": {
                            "type": "string", "minLength": 1, "maxLength": MAX_PLAN_SCOPE_CHARACTERS,
                            "description": format!(
                                "What the section must assert, in at most {MAX_PLAN_SCOPE_CHARACTERS} Unicode characters. Required when decision is \"rewrite\" or \"draft\"; omit it for \"keep\" and \"skip\"."
                            )
                        },
                        "evidence": evidence_schema(true)
                    }
                }
            }
        }
    })
}

// ── Typed rows ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SectionPlanRequest {
    pub rows: Vec<SectionPlanRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SectionPlanRow {
    pub section_id: String,
    pub action: PlanAction,
    pub scope: String,
    pub evidence: Vec<PlanEvidence>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanAction {
    Draft,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanEvidence {
    pub filename: String,
    pub quote: String,
    #[serde(default)]
    pub relevance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResumePlanRequest {
    pub rows: Vec<ResumePlanRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResumePlanRow {
    pub section_id: String,
    pub decision: ResumeDecision,
    pub reason: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub evidence: Option<Vec<PlanEvidence>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResumeDecision {
    Keep,
    Rewrite,
    Draft,
    Skip,
}

/// Decode a forced `submit_section_plan` input, returning the serde
/// diagnostic verbatim so the caller's repair round can hand the model the
/// exact field it got wrong. These messages name JSON structure, never record
/// content.
pub fn decode_section_plan(input: &serde_json::Value) -> Result<SectionPlanRequest, BedrockError> {
    serde_json::from_value(input.clone())
        .map_err(|error| BedrockError::SchemaViolation(error.to_string()))
}

/// [`decode_section_plan`] for the resume tool.
pub fn decode_resume_plan(input: &serde_json::Value) -> Result<ResumePlanRequest, BedrockError> {
    serde_json::from_value(input.clone())
        .map_err(|error| BedrockError::SchemaViolation(error.to_string()))
}
