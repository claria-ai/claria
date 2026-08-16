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
mod prompt_cache;
mod prompts;
mod record_context;
mod run;
mod tools;
mod turn;

use claria_core::models::report::{MAX_REPORT_TURNS, ReportWorkspace};
use claria_report_store::ReportAttemptMetadata;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use error::ReportPipelineError;
pub use prompt_cache::ReportPromptCache;
pub use prompts::{
    FULL_REPORT_SYSTEM_PROMPT_BODY, FULL_REPORT_TRUST_RULES, REPORT_SYSTEM_PROMPT_BODY,
    REPORT_TRUST_RULES, full_report_system_prompt, report_system_prompt,
};
pub use run::resume_draft_run;
pub use turn::{
    FullReportRequest, ReportMessageRequest, ReportTurnProgress, generate_full_report,
    generate_full_report_for_report, send_report_message, send_report_message_for_report,
};

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
pub(crate) const MAX_REPORT_REFERENCES: usize = 10;

/// Where a clinician raises the guardrails below, and the exact field labels
/// they will read there. These mirror `WRITER_LIMIT_FIELDS` in
/// `pages/Preferences.tsx`; renaming a field there means renaming it here, or
/// the failure message sends the reader looking for a control that is gone.
pub(crate) const WRITER_LIMITS_SECTION: &str =
    "Preferences \u{2192} Document Writer \u{2192} Writer Limits";
pub(crate) const TOOL_ROUNDS_FIELD_LABEL: &str = "Tool-use rounds per request";
pub(crate) const CONVERSE_CALLS_FIELD_LABEL: &str = "Bedrock calls per request";

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
        })
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
}

impl Default for ReportTurnLimits {
    fn default() -> Self {
        Self {
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            max_converse_calls: DEFAULT_MAX_CONVERSE_CALLS,
            max_tool_uses_per_response: DEFAULT_MAX_TOOL_USES_PER_RESPONSE,
            max_retained_turns: DEFAULT_MAX_RETAINED_TURNS,
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
