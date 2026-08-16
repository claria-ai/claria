//! Parallel section drafting: one transient conversation per planned section,
//! over one shared prefix, with the host owning everything durable.
//!
//! The serial writer holds a single conversation open for the whole document
//! and writes one section per response. That is the right shape when the
//! sections have to be written in order and each one can read the last, and
//! the wrong shape once a plan exists: a forty-section report becomes forty
//! sequential round trips, each one waiting on a model that has already been
//! told everything it needs. A plan a clinician approved is exactly the
//! statement that the sections are independent — it says what each one must
//! assert and which records it rests on — so they can be written at once.
//!
//! ```text
//! system:      composed writer prompt + trust rules + parallel rules  ┐ identical
//! messages[0]: <untrusted_record_context>                             │ across
//!              <untrusted_template_context>   ● CachePoint 1 (1h)     │ branches
//!              <plan_context>                 ● CachePoint 2 (1h)     ┘
//!              "Assigned section: {uuid} ({heading})" …  ← the only differing bytes
//! ```
//!
//! Blocks 0–2 are built by the same functions the serial conversation uses
//! ([`crate::full_draft_context`]) and the checkpoints sit at the same
//! coordinates, so a parallel run reads the prefix a serial attempt at the
//! same run already paid to write, and vice versa. Execution is
//! warm-then-fan-out for the same reason the review sweep is: firing every
//! branch at a cold cache would have each of them write the same prefix at the
//! one-hour write rate.
//!
//! **Branches never touch the run.** A branch validates its section and hands
//! the result back; the coordinator, which is single-threaded, is the only
//! thing that mutates [`LoadedRun`], persists it, and emits progress. That is
//! what makes the progress counters monotonic and the ETag chain a chain. It
//! also means the title and the finish are the host's: the title is one more
//! branch whose answer the coordinator applies, and the finish is pure code
//! with no model call at all, because a plan every section was written from
//! already says what "finished" means.

use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU32, Ordering},
};

use aws_sdk_s3::Client as S3Client;
use claria_bedrock::{
    converse::CachePlan,
    error::BedrockError,
    report::{
        self, ReportStopReason, ReportToolCall, ReportToolRequest, WriteFullDraftSectionRequest,
    },
    retry::with_throttle_retry,
};
use claria_core::models::{
    report::{
        ReportAuthoringTurn, ReportContent, ReportProtocolBlock, ReportProtocolMessage,
        ReportProtocolRole, ReportToolResultStatus, validate_report_content,
    },
    report_run::{
        DraftRun, DraftRunStatus, MAX_SECTION_ERROR_CHARACTERS, PlanEntry, RunSectionState,
        SectionIntent,
    },
    turn_usage::TurnUsage,
};
use claria_report_store::{
    LoadedRun, LoadedWorkspace, REPORT_CONFLICT_MESSAGE, ReportAttemptStatus, ReportFailureCode,
    ReportStoreError, mark_template_current, persist_call_usage, save_draft_run, save_loaded,
};
use futures::stream::{self, StreamExt};
use uuid::Uuid;

use crate::{
    BEDROCK_FAN_OUT_CONCURRENCY, CONVERSE_CALLS_FIELD_LABEL, FullReportGenerationOutcome,
    ReportPipelineError, ReportTurnLimits, ReportTurnOutcome, WRITER_LIMITS_SECTION,
    context::sha256_hex,
    full_draft_context::{
        DraftTurnKind, cache_checkpoints, plan_context, section_kickoff_instruction,
        template_context, title_kickoff_instruction,
    },
    prompts::full_report_parallel_system_prompt,
    record_context::FullRecordContext,
    run::{park_stopped_run, release_failed_run, stamp_run_authorship},
    tools::{
        MAX_SECTION_WRITE_ATTEMPTS, PreparedSectionContent, ToolRejection, apply_section_content,
        apply_section_failure, assemble_finished_draft, plan_total, renumber_positions_plan_order,
        section_failure_diagnostic, staged_section, terminal_section_count,
        validate_branch_section_write, written_sections,
    },
    turn::{
        AttemptProgress, FailedReportCall, FullReportRequest, ReportTurnProgress, TurnInterruption,
        TurnRunFailure, call_usage_record, map_bedrock_failure, merge_usage,
        prepare_full_draft_context, settle_attempt, terminal_stop_failure,
    },
};

/// Converse calls one branch may spend, derived from what it is allowed to
/// do: the opening request, a repair round for every write attempt after the
/// first, and one corrective round for a response that produced no tool call
/// at all. A branch that has used all four has stopped making progress, and
/// spending more of the run's shared budget on it costs the sections behind
/// it in the queue.
const MAX_BRANCH_CONVERSE_CALLS: u32 = 1 + (MAX_SECTION_WRITE_ATTEMPTS - 1) + 1;

/// The label this fan-out's Bedrock calls carry in the retry log.
const OPERATION: &str = "report_parallel_draft";

// ── Entry point ─────────────────────────────────────────────────────────────

/// Draft every pending planned section of a run in parallel, then cut the
/// revision from what landed.
///
/// The same contract as [`crate::turn::execute_full_draft_turn`], stated once
/// because both paths have to keep it: on success the run owns the revision it
/// cut, on failure the workspace pointer is released while the sections that
/// landed stay durable, and on a stop the run is parked with the pointer still
/// on it so the writing surface can pick it back up.
// The run and how the turn was entered ride alongside the workspace rather
// than inside the request: both outlive the request and neither is the user's.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_parallel_draft_turn(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    mut loaded: LoadedWorkspace,
    model_id: &str,
    request: FullReportRequest<'_>,
    mut run: LoadedRun,
    turn_kind: DraftTurnKind,
) -> Result<FullReportGenerationOutcome, ReportPipelineError> {
    let client_id = loaded.workspace.client_id;
    let report_id = loaded.workspace.report_id;
    let record_context =
        match prepare_full_draft_context(s3, bucket, client_id, model_id, request).await {
            Ok(record_context) => record_context,
            Err(error) => {
                release_failed_run(s3, bucket, client_id, report_id, &mut run).await;
                return Err(error);
            }
        };

    let started_at = jiff::Timestamp::now();
    let mut progress = AttemptProgress::new(&loaded.workspace, model_id, started_at);
    let run_id = run.run.run_id;
    let result = run_parallel_draft(
        sdk_config,
        s3,
        bucket,
        request,
        &record_context,
        model_id,
        &mut loaded,
        &mut run,
        &mut progress,
        turn_kind,
    )
    .await;
    let outcome = settle_attempt(s3, bucket, &mut progress, Some(run_id), result).await;

    match outcome {
        Ok(outcome) => Ok(FullReportGenerationOutcome {
            workspace: outcome.workspace,
            turn_id: outcome.turn_id,
            assistant_text: outcome.assistant_text,
            record_context: record_context.summary,
            attempt: outcome.attempt,
        }),
        Err(error) if error.is_stopped() => {
            park_stopped_run(s3, bucket, &mut run).await;
            Err(error)
        }
        Err(error) => {
            release_failed_run(s3, bucket, client_id, report_id, &mut run).await;
            Err(error)
        }
    }
}

// ── Coordinator ─────────────────────────────────────────────────────────────

/// One section a branch has been given, and the document slot it will occupy
/// however long it takes to arrive.
#[derive(Clone)]
struct SectionAssignment {
    entry: PlanEntry,
    /// Plan-order position, fixed before any branch starts.
    position: u32,
}

enum BranchKind {
    Section { assignment: SectionAssignment },
    Title,
}

/// What one branch decided, kept apart from what it spent.
enum BranchVerdict {
    /// Host-validated section content, not yet persisted — the coordinator
    /// owns the run.
    Drafted(PreparedSectionContent),
    TitleSet(String),
    SectionFailed {
        code: &'static str,
        message: String,
    },
    Stopped,
    Errored(TurnRunFailure),
}

/// One completed Converse call inside a branch.
struct BranchCallReceipt {
    usage: Option<TurnUsage>,
    stop_reason: ReportStopReason,
    latency_ms: Option<u64>,
}

/// A finished branch.
///
/// The verdict lives inside the outcome rather than the outcome being a
/// `Result`, because drafting bills per call: a branch that failed on its
/// third repair round still spent three calls, and losing those receipts to an
/// `Err` would under-report what the run cost.
struct BranchOutcome {
    /// The section this branch was assigned; `None` for the title branch.
    kind_id: Option<Uuid>,
    verdict: BranchVerdict,
    receipts: Vec<BranchCallReceipt>,
    tool_uses: u32,
    /// The exact input count this branch took, if it took one. Only the warm
    /// branch does.
    verified: Option<(u32, u64)>,
}

/// Everything every branch reads, borrowed rather than cloned: the shared
/// prefix is by far the largest thing in the request, and copying it once per
/// section to satisfy the borrow checker would defeat the point of sharing it.
struct DraftBranchContext<'a> {
    sdk_config: &'a aws_config::SdkConfig,
    model_id: &'a str,
    system_prompt: &'a str,
    /// Blocks 0–2 of message 0, identical in every branch's request.
    prefix: &'a [ReportProtocolBlock],
    /// The run as it stood when the fan-out started. Read-only: a branch
    /// validates against it and never learns what its siblings did.
    snapshot: &'a DraftRun,
    base: &'a ReportContent,
    citable_filenames: &'a [String],
    guidance: &'a str,
    cache_plan: &'a CachePlan,
    tuning: claria_bedrock::converse::ModelTuning,
    max_tool_uses_per_response: usize,
    limits: ReportTurnLimits,
    stop: &'a claria_bedrock::converse::StopSignal,
    /// Calls the whole fan-out has opened, against the run's shared ceiling.
    calls: &'a AtomicU32,
    progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
}

impl DraftBranchContext<'_> {
    fn emit(&self, event: ReportTurnProgress) {
        if let Some(progress) = self.progress {
            progress(event);
        }
    }

    /// Take one call off the run's shared ceiling, or refuse.
    fn reserve_call(&self) -> Result<u32, String> {
        let taken = self.calls.fetch_add(1, Ordering::SeqCst);
        if taken >= self.limits.max_converse_calls() {
            return Err(format!(
                "This whole-report draft reached its ceiling of {} Bedrock calls before this section was written. Raise \"{CONVERSE_CALLS_FIELD_LABEL}\" in {WRITER_LIMITS_SECTION}, or plan fewer sections. Each added call is billed.",
                self.limits.max_converse_calls()
            ));
        }
        Ok(taken.saturating_add(1))
    }
}

/// What the coordinator has accumulated while draining the fan-out.
#[derive(Default)]
struct MergeState {
    /// Coordinator-assigned call numbers. A branch cannot number its own
    /// calls without racing its siblings, so numbering happens where the
    /// receipts are written, in commit order.
    call_number: u32,
    stopped: bool,
    /// The first branch failure with a typed cause, kept in case every branch
    /// fails and the run has to name one reason rather than none.
    first_failure: Option<TurnRunFailure>,
    first_diagnostic: Option<String>,
    warm_cache_write_tokens: u64,
    branch_cache_write_tokens: u64,
    branch_cache_read_tokens: u64,
    branches: usize,
    section_ids: Vec<Uuid>,
}

// The run, the workspace, and the attempt receipt are all mutated for the life
// of the turn, and none of them belongs inside the user's request.
#[allow(clippy::too_many_arguments)]
async fn run_parallel_draft(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    request: FullReportRequest<'_>,
    record_context: &FullRecordContext,
    model_id: &str,
    loaded: &mut LoadedWorkspace,
    run: &mut LoadedRun,
    progress: &mut AttemptProgress,
    turn_kind: DraftTurnKind,
) -> Result<ReportTurnOutcome, TurnInterruption> {
    let plan_len = run
        .run
        .plan
        .as_ref()
        .map_or(run.run.sections.len(), |plan| plan.entries.len());
    request.emit_progress(ReportTurnProgress::PlanReady {
        section_count: u32::try_from(plan_len).unwrap_or(u32::MAX),
    });
    let limits = request.limits.scaled_for_plan(plan_len);
    loaded
        .workspace
        .prune_turns(limits.max_retained_turns() as usize)
        .map_err(|error| {
            TurnRunFailure::new(ReportFailureCode::InvalidProtocol, error.to_string())
        })?;

    let guidance = run_guidance(&run.run, request.guidance.trim());
    let base = loaded.workspace.draft.content.clone();
    let template = template_context(&loaded.workspace, &base)
        .map_err(|message| TurnRunFailure::new(ReportFailureCode::InvalidProtocol, message))?;
    let plan_block = plan_context(run.run.plan.as_ref())
        .map_err(|message| TurnRunFailure::new(ReportFailureCode::InvalidProtocol, message))?;
    // Blocks 0-2, byte-identical to the serial conversation's, so both shapes
    // read the same cached prefix for the same run.
    let prefix = vec![
        ReportProtocolBlock::Text {
            text: record_context.prompt.clone(),
        },
        ReportProtocolBlock::Text { text: template },
        ReportProtocolBlock::Text { text: plan_block },
    ];

    let system_prompt = full_report_parallel_system_prompt(request.system_prompt_body);
    progress.system_prompt_sha256 = sha256_hex(&system_prompt);
    // A drafting run's prefix has nothing in common with a targeted edit's.
    if let Some(cache) = request.prompt_cache {
        cache.invalidate(loaded.workspace.report_id);
    }
    let capabilities = claria_core::model_id::ModelCapabilities::for_id(model_id);
    let cache_plan =
        CachePlan::parallel_draft(capabilities, cache_checkpoints()).map_err(|error| {
            TurnRunFailure::new(
                ReportFailureCode::InvalidProtocol,
                "Claria could not lay out the drafting conversation's prompt cache.",
            )
            .with_internal(error)
        })?;

    // Everything the plan already decided, decided here: a keep or a skip row
    // buys nothing from a model, and the positions have to be settled before a
    // branch can be told which slot its section owns.
    let now = jiff::Timestamp::now();
    renumber_positions_plan_order(&mut run.run);
    let host_skipped = apply_host_side_rows(&mut run.run, now);
    commit_run(s3, bucket, run).await?;
    for section_id in &host_skipped {
        let (drafted, total) = counters(&run.run);
        request.emit_progress(ReportTurnProgress::SectionSkipped {
            section_id: section_id.to_string(),
            drafted,
            total,
        });
    }

    let assignments = pending_assignments(&run.run);
    let needs_title = run.run.title.is_none();
    let snapshot = run.run.clone();
    let calls = AtomicU32::new(0);
    let context = DraftBranchContext {
        sdk_config,
        model_id,
        system_prompt: &system_prompt,
        prefix: &prefix,
        snapshot: &snapshot,
        base: &base,
        citable_filenames: &record_context.citable_filenames,
        guidance: &guidance,
        cache_plan: &cache_plan,
        tuning: request.model_tuning,
        max_tool_uses_per_response: limits.max_tool_uses_per_response() as usize,
        limits,
        stop: request.stop,
        calls: &calls,
        progress: request.progress,
    };

    let mut state = MergeState::default();
    // Warm-then-fan-out. The first plan section runs alone with the run's one
    // CountTokens and writes both checkpoints; everything after it reads them
    // and estimates forward from its count. If the warm branch errors it seeds
    // nothing and the siblings run unseeded — one wasted cache write is a far
    // cheaper failure than refusing to draft the rest of the document.
    let mut seed = None;
    let mut queued: Vec<BranchKind> = Vec::new();
    match assignments.split_first() {
        Some((warm, rest)) => {
            let outcome = run_branch(
                &context,
                BranchKind::Section {
                    assignment: warm.clone(),
                },
                None,
            )
            .await;
            seed = outcome.verified;
            state.warm_cache_write_tokens = cache_write_tokens(&outcome);
            commit_branch(s3, bucket, run, progress, &mut state, request, outcome).await?;
            queued.extend(
                rest.iter()
                    .cloned()
                    .map(|assignment| BranchKind::Section { assignment }),
            );
        }
        None => {
            // Nothing to draft: every row was kept, skipped, or already
            // landed. The title branch, if the run still needs one, is the
            // whole fan-out and pays for its own count.
        }
    }
    if needs_title {
        queued.push(BranchKind::Title);
    }

    // `buffer_unordered`, not `buffered`: commits land in COMPLETION order, so
    // a run that dies mid-fan-out keeps every section that had finished rather
    // than only the ones ahead of the slowest branch. Document order is not at
    // stake — positions were assigned from the plan before any branch started.
    let mut fanned = stream::iter(
        queued
            .into_iter()
            .map(|kind| run_branch(&context, kind, seed)),
    )
    .buffer_unordered(BEDROCK_FAN_OUT_CONCURRENCY);
    while let Some(outcome) = fanned.next().await {
        state.branches = state.branches.saturating_add(1);
        state.branch_cache_write_tokens = state
            .branch_cache_write_tokens
            .saturating_add(cache_write_tokens(&outcome));
        state.branch_cache_read_tokens = state
            .branch_cache_read_tokens
            .saturating_add(cache_read_tokens(&outcome));
        commit_branch(s3, bucket, run, progress, &mut state, request, outcome).await?;
    }
    drop(fanned);
    log_fan_out(model_id, turn_kind, &state);

    // A Stop that arrived while sections were being written to S3 belongs to
    // this round: what landed stands, and the run is parked exactly here.
    if state.stopped || request.stop.is_stopped() {
        return Err(TurnInterruption::Stopped);
    }

    finish_run(
        s3,
        bucket,
        request,
        record_context,
        loaded,
        run,
        progress,
        limits,
        &guidance,
        state,
    )
    .await
}

/// Everything the clinician has asked this run for, oldest first.
///
/// The run is the authority on its own guidance: a gated draft was planned
/// against instructions typed at the plan step, and the Start button that
/// follows carries none of its own. Instructions added at a resume are here
/// too. A re-plan folds most of what they say into the per-section rows, but
/// only most: a run-wide "keep it under two pages" has no row to live on, and
/// a branch that never saw it would write as though it had never been said.
fn run_guidance(run: &DraftRun, fallback: &str) -> String {
    if run.instructions.is_empty() {
        return fallback.to_string();
    }
    run.instructions
        .iter()
        .map(|instruction| instruction.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Carry out the plan rows nobody needs a model for.
///
/// A `skip` row is already skipped by the time the run reaches here — the plan
/// gate applies it — but a `keep` row is not: it means the base revision's
/// content stands, which is [`RunSectionState::Kept`], and nothing had ever
/// produced that state before the fan-out existed. Returns the skipped section
/// IDs so the coordinator can report them once the decision is durable.
fn apply_host_side_rows(run: &mut DraftRun, now: jiff::Timestamp) -> Vec<Uuid> {
    let Some(plan) = run.plan.clone() else {
        return Vec::new();
    };
    let mut skipped = Vec::new();
    for entry in &plan.entries {
        let Some(section) = run
            .sections
            .iter_mut()
            .find(|section| section.section_id == entry.section_id)
        else {
            continue;
        };
        match entry.intent {
            SectionIntent::Skip => {
                if section.state != RunSectionState::Skipped {
                    section.state = RunSectionState::Skipped;
                    section.blocks.clear();
                    section.citations.clear();
                    section.error = None;
                    section.updated_at = now;
                }
                skipped.push(section.section_id);
            }
            // A keep row over a section that already landed is satisfied by
            // the section itself; over one that never started, it is the
            // statement that the base revision's text is the answer.
            SectionIntent::Keep if section.state == RunSectionState::Pending => {
                section.state = RunSectionState::Kept;
                section.blocks.clear();
                section.citations.clear();
                section.error = None;
                section.updated_at = now;
            }
            _ => {}
        }
    }
    skipped
}

/// The sections this attempt owes a draft: a `draft` or `rewrite` plan row
/// over a section still pending, in plan order.
///
/// A resume needs no special case. Sections that already landed are `Drafted`
/// and never appear here, so a picked-up run fans out over exactly what is
/// left — which is also why a branch kick-off needs no section-state table:
/// there is nothing in the run's history it could act on.
fn pending_assignments(run: &DraftRun) -> Vec<SectionAssignment> {
    run.plan
        .iter()
        .flat_map(|plan| plan.entries.iter())
        .filter(|entry| matches!(entry.intent, SectionIntent::Draft | SectionIntent::Rewrite))
        .filter_map(|entry| {
            let section = run
                .sections
                .iter()
                .find(|section| section.section_id == entry.section_id)?;
            (section.state == RunSectionState::Pending).then(|| SectionAssignment {
                entry: entry.clone(),
                position: section.position,
            })
        })
        .collect()
}

fn counters(run: &DraftRun) -> (u32, u32) {
    (terminal_section_count(run), plan_total(run))
}

async fn commit_run(
    s3: &S3Client,
    bucket: &str,
    run: &mut LoadedRun,
) -> Result<(), TurnRunFailure> {
    save_draft_run(s3, bucket, run).await.map_err(|error| {
        TurnRunFailure::new(
            ReportFailureCode::RecordStorage,
            "Claria could not save the drafting run to managed storage, so the draft was stopped. Sections saved before this point were kept.",
        )
        .with_internal(error)
    })
}

/// Fold one finished branch into the run.
///
/// Single-threaded by construction, which is what every guarantee downstream
/// rests on: call numbers are assigned in one place, the run's ETag chain is a
/// chain, and a `SectionCompleted` event is only ever emitted after the write
/// it describes is durable — so the counters a reader watches never go
/// backwards and never run ahead of S3.
async fn commit_branch(
    s3: &S3Client,
    bucket: &str,
    run: &mut LoadedRun,
    progress: &mut AttemptProgress,
    state: &mut MergeState,
    request: FullReportRequest<'_>,
    outcome: BranchOutcome,
) -> Result<(), TurnRunFailure> {
    let BranchOutcome {
        kind_id,
        verdict,
        receipts,
        tool_uses,
        ..
    } = outcome;

    // Receipts first, whatever the verdict: a branch that failed on its third
    // repair round still spent three billed calls.
    progress.tool_uses = progress.tool_uses.saturating_add(tool_uses);
    for receipt in receipts {
        state.call_number = state.call_number.saturating_add(1);
        let call_number = state.call_number;
        request.emit_progress(ReportTurnProgress::ModelCallStarted { call_number });
        match &receipt.usage {
            Some(usage) => merge_usage(&mut progress.usage, usage, call_number == 1),
            None => progress.usage_complete = false,
        }
        progress.converse_calls = call_number;
        persist_call_usage(
            s3,
            bucket,
            &call_usage_record(
                progress,
                call_number,
                receipt.usage,
                receipt.stop_reason,
                receipt.latency_ms,
            ),
        )
        .await
        .map_err(|error| {
            TurnRunFailure::new(
                ReportFailureCode::UsagePersistence,
                "Claria could not record usage for a completed Bedrock call, so the draft was stopped.",
            )
            .with_internal(error)
        })?;
    }

    let now = jiff::Timestamp::now();
    match verdict {
        BranchVerdict::Drafted(content) => {
            let Some(section_id) = kind_id else {
                return Ok(());
            };
            state.section_ids.push(section_id);
            apply_section_content(&mut run.run, section_id, content, now);
            commit_run(s3, bucket, run).await?;
            let (drafted, total) = counters(&run.run);
            if let Some(section) = run
                .run
                .sections
                .iter()
                .find(|section| section.section_id == section_id)
            {
                request.emit_progress(ReportTurnProgress::SectionCompleted {
                    section_id: section_id.to_string(),
                    section: staged_section(section),
                    drafted,
                    total,
                });
            }
        }
        BranchVerdict::TitleSet(title) => {
            run.run.title = Some(title.clone());
            commit_run(s3, bucket, run).await?;
            request.emit_progress(ReportTurnProgress::TitleSet { title });
        }
        BranchVerdict::SectionFailed { code, message } => {
            record_branch_failure(
                s3, bucket, run, state, request, kind_id, code, &message, now,
            )
            .await?;
        }
        BranchVerdict::Stopped => state.stopped = true,
        BranchVerdict::Errored(failure) => {
            let message = failure.message().to_string();
            if state.first_failure.is_none() {
                state.first_failure = Some(failure);
            }
            record_branch_failure(
                s3,
                bucket,
                run,
                state,
                request,
                kind_id,
                "branch_failed",
                &message,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

/// Record a branch that produced nothing usable.
///
/// A section branch fails its own section and the run carries on; a title
/// branch fails nothing at all — the base revision's title is a perfectly good
/// answer, and losing a finished document because nobody named it would be
/// absurd.
// The failure has to be attributed, persisted, and announced, and each of
// those needs a different one of these.
#[allow(clippy::too_many_arguments)]
async fn record_branch_failure(
    s3: &S3Client,
    bucket: &str,
    run: &mut LoadedRun,
    state: &mut MergeState,
    request: FullReportRequest<'_>,
    kind_id: Option<Uuid>,
    code: &str,
    message: &str,
    now: jiff::Timestamp,
) -> Result<(), TurnRunFailure> {
    let diagnostic = section_failure_diagnostic(code, message);
    if state.first_diagnostic.is_none() {
        state.first_diagnostic = Some(diagnostic.clone());
    }
    let Some(section_id) = kind_id else {
        tracing::warn!(
            code,
            "the title branch produced no title; the base revision's title stands"
        );
        return Ok(());
    };
    state.section_ids.push(section_id);
    apply_section_failure(&mut run.run, section_id, diagnostic.clone(), now);
    commit_run(s3, bucket, run).await?;
    let (drafted, total) = counters(&run.run);
    request.emit_progress(ReportTurnProgress::SectionFailed {
        section_id: section_id.to_string(),
        message: diagnostic,
        drafted,
        total,
    });
    Ok(())
}

/// Cut the revision from what the fan-out landed. No model call: the plan
/// every branch wrote from already says what the document is.
// The finish touches the workspace, the run, and the attempt receipt, in that
// fixed order; folding them into one argument would hide the ordering.
#[allow(clippy::too_many_arguments)]
async fn finish_run(
    s3: &S3Client,
    bucket: &str,
    request: FullReportRequest<'_>,
    record_context: &FullRecordContext,
    loaded: &mut LoadedWorkspace,
    run: &mut LoadedRun,
    progress: &mut AttemptProgress,
    limits: ReportTurnLimits,
    guidance: &str,
    state: MergeState,
) -> Result<ReportTurnOutcome, TurnInterruption> {
    // Every branch failing is one failure with many symptoms — a bad model ID,
    // an unreachable region, an exhausted quota. Reporting the first of them
    // is more use than a revision with nothing in it.
    if !run
        .run
        .sections
        .iter()
        .any(|section| section.state == RunSectionState::Drafted)
    {
        return Err(TurnInterruption::Failed(
            state.first_failure.unwrap_or_else(|| {
                let diagnostic = state
                    .first_diagnostic
                    .unwrap_or_else(|| "no branch produced a section".to_string());
                TurnRunFailure::new(
                    ReportFailureCode::InvalidProtocol,
                    format!(
                        "No section of this whole-report draft could be written, so nothing was saved. First failure: {diagnostic}"
                    ),
                )
            }),
        ));
    }

    let fallback_title = loaded.workspace.draft.content.title.clone();
    let assembled = assemble_finished_draft(
        &run.run,
        &loaded.workspace.draft.content.sections,
        Some(&fallback_title),
    )
    .map_err(|rejection| {
        TurnRunFailure::new(
            ReportFailureCode::InvalidProtocol,
            format!(
                "Claria could not assemble the finished whole-report draft: {}",
                rejection.message
            ),
        )
    })?;
    let summary = host_summary(&run.run);

    // Workspace first, run second — the revision is the clinician's document
    // and the run is bookkeeping about it, so a lost run write heals from the
    // workspace rather than losing a revision that already landed.
    let completed_at = jiff::Timestamp::now();
    let expected_revision = loaded.workspace.draft.revision;
    loaded.workspace.draft = loaded
        .workspace
        .draft
        .replace_content(expected_revision, assembled.content, completed_at)
        .map_err(|error| {
            TurnRunFailure::new(ReportFailureCode::InvalidProtocol, error.to_string())
        })?;
    let revision = loaded.workspace.draft.revision;
    stamp_run_authorship(
        &mut loaded.workspace.draft.content,
        &run.run,
        revision,
        completed_at,
    );
    // The run no longer owns the workspace: the revision it was building
    // toward is cut, so edits, retries, and saves are free again.
    loaded.workspace.active_run_id = None;
    loaded.workspace.session.pending_proposal = None;
    loaded.workspace.session.last_agent_revision = Some(revision);
    mark_template_current(&mut loaded.workspace);

    // Nothing in this turn's persisted messages came from a model: the writer
    // never held a conversation the session could show. The instruction the
    // user gave and the host's own account of what the run did are what the
    // Writing pane reads back, and both are PHI-free by construction, so no
    // sanitization pass is needed on the way in.
    let persisted_instruction = if guidance.is_empty() {
        "Fill the complete report from all readable client records.".to_string()
    } else {
        format!(
            "Fill the complete report from all readable client records.\n\nGuidance: {guidance}"
        )
    };
    let turn = ReportAuthoringTurn {
        id: Uuid::new_v4(),
        model_id: progress.model_id.clone(),
        messages: vec![
            ReportProtocolMessage {
                role: ReportProtocolRole::User,
                content: vec![ReportProtocolBlock::Text {
                    text: persisted_instruction,
                }],
                created_at: progress.started_at,
            },
            ReportProtocolMessage {
                role: ReportProtocolRole::Assistant,
                content: vec![ReportProtocolBlock::Text {
                    text: summary.clone(),
                }],
                created_at: completed_at,
            },
        ],
        record_context_files: record_context.files.clone(),
        usage: progress.usage.clone(),
        usage_complete: progress.usage_complete,
        converse_calls: progress.converse_calls,
        // Zero, and correct: the turn's count is a property of the protocol
        // it persists, which the domain validator checks, and the branches'
        // tool calls happened in conversations no session ever sees. What the
        // fan-out actually spent is on the attempt receipt.
        tool_uses: 0,
        created_at: progress.started_at,
        completed_at,
    };
    let turn_id = turn.id;
    loaded
        .workspace
        .push_turn_with_limit(turn, limits.max_retained_turns() as usize)
        .map_err(|error| {
            TurnRunFailure::new(ReportFailureCode::InvalidProtocol, error.to_string())
        })?;
    loaded.workspace.updated_at = completed_at;
    save_loaded(s3, bucket, loaded).await.map_err(|error| match error {
        ReportStoreError::Conflict => TurnRunFailure::new(
            ReportFailureCode::WorkspaceConflict,
            REPORT_CONFLICT_MESSAGE,
        ),
        other => TurnRunFailure::new(
            ReportFailureCode::RecordStorage,
            "Claria could not save the completed report turn to managed storage. Reload before retrying.",
        )
        .with_internal(other),
    })?;

    run.run.status = DraftRunStatus::Completed;
    run.run.finalized_revision = Some(revision);
    if let Err(error) = save_draft_run(s3, bucket, run).await {
        tracing::warn!(
            run_id = %run.run.run_id,
            error = %error,
            "the drafting run completed but could not record its own finish"
        );
    }

    // A whole-draft system prompt cannot share the targeted-edit prefix.
    if let Some(cache) = request.prompt_cache {
        cache.invalidate(loaded.workspace.report_id);
    }

    Ok(ReportTurnOutcome {
        workspace: loaded.workspace.clone(),
        turn_id,
        assistant_text: summary,
        proposal_id: None,
        // Replaced after the append-only final attempt record succeeds.
        attempt: progress.metadata(ReportAttemptStatus::Completed, None),
    })
}

/// The host's own account of the run, in place of the closing summary a serial
/// conversation would have written. Counts and failure codes only — no
/// heading, no section text.
fn host_summary(run: &DraftRun) -> String {
    let count = |state: RunSectionState| {
        run.sections
            .iter()
            .filter(|section| section.state == state)
            .count()
    };
    let drafted = count(RunSectionState::Drafted);
    let skipped = count(RunSectionState::Skipped);
    let kept = count(RunSectionState::Kept);
    let failed = count(RunSectionState::Failed);
    let planned = plan_total(run);
    let mut summary =
        format!("Drafted {drafted} of {planned} planned sections; {skipped} deferred");
    if kept > 0 {
        summary.push_str(&format!("; {kept} left as they were"));
    }
    let codes: BTreeSet<&str> = run
        .sections
        .iter()
        .filter(|section| section.state == RunSectionState::Failed)
        .filter_map(|section| section.error.as_deref())
        .map(|error| error.split(':').next().unwrap_or(error).trim())
        .collect();
    if failed == 0 {
        summary.push_str("; none failed.");
    } else if codes.is_empty() {
        summary.push_str(&format!("; {failed} failed."));
    } else {
        summary.push_str(&format!(
            "; {failed} failed ({}).",
            codes.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    summary
}

// ── Branches ────────────────────────────────────────────────────────────────

/// One branch: a single-shot conversation over the shared prefix, its repair
/// rounds, and the one decision it is allowed to hand back.
async fn run_branch(
    context: &DraftBranchContext<'_>,
    kind: BranchKind,
    seed: Option<(u32, u64)>,
) -> BranchOutcome {
    let kind_id = match &kind {
        BranchKind::Section { assignment } => Some(assignment.entry.section_id),
        BranchKind::Title => None,
    };
    let mut receipts = Vec::new();
    let mut tool_uses = 0_u32;
    let finished = |verdict, receipts, tool_uses, verified| BranchOutcome {
        kind_id,
        verdict,
        receipts,
        tool_uses,
        verified,
    };

    // Checked before the first call, not only between rounds: a branch still
    // waiting its turn in the queue must never open a billed conversation the
    // reader has already cancelled.
    if context.stop.is_stopped() {
        return finished(BranchVerdict::Stopped, receipts, tool_uses, None);
    }

    let kickoff = match &kind {
        BranchKind::Section { assignment } => {
            context.emit(ReportTurnProgress::SectionStarted {
                section_id: assignment.entry.section_id.to_string(),
                index: assignment.position,
                total: plan_total(context.snapshot),
            });
            section_kickoff_instruction(context.guidance, &assignment.entry)
        }
        BranchKind::Title => title_kickoff_instruction(context.guidance, context.base),
    };
    let mut content = context.prefix.to_vec();
    content.push(ReportProtocolBlock::Text { text: kickoff });
    let mut messages = vec![ReportProtocolMessage {
        role: ReportProtocolRole::User,
        content,
        created_at: jiff::Timestamp::now(),
    }];

    let budget = tokio::sync::Mutex::new(match seed {
        Some((tokens, chars)) => report::ReportInputBudget::seeded(context.model_id, tokens, chars),
        None => report::ReportInputBudget::new(context.model_id),
    });
    let mut strikes = 0_u32;
    let mut corrective_used = false;
    let mut last_rejection: Option<ToolRejection> = None;

    for _ in 0..MAX_BRANCH_CONVERSE_CALLS {
        let call_number = match context.reserve_call() {
            Ok(call_number) => call_number,
            Err(message) => {
                return finished(
                    BranchVerdict::SectionFailed {
                        code: "writer_call_limit",
                        message,
                    },
                    receipts,
                    tool_uses,
                    verified(&budget).await,
                );
            }
        };
        let started = tokio::time::Instant::now();
        // One Bedrock call, with the shared backoff underneath: a fan-out that
        // invented its own retry schedule would hammer a throttled account.
        // A failed attempt commits nothing — `messages` still holds only
        // completed messages — so re-sending the identical request is safe.
        let attempt = with_throttle_retry(OPERATION, || {
            let messages = &messages;
            let budget = &budget;
            async move {
                report::converse_full_report_with_tool_limit(
                    context.sdk_config,
                    context.model_id,
                    context.system_prompt,
                    messages,
                    context.max_tool_uses_per_response,
                    &mut *budget.lock().await,
                    context.tuning,
                    context.cache_plan.clone(),
                    context.stop,
                )
                .await
            }
        })
        .await;
        let output = match attempt {
            Ok(output) => output,
            // A stopped call is the reader's decision, never a transient
            // fault: retrying it would re-issue what they just cancelled.
            Err(BedrockError::Stopped) => {
                return finished(
                    BranchVerdict::Stopped,
                    receipts,
                    tool_uses,
                    verified(&budget).await,
                );
            }
            Err(error) => {
                let failure = map_bedrock_failure(
                    error,
                    FailedReportCall {
                        number: call_number,
                        ceiling: context.limits.max_converse_calls(),
                        model_id: context.model_id,
                    },
                    1,
                    started.elapsed(),
                );
                return finished(
                    BranchVerdict::Errored(failure),
                    receipts,
                    tool_uses,
                    verified(&budget).await,
                );
            }
        };
        receipts.push(BranchCallReceipt {
            usage: output.usage.clone(),
            stop_reason: output.stop_reason,
            latency_ms: output.latency_ms,
        });
        tool_uses = tool_uses.saturating_add(output.tool_calls.len() as u32);
        messages.push(output.message.clone());

        // Filtered, guardrailed, malformed, over the context window: the same
        // terminal set the serial loop refuses, and refused here for the same
        // reason — a branch cannot repair its way out of any of them, and a
        // tool call beside one is not work anybody should trust.
        if !matches!(
            output.stop_reason,
            ReportStopReason::EndTurn | ReportStopReason::ToolUse | ReportStopReason::MaxTokens
        ) {
            return finished(
                BranchVerdict::Errored(terminal_stop_failure(output.stop_reason)),
                receipts,
                tool_uses,
                verified(&budget).await,
            );
        }
        if output.tool_calls.is_empty() {
            // A response that ran out of output tokens before it produced its
            // one tool call has nothing salvageable in it either.
            if matches!(output.stop_reason, ReportStopReason::MaxTokens) {
                return finished(
                    BranchVerdict::Errored(terminal_stop_failure(output.stop_reason)),
                    receipts,
                    tool_uses,
                    verified(&budget).await,
                );
            }
            // A response that ended in prose, or one whose stop reason claimed
            // a tool call it never made. Either way the branch has nothing to
            // hand back, and one reminder is worth more than a failure.
            if corrective_used {
                return finished(
                    BranchVerdict::SectionFailed {
                        code: "no_tool_call",
                        message: "The writer answered in prose twice instead of calling the tool it was assigned.".to_string(),
                    },
                    receipts,
                    tool_uses,
                    verified(&budget).await,
                );
            }
            corrective_used = true;
            messages.push(reminder_message(&kind));
            continue;
        }

        let mut results = Vec::with_capacity(output.tool_calls.len());
        let mut rejected = None;
        for call in &output.tool_calls {
            match classify(context, &kind, call) {
                // A valid write ends the branch here, without sending the
                // success tool_result back: the conversation is over, and
                // returning it would buy one more billed call per section for
                // an answer nobody reads.
                Ok(verdict) => {
                    return finished(verdict, receipts, tool_uses, verified(&budget).await);
                }
                Err(rejection) => {
                    results.push(ReportProtocolBlock::ToolResult {
                        tool_use_id: call.tool_use_id.clone(),
                        status: ReportToolResultStatus::Error,
                        content: serde_json::json!({"error": {
                            "code": rejection.code,
                            "message": rejection.message,
                        }}),
                    });
                    rejected = Some(rejection);
                }
            }
        }
        if let Some(rejection) = rejected {
            strikes = strikes.saturating_add(1);
            let exhausted = strikes >= MAX_SECTION_WRITE_ATTEMPTS;
            last_rejection = Some(rejection);
            if exhausted {
                break;
            }
        }
        messages.push(ReportProtocolMessage {
            role: ReportProtocolRole::User,
            content: results,
            created_at: jiff::Timestamp::now(),
        });
    }

    let (code, message) = last_rejection.map_or(
        (
            "branch_exhausted",
            format!(
                "The writer used all {MAX_BRANCH_CONVERSE_CALLS} of this section's model calls without producing it."
            ),
        ),
        |rejection| (rejection.code, rejection.message),
    );
    finished(
        BranchVerdict::SectionFailed { code, message },
        receipts,
        tool_uses,
        verified(&budget).await,
    )
}

async fn verified(budget: &tokio::sync::Mutex<report::ReportInputBudget>) -> Option<(u32, u64)> {
    budget.lock().await.verified()
}

fn cache_write_tokens(outcome: &BranchOutcome) -> u64 {
    outcome
        .receipts
        .iter()
        .filter_map(|receipt| receipt.usage.as_ref())
        .fold(0_u64, |total, usage| {
            total.saturating_add(usage.cache_write_input_tokens)
        })
}

fn cache_read_tokens(outcome: &BranchOutcome) -> u64 {
    outcome
        .receipts
        .iter()
        .filter_map(|receipt| receipt.usage.as_ref())
        .fold(0_u64, |total, usage| {
            total.saturating_add(usage.cache_read_input_tokens)
        })
}

/// The one corrective round a branch gets when a response produced no tool
/// call, mirroring the serial loop's completion reminder.
fn reminder_message(kind: &BranchKind) -> ReportProtocolMessage {
    let text = match kind {
        BranchKind::Section { assignment } => format!(
            "You have not written your assigned section. Call write_full_draft_section once with section_id \"{}\", or mark_section_failed with the same ID if the records cannot support it. Do not answer in prose.",
            assignment.entry.section_id
        ),
        BranchKind::Title => "You have not set the report title. Call set_full_draft_title once. Do not answer in prose.".to_string(),
    };
    ReportProtocolMessage {
        role: ReportProtocolRole::User,
        content: vec![ReportProtocolBlock::Text { text }],
        created_at: jiff::Timestamp::now(),
    }
}

/// What one tool call from a branch means, or why it is refused.
///
/// The refusals are the shape of the mode: a branch owns one section and one
/// tool, and every other report tool — skipping, titling off the title branch,
/// finishing — is a decision the host makes with the plan in front of it.
fn classify(
    context: &DraftBranchContext<'_>,
    kind: &BranchKind,
    call: &ReportToolCall,
) -> Result<BranchVerdict, ToolRejection> {
    let request = report::decode_tool_request(call).map_err(|error| {
        // Carry the decode diagnostic verbatim: these messages describe JSON
        // structure, never report or record content, and a generic sentence
        // wastes the model's repair round.
        let diagnostic = match &error {
            BedrockError::SchemaViolation(message) => message.clone(),
            other => other.to_string(),
        };
        ToolRejection::new(
            "invalid_tool_input",
            format!("The tool input did not match the configured schema: {diagnostic}"),
        )
    })?;
    match (kind, request) {
        (BranchKind::Section { assignment }, ReportToolRequest::WriteFullDraftSection(request)) => {
            classify_write(context, assignment, request)
        }
        (BranchKind::Section { assignment }, ReportToolRequest::MarkSectionFailed(request)) => {
            let assigned = assignment.entry.section_id;
            if request.section_id.parse::<Uuid>() != Ok(assigned) {
                return Err(wrong_section(assigned));
            }
            let reason = request.reason.trim().to_string();
            if reason.is_empty() || reason.chars().count() > MAX_SECTION_ERROR_CHARACTERS {
                return Err(ToolRejection::new(
                    "invalid_failure_reason",
                    format!("reason must be 1–{MAX_SECTION_ERROR_CHARACTERS} characters."),
                ));
            }
            Ok(BranchVerdict::SectionFailed {
                code: "section_marked_failed",
                message: reason,
            })
        }
        (BranchKind::Title, ReportToolRequest::SetFullDraftTitle(request)) => {
            let proposed = ReportContent {
                title: request.title.clone(),
                sections: written_sections(context.snapshot),
            };
            validate_report_content(&proposed).map_err(|error| {
                ToolRejection::new(
                    "invalid_full_draft_title",
                    format!("The full-draft title was invalid: {error}"),
                )
            })?;
            Ok(BranchVerdict::TitleSet(request.title))
        }
        _ => Err(ToolRejection::new(
            "tool_not_available_in_parallel_mode",
            "This request drafts one assigned part of the document; the other parts, the title, and finishing the draft are handled elsewhere. Call only the tool your kick-off message named.",
        )),
    }
}

fn classify_write(
    context: &DraftBranchContext<'_>,
    assignment: &SectionAssignment,
    request: WriteFullDraftSectionRequest,
) -> Result<BranchVerdict, ToolRejection> {
    let assigned = assignment.entry.section_id;
    // A null section_id means "a new section" in the serial protocol. A branch
    // has no authority to invent one: the plan is the document's section list,
    // and a section nobody planned has no slot, no scope, and no reviewer.
    match request.section_id.as_deref().map(str::parse::<Uuid>) {
        Some(Ok(id)) if id == assigned => {}
        _ => return Err(wrong_section(assigned)),
    }
    validate_branch_section_write(
        context.snapshot,
        &context.base.title,
        context.citable_filenames,
        assigned,
        request,
    )
    .map(BranchVerdict::Drafted)
}

fn wrong_section(assigned: Uuid) -> ToolRejection {
    ToolRejection::new(
        "wrong_section",
        format!(
            "This request writes only section {assigned}. Copy that ID exactly into section_id; another writer owns every other section, and a new section cannot be added here."
        ),
    )
}

/// What the fan-out actually got out of the shared prefix.
///
/// The warm branch writes the cache and every branch after it should read it,
/// so the branch write total should stay near zero. It will not when Bedrock
/// scatters the branches across regional capacity — each lands somewhere that
/// has never seen the prefix and writes its own copy — and the only signal of
/// that is write tokens scaling with the branch count. Section UUIDs, counts,
/// and the model ID only: headings ride the progress channel, never the log.
fn log_fan_out(model_id: &str, turn_kind: DraftTurnKind, state: &MergeState) {
    tracing::info!(
        target: "claria_bedrock::cache",
        model_id,
        resumed = turn_kind == DraftTurnKind::Resume,
        branches = state.branches,
        sections = ?state.section_ids,
        converse_calls = state.call_number,
        warm_cache_write_tokens = state.warm_cache_write_tokens,
        branch_cache_write_tokens_total = state.branch_cache_write_tokens,
        branch_cache_read_tokens_total = state.branch_cache_read_tokens,
        "parallel_draft_fan_out_complete"
    );
    let scattering_threshold = state.warm_cache_write_tokens as f64 * 0.5 * state.branches as f64;
    if state.warm_cache_write_tokens > 0
        && state.branch_cache_write_tokens as f64 > scattering_threshold
    {
        tracing::warn!(
            target: "claria_bedrock::cache",
            model_id,
            branches = state.branches,
            warm_cache_write_tokens = state.warm_cache_write_tokens,
            branch_cache_write_tokens_total = state.branch_cache_write_tokens,
            "drafting branches re-wrote the shared prefix instead of reading it; the fan-out is probably being scattered across regional capacity"
        );
    }
}

/// Whether a resumed run's plan came from a planning pass rather than being
/// manufactured by the un-gated command.
///
/// The gate is the whole justification for drafting in parallel: a plan a
/// clinician read and approved states what each section must assert, which is
/// what makes the sections independent enough to write at once. A synthetic
/// 1:1 plan is nobody's decision and states nothing, so a run built on one
/// stays on the serial path.
pub(crate) fn is_gated_plan(run: &DraftRun) -> bool {
    run.plan.as_ref().is_some_and(|plan| !plan.synthetic)
}
