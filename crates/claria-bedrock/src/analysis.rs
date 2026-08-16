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
        MAX_PLAN_SCOPE_CHARACTERS,
    },
    turn_usage::TurnUsage,
};
use serde::{Deserialize, Serialize};

use crate::{
    converse::{self, CachePlan, StopSignal},
    error::BedrockError,
    report::{self, ReportStopReason},
};

/// The analysis tool set's wire type, re-exported so an orchestrating crate
/// can hold one between calls without taking a direct dependency on the
/// Bedrock SDK. Wire types are this crate's job.
pub use aws_sdk_bedrockruntime::types::ToolConfiguration as AnalysisToolConfiguration;

pub const SUBMIT_SECTION_PLAN_TOOL: &str = "submit_section_plan";
pub const SUBMIT_RESUME_PLAN_TOOL: &str = "submit_resume_plan";
pub const SUBMIT_REVIEW_ROWS_TOOL: &str = "submit_review_rows";

/// Shortest span a review finding may anchor on. Shorter than a citation
/// quote's floor because a review points at a phrase inside one section, not
/// at a passage inside a whole corpus: "was administered" is a real tense
/// anchor, and the section it must be unique within is a few paragraphs long.
pub const MIN_REVIEW_QUOTE_CHARACTERS: usize = 5;

/// Ceiling for one finding's description. Well inside the persisted model's
/// own ceiling, which leaves the host room to append its anchor notes to a
/// description that arrived at the limit.
pub const MAX_REVIEW_DESCRIPTION_CHARACTERS: usize = 400;

/// Findings one property may raise about one section. A property that finds
/// ten separate problems in one section has found a section that needs
/// rewriting, not ten findings worth resolving one at a time.
pub const MAX_REVIEW_SECTION_FINDINGS: usize = 10;

const _: () = assert!(
    MAX_REVIEW_DESCRIPTION_CHARACTERS
        <= claria_core::models::findings::MAX_FINDING_DESCRIPTION_CHARACTERS,
    "a description the schema accepts must fit the description the store persists"
);
const _: () = assert!(
    MIN_REVIEW_QUOTE_CHARACTERS < MAX_CITATION_QUOTE_CHARACTERS,
    "review quote bounds must describe a non-empty range"
);

/// The seven properties one review sweep fans out over, one request each.
///
/// The split is structural, not editorial. A style property's finding carries
/// an anchored replacement the clinician can apply and undo; a consistency
/// property's finding is a claim about the document that no replacement could
/// settle, so it has no write path at all — the persisted model refuses the
/// combination, and [`Self::pass`] is the one place the mapping lives.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReviewProperty {
    TenseDrift,
    Terminology,
    Transitions,
    Redundancy,
    InternalContradiction,
    UnsupportedClaim,
    CrossSectionConflict,
}

impl ReviewProperty {
    /// Every property, in the order the fan-out runs them. The first is the
    /// warming branch: it writes the cache prefix the other six read.
    pub const ALL: [Self; 7] = [
        Self::TenseDrift,
        Self::Terminology,
        Self::Transitions,
        Self::Redundancy,
        Self::InternalContradiction,
        Self::UnsupportedClaim,
        Self::CrossSectionConflict,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TenseDrift => "tense_drift",
            Self::Terminology => "terminology",
            Self::Transitions => "transitions",
            Self::Redundancy => "redundancy",
            Self::InternalContradiction => "internal_contradiction",
            Self::UnsupportedClaim => "unsupported_claim",
            Self::CrossSectionConflict => "cross_section_conflict",
        }
    }

    /// Whether findings from this property carry an applicable replacement.
    pub const fn is_style(self) -> bool {
        matches!(
            self,
            Self::TenseDrift | Self::Terminology | Self::Transitions | Self::Redundancy
        )
    }
}

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
    /// Called with each fragment of the forced tool's input as it streams,
    /// for callers that would otherwise show a spinner for minutes.
    ///
    /// Display only, and deliberately raw: the fragments are partial JSON,
    /// so the caller may count structure off them and must not parse, store,
    /// or log them. What reaches the reader is the caller's own decision,
    /// which is also where the cadence is set — this fires as often as the
    /// service sends bytes.
    pub on_partial_tool_input: Option<&'a (dyn Fn(&str) + Send + Sync)>,
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

    /// A budget seeded from a count already taken against a request of the
    /// same shape.
    ///
    /// The review fan-out is the reason this exists: seven branches send
    /// byte-identical system blocks and a byte-identical draft block, and
    /// differ only in a few hundred tokens of instruction. Counting each of
    /// them separately would spend seven `CountTokens` calls to learn the same
    /// number, so the warming branch counts once and the siblings estimate
    /// forward from its result — the incremental rule the review rules already
    /// require of a multi-call tool loop.
    pub fn seeded(
        model_id: &str,
        output_token_reserve: u32,
        verified_tokens: u32,
        verified_chars: u64,
    ) -> Self {
        let budget_tokens = analysis_input_token_budget(model_id, output_token_reserve);
        Self {
            inner: converse::InputTokenBudget::seeded(
                budget_tokens,
                verified_tokens,
                verified_chars,
            ),
            budget_tokens,
        }
    }

    /// The input budget this analysis call runs under, so a caller sizing the
    /// corpus against it reads the same number the call will enforce.
    pub const fn budget_tokens(&self) -> u32 {
        self.budget_tokens
    }

    /// The exact count this budget has taken, if it has taken one, as
    /// `(input_tokens, request_characters)` — the seed for [`Self::seeded`].
    pub fn verified(&self) -> Option<(u32, u64)> {
        self.inner.verified()
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
/// while it is produced. The stream loop also awaits the stop signal beside
/// the next frame, so Stop does not have to wait for a model that is
/// thinking.
///
/// Partial tool input is handed to `on_partial_tool_input` as it arrives, for
/// display only — a caller can count rows off it and say how far the answer
/// has got. It is never a result: a half-written plan is not a plan, and
/// validation still happens on the whole document once the stream closes.
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
        on_partial_tool_input,
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
    let budget_tokens = budget.budget_tokens();
    budget
        .inner
        .ensure_within(
            report::protocol_chars(&system.join("\n"), messages),
            || async {
                let measured =
                    report::count_report_tokens(config, &client, model_id, token_request).await?;
                // The allowance is logged when the budget is built; this is
                // what the request actually measured against it, which is the
                // number a console export needs to answer "how close was it?".
                // Counts and model IDs only — never corpus or prompt text.
                tracing::info!(
                    target: "claria_bedrock::budget",
                    operation,
                    model_id,
                    measured_input_tokens = measured,
                    input_budget_tokens = budget_tokens,
                    "analysis input measured"
                );
                Ok(measured)
            },
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

    let bounds = converse::StreamBounds::analysis();
    let started = std::time::Instant::now();
    // How long the service took to produce the first frame, filled in the
    // moment it does. It is the number that separates "the model is still
    // reading the corpus" from "the request never landed", and the failure
    // paths below report it too.
    let mut first_token_ms: Option<u64> = None;
    let streamed = async {
        let response = converse::start_converse_stream(
            operation,
            bounds,
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
        first_token_ms = Some(elapsed_ms(started));

        let mut stream = response.stream;
        let mut collector = report::ReportStreamCollector::default();
        let stopped = loop {
            let event = tokio::select! {
                biased;
                () = stop.stopped() => break true,
                event = converse::recv_stream_event(operation, bounds, &mut stream) => event?,
            };
            let Some(event) = event else { break false };
            if let Some(fragment) = collector.absorb(event)
                && let Some(on_partial_tool_input) = on_partial_tool_input
            {
                on_partial_tool_input(&fragment);
            }
        };
        if stopped {
            // Dropping the stream closes the connection so the model stops
            // producing a structured answer nobody is going to read.
            drop(stream);
            return Err(BedrockError::Stopped);
        }
        collector.finish()
    }
    .await;

    let (content, wire_stop_reason, wire_usage) = match streamed {
        Ok(streamed) => {
            log_call_timing(operation, model_id, first_token_ms, started, "completed");
            streamed
        }
        Err(error) => {
            let outcome = if matches!(error, BedrockError::Stopped) {
                "stopped"
            } else {
                "failed"
            };
            log_call_timing(operation, model_id, first_token_ms, started, outcome);
            if matches!(error, BedrockError::Stopped) {
                tracing::info!(operation, model_id, "analysis call stopped by the reader");
            }
            return Err(error);
        }
    };
    let latency_ms = Some(elapsed_ms(started));
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

fn elapsed_ms(since: std::time::Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Timing line for one attempt of an analysis call, on every exit.
///
/// The retry layer re-sends a call that never landed, so "how long did the
/// turn take" is not the same question as "how long did this attempt wait" —
/// and a stall shows up as a missing `first_token_ms` beside a full-length
/// `attempt_elapsed_ms`, which is the difference between a request the
/// service never began and one whose socket died mid-answer. Fields are
/// labels, model IDs, and durations — never prompt, corpus, or response
/// content.
fn log_call_timing(
    operation: &'static str,
    model_id: &str,
    first_token_ms: Option<u64>,
    started: std::time::Instant,
    outcome: &'static str,
) {
    tracing::info!(
        target: "claria_bedrock::budget",
        operation,
        model_id,
        first_token_ms,
        attempt_elapsed_ms = elapsed_ms(started),
        outcome,
        "analysis call finished"
    );
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
                 section IDs. Every row states what the section must assert and names the \
                 records it must be written from; a row that skips a section states why \
                 instead. This is an outline, not a draft: never copy record text into it.",
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
            report::tool(
                SUBMIT_REVIEW_ROWS_TOOL,
                "Submit this review pass's verdict on the drafted report. Called exactly once, \
                 for the single property named in \"property\" and no other. Return exactly one \
                 row per section listed in the supplied drafted-section block, in the order that \
                 block lists them, and include a row with status \"no_issues\" for every section \
                 this property is clean in — an omitted row is not read as \"nothing found\", it \
                 fails validation and costs the pass a correction round. Copy every section_id \
                 exactly; never invent, merge, reorder, or omit a section. Every quote you return \
                 is searched for in the text of the section it belongs to, with runs of \
                 whitespace treated as equal; a quote you have corrected, shortened, reflowed, or \
                 paraphrased matches nothing and the finding is discarded. The style properties \
                 (tense_drift, terminology, transitions, redundancy) must attach a \"replacement\" \
                 to every finding. The consistency properties (internal_contradiction, \
                 unsupported_claim, cross_section_conflict) report only: a consistency finding \
                 that carries a \"replacement\" is rejected outright.",
                review_rows_schema(),
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
            "Up to {MAX_PLAN_EVIDENCE} records the section must be written from, most decisive first."
        )
    } else {
        format!(
            "Up to {MAX_PLAN_EVIDENCE} records the section must be written from, most decisive first. May be empty only when action is \"skip\"."
        )
    };
    serde_json::json!({
        "type": "array",
        "maxItems": MAX_PLAN_EVIDENCE,
        "description": description,
        "items": {
            "type": "object",
            "required": ["filename"],
            "additionalProperties": false,
            "properties": {
                "filename": {
                    "type": "string", "minLength": 1, "maxLength": 1024,
                    "description": "A filename copied exactly from the record corpus in the untrusted context. The host checks it against that corpus; an invented or approximated name is dropped from the plan."
                },
                "relevance": {
                    "type": "string", "minLength": 1, "maxLength": MAX_EVIDENCE_RELEVANCE_CHARACTERS,
                    "description": format!(
                        "One line on why this record matters to this section, in at most {MAX_EVIDENCE_RELEVANCE_CHARACTERS} Unicode characters. A pointer, not a quotation: do not copy record text here."
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

/// A quote field: the same bounds and the same "whitespace may differ,
/// nothing else may" contract wherever a review returns copied text.
fn review_quote_schema(what: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "minLength": MIN_REVIEW_QUOTE_CHARACTERS,
        "maxLength": MAX_CITATION_QUOTE_CHARACTERS,
        "description": format!(
            "{what} Between {MIN_REVIEW_QUOTE_CHARACTERS} and {MAX_CITATION_QUOTE_CHARACTERS} Unicode characters, copied verbatim. Whitespace may differ; nothing else may."
        )
    })
}

fn review_span_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["quote"],
        "additionalProperties": false,
        "description": "Where in THIS section the problem is.",
        "properties": {
            "quote": review_quote_schema(
                "The passage in this section the finding is about, copied from this section's own text."
            ),
            "block_index": {
                "type": "integer",
                "minimum": 0,
                "description": "Optional zero-based index of the block the quote came from, numbered as the drafted-section block numbers this section's blocks. A hint only: the host searches that block first and then the rest of the section, so a wrong index costs nothing and a missing one costs nothing."
            }
        }
    })
}

fn review_conflicting_span_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["quote"],
        "additionalProperties": false,
        "description": "The passage this finding conflicts with. Consistency properties only.",
        "properties": {
            "section_id": {
                "type": "string", "minLength": 36, "maxLength": 36,
                "description": "The 36-character UUID of the section holding the conflicting passage, copied exactly. Omit it when the conflict is inside the same section as the span above."
            },
            "quote": review_quote_schema("The conflicting passage, copied from the section it is in.")
        }
    })
}

fn review_record_citation_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["filename", "quote"],
        "additionalProperties": false,
        "description": "The record text that settles the point. Consistency properties only.",
        "properties": {
            "filename": {
                "type": "string", "minLength": 1, "maxLength": 1024,
                "description": "A filename copied exactly from the record corpus in the untrusted context."
            },
            "quote": review_quote_schema("The span of that file that settles the point, copied verbatim.")
        }
    })
}

fn review_replacement_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["find", "replace"],
        "additionalProperties": false,
        "description": "The anchored edit that fixes this finding. Required on every finding from a style property (tense_drift, terminology, transitions, redundancy), and never valid on a consistency property (internal_contradiction, unsupported_claim, cross_section_conflict), which reports only.",
        "properties": {
            "find": {
                "type": "string",
                "minLength": MIN_REVIEW_QUOTE_CHARACTERS,
                "maxLength": MAX_CITATION_QUOTE_CHARACTERS,
                "description": format!(
                    "The exact text to replace, copied character for character from this section and occurring exactly once in it. {MIN_REVIEW_QUOTE_CHARACTERS}\u{2013}{MAX_CITATION_QUOTE_CHARACTERS} Unicode characters. Widen it until it is unique rather than pointing at a word that repeats."
                )
            },
            "replace": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_CITATION_QUOTE_CHARACTERS,
                "description": format!(
                    "The text to put in its place, at most {MAX_CITATION_QUOTE_CHARACTERS} Unicode characters. It must differ from \"find\", and it must not be empty \u{2014} shorten a passage by rewriting it, not by deleting it."
                )
            }
        }
    })
}

fn review_finding_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["span", "description"],
        "additionalProperties": false,
        "properties": {
            "span": review_span_schema(),
            "description": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_REVIEW_DESCRIPTION_CHARACTERS,
                "description": format!(
                    "What is wrong and why, in at most {MAX_REVIEW_DESCRIPTION_CHARACTERS} Unicode characters. Written for the clinician who will act on it."
                )
            },
            "conflicting_span": review_conflicting_span_schema(),
            "record_citation": review_record_citation_schema(),
            "replacement": review_replacement_schema()
        }
    })
}

fn review_row_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["section_id", "status", "findings"],
        "additionalProperties": false,
        "properties": {
            "section_id": section_id_schema(),
            "status": {
                "enum": ["no_issues", "findings"],
                "description": "\"no_issues\" means you read this section for this property and it is clean; send an empty findings array with it. \"findings\" means the findings array below is not empty."
            },
            "findings": {
                "type": "array",
                "maxItems": MAX_REVIEW_SECTION_FINDINGS,
                "description": format!(
                    "Up to {MAX_REVIEW_SECTION_FINDINGS} findings about this section, strongest first. Empty when status is \"no_issues\"."
                ),
                "items": review_finding_schema()
            }
        }
    })
}

/// The union review tool's schema.
///
/// One schema for all seven properties, because the tool list is the first
/// tier Bedrock invalidates: a per-property tool would move that tier and
/// cost every branch of the fan-out the corpus prefix its sibling paid to
/// write. Which fields are legal for which property is therefore stated in
/// the descriptions and enforced by the host, not by the shape.
fn review_rows_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["property", "rows"],
        "additionalProperties": false,
        "properties": {
            "property": {
                "enum": ReviewProperty::ALL.map(ReviewProperty::as_str),
                "description": "The single property this pass was asked about. It must equal the property named in the instruction; a row set submitted under any other property is rejected."
            },
            "rows": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_REPORT_SECTIONS,
                "description": "Exactly one row per drafted section, in the order the drafted-section block lists them, including every section where this property found nothing.",
                "items": review_row_schema()
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewRowsRequest {
    pub property: ReviewProperty,
    pub rows: Vec<ReviewRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewRow {
    pub section_id: String,
    pub status: ReviewRowStatus,
    /// Defaulted rather than required so a clean row that omits the empty
    /// array is accepted. The schema still asks for it, because "send an
    /// empty array" is a clearer contract than "omit the field".
    #[serde(default)]
    pub findings: Vec<ReviewRowFinding>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRowStatus {
    NoIssues,
    Findings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewRowFinding {
    pub span: ReviewSpan,
    pub description: String,
    #[serde(default)]
    pub conflicting_span: Option<ReviewConflictingSpan>,
    #[serde(default)]
    pub record_citation: Option<ReviewRecordCitation>,
    #[serde(default)]
    pub replacement: Option<ReviewReplacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewSpan {
    pub quote: String,
    /// A hint the host searches first, not an address it trusts.
    #[serde(default)]
    pub block_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewConflictingSpan {
    /// `None` means the conflicting passage is in the same section.
    #[serde(default)]
    pub section_id: Option<String>,
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewRecordCitation {
    pub filename: String,
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewReplacement {
    pub find: String,
    pub replace: String,
}

/// [`decode_section_plan`] for the review tool.
pub fn decode_review_rows(input: &serde_json::Value) -> Result<ReviewRowsRequest, BedrockError> {
    serde_json::from_value(input.clone())
        .map_err(|error| BedrockError::SchemaViolation(error.to_string()))
}
