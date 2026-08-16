//! The review fan-out: one request per property, each sweeping every drafted
//! section of the report.
//!
//! Seven properties, seven requests, one shared prefix. The alternative —
//! one request asked to look for everything — is what produces a review that
//! finds four problems in the first section and nothing after it: attention
//! is finite, and a pass told to watch for seven things watches for whichever
//! one it noticed last. Splitting the properties makes each pass's job small
//! enough to carry across a whole document, and asking every pass for an
//! explicit row per section — including the sections it found nothing in —
//! turns "found nothing" into a claim the host can check rather than a
//! silence it has to guess about.
//!
//! ```text
//! system:      analysis policy + record corpus + template structure   ┐ identical
//!              ● CachePoint 1 (1h)                                    ┘ across branches
//! messages[0]: <untrusted_draft_sections>   pretty, revision-anchored ┐ identical
//!              ● CachePoint 2 (1h)                                    ┘ across branches
//!              per-property instruction                    ← the only differing bytes
//! ```
//!
//! The prefix is byte-identical because the branches share the builders, not
//! because they agree to: [`crate::plan::analysis_system_blocks`] is the same
//! function the planner calls, and the drafted-section block is built once and
//! handed to all seven. The tool configuration and the forced `tool_choice`
//! are identical too — the union `submit_review_rows` tool exists so they can
//! be. A per-property tool would move the tools tier of the cache, which sits
//! above system and messages, and every branch would pay full input rates.
//!
//! Execution is warm-then-fan-out. The first branch runs alone and writes the
//! two checkpoints; the remaining six then read them. Firing all seven at a
//! cold cache would have each of them write the same prefix at the one-hour
//! write rate, which is the expensive half of prompt caching paid seven times
//! over for nothing.
//!
//! Nothing the model says about where a finding belongs is trusted. Quotes
//! are resolved against the section's own text host-side, the anchor and the
//! model ID are stamped from the request the host sent, and a consistency
//! property that proposes text is refused — the persisted model has no shape
//! for it.

use std::collections::HashSet;

use aws_sdk_s3::Client as S3Client;
use claria_bedrock::{
    analysis::{
        self, AnalysisInputBudget, AnalysisToolConfiguration, MAX_REVIEW_DESCRIPTION_CHARACTERS,
        ReviewProperty, ReviewRow, ReviewRowFinding, ReviewRowStatus, ReviewRowsRequest,
        StructuredCallOutput, StructuredCallRequest,
    },
    converse::StopSignal,
    retry::{MAX_ATTEMPTS, with_throttle_retry_observed},
};
use claria_core::models::{
    findings::{
        ConflictingRef, Finding, FindingAnchor, FindingStatus, MAX_FINDING_DESCRIPTION_CHARACTERS,
        MAX_REPORT_FINDINGS, RecordCitation, ReportFindings, ReviewCoverage, ReviewPass,
        StyleProposal, TextSpan,
    },
    report::{
        ReportBlock, ReportContent, ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole,
        ReportSection, ReportToolResultStatus, ReportWorkspace, prompt_content_view,
    },
    turn_usage::TurnUsage,
};
use claria_report_store::{
    load_report_findings, replace_findings_for_revision, save_report_findings,
};
use futures::stream::{self, StreamExt};
use uuid::Uuid;

use crate::{
    ReportPipelineError,
    context::escape_delimiter_characters,
    plan::{
        AnalysisRoleLabels, Warnings, analysis_system_blocks, bedrock_diagnostic, index_rows,
        map_bedrock_error, merge_optional_usage, truncate_characters,
    },
    record_context::{
        BudgetRole, FullRecordContext, STRUCTURAL_ALLOWANCE_BYTES, load_full_record_context,
        load_record_inventory,
    },
    review_instructions::instruction,
    text::normalize_whitespace,
    turn::{ReportTurnProgress, ensure_ready_for_turn, validate_model_choice},
};

/// Output-token ceiling for one review branch, and the reserve subtracted
/// from the reviewer's context window to form its input budget.
///
/// Twice the planner's, because a review's worst case is larger than a plan's:
/// a hundred rows each carrying up to ten findings, and every finding carries
/// two or three quotes copied out of the document. A branch that hits the
/// ceiling is a typed failure — truncated forced-tool JSON is not a review
/// with some sections missing, it is unparseable.
pub const REVIEW_OUTPUT_TOKEN_RESERVE: u32 = 16_384;

/// Branches in flight at once after the warming branch.
///
/// Bedrock meters a model by requests per minute against the whole account,
/// and a review branch is a single long request rather than a burst, so the
/// figure that matters is how many of these can be outstanding before the
/// account's other work starts being throttled. Three leaves headroom on the
/// smallest on-demand RPM allocations while still finishing six branches in
/// two waves; the throttle retry underneath absorbs the case where an
/// account is busier than that, and raising this would mostly buy more
/// retries.
pub const REVIEW_FAN_OUT_CONCURRENCY: usize = 3;

/// Repair rounds granted to one branch whose answer failed host validation.
/// One, for the same reason the planner gets one: the diagnostic names the
/// rows that were wrong, and a model that cannot use it once will not use it
/// twice. A branch that fails twice fails alone — the other six keep their
/// findings.
const REVIEW_REPAIR_ROUNDS: u32 = 1;

/// Zero-based index of the drafted-section block inside `messages[0]`. The
/// second cache checkpoint lands right after it, so the coordinate and the
/// layout are named once rather than agreed twice.
const DRAFT_SECTIONS_BLOCK: usize = 0;

const REVIEWER_LABELS: AnalysisRoleLabels = AnalysisRoleLabels {
    pass: "review pass",
    model: "reviewing model",
    artifact: "review",
};

/// Appended to a description whose quote appears more than once in its
/// section. The first occurrence is anchored, and the clinician is told the
/// anchor is one of several rather than left to wonder why the highlight
/// landed where it did.
const DUPLICATE_ANCHOR_NOTE: &str = " [duplicate_anchor: this passage appears more than once in the section; the first occurrence \
     is anchored.]";

/// What one review sweep produced, and what it cost.
#[derive(Debug, Clone)]
pub struct ReviewSweepOutcome {
    pub findings: ReportFindings,
    /// One entry per property, in fan-out order, whether it completed or not.
    pub properties: Vec<ReviewPropertyStatus>,
    pub usage: Option<TurnUsage>,
    pub converse_calls: u32,
}

/// How one branch of the fan-out ended.
#[derive(Debug, Clone)]
pub struct ReviewPropertyStatus {
    pub property: &'static str,
    /// `None` when the branch completed; the user-facing failure otherwise.
    /// A branch that failed contributes no coverage row, so a property with a
    /// message here is one nobody has reviewed this revision for.
    pub error: Option<String>,
    pub findings: u32,
}

impl ReviewSweepOutcome {
    /// Findings raised per property, for the audit event. Counts only.
    pub fn findings_by_property(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for status in &self.properties {
            map.insert(status.property.to_string(), status.findings.into());
        }
        serde_json::Value::Object(map)
    }

    /// The properties that did not complete, for the audit event.
    pub fn failed_properties(&self) -> Vec<&'static str> {
        self.properties
            .iter()
            .filter(|status| status.error.is_some())
            .map(|status| status.property)
            .collect()
    }
}

/// A review request: the optional channels a live sweep reports and cancels
/// through.
#[derive(Clone, Copy, Default)]
pub struct ReviewSweepRequest<'a> {
    progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
    stop: Option<&'a StopSignal>,
}

impl<'a> ReviewSweepRequest<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_progress(
        mut self,
        progress: &'a (dyn Fn(ReportTurnProgress) + Send + Sync),
    ) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Fire this signal to abandon every branch mid-call.
    pub fn with_stop(mut self, stop: &'a StopSignal) -> Self {
        self.stop = Some(stop);
        self
    }

    fn emit_progress(self, event: ReportTurnProgress) {
        if let Some(progress) = self.progress {
            progress(event);
        }
    }
}

/// Review one accepted revision for all seven properties and persist the
/// findings.
///
/// Guarded like a writer turn — the revision the caller expects, no pending
/// proposal, no drafting run in flight — because a review anchors every
/// finding to a revision, and a revision that moves underneath the sweep
/// produces findings that are stale before they are saved.
// Client, report, and the revision the caller expects are the same explicit
// orchestration boundary every other entry point in this crate keeps.
#[allow(clippy::too_many_arguments)]
pub async fn run_review_sweeps(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    reviewer_model_id: &str,
    request: ReviewSweepRequest<'_>,
) -> Result<ReviewSweepOutcome, ReportPipelineError> {
    validate_model_choice(reviewer_model_id)?;
    let loaded = claria_report_store::load_for_report(s3, bucket, client_id, report_id).await?;
    ensure_ready_for_turn(&loaded.workspace, revision)?;

    let reviewed: Vec<&ReportSection> = loaded
        .workspace
        .draft
        .content
        .sections
        .iter()
        .filter(|section| !section.skipped)
        .collect();
    if reviewed.is_empty() {
        return Err(ReportPipelineError::InvalidInput(
            "This report has no drafted sections to review. Write at least one section first."
                .to_string(),
        ));
    }

    let corpus = prepare_review_corpus(s3, bucket, client_id, reviewer_model_id, request).await?;
    let system = analysis_system_blocks(&loaded.workspace, &corpus)?;
    let draft_block = draft_sections_block(&loaded.workspace, &reviewed)?;
    preflight_request_size(reviewer_model_id, &system, &draft_block)?;

    let branches = fan_out(
        sdk_config,
        &BranchContext {
            reviewer_model_id,
            system: &system,
            draft_block: &draft_block,
            content: &loaded.workspace.draft.content,
            reviewed: &reviewed,
            corpus: &corpus,
            revision,
        },
        request,
    )
    .await?;

    persist_sweep(
        s3,
        bucket,
        client_id,
        report_id,
        revision,
        reviewer_model_id,
        branches,
    )
    .await
}

// ── Execution ───────────────────────────────────────────────────────────────

/// Everything every branch reads, borrowed rather than cloned: the shared
/// prefix is the largest thing in the request and copying it seven times to
/// satisfy the borrow checker would defeat the point of sharing it.
struct BranchContext<'a> {
    reviewer_model_id: &'a str,
    system: &'a [String],
    draft_block: &'a str,
    content: &'a ReportContent,
    reviewed: &'a [&'a ReportSection],
    corpus: &'a FullRecordContext,
    revision: u64,
}

/// One completed branch.
struct BranchOutcome {
    property: ReviewProperty,
    findings: Vec<Finding>,
    sections_reviewed: u32,
    usage: Option<TurnUsage>,
    converse_calls: u32,
    /// The exact input count this branch took, if it took one. Only the
    /// warming branch does.
    verified: Option<(u32, u64)>,
    warnings: Vec<String>,
}

/// Run the warming branch to completion, then the remaining six in parallel.
async fn fan_out(
    sdk_config: &aws_config::SdkConfig,
    context: &BranchContext<'_>,
    request: ReviewSweepRequest<'_>,
) -> Result<Vec<Result<BranchOutcome, ReportPipelineError>>, ReportPipelineError> {
    let total = u32::try_from(ReviewProperty::ALL.len()).unwrap_or(u32::MAX);
    let tools = analysis::analysis_tool_configuration()
        .map_err(|error| map_bedrock_error(error, REVIEWER_LABELS))?;
    let cache_plan = claria_bedrock::converse::CachePlan::review(
        claria_core::model_id::ModelCapabilities::for_id(context.reviewer_model_id),
        vec![(0, DRAFT_SECTIONS_BLOCK)],
    )
    .map_err(|error| map_bedrock_error(error, REVIEWER_LABELS))?;
    let unstoppable = StopSignal::new();
    let stop = request.stop.unwrap_or(&unstoppable);

    let (warm_property, siblings) = ReviewProperty::ALL
        .split_first()
        .expect("the property list is a fixed non-empty array");

    request.emit_progress(ReportTurnProgress::ReviewPassStarted {
        property: warm_property.as_str().to_string(),
        index: 1,
        total,
    });
    let warm = run_branch(
        sdk_config,
        context,
        request,
        *warm_property,
        &tools,
        &cache_plan,
        stop,
        None,
    )
    .await;
    emit_completed(request, &warm, *warm_property, 1, total);

    // The seed is what makes the siblings free of CountTokens: they send the
    // same bytes plus a different instruction, so the warming branch's exact
    // count plus a character delta is a better estimate than six more calls
    // would buy.
    let seed = warm.as_ref().ok().and_then(|branch| branch.verified);
    let mut results = vec![warm];

    let fanned = stream::iter(
        siblings
            .iter()
            .copied()
            .enumerate()
            .map(|(offset, property)| {
                let tools = &tools;
                let cache_plan = &cache_plan;
                async move {
                    let index = u32::try_from(offset + 2).unwrap_or(u32::MAX);
                    request.emit_progress(ReportTurnProgress::ReviewPassStarted {
                        property: property.as_str().to_string(),
                        index,
                        total,
                    });
                    let outcome = run_branch(
                        sdk_config, context, request, property, tools, cache_plan, stop, seed,
                    )
                    .await;
                    (outcome, property, index)
                }
            }),
    )
    .buffered(REVIEW_FAN_OUT_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    for (outcome, property, index) in fanned {
        emit_completed(request, &outcome, property, index, total);
        results.push(outcome);
    }

    log_fan_out(context.reviewer_model_id, &results);
    if results.iter().all(Result::is_err) {
        // Every branch failing is one failure with seven symptoms — a bad
        // model ID, an unreachable region, an exhausted quota. Reporting the
        // first of them is more use than an empty review nobody asked for.
        let first = results
            .into_iter()
            .find_map(Result::err)
            .unwrap_or_else(|| ReportPipelineError::InvalidInput("The review failed.".to_string()));
        return Err(first);
    }
    Ok(results)
}

fn emit_completed(
    request: ReviewSweepRequest<'_>,
    outcome: &Result<BranchOutcome, ReportPipelineError>,
    property: ReviewProperty,
    index: u32,
    total: u32,
) {
    request.emit_progress(ReportTurnProgress::ReviewPassCompleted {
        property: property.as_str().to_string(),
        findings: outcome
            .as_ref()
            .map(|branch| u32::try_from(branch.findings.len()).unwrap_or(u32::MAX))
            .unwrap_or(0),
        completed: index,
        total,
    });
}

/// One branch: the forced call, its one repair round, and host validation of
/// whatever came back.
#[allow(clippy::too_many_arguments)]
async fn run_branch(
    sdk_config: &aws_config::SdkConfig,
    context: &BranchContext<'_>,
    request: ReviewSweepRequest<'_>,
    property: ReviewProperty,
    tools: &AnalysisToolConfiguration,
    cache_plan: &claria_bedrock::converse::CachePlan,
    stop: &StopSignal,
    seed: Option<(u32, u64)>,
) -> Result<BranchOutcome, ReportPipelineError> {
    let budget = tokio::sync::Mutex::new(match seed {
        Some((tokens, chars)) => AnalysisInputBudget::seeded(
            context.reviewer_model_id,
            REVIEW_OUTPUT_TOKEN_RESERVE,
            tokens,
            chars,
        ),
        None => AnalysisInputBudget::new(
            context.reviewer_model_id,
            "report_review",
            REVIEW_OUTPUT_TOKEN_RESERVE,
        ),
    });
    let mut messages = vec![ReportProtocolMessage {
        role: ReportProtocolRole::User,
        content: vec![
            ReportProtocolBlock::Text {
                text: context.draft_block.to_string(),
            },
            ReportProtocolBlock::Text {
                text: instruction(property),
            },
        ],
        created_at: jiff::Timestamp::now(),
    }];

    let mut usage: Option<TurnUsage> = None;
    let mut converse_calls = 0_u32;
    let mut rejection: Option<String> = None;
    for round in 0..=REVIEW_REPAIR_ROUNDS {
        converse_calls = converse_calls.saturating_add(1);
        let call_number = converse_calls;
        let on_retry = |attempt, delay| {
            request.emit_progress(ReportTurnProgress::retrying(
                call_number,
                attempt,
                MAX_ATTEMPTS,
                delay,
            ));
        };
        let output: StructuredCallOutput =
            with_throttle_retry_observed("report_review", Some(&on_retry), || {
                let messages = &messages;
                let budget = &budget;
                async move {
                    analysis::converse_structured(
                        sdk_config,
                        StructuredCallRequest {
                            model_id: context.reviewer_model_id,
                            system: context.system,
                            messages,
                            tools,
                            forced_tool: analysis::SUBMIT_REVIEW_ROWS_TOOL,
                            max_tokens: REVIEW_OUTPUT_TOKEN_RESERVE,
                            cache_plan: cache_plan.clone(),
                            stop,
                            // A review branch already reports itself pass by
                            // pass; a row count inside one pass says nothing
                            // the reader can act on.
                            on_partial_tool_input: None,
                            operation: "report_review",
                        },
                        &mut *budget.lock().await,
                    )
                    .await
                }
            })
            .await
            .map_err(|error| map_bedrock_error(error, REVIEWER_LABELS))?;
        usage = merge_optional_usage(usage, output.usage.clone());

        let validated = analysis::decode_review_rows(&output.input)
            .map_err(bedrock_diagnostic)
            .and_then(|rows| validate_review_rows(&rows, property, context));
        match validated {
            Ok(validated) => {
                return Ok(BranchOutcome {
                    property,
                    findings: validated.findings,
                    sections_reviewed: u32::try_from(context.reviewed.len()).unwrap_or(u32::MAX),
                    usage,
                    converse_calls,
                    verified: budget.lock().await.verified(),
                    warnings: validated.warnings,
                });
            }
            Err(diagnostic) => {
                if round == REVIEW_REPAIR_ROUNDS {
                    rejection = Some(diagnostic);
                    break;
                }
                messages.push(ReportProtocolMessage {
                    role: ReportProtocolRole::Assistant,
                    content: vec![ReportProtocolBlock::ToolUse {
                        tool_use_id: output.tool_use_id.clone(),
                        name: analysis::SUBMIT_REVIEW_ROWS_TOOL.to_string(),
                        input: output.input.clone(),
                    }],
                    created_at: jiff::Timestamp::now(),
                });
                messages.push(ReportProtocolMessage {
                    role: ReportProtocolRole::User,
                    content: vec![ReportProtocolBlock::ToolResult {
                        tool_use_id: output.tool_use_id,
                        status: ReportToolResultStatus::Error,
                        content: serde_json::json!({"error": {
                            "code": "invalid_review",
                            "message": diagnostic,
                        }}),
                    }],
                    created_at: jiff::Timestamp::now(),
                });
                rejection = None;
            }
        }
    }

    let diagnostic = rejection.unwrap_or_else(|| "the review could not be validated".to_string());
    tracing::error!(
        reviewer_model_id = context.reviewer_model_id,
        property = property.as_str(),
        converse_calls,
        diagnostic,
        "a review branch could not produce a usable answer"
    );
    Err(ReportPipelineError::InvalidInput(format!(
        "The {} review pass could not produce a usable answer, even after a correction. The other passes were unaffected. Details: {diagnostic}",
        property.as_str()
    )))
}

// ── Request assembly ────────────────────────────────────────────────────────

async fn prepare_review_corpus(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    reviewer_model_id: &str,
    request: ReviewSweepRequest<'_>,
) -> Result<FullRecordContext, ReportPipelineError> {
    let inventory = load_record_inventory(s3, bucket, client_id)
        .await
        .map_err(|source| ReportPipelineError::storage("listing records for the review", source))?;
    let corpus = load_full_record_context(
        s3,
        bucket,
        &[BudgetRole::reviewer(reviewer_model_id)],
        &inventory,
    )
    .await?;
    request.emit_progress(ReportTurnProgress::RecordContextPrepared {
        included_files: corpus.summary.included_files,
        unavailable_files: corpus.summary.unavailable_files,
        total_characters: corpus.summary.total_characters,
    });
    Ok(corpus)
}

/// The accepted draft as the reviewer reads it: the revision it is anchored
/// to, and every section that is actually drafted.
///
/// Built once and sent by all seven branches, so it has to be a pure function
/// of the workspace — a reordered key here costs six branches their cache
/// read. Skipped sections are left out entirely: they have no content to
/// review, and every property's coverage rule is "one row per section in this
/// block", so listing them would demand rows about nothing.
fn draft_sections_block(
    workspace: &ReportWorkspace,
    reviewed: &[&ReportSection],
) -> Result<String, ReportPipelineError> {
    // `prompt_content_view` is what strips the template copies and authorship
    // stamps from anything a model sees; routing through it rather than
    // re-serializing the sections is what keeps that stripping in one place.
    let visible = ReportContent {
        title: workspace.draft.content.title.clone(),
        sections: reviewed.iter().map(|section| (*section).clone()).collect(),
    };
    let value = serde_json::json!({
        "revision": workspace.draft.revision,
        "report": prompt_content_view(&visible),
    });
    serde_json::to_string_pretty(&value)
        .map(|json| {
            format!(
                "<untrusted_draft_sections>\n{}\n</untrusted_draft_sections>",
                escape_delimiter_characters(&json)
            )
        })
        .map_err(|_| {
            ReportPipelineError::InvalidInput(
                "Claria could not serialize the drafted sections for review.".to_string(),
            )
        })
}

/// Refuse a sweep whose request cannot fit before seven of them are sent.
///
/// The corpus preflight in [`load_full_record_context`] sizes records alone;
/// this one adds the drafted report, which the planner never had to carry,
/// and the fixed structure around both. Failing here costs nothing; failing
/// on the wire costs the warming branch's cache write and a `ContextBudget`
/// error per branch that says nothing about which half was too big.
fn preflight_request_size(
    reviewer_model_id: &str,
    system: &[String],
    draft_block: &str,
) -> Result<(), ReportPipelineError> {
    let budget_tokens =
        analysis::analysis_input_token_budget(reviewer_model_id, REVIEW_OUTPUT_TOKEN_RESERVE);
    let characters = system
        .iter()
        .map(|block| block.chars().count() as u64)
        .fold(0_u64, u64::saturating_add)
        .saturating_add(draft_block.chars().count() as u64)
        .saturating_add(STRUCTURAL_ALLOWANCE_BYTES);
    // The same four-characters-per-token rule the incremental budget uses, so
    // this preflight and the exact count it precedes cannot disagree about
    // what "nearly full" means.
    let estimated_tokens = u32::try_from(characters / 4).unwrap_or(u32::MAX);
    if estimated_tokens > budget_tokens {
        return Err(ReportPipelineError::InvalidInput(format!(
            "This report is too large to review against the full record corpus: the drafted sections and this client's records together need about {estimated_tokens} tokens, and {reviewer_model_id} allows {budget_tokens} for one review. Remove or split oversized records, or choose a reviewing model with a larger context window."
        )));
    }
    Ok(())
}

// ── Host validation ─────────────────────────────────────────────────────────

struct ValidatedBranch {
    findings: Vec<Finding>,
    warnings: Vec<String>,
}

/// Check one branch's answer against the document it reviewed.
///
/// Coverage, identity, and the style/consistency split are hard: they produce
/// an `Err` the repair round hands back verbatim. A quote that does not
/// resolve is soft — the finding is dropped or degraded and the sweep records
/// a PHI-free warning, because refusing a whole branch over one reflowed
/// quote loses six good findings to fix one bad one.
fn validate_review_rows(
    rows: &ReviewRowsRequest,
    property: ReviewProperty,
    context: &BranchContext<'_>,
) -> Result<ValidatedBranch, String> {
    if rows.property != property {
        return Err(format!(
            "this pass was asked about {}, but the rows were submitted under {}; submit the rows for the property you were asked about",
            property.as_str(),
            rows.property.as_str()
        ));
    }
    let expected: Vec<(Uuid, &str)> = context
        .reviewed
        .iter()
        .map(|section| (section.id, section.heading.as_str()))
        .collect();
    let by_id = index_rows(
        rows.rows.iter().map(|row| row.section_id.as_str()),
        &expected,
    )?;

    let pass = if property.is_style() {
        ReviewPass::Style
    } else {
        ReviewPass::Consistency
    };
    let now = jiff::Timestamp::now();
    let mut warnings = Warnings::default();
    let mut findings = Vec::new();
    for (section_id, _) in &expected {
        let row = &rows.rows[by_id[section_id]];
        let section = context
            .reviewed
            .iter()
            .find(|section| section.id == *section_id)
            .ok_or_else(|| format!("section {section_id} is not part of this review"))?;
        check_row_status(row, *section_id)?;

        // Replacements are checked against each other per section, so two
        // findings cannot both edit the same words and leave the second
        // unapplicable the moment the first lands.
        let mut claimed: Vec<(usize, u32, u32)> = Vec::new();
        for raw in &row.findings {
            if let Some(finding) = build_finding(
                raw,
                section,
                property,
                pass,
                context,
                now,
                &mut claimed,
                &mut warnings,
            )? {
                findings.push(finding);
            }
        }
    }
    Ok(ValidatedBranch {
        findings,
        warnings: warnings.into_vec(),
    })
}

fn check_row_status(row: &ReviewRow, section_id: Uuid) -> Result<(), String> {
    match row.status {
        ReviewRowStatus::NoIssues if !row.findings.is_empty() => Err(format!(
            "section {section_id} is marked \"no_issues\" but carries {} findings; use status \"findings\" when the array is not empty",
            row.findings.len()
        )),
        ReviewRowStatus::Findings if row.findings.is_empty() => Err(format!(
            "section {section_id} is marked \"findings\" but carries none; use status \"no_issues\" for a section you found nothing in"
        )),
        _ => Ok(()),
    }
}

/// Turn one model-supplied finding into a persisted one, or drop it.
///
/// `Err` is a hard rejection the repair round sees. `Ok(None)` is a finding
/// whose anchor could not be resolved: the quote was altered, so there is
/// nowhere to point the clinician and nothing to show them.
#[allow(clippy::too_many_arguments)]
fn build_finding(
    raw: &ReviewRowFinding,
    section: &ReportSection,
    property: ReviewProperty,
    pass: ReviewPass,
    context: &BranchContext<'_>,
    now: jiff::Timestamp,
    claimed: &mut Vec<(usize, u32, u32)>,
    warnings: &mut Warnings,
) -> Result<Option<Finding>, String> {
    // Structural, and therefore hard: the persisted model has no shape for a
    // consistency finding that proposes text, so accepting one here would
    // only move the failure to the save.
    if !property.is_style() && raw.replacement.is_some() {
        return Err(format!(
            "the {} property proposed a replacement for a passage in section {}; consistency properties report only and must never return a \"replacement\"",
            property.as_str(),
            section.id
        ));
    }
    if property.is_style() && raw.replacement.is_none() {
        return Err(format!(
            "the {} property returned a finding for section {} with no \"replacement\"; every style finding must carry the edit that fixes it",
            property.as_str(),
            section.id
        ));
    }

    let Some(anchor) = resolve_in_section(section, &raw.span.quote, raw.span.block_index) else {
        warnings.push(format!("unresolved_span:{}", section.id));
        return Ok(None);
    };
    let mut description =
        truncate_characters(raw.description.trim(), MAX_REVIEW_DESCRIPTION_CHARACTERS);
    if anchor.duplicate {
        description.push_str(DUPLICATE_ANCHOR_NOTE);
        description = truncate_characters(&description, MAX_FINDING_DESCRIPTION_CHARACTERS);
    }

    let proposal = match &raw.replacement {
        None => None,
        Some(replacement) => {
            build_proposal(replacement, section, claimed, warnings, &mut description)?
        }
    };

    Ok(Some(Finding {
        id: Uuid::new_v4(),
        pass,
        property: property.as_str().to_string(),
        // Stamped from the request the host sent, never read back out of the
        // answer: a model that named a different model, section, or revision
        // would be describing a document nobody reviewed.
        model_id: context.reviewer_model_id.to_string(),
        anchor: FindingAnchor {
            section_id: section.id,
            revision: context.revision,
        },
        description,
        span: Some(anchor.span),
        conflicting: raw
            .conflicting_span
            .as_ref()
            .and_then(|conflicting| resolve_conflicting(conflicting, section, context, warnings)),
        record_citation: raw
            .record_citation
            .as_ref()
            .and_then(|citation| resolve_citation(citation, context.corpus, warnings)),
        proposal,
        status: FindingStatus::Open,
        applied_revision: None,
        resolved_at: None,
        created_at: now,
    }))
}

/// Resolve a style replacement into an applicable proposal, or explain in the
/// description why there is none.
fn build_proposal(
    replacement: &analysis::ReviewReplacement,
    section: &ReportSection,
    claimed: &mut Vec<(usize, u32, u32)>,
    warnings: &mut Warnings,
    description: &mut String,
) -> Result<Option<StyleProposal>, String> {
    if replacement.find == replacement.replace {
        return Err(format!(
            "a replacement in section {} does not change the text; return the edit you want made, or no finding",
            section.id
        ));
    }
    let Some(target) = resolve_in_section(section, &replacement.find, None) else {
        warnings.push(format!("unresolved_replacement:{}", section.id));
        note_unapplicable(
            description,
            "the quoted text is not in this section as written",
        );
        return Ok(None);
    };
    if target.duplicate {
        // Two occurrences means the anchor cannot say which one was meant,
        // and an apply that guessed would edit the wrong sentence.
        warnings.push(format!("ambiguous_replacement:{}", section.id));
        note_unapplicable(
            description,
            "the text it replaces appears more than once in this section",
        );
        return Ok(None);
    }
    let block_index = usize::try_from(target.span.block_index).unwrap_or(usize::MAX);
    if !matches!(
        section.blocks.get(block_index),
        Some(ReportBlock::Paragraph { .. })
    ) {
        warnings.push(format!("non_paragraph_replacement:{}", section.id));
        note_unapplicable(
            description,
            "it points at a list or a table rather than a paragraph",
        );
        return Ok(None);
    }
    for (other_block, start, end) in claimed.iter().copied() {
        if other_block == block_index
            && target.span.start_char < end
            && start < target.span.end_char
        {
            return Err(format!(
                "two replacements in section {} overlap the same text; each finding must edit a span no other finding in this section touches",
                section.id
            ));
        }
    }
    claimed.push((block_index, target.span.start_char, target.span.end_char));
    Ok(Some(StyleProposal {
        block_index: target.span.block_index,
        original_text: replacement.find.clone(),
        replacement_text: replacement.replace.clone(),
    }))
}

fn note_unapplicable(description: &mut String, reason: &str) {
    description.push_str(&format!(
        " [no anchored replacement: {reason}. Fix it by hand.]"
    ));
    *description = truncate_characters(description, MAX_FINDING_DESCRIPTION_CHARACTERS);
}

fn resolve_conflicting(
    conflicting: &analysis::ReviewConflictingSpan,
    anchored: &ReportSection,
    context: &BranchContext<'_>,
    warnings: &mut Warnings,
) -> Option<ConflictingRef> {
    let section_id = match conflicting.section_id.as_deref() {
        None => None,
        Some(raw) => match raw.parse::<Uuid>() {
            Ok(id) => Some(id),
            Err(_) => {
                warnings.push("unparseable_conflicting_section".to_string());
                return None;
            }
        },
    };
    let target = match section_id {
        None => Some(anchored),
        Some(id) => context.content.sections.iter().find(|s| s.id == id),
    };
    let span = match target {
        None => {
            warnings.push("unknown_conflicting_section".to_string());
            None
        }
        Some(section) => resolve_in_section(section, &conflicting.quote, None)
            .map(|anchor| anchor.span)
            .or_else(|| {
                // Keeping the quote without a span is deliberate: the
                // clinician can still read what the two passages were, which
                // is most of a consistency finding's value.
                warnings.push(format!("unresolved_conflicting_span:{}", section.id));
                None
            }),
    };
    Some(ConflictingRef {
        section_id: section_id.filter(|id| *id != anchored.id),
        quote: truncate_characters(
            &conflicting.quote,
            claria_core::models::report_run::MAX_CITATION_QUOTE_CHARACTERS,
        ),
        span,
    })
}

fn resolve_citation(
    citation: &analysis::ReviewRecordCitation,
    corpus: &FullRecordContext,
    warnings: &mut Warnings,
) -> Option<RecordCitation> {
    if !corpus.resolves_quote(&citation.filename, &citation.quote) {
        // Filenames are client-chosen and can carry a name, so the warning
        // records only that a citation did not resolve.
        warnings.push("unresolved_record_citation".to_string());
        return None;
    }
    Some(RecordCitation {
        filename: citation.filename.clone(),
        quote: truncate_characters(
            &citation.quote,
            claria_core::models::report_run::MAX_CITATION_QUOTE_CHARACTERS,
        ),
    })
}

/// A resolved anchor: where the quote sits, and whether it sits there twice.
struct ResolvedAnchor {
    span: TextSpan,
    duplicate: bool,
}

/// Find `quote` inside one section, returning the character range it occupies
/// in the block that holds it.
///
/// `hint` is the model's guess at the block and is searched first; the rest
/// of the section follows in order, so a wrong hint costs a few string
/// searches and a missing one costs nothing. Matching is
/// whitespace-normalized on both sides — a model copying a span out of a
/// wrapped paragraph reliably reflows the line breaks and nothing else, and
/// failing an otherwise verbatim quote over a newline would make the whole
/// mechanism unusable. Anything past whitespace still fails, which is the
/// point.
fn resolve_in_section(
    section: &ReportSection,
    quote: &str,
    hint: Option<u32>,
) -> Option<ResolvedAnchor> {
    let order = hint
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < section.blocks.len())
        .into_iter()
        .chain(0..section.blocks.len());
    let mut seen = HashSet::new();
    for index in order {
        if !seen.insert(index) {
            continue;
        }
        let text = block_text(&section.blocks[index]);
        if let Some((start_char, end_char, duplicate)) = find_normalized(&text, quote) {
            return Some(ResolvedAnchor {
                span: TextSpan {
                    block_index: u32::try_from(index).unwrap_or(u32::MAX),
                    start_char,
                    end_char,
                },
                duplicate,
            });
        }
    }
    None
}

/// One block's reviewable text. A list's items and a table's cells are joined
/// by newlines, which whitespace normalization then collapses — so a quote
/// spanning two bullets resolves the same way a quote spanning two lines of a
/// paragraph does.
fn block_text(block: &ReportBlock) -> String {
    match block {
        ReportBlock::Paragraph { text } => text.clone(),
        ReportBlock::BulletList { items } => items.join("\n"),
        ReportBlock::Table { rows, .. } => rows
            .iter()
            .map(|row| row.join("\t"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Locate `quote` in `text` by whitespace-normalized comparison, returning
/// `(start_char, end_char, appears_more_than_once)` in the ORIGINAL text.
///
/// The offsets have to address the original because that is what the report
/// stores and what a highlight has to land on; the comparison has to be
/// normalized because that is what the model can reliably reproduce. The map
/// built here is what reconciles the two.
fn find_normalized(text: &str, quote: &str) -> Option<(u32, u32, bool)> {
    let (normalized, origins) = normalize_with_origins(text);
    let needle = normalize_whitespace(quote);
    if needle.is_empty() || normalized.is_empty() {
        return None;
    }
    let needle_len = needle.chars().count();
    let haystack: Vec<char> = normalized.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();
    let mut matches = haystack
        .windows(needle_len)
        .enumerate()
        .filter(|(_, window)| *window == needle_chars.as_slice())
        .map(|(index, _)| index);
    let first = matches.next()?;
    let duplicate = matches.next().is_some();
    let start = origins[first].0;
    let end = origins[first + needle_len - 1].1;
    Some((
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
        duplicate,
    ))
}

/// The whitespace-normalized form of `text`, plus, for each of its
/// characters, the `(start, end)` character offsets it came from in `text`.
fn normalize_with_origins(text: &str) -> (String, Vec<(usize, usize)>) {
    let mut normalized = String::with_capacity(text.len());
    let mut origins = Vec::new();
    let mut pending_space = false;
    for (index, character) in text.chars().enumerate() {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            normalized.push(' ');
            origins.push((index, index));
            pending_space = false;
        }
        normalized.push(character);
        origins.push((index, index + 1));
    }
    (normalized, origins)
}

// ── Persistence and instrumentation ─────────────────────────────────────────

async fn persist_sweep(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    reviewer_model_id: &str,
    branches: Vec<Result<BranchOutcome, ReportPipelineError>>,
) -> Result<ReviewSweepOutcome, ReportPipelineError> {
    let now = jiff::Timestamp::now();
    let mut statuses = Vec::with_capacity(branches.len());
    let mut fresh = Vec::new();
    let mut coverage = Vec::new();
    let mut usage: Option<TurnUsage> = None;
    let mut converse_calls = 0_u32;

    for (property, branch) in ReviewProperty::ALL.into_iter().zip(branches) {
        match branch {
            Ok(branch) => {
                usage = merge_optional_usage(usage, branch.usage.clone());
                converse_calls = converse_calls.saturating_add(branch.converse_calls);
                let count = u32::try_from(branch.findings.len()).unwrap_or(u32::MAX);
                if !branch.warnings.is_empty() {
                    tracing::warn!(
                        property = branch.property.as_str(),
                        warnings = ?branch.warnings,
                        "a review branch returned quotes the host could not resolve"
                    );
                }
                // Only a completed branch earns a coverage row: the row is a
                // receipt that this property was read over this revision, and
                // a failed branch read nothing anyone can rely on.
                coverage.push(ReviewCoverage {
                    pass: if branch.property.is_style() {
                        ReviewPass::Style
                    } else {
                        ReviewPass::Consistency
                    },
                    property: branch.property.as_str().to_string(),
                    model_id: reviewer_model_id.to_string(),
                    revision,
                    sections_reviewed: branch.sections_reviewed,
                    findings: count,
                    completed_at: now,
                });
                fresh.extend(branch.findings);
                statuses.push(ReviewPropertyStatus {
                    property: branch.property.as_str(),
                    error: None,
                    findings: count,
                });
            }
            Err(error) => {
                tracing::warn!(
                    property = property.as_str(),
                    "a review branch failed; the rest of the sweep was unaffected"
                );
                statuses.push(ReviewPropertyStatus {
                    property: property.as_str(),
                    error: Some(error.to_string()),
                    findings: 0,
                });
            }
        }
    }

    if fresh.len() > MAX_REPORT_FINDINGS {
        // The store keeps the first MAX_REPORT_FINDINGS. Saying so is the
        // difference between a truncated review and a review that appears to
        // have found exactly the ceiling.
        tracing::warn!(
            raised = fresh.len(),
            retained = MAX_REPORT_FINDINGS,
            "the review raised more findings than one report retains; the excess was dropped"
        );
    }

    let mut loaded = load_report_findings(s3, bucket, client_id, report_id).await?;
    replace_findings_for_revision(&mut loaded.findings, revision, fresh, coverage, now)?;
    save_report_findings(s3, bucket, &mut loaded).await?;

    Ok(ReviewSweepOutcome {
        findings: loaded.findings,
        properties: statuses,
        usage,
        converse_calls,
    })
}

/// What the fan-out actually got out of the shared prefix.
///
/// The warming branch writes the cache and the six after it should read it,
/// so the branch write total should stay near zero. It will not when Bedrock
/// scatters the branches across regional capacity — each one lands somewhere
/// that has never seen the prefix and writes its own copy — and the only
/// signal of that is the write tokens scaling with the branch count. Counts
/// and model IDs only; nothing here describes what was reviewed.
fn log_fan_out(model_id: &str, branches: &[Result<BranchOutcome, ReportPipelineError>]) {
    let usage = |branch: &Result<BranchOutcome, ReportPipelineError>| {
        branch.as_ref().ok().and_then(|branch| branch.usage.clone())
    };
    let warm_cache_write_tokens = branches
        .first()
        .and_then(usage)
        .map_or(0, |usage| usage.cache_write_input_tokens);
    let siblings = &branches[branches.len().min(1)..];
    let branch_cache_write_tokens_total = siblings
        .iter()
        .filter_map(usage)
        .fold(0_u64, |total, usage| {
            total.saturating_add(usage.cache_write_input_tokens)
        });
    let branch_cache_read_tokens_total = siblings
        .iter()
        .filter_map(usage)
        .fold(0_u64, |total, usage| {
            total.saturating_add(usage.cache_read_input_tokens)
        });
    let branch_count = siblings.len();
    tracing::info!(
        target: "claria_bedrock::cache",
        model_id,
        branches = branch_count,
        warm_cache_write_tokens,
        branch_cache_write_tokens_total,
        branch_cache_read_tokens_total,
        "fan_out_complete"
    );
    let scattering_threshold = warm_cache_write_tokens as f64 * 0.5 * branch_count as f64;
    if warm_cache_write_tokens > 0 && branch_cache_write_tokens_total as f64 > scattering_threshold
    {
        tracing::warn!(
            target: "claria_bedrock::cache",
            model_id,
            branches = branch_count,
            warm_cache_write_tokens,
            branch_cache_write_tokens_total,
            "review branches re-wrote the shared prefix instead of reading it; the fan-out is probably being scattered across regional capacity"
        );
    }
}
