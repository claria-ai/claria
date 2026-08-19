//! Report-writing orchestration for durable whole-report runs and targeted
//! proposals.
//!
//! This crate owns report workflow policy, bounded record tools, proposal
//! staging, the drafting-run executor, and the turn loop that drives them.
//! Durable writer state lives in `claria-report-store`, Bedrock wire details
//! stay in `claria-bedrock`, generic S3 operations stay in `claria-storage`,
//! and persisted domain types stay in `claria-core`.

mod context;
mod error;
mod full_draft_context;
mod gate;
mod parallel_draft;
mod plan;
mod prompt_cache;
mod prompts;
mod record_context;
mod review;
mod review_instructions;
mod run;
mod text;
mod tools;
mod turn;

use claria_core::models::report::{MAX_REPORT_TURNS, ReportWorkspace};
use claria_report_store::ReportAttemptMetadata;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use error::ReportPipelineError;
pub use gate::{
    CompletionCheck, CompletionCheckKind, CompletionReport, evaluate_report_completion,
};
pub use plan::{
    DETERMINISTIC_PLAN_MODEL_ID, DraftPlanOutcome, DraftPlanRequest, PLAN_BATCH_SECTIONS,
    PLAN_OUTPUT_TOKEN_RESERVE, PlanModels, generate_draft_plan, plan_draft_resume,
    resume_planned_draft_run, start_draft_run, update_draft_plan,
};
pub use prompt_cache::ReportPromptCache;
pub use prompts::{
    FULL_REPORT_SYSTEM_PROMPT_BODY, FULL_REPORT_TRUST_RULES, PLANNER_SYSTEM_PROMPT_BODY,
    PLANNER_TRUST_RULES, REPORT_SYSTEM_PROMPT_BODY, REPORT_TRUST_RULES, full_report_system_prompt,
    planner_system_prompt, report_system_prompt,
};
pub use review::{
    REVIEW_FAN_OUT_CONCURRENCY, REVIEW_OUTPUT_TOKEN_RESERVE, ReviewPropertyStatus,
    ReviewSweepOutcome, ReviewSweepRequest, run_review_sweeps,
};
pub use run::{
    PartialDraftFinalization, abandon_draft_run, finalize_partial_draft, load_resumable_draft_run,
    resume_draft_run,
};
pub use turn::{
    FullReportRequest, ReportMessageRequest, ReportTurnProgress, generate_full_report,
    generate_full_report_for_report, interruption_advice, send_report_message,
    send_report_message_for_report,
};

/// Bedrock requests one fan-out keeps in flight at once.
///
/// Bedrock meters a model by requests per minute against the whole account,
/// and a fanned-out branch is a single long request rather than a burst, so
/// the figure that matters is how many of these can be outstanding before the
/// account's other work starts being throttled. Three leaves headroom on the
/// smallest on-demand RPM allocations while still clearing a seven-branch
/// review in two waves; the throttle retry underneath absorbs the case where
/// an account is busier than that, and raising this would mostly buy more
/// retries.
///
/// Shared by the review sweep and the parallel drafting run, because the
/// account they both spend is the same one.
pub(crate) const BEDROCK_FAN_OUT_CONCURRENCY: usize = 3;

pub(crate) const DEFAULT_READ_LIMIT: u32 = 8_000;
pub(crate) const MAX_READ_LIMIT: u32 = 12_000;
pub(crate) const MAX_READ_CHARACTERS_PER_TURN: u32 = 48_000;
pub(crate) const MAX_LISTED_FILES: usize = 500;
pub(crate) const MAX_LIST_RESULT_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_TOOL_ROUNDS: u32 = 40;
pub const DEFAULT_MAX_CONVERSE_CALLS: u32 = 50;
pub const DEFAULT_MAX_TOOL_USES_PER_RESPONSE: u32 =
    claria_bedrock::report::DEFAULT_MAX_TOOL_USES_PER_RESPONSE as u32;
pub const DEFAULT_MAX_RETAINED_TURNS: u32 = MAX_REPORT_TURNS as u32;
pub const MAX_CONFIGURABLE_TOOL_ROUNDS: u32 = 100;
/// Every tool round costs one Converse call, and the turn always ends with
/// one more call that produces the final reply — hence rounds + 1, derived
/// so the two ceilings cannot drift apart.
pub const MAX_CONFIGURABLE_CONVERSE_CALLS: u32 = MAX_CONFIGURABLE_TOOL_ROUNDS + 1;
pub const MAX_CONFIGURABLE_TOOL_USES_PER_RESPONSE: u32 =
    claria_bedrock::report::MAX_TOOL_USES_PER_RESPONSE as u32;
pub const MAX_CONFIGURABLE_RETAINED_TURNS: u32 = MAX_REPORT_TURNS as u32;
pub const DEFAULT_WRITER_FIRST_FRAME_TIMEOUT_SECS: u32 =
    claria_bedrock::converse::DEFAULT_STREAM_FIRST_FRAME_TIMEOUT_SECS as u32;
pub const DEFAULT_WRITER_IDLE_TIMEOUT_SECS: u32 =
    claria_bedrock::converse::DEFAULT_STREAM_IDLE_TIMEOUT_SECS as u32;
pub const DEFAULT_ANALYSIS_FIRST_FRAME_TIMEOUT_SECS: u32 =
    claria_bedrock::converse::DEFAULT_ANALYSIS_STREAM_FIRST_FRAME_TIMEOUT_SECS as u32;
pub const DEFAULT_ANALYSIS_IDLE_TIMEOUT_SECS: u32 =
    claria_bedrock::converse::DEFAULT_ANALYSIS_STREAM_IDLE_TIMEOUT_SECS as u32;
pub const MAX_CONFIGURABLE_TIMEOUT_SECS: u32 =
    claria_bedrock::converse::MAX_STREAM_TIMEOUT_SECS as u32;
pub const DEFAULT_WRITER_MAX_OUTPUT_TOKENS: u32 =
    claria_bedrock::report::DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE;
pub const MIN_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS: u32 =
    claria_bedrock::report::MIN_REPORT_OUTPUT_TOKEN_RESERVE;
pub const MAX_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS: u32 =
    claria_bedrock::report::MAX_REPORT_OUTPUT_TOKEN_RESERVE;
pub(crate) const MAX_REPORT_REFERENCES: usize = 10;

/// Where a clinician raises the guardrails below, and the exact field labels
/// they will read there. These mirror `WRITER_LIMIT_FIELDS` in
/// `pages/Preferences.tsx`; renaming a field there means renaming it here, or
/// the failure message sends the reader looking for a control that is gone.
pub(crate) const WRITER_LIMITS_SECTION: &str =
    "Preferences \u{2192} Document Writer \u{2192} Writer Limits";
pub(crate) const TOOL_ROUNDS_FIELD_LABEL: &str = "Tool-use rounds per request";
pub(crate) const CONVERSE_CALLS_FIELD_LABEL: &str = "Bedrock calls per request";
pub const FIRST_FRAME_TIMEOUT_FIELD_LABEL: &str = "Wait for the writer to start responding";
pub const IDLE_TIMEOUT_FIELD_LABEL: &str = "Wait between writer response chunks";
pub const WRITER_MAX_OUTPUT_TOKENS_FIELD_LABEL: &str = "Writer response length ceiling";
pub const ANALYSIS_FIRST_FRAME_TIMEOUT_FIELD_LABEL: &str =
    "Wait for the planner and reviewer to start responding";
pub const ANALYSIS_IDLE_TIMEOUT_FIELD_LABEL: &str =
    "Wait between planner and reviewer response chunks";

/// Appended to the visible reply when the final response hit its
/// output-token limit after a proposal was already staged: the staged work
/// survives instead of the whole turn being discarded.
pub const REPORT_TRUNCATED_NOTICE: &str = "Claria note: the assistant reached its \
response-length limit, so this reply was cut short. The staged proposal is complete \
and awaiting your review.";

/// User-configurable guardrails for one report-writing request and its retained
/// conversation context.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportTurnLimits {
    max_tool_rounds: u32,
    max_converse_calls: u32,
    max_tool_uses_per_response: u32,
    max_retained_turns: u32,
    runtime: BedrockRuntimeLimits,
}

/// The low-level dials one Bedrock call is made under, as opposed to the
/// guardrails above that bound how many calls a request may make.
///
/// Separate from [`ReportTurnLimits`]'s counts, and named rather than
/// positional, because these are times and token counts: six bare `u32`
/// arguments in a row is a transposition waiting to happen. Every field has
/// a compile-time default that reproduces the behaviour these values were
/// frozen at, so a host that sets none of them changes nothing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BedrockRuntimeLimits {
    /// Longest wait for a writer call to start answering.
    pub writer_first_frame_timeout_secs: u32,
    /// Longest silence inside a writer response already under way.
    pub writer_idle_timeout_secs: u32,
    /// `max_tokens` for one writer call, and the reserve subtracted from the
    /// model's window to form its input allowance.
    pub writer_max_output_tokens: u32,
    /// Longest wait for a planner or reviewer call to start answering.
    pub analysis_first_frame_timeout_secs: u32,
    /// Longest silence inside a planner or reviewer response already under
    /// way.
    pub analysis_idle_timeout_secs: u32,
}

impl BedrockRuntimeLimits {
    /// Reject anything that would make every call fail rather than making a
    /// slow one survive: a zero wait, a wait past the point where a dead
    /// socket has to be declared dead, or an output ceiling outside what a
    /// Claude model will produce.
    pub fn validate(self) -> Result<(), ReportPipelineError> {
        for (label, secs) in [
            (
                FIRST_FRAME_TIMEOUT_FIELD_LABEL,
                self.writer_first_frame_timeout_secs,
            ),
            (IDLE_TIMEOUT_FIELD_LABEL, self.writer_idle_timeout_secs),
            (
                ANALYSIS_FIRST_FRAME_TIMEOUT_FIELD_LABEL,
                self.analysis_first_frame_timeout_secs,
            ),
            (
                ANALYSIS_IDLE_TIMEOUT_FIELD_LABEL,
                self.analysis_idle_timeout_secs,
            ),
        ] {
            if secs == 0 || secs > MAX_CONFIGURABLE_TIMEOUT_SECS {
                return Err(ReportPipelineError::InvalidInput(format!(
                    "\"{label}\" must be between 1 and {MAX_CONFIGURABLE_TIMEOUT_SECS} seconds."
                )));
            }
        }
        if self.writer_max_output_tokens < MIN_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS
            || self.writer_max_output_tokens > MAX_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS
        {
            return Err(ReportPipelineError::InvalidInput(format!(
                "\"{WRITER_MAX_OUTPUT_TOKENS_FIELD_LABEL}\" must be between \
                 {MIN_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS} and \
                 {MAX_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS} tokens."
            )));
        }
        Ok(())
    }

    /// The two writer waits in the shape one Bedrock call takes them.
    pub const fn writer_stream_bounds(self) -> claria_bedrock::converse::StreamBounds {
        claria_bedrock::converse::StreamBounds::writer(
            self.writer_first_frame_timeout_secs as u64,
            self.writer_idle_timeout_secs as u64,
        )
    }

    /// The two planner/reviewer waits in the same shape.
    pub const fn analysis_stream_bounds(self) -> claria_bedrock::converse::StreamBounds {
        claria_bedrock::converse::StreamBounds::structured(
            self.analysis_first_frame_timeout_secs as u64,
            self.analysis_idle_timeout_secs as u64,
        )
    }
}

impl Default for BedrockRuntimeLimits {
    fn default() -> Self {
        Self {
            writer_first_frame_timeout_secs: DEFAULT_WRITER_FIRST_FRAME_TIMEOUT_SECS,
            writer_idle_timeout_secs: DEFAULT_WRITER_IDLE_TIMEOUT_SECS,
            writer_max_output_tokens: DEFAULT_WRITER_MAX_OUTPUT_TOKENS,
            analysis_first_frame_timeout_secs: DEFAULT_ANALYSIS_FIRST_FRAME_TIMEOUT_SECS,
            analysis_idle_timeout_secs: DEFAULT_ANALYSIS_IDLE_TIMEOUT_SECS,
        }
    }
}

impl ReportTurnLimits {
    pub fn try_new(
        max_tool_rounds: u32,
        max_converse_calls: u32,
        max_tool_uses_per_response: u32,
        max_retained_turns: u32,
    ) -> Result<Self, ReportPipelineError> {
        if max_tool_rounds == 0 || max_tool_rounds > MAX_CONFIGURABLE_TOOL_ROUNDS {
            return Err(ReportPipelineError::InvalidInput(format!(
                "Writer tool rounds must be between 1 and {MAX_CONFIGURABLE_TOOL_ROUNDS}."
            )));
        }
        if max_converse_calls == 0 || max_converse_calls > MAX_CONFIGURABLE_CONVERSE_CALLS {
            return Err(ReportPipelineError::InvalidInput(format!(
                "Writer Bedrock calls must be between 1 and {MAX_CONFIGURABLE_CONVERSE_CALLS}."
            )));
        }
        if max_tool_uses_per_response == 0
            || max_tool_uses_per_response > MAX_CONFIGURABLE_TOOL_USES_PER_RESPONSE
        {
            return Err(ReportPipelineError::InvalidInput(format!(
                "Writer tools per response must be between 1 and {MAX_CONFIGURABLE_TOOL_USES_PER_RESPONSE}."
            )));
        }
        if max_retained_turns == 0 || max_retained_turns > MAX_CONFIGURABLE_RETAINED_TURNS {
            return Err(ReportPipelineError::InvalidInput(format!(
                "Retained writer turns must be between 1 and {MAX_CONFIGURABLE_RETAINED_TURNS}."
            )));
        }
        Ok(Self {
            max_tool_rounds,
            max_converse_calls,
            max_tool_uses_per_response,
            max_retained_turns,
            runtime: BedrockRuntimeLimits::default(),
        })
    }

    /// Replace the per-call runtime dials, leaving the guardrail counts
    /// alone. Every existing caller means "use the compiled-in defaults".
    pub fn with_runtime(self, runtime: BedrockRuntimeLimits) -> Result<Self, ReportPipelineError> {
        runtime.validate()?;
        Ok(Self { runtime, ..self })
    }

    /// Raise these ceilings to fit a plan of `plan_len` sections, never
    /// lowering what the clinician configured.
    ///
    /// The defaults are sized for a request, not for a document. A 45-section
    /// plan cannot fit under the default 50 Bedrock calls: one section per
    /// response means at least one call per section, plus the title, the
    /// finish, corrective rounds, and the closing reply. Leaving the fixed
    /// default in place would fail those runs on the call ceiling with advice
    /// to raise a setting the run had already outgrown, so the plan supplies a
    /// floor of `plan_len * 2 + 8` — two calls per section for a write that
    /// needs a repair round, plus fixed overhead — clamped to the same
    /// maximum a clinician could type.
    ///
    /// Rounds move with calls because every round spends a call: the call
    /// ceiling has to stay at least one above the round ceiling or the last
    /// round has no call left to answer in.
    pub fn scaled_for_plan(self, plan_len: usize) -> Self {
        let target_calls = u32::try_from(plan_len.saturating_mul(2).saturating_add(8))
            .unwrap_or(u32::MAX)
            .min(MAX_CONFIGURABLE_CONVERSE_CALLS);
        let max_converse_calls = self.max_converse_calls.max(target_calls);
        let target_rounds = target_calls
            .saturating_sub(1)
            .min(MAX_CONFIGURABLE_TOOL_ROUNDS);
        let max_tool_rounds = self
            .max_tool_rounds
            .max(target_rounds)
            .min(max_converse_calls.saturating_sub(1));
        Self {
            max_tool_rounds,
            max_converse_calls,
            ..self
        }
    }

    pub const fn max_tool_rounds(self) -> u32 {
        self.max_tool_rounds
    }

    pub const fn max_converse_calls(self) -> u32 {
        self.max_converse_calls
    }

    pub const fn max_tool_uses_per_response(self) -> u32 {
        self.max_tool_uses_per_response
    }

    pub const fn max_retained_turns(self) -> u32 {
        self.max_retained_turns
    }

    pub const fn runtime(self) -> BedrockRuntimeLimits {
        self.runtime
    }

    pub const fn writer_first_frame_timeout_secs(self) -> u32 {
        self.runtime.writer_first_frame_timeout_secs
    }

    pub const fn writer_idle_timeout_secs(self) -> u32 {
        self.runtime.writer_idle_timeout_secs
    }

    pub const fn writer_max_output_tokens(self) -> u32 {
        self.runtime.writer_max_output_tokens
    }

    /// The configured writer waits in the shape one Bedrock call takes them.
    pub const fn stream_bounds(self) -> claria_bedrock::converse::StreamBounds {
        self.runtime.writer_stream_bounds()
    }
}

impl Default for ReportTurnLimits {
    fn default() -> Self {
        Self {
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            max_converse_calls: DEFAULT_MAX_CONVERSE_CALLS,
            max_tool_uses_per_response: DEFAULT_MAX_TOOL_USES_PER_RESPONSE,
            max_retained_turns: DEFAULT_MAX_RETAINED_TURNS,
            runtime: BedrockRuntimeLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportTurnOutcome {
    pub workspace: ReportWorkspace,
    pub turn_id: Uuid,
    pub assistant_text: String,
    pub proposal_id: Option<Uuid>,
    pub attempt: ReportAttemptMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FullRecordContextSummary {
    pub included_files: u32,
    pub unavailable_files: u32,
    pub total_characters: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FullReportGenerationOutcome {
    pub workspace: ReportWorkspace,
    pub turn_id: Uuid,
    pub assistant_text: String,
    pub record_context: FullRecordContextSummary,
    pub attempt: ReportAttemptMetadata,
}

/// A paragraph or table the user explicitly attached to their next writing
/// message. The host validates the stable section ID and current block index
/// before exposing the current block to the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportBlockReference {
    pub section_id: Uuid,
    pub block_index: u32,
}
