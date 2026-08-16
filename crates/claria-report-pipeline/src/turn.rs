//! One report-writing request: the Bedrock tool loop shared by targeted
//! editing and whole-document generation, and its failure mapping.

use aws_sdk_s3::Client as S3Client;
use claria_bedrock::{
    error::BedrockError,
    report::{self, ReportStopReason},
};
use claria_core::models::{
    report::{
        ReportAuthoringTurn, ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole,
        ReportToolResultStatus, ReportWorkspace,
    },
    turn_usage::TurnUsage,
};
use claria_report_store::{
    ATTEMPT_SCHEMA_VERSION, LoadedRun, LoadedWorkspace, MAX_INSTRUCTION_CHARACTERS,
    REPORT_CONFLICT_MESSAGE, ReportAttemptMetadata, ReportAttemptStatus, ReportCallUsageRecord,
    ReportFailureCode, ReportStoreError, ensure_no_active_run, ensure_revision, load_for_report,
    load_or_create, mark_template_current, persist_attempt, persist_call_usage, save_draft_run,
    save_loaded,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    CONVERSE_CALLS_FIELD_LABEL, FullReportGenerationOutcome, MAX_CONFIGURABLE_CONVERSE_CALLS,
    MAX_CONFIGURABLE_TOOL_ROUNDS, MAX_REPORT_REFERENCES, REPORT_TRUNCATED_NOTICE,
    ReportBlockReference, ReportPipelineError, ReportPromptCache, ReportTurnLimits,
    ReportTurnOutcome, TOOL_ROUNDS_FIELD_LABEL, WRITER_LIMITS_SECTION,
    context::{
        build_untrusted_context, flatten_protocol_history, sanitize_turn_messages, sha256_hex,
    },
    full_draft_context::{
        DraftTurnKind, cache_checkpoints, kickoff_instruction, plan_context, template_context,
    },
    prompts::{full_report_system_prompt, report_system_prompt},
    record_context::{FullRecordContext, load_full_record_context, load_record_inventory},
    run::{release_failed_run, stamp_run_authorship, start_draft_run},
    tools::ToolExecutionContext,
};

/// Ephemeral progress emitted while a single report-writing request runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportTurnProgress {
    RecordContextPrepared {
        included_files: u32,
        unavailable_files: u32,
        total_characters: u64,
    },
    ModelCallStarted {
        call_number: u32,
    },
    ToolStarted {
        name: String,
        context: Option<String>,
    },
    ToolFinished {
        name: String,
        context: Option<String>,
        status: ReportToolResultStatus,
    },
}

#[derive(Clone, Copy)]
pub struct ReportMessageRequest<'a> {
    pub instruction: &'a str,
    pub references: &'a [ReportBlockReference],
    pub limits: ReportTurnLimits,
    progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
    prompt_cache: Option<&'a ReportPromptCache>,
    system_prompt_body: Option<&'a str>,
    model_tuning: claria_bedrock::converse::ModelTuning,
}

impl<'a> ReportMessageRequest<'a> {
    pub fn new(instruction: &'a str) -> Self {
        Self {
            instruction,
            references: &[],
            limits: ReportTurnLimits::default(),
            progress: None,
            prompt_cache: None,
            system_prompt_body: None,
            model_tuning: claria_bedrock::converse::ModelTuning::default(),
        }
    }

    /// Use a customized system-prompt body; the fixed trust rules are still
    /// appended during composition.
    pub fn with_system_prompt_body(mut self, body: &'a str) -> Self {
        self.system_prompt_body = Some(body);
        self
    }

    /// Apply capability-gated model tuning (adaptive thinking, effort,
    /// temperature) to every Converse call of the turn.
    pub fn with_model_tuning(mut self, tuning: claria_bedrock::converse::ModelTuning) -> Self {
        self.model_tuning = tuning;
        self
    }

    pub fn with_references(mut self, references: &'a [ReportBlockReference]) -> Self {
        self.references = references;
        self
    }

    pub fn with_limits(mut self, limits: ReportTurnLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_progress(
        mut self,
        progress: &'a (dyn Fn(ReportTurnProgress) + Send + Sync),
    ) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Reuse the exact transient Bedrock protocol for a recently active
    /// Writing session. The cache is process-local and expires after the
    /// provider's five-minute prompt-cache window; persisted history remains
    /// sanitized.
    pub fn with_prompt_cache(mut self, prompt_cache: &'a ReportPromptCache) -> Self {
        self.prompt_cache = Some(prompt_cache);
        self
    }
}

#[derive(Clone, Copy)]
pub struct FullReportRequest<'a> {
    /// Optional user guidance applied on top of the fixed requirement to fill
    /// the complete document from the readable-record snapshot.
    pub guidance: &'a str,
    pub limits: ReportTurnLimits,
    progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
    prompt_cache: Option<&'a ReportPromptCache>,
    system_prompt_body: Option<&'a str>,
    model_tuning: claria_bedrock::converse::ModelTuning,
}

impl<'a> FullReportRequest<'a> {
    pub fn new(guidance: &'a str) -> Self {
        Self {
            guidance,
            limits: ReportTurnLimits::default(),
            progress: None,
            prompt_cache: None,
            system_prompt_body: None,
            model_tuning: claria_bedrock::converse::ModelTuning::default(),
        }
    }

    /// Use a customized system-prompt body; the fixed trust rules are still
    /// appended during composition.
    pub fn with_system_prompt_body(mut self, body: &'a str) -> Self {
        self.system_prompt_body = Some(body);
        self
    }

    /// Apply capability-gated model tuning (adaptive thinking, effort,
    /// temperature) to every Converse call of the turn.
    pub fn with_model_tuning(mut self, tuning: claria_bedrock::converse::ModelTuning) -> Self {
        self.model_tuning = tuning;
        self
    }

    pub fn with_limits(mut self, limits: ReportTurnLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_progress(
        mut self,
        progress: &'a (dyn Fn(ReportTurnProgress) + Send + Sync),
    ) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_prompt_cache(mut self, prompt_cache: &'a ReportPromptCache) -> Self {
        self.prompt_cache = Some(prompt_cache);
        self
    }

    pub(crate) fn emit_progress(self, event: ReportTurnProgress) {
        if let Some(progress) = self.progress {
            progress(event);
        }
    }
}

pub async fn send_report_message(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    expected_revision: u64,
    model_id: &str,
    message: ReportMessageRequest<'_>,
) -> Result<ReportTurnOutcome, ReportPipelineError> {
    let instruction = message.instruction.trim();
    let references = message.references;
    validate_instruction(instruction)?;
    validate_model_choice(model_id)?;
    if references.len() > MAX_REPORT_REFERENCES {
        return Err(ReportPipelineError::InvalidInput(format!(
            "Attach at most {MAX_REPORT_REFERENCES} report paragraphs or tables to one message."
        )));
    }

    let loaded = load_or_create(s3, bucket, client_id).await?;
    send_report_message_loaded(
        sdk_config,
        s3,
        bucket,
        loaded,
        expected_revision,
        model_id,
        message,
    )
    .await
}

// The storage/session identity adds one argument to the legacy turn API; keep
// the explicit fields at this orchestration boundary rather than hiding them
// in an untyped tuple.
#[allow(clippy::too_many_arguments)]
pub async fn send_report_message_for_report(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    model_id: &str,
    message: ReportMessageRequest<'_>,
) -> Result<ReportTurnOutcome, ReportPipelineError> {
    let instruction = message.instruction.trim();
    validate_instruction(instruction)?;
    validate_model_choice(model_id)?;
    if message.references.len() > MAX_REPORT_REFERENCES {
        return Err(ReportPipelineError::InvalidInput(format!(
            "Attach at most {MAX_REPORT_REFERENCES} report paragraphs or tables to one message."
        )));
    }
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    send_report_message_loaded(
        sdk_config,
        s3,
        bucket,
        loaded,
        expected_revision,
        model_id,
        message,
    )
    .await
}

async fn send_report_message_loaded(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    mut loaded: LoadedWorkspace,
    expected_revision: u64,
    model_id: &str,
    message: ReportMessageRequest<'_>,
) -> Result<ReportTurnOutcome, ReportPipelineError> {
    ensure_ready_for_turn(&loaded.workspace, expected_revision)?;
    execute_turn(
        sdk_config,
        s3,
        bucket,
        model_id,
        TurnRunRequest::targeted(
            message.instruction.trim(),
            message.references,
            message.limits,
            message.progress,
            message.prompt_cache,
            message.system_prompt_body,
            message.model_tuning,
        ),
        &mut loaded,
        None,
    )
    .await
}

/// Generate and directly save one complete working-draft revision from every
/// readable client-record file.
///
/// The model may use many internal tool rounds. Each section it lands is
/// written through to a durable drafting run before the model is told the
/// write succeeded, so an interrupted generation keeps its work; the report
/// itself still moves in one atomic revision at the end, with no proposal
/// acceptance required.
pub async fn generate_full_report(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    expected_revision: u64,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullReportGenerationOutcome, ReportPipelineError> {
    let guidance = request.guidance.trim();
    if guidance.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportPipelineError::InvalidInput(format!(
            "The full-draft guidance exceeds {MAX_INSTRUCTION_CHARACTERS} characters."
        )));
    }
    validate_model_choice(model_id)?;

    let loaded = load_or_create(s3, bucket, client_id).await?;
    generate_full_report_loaded(
        sdk_config,
        s3,
        bucket,
        loaded,
        expected_revision,
        model_id,
        request,
    )
    .await
}

// As above, explicit client + report identity is part of this public
// orchestration contract.
#[allow(clippy::too_many_arguments)]
pub async fn generate_full_report_for_report(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullReportGenerationOutcome, ReportPipelineError> {
    let guidance = request.guidance.trim();
    if guidance.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportPipelineError::InvalidInput(format!(
            "The full-draft guidance exceeds {MAX_INSTRUCTION_CHARACTERS} characters."
        )));
    }
    validate_model_choice(model_id)?;
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    generate_full_report_loaded(
        sdk_config,
        s3,
        bucket,
        loaded,
        expected_revision,
        model_id,
        request,
    )
    .await
}

async fn generate_full_report_loaded(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    mut loaded: LoadedWorkspace,
    expected_revision: u64,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullReportGenerationOutcome, ReportPipelineError> {
    ensure_ready_for_turn(&loaded.workspace, expected_revision)?;
    let run = start_draft_run(s3, bucket, &mut loaded, model_id, request.guidance.trim()).await?;
    execute_full_draft_turn(
        sdk_config,
        s3,
        bucket,
        loaded,
        model_id,
        request,
        run,
        DraftTurnKind::Fresh,
    )
    .await
}

/// Drive one whole-report turn against an already-created run, then release
/// the workspace whichever way the turn ends.
///
/// Both the first attempt and a resume land here, so the guarantee is stated
/// once: on success the run owns the revision it cut, and on any failure the
/// workspace pointer is cleared while the sections that landed stay durable in
/// the run object.
// The run and how the turn was entered ride alongside the workspace rather
// than inside the request: both outlive the request and neither is the user's.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_full_draft_turn(
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
    let prepared = prepare_full_draft_context(s3, bucket, client_id, model_id, request).await;
    let record_context = match prepared {
        Ok(record_context) => record_context,
        Err(error) => {
            release_failed_run(s3, bucket, client_id, report_id, &mut run).await;
            return Err(error);
        }
    };

    let outcome = execute_turn(
        sdk_config,
        s3,
        bucket,
        model_id,
        TurnRunRequest::full_draft(
            request.guidance.trim(),
            &record_context,
            turn_kind,
            request.limits,
            request.progress,
            request.prompt_cache,
            request.system_prompt_body,
            request.model_tuning,
        ),
        &mut loaded,
        Some(&mut run),
    )
    .await;

    match outcome {
        Ok(outcome) => Ok(FullReportGenerationOutcome {
            workspace: outcome.workspace,
            turn_id: outcome.turn_id,
            assistant_text: outcome.assistant_text,
            record_context: record_context.summary,
            attempt: outcome.attempt,
        }),
        Err(error) => {
            release_failed_run(s3, bucket, client_id, report_id, &mut run).await;
            Err(error)
        }
    }
}

async fn prepare_full_draft_context(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullRecordContext, ReportPipelineError> {
    let inventory = load_record_inventory(s3, bucket, client_id)
        .await
        .map_err(|source| ReportPipelineError::storage("listing full-draft records", source))?;
    let record_context = load_full_record_context(s3, bucket, model_id, &inventory).await?;
    request.emit_progress(ReportTurnProgress::RecordContextPrepared {
        included_files: record_context.summary.included_files,
        unavailable_files: record_context.summary.unavailable_files,
        total_characters: record_context.summary.total_characters,
    });
    Ok(record_context)
}

fn validate_instruction(instruction: &str) -> Result<(), ReportPipelineError> {
    if instruction.is_empty() {
        return Err(ReportPipelineError::InvalidInput(
            "Enter an instruction for the report assistant.".to_string(),
        ));
    }
    if instruction.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportPipelineError::InvalidInput(format!(
            "The instruction exceeds {MAX_INSTRUCTION_CHARACTERS} characters."
        )));
    }
    Ok(())
}

pub(crate) fn validate_model_choice(model_id: &str) -> Result<(), ReportPipelineError> {
    if model_id.trim().is_empty() {
        Err(ReportPipelineError::InvalidInput(
            "Choose a model before starting the writer.".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn ensure_ready_for_turn(
    workspace: &ReportWorkspace,
    expected_revision: u64,
) -> Result<(), ReportPipelineError> {
    ensure_revision(workspace, expected_revision)?;
    ensure_no_active_run(workspace)?;
    if workspace.session.pending_proposal.is_some() {
        return Err(ReportPipelineError::InvalidInput(
            "Accept or reject the pending proposal before starting another writer action."
                .to_string(),
        ));
    }
    Ok(())
}

async fn execute_turn(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    model_id: &str,
    request: TurnRunRequest<'_>,
    loaded: &mut LoadedWorkspace,
    run: Option<&mut LoadedRun>,
) -> Result<ReportTurnOutcome, ReportPipelineError> {
    let started_at = jiff::Timestamp::now();
    let mut progress = AttemptProgress::new(&loaded.workspace, model_id, started_at);
    let result = run_turn(sdk_config, s3, bucket, request, loaded, run, &mut progress).await;

    match result {
        Ok(mut outcome) => {
            let metadata = progress.metadata(ReportAttemptStatus::Completed, None);
            persist_attempt(s3, bucket, &metadata).await.map_err(|source| {
                ReportPipelineError::TurnFailed {
                    message: "The report turn completed, but Claria could not record its final usage status. Reload the report before retrying.".to_string(),
                    code: ReportFailureCode::UsagePersistence,
                    attempt: Box::new(metadata.clone()),
                    source: Some(Box::new(source)),
                }
            })?;
            outcome.attempt = metadata;
            Ok(outcome)
        }
        Err(failure) => {
            if failure.usage_may_be_incomplete {
                progress.usage_complete = false;
            }
            let metadata = progress.metadata(ReportAttemptStatus::Aborted, Some(failure.code));
            let status_result = persist_attempt(s3, bucket, &metadata).await;
            let (message, source) = match status_result {
                Ok(()) => (failure.message, failure.source),
                Err(status_error) => (
                    format!(
                        "{} Claria also could not record the final attempt status; completed call receipts were retained.",
                        failure.message
                    ),
                    Some(Box::new(status_error) as Box<dyn std::error::Error + Send + Sync>),
                ),
            };
            Err(ReportPipelineError::TurnFailed {
                message,
                code: failure.code,
                attempt: Box::new(metadata),
                source,
            })
        }
    }
}

#[derive(Clone, Copy)]
struct TurnRunRequest<'a> {
    kind: TurnRunKind<'a>,
    limits: ReportTurnLimits,
    progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
    prompt_cache: Option<&'a ReportPromptCache>,
    /// Customized system-prompt body; the fixed trust rules are always
    /// appended during composition.
    system_prompt_body: Option<&'a str>,
    model_tuning: claria_bedrock::converse::ModelTuning,
}

#[derive(Clone, Copy)]
enum TurnRunKind<'a> {
    Targeted {
        instruction: &'a str,
        references: &'a [ReportBlockReference],
    },
    FullDraft {
        guidance: &'a str,
        record_context: &'a FullRecordContext,
        turn_kind: DraftTurnKind,
    },
}

impl<'a> TurnRunRequest<'a> {
    fn targeted(
        instruction: &'a str,
        references: &'a [ReportBlockReference],
        limits: ReportTurnLimits,
        progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
        prompt_cache: Option<&'a ReportPromptCache>,
        system_prompt_body: Option<&'a str>,
        model_tuning: claria_bedrock::converse::ModelTuning,
    ) -> Self {
        Self {
            kind: TurnRunKind::Targeted {
                instruction,
                references,
            },
            limits,
            progress,
            prompt_cache,
            system_prompt_body,
            model_tuning,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn full_draft(
        guidance: &'a str,
        record_context: &'a FullRecordContext,
        turn_kind: DraftTurnKind,
        limits: ReportTurnLimits,
        progress: Option<&'a (dyn Fn(ReportTurnProgress) + Send + Sync)>,
        prompt_cache: Option<&'a ReportPromptCache>,
        system_prompt_body: Option<&'a str>,
        model_tuning: claria_bedrock::converse::ModelTuning,
    ) -> Self {
        Self {
            kind: TurnRunKind::FullDraft {
                guidance,
                record_context,
                turn_kind,
            },
            limits,
            progress,
            prompt_cache,
            system_prompt_body,
            model_tuning,
        }
    }

    fn emit_progress(self, event: ReportTurnProgress) {
        if let Some(progress) = self.progress {
            progress(event);
        }
    }

    const fn is_full_draft(self) -> bool {
        matches!(self.kind, TurnRunKind::FullDraft { .. })
    }
}

pub(crate) struct AttemptProgress {
    pub(crate) attempt_id: Uuid,
    pub(crate) report_id: Uuid,
    pub(crate) client_id: Uuid,
    pub(crate) model_id: String,
    pub(crate) usage: TurnUsage,
    pub(crate) usage_complete: bool,
    pub(crate) converse_calls: u32,
    pub(crate) tool_uses: u32,
    pub(crate) started_at: jiff::Timestamp,
    /// Digest of the system prompt in effect for this attempt, set by
    /// `run_turn` once the mode (targeted vs full draft) is known.
    pub(crate) system_prompt_sha256: String,
}

impl AttemptProgress {
    fn new(workspace: &ReportWorkspace, model_id: &str, started_at: jiff::Timestamp) -> Self {
        Self {
            attempt_id: Uuid::new_v4(),
            report_id: workspace.report_id,
            client_id: workspace.client_id,
            model_id: model_id.to_string(),
            usage: empty_usage(model_id),
            usage_complete: true,
            converse_calls: 0,
            tool_uses: 0,
            started_at,
            system_prompt_sha256: String::new(),
        }
    }

    fn metadata(
        &self,
        status: ReportAttemptStatus,
        failure_code: Option<ReportFailureCode>,
    ) -> ReportAttemptMetadata {
        ReportAttemptMetadata {
            schema_version: ATTEMPT_SCHEMA_VERSION,
            attempt_id: self.attempt_id,
            report_id: self.report_id,
            client_id: self.client_id,
            model_id: self.model_id.clone(),
            status,
            failure_code,
            usage: self.usage.clone(),
            usage_complete: self.usage_complete,
            converse_calls: self.converse_calls,
            tool_uses: self.tool_uses,
            started_at: self.started_at,
            completed_at: jiff::Timestamp::now(),
        }
    }
}

/// Build the durable receipt for one completed Converse call.
///
/// The store persists these but does not know Bedrock, so the wire stop
/// reason, the enforced output ceiling, and the app version are stamped here,
/// where the call was actually made.
fn call_usage_record(
    progress: &AttemptProgress,
    call_number: u32,
    usage: Option<TurnUsage>,
    stop_reason: ReportStopReason,
    latency_ms: Option<u64>,
) -> ReportCallUsageRecord {
    ReportCallUsageRecord {
        schema_version: ATTEMPT_SCHEMA_VERSION,
        attempt_id: progress.attempt_id,
        report_id: progress.report_id,
        client_id: progress.client_id,
        model_id: progress.model_id.clone(),
        call_number,
        usage_complete: usage.is_some(),
        usage,
        recorded_at: jiff::Timestamp::now(),
        stop_reason: Some(stop_reason.as_str().to_string()),
        latency_ms,
        max_output_tokens: report::REPORT_OUTPUT_TOKEN_RESERVE,
        system_prompt_sha256: progress.system_prompt_sha256.clone(),
        // Workspace crates version in lockstep with the desktop binary.
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
    }
}

pub(crate) struct TurnRunFailure {
    code: ReportFailureCode,
    message: String,
    usage_may_be_incomplete: bool,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl TurnRunFailure {
    pub(crate) fn new(code: ReportFailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            usage_may_be_incomplete: false,
            source: None,
        }
    }

    pub(crate) fn uncertain_usage(mut self) -> Self {
        self.usage_may_be_incomplete = true;
        self
    }
}

// The run is threaded alongside the workspace rather than inside the request
// because both are mutated for the life of the turn.
#[allow(clippy::too_many_arguments)]
async fn run_turn(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    request: TurnRunRequest<'_>,
    loaded: &mut LoadedWorkspace,
    mut run: Option<&mut LoadedRun>,
    progress: &mut AttemptProgress,
) -> Result<ReportTurnOutcome, TurnRunFailure> {
    if request.is_full_draft() && run.is_none() {
        return Err(TurnRunFailure::new(
            ReportFailureCode::InvalidProtocol,
            "Claria could not start a durable drafting run for this whole-report request.",
        ));
    }
    // A whole-report run's ceilings follow its plan; a targeted edit keeps
    // exactly what the clinician configured.
    let limits = match run.as_deref() {
        Some(run) if request.is_full_draft() => request.limits.scaled_for_plan(
            run.run
                .plan
                .as_ref()
                .map_or(run.run.sections.len(), |plan| plan.entries.len()),
        ),
        _ => request.limits,
    };
    loaded
        .workspace
        .prune_turns(limits.max_retained_turns as usize)
        .map_err(|error| {
            TurnRunFailure::new(ReportFailureCode::InvalidProtocol, error.to_string())
        })?;

    let mut inventory = Vec::new();
    let protocol_content;
    let persisted_instruction;
    match request.kind {
        TurnRunKind::Targeted {
            instruction,
            references,
        } => {
            inventory = load_record_inventory(s3, bucket, loaded.workspace.client_id)
                .await
                .map_err(|error| {
                    TurnRunFailure::new(
                        ReportFailureCode::RecordStorage,
                        "Claria could not list this client's records from managed storage. Try again.",
                    )
                    .with_internal(error)
                })?;
            let report_context =
                build_untrusted_context(&loaded.workspace, references).map_err(|message| {
                    TurnRunFailure::new(ReportFailureCode::InvalidProtocol, message)
                })?;
            protocol_content = vec![
                ReportProtocolBlock::Text {
                    text: report_context,
                },
                ReportProtocolBlock::Text {
                    text: format!("User instruction:\n{instruction}"),
                },
            ];
            persisted_instruction = instruction.to_string();
        }
        TurnRunKind::FullDraft {
            guidance,
            record_context,
            turn_kind,
        } => {
            // Checkpoint layout, in the order the cache reads it: the frozen
            // record corpus and template structure, then the plan, then the
            // one block a resume rewrites. `run` is read-only here — the tool
            // loop takes it mutably further down, after message 0 is fixed.
            let run = run.as_deref().ok_or_else(|| {
                TurnRunFailure::new(
                    ReportFailureCode::InvalidProtocol,
                    "Claria could not start a durable drafting run for this whole-report request.",
                )
            })?;
            let base = &loaded.workspace.draft.content;
            let template = template_context(&loaded.workspace, base).map_err(|message| {
                TurnRunFailure::new(ReportFailureCode::InvalidProtocol, message)
            })?;
            let plan = plan_context(run.run.plan.as_ref()).map_err(|message| {
                TurnRunFailure::new(ReportFailureCode::InvalidProtocol, message)
            })?;
            protocol_content = vec![
                ReportProtocolBlock::Text {
                    text: record_context.prompt.clone(),
                },
                ReportProtocolBlock::Text { text: template },
                ReportProtocolBlock::Text { text: plan },
                ReportProtocolBlock::Text {
                    text: kickoff_instruction(turn_kind, guidance, &run.run, base),
                },
            ];
            persisted_instruction = if guidance.is_empty() {
                "Fill the complete report from all readable client records.".to_string()
            } else {
                format!(
                    "Fill the complete report from all readable client records.\n\nGuidance: {guidance}"
                )
            };
        }
    }
    let protocol_user_message = ReportProtocolMessage {
        role: ReportProtocolRole::User,
        content: protocol_content,
        created_at: progress.started_at,
    };
    let persisted_user_message = ReportProtocolMessage {
        role: ReportProtocolRole::User,
        content: vec![ReportProtocolBlock::Text {
            text: persisted_instruction,
        }],
        created_at: progress.started_at,
    };
    let mut turn_messages = vec![persisted_user_message];
    let prior_turn_count = loaded.workspace.session.turns.len();
    // Whole-document generation is a self-contained job over the current
    // report/template plus the fresh record snapshot. Targeted editing may
    // resume the exact transient protocol for five minutes; after that it
    // falls back to the sanitized persisted history.
    let mut protocol = if request.is_full_draft() {
        if let Some(cache) = request.prompt_cache {
            cache.invalidate(loaded.workspace.report_id);
        }
        Vec::new()
    } else {
        request
            .prompt_cache
            .and_then(|cache| {
                cache.reusable_protocol(
                    loaded.workspace.report_id,
                    &progress.model_id,
                    prior_turn_count,
                )
            })
            .unwrap_or_else(|| flatten_protocol_history(&loaded.workspace))
    };
    protocol.push(protocol_user_message);

    let mut rounds = 0_u32;
    let mut corrective_round_used = false;
    let mut completion_reminder_used = false;
    // Compose the effective system prompt once per turn: the (possibly
    // user-customized) body plus the fixed trust rules.
    let system_prompt = if request.is_full_draft() {
        full_report_system_prompt(request.system_prompt_body)
    } else {
        report_system_prompt(request.system_prompt_body)
    };
    progress.system_prompt_sha256 = sha256_hex(&system_prompt);
    // One exact CountTokens per turn; later calls estimate appended messages
    // and re-verify only near the budget.
    let mut input_budget = report::ReportInputBudget::new(&progress.model_id);
    let citable_filenames = match request.kind {
        TurnRunKind::FullDraft { record_context, .. } => {
            record_context.citable_filenames.as_slice()
        }
        TurnRunKind::Targeted { .. } => &[],
    };
    let mut tool_context = ToolExecutionContext::new(
        s3,
        bucket,
        &inventory,
        &loaded.workspace,
        &progress.model_id,
        run.as_deref_mut().filter(|_| request.is_full_draft()),
        citable_filenames,
    );
    let terminal_text: String;
    // Named once for the failure messages: `progress` is borrowed mutably
    // across the loop body, and the model is fixed for the whole attempt.
    let model_id = progress.model_id.clone();

    // Where this turn's `cachePoint` blocks go. Fixed for the attempt
    // because the model is: a plan built per call could drift mid-loop and
    // move the prefix boundary the previous call just paid to write.
    let capabilities = claria_core::model_id::ModelCapabilities::for_id(&model_id);
    let cache_plan = if request.is_full_draft() {
        claria_bedrock::converse::CachePlan::full_draft(capabilities, cache_checkpoints()).map_err(
            |error| {
                TurnRunFailure::new(
                    ReportFailureCode::InvalidProtocol,
                    "Claria could not lay out the drafting conversation's prompt cache.",
                )
                .with_internal(error)
            },
        )?
    } else {
        claria_bedrock::converse::CachePlan::report_default(capabilities)
    };

    loop {
        if progress.converse_calls >= limits.max_converse_calls {
            return Err(limit_exhausted_failure(
                ExhaustedWriterLimit::ConverseCalls,
                rounds,
                progress.converse_calls,
                limits,
            ));
        }
        let call_number = progress.converse_calls.saturating_add(1);
        request.emit_progress(ReportTurnProgress::ModelCallStarted { call_number });
        // One Bedrock call, with bounded retries when its response stream
        // stalls or drops. A failed attempt commits nothing — `protocol`
        // still holds only completed messages — so re-sending the identical
        // request is safe, and a quick retry re-reads the prompt cache the
        // previous completed call wrote moments earlier.
        let call_started = tokio::time::Instant::now();
        let mut interruptions = 0_u32;
        let output = loop {
            let attempt = if request.is_full_draft() {
                report::converse_full_report_with_tool_limit(
                    sdk_config,
                    &progress.model_id,
                    &system_prompt,
                    &protocol,
                    limits.max_tool_uses_per_response as usize,
                    &mut input_budget,
                    request.model_tuning,
                    cache_plan.clone(),
                )
                .await
            } else {
                report::converse_report_with_tool_limit(
                    sdk_config,
                    &progress.model_id,
                    &system_prompt,
                    &protocol,
                    limits.max_tool_uses_per_response as usize,
                    &mut input_budget,
                    request.model_tuning,
                    cache_plan.clone(),
                )
                .await
            };
            match attempt {
                Ok(output) => break output,
                Err(error)
                    if error.is_interrupted_before_completion()
                        && interruptions < STREAM_INTERRUPTION_RETRIES =>
                {
                    interruptions += 1;
                    tracing::warn!(
                        call_number,
                        failed_attempts = interruptions,
                        error = %error,
                        "Bedrock stream interrupted; retrying the call"
                    );
                    tokio::time::sleep(STREAM_INTERRUPTION_RETRY_DELAY).await;
                }
                Err(error) => {
                    return Err(map_bedrock_failure(
                        error,
                        FailedReportCall {
                            number: call_number,
                            ceiling: limits.max_converse_calls,
                            model_id: &model_id,
                        },
                        interruptions.saturating_add(1),
                        call_started.elapsed(),
                    ));
                }
            }
        };
        progress.converse_calls = call_number;
        if let Some(usage) = &output.usage {
            merge_usage(&mut progress.usage, usage, call_number == 1);
        } else {
            progress.usage_complete = false;
        }
        persist_call_usage(
            s3,
            bucket,
            &call_usage_record(
                progress,
                call_number,
                output.usage.clone(),
                output.stop_reason,
                output.latency_ms,
            ),
        )
        .await
        .map_err(|error| {
            TurnRunFailure::new(
                ReportFailureCode::UsagePersistence,
                "Claria could not record usage for a completed Bedrock call, so the turn was stopped.",
            )
            .with_internal(error)
        })?;

        let response_text = output
            .message
            .content
            .iter()
            .filter_map(|block| match block {
                ReportProtocolBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        let stop_tool_mismatch = output.stop_tool_mismatch();
        protocol.push(output.message.clone());
        turn_messages.push(output.message);

        // Stop-reason/tool-presence mismatch: attempt one corrective round
        // telling the model its response ended inconsistently, instead of
        // discarding the whole turn on a transient provider glitch. The
        // allowance is per streak, not per turn — a long multi-round turn
        // only fails on two mismatches in a row, and any well-formed
        // response in between re-arms it.
        if stop_tool_mismatch {
            if corrective_round_used {
                return Err(TurnRunFailure::new(
                    ReportFailureCode::InvalidProtocol,
                    "Bedrock repeatedly returned an inconsistent report stop reason. Nothing from this turn was applied.",
                ));
            }
            corrective_round_used = true;
            // The mismatched tool calls stay in the persisted protocol, so
            // they count as (unexecuted) tool uses for the attempt record.
            progress.tool_uses = progress
                .tool_uses
                .saturating_add(output.tool_calls.len() as u32);
            let content = if output.tool_calls.is_empty() {
                vec![ReportProtocolBlock::Text {
                    text: "Your previous response ended inconsistently: it signaled tool use \
                           without issuing a tool call. Re-issue the tool call, or answer in text."
                        .to_string(),
                }]
            } else {
                output
                    .tool_calls
                    .iter()
                    .map(|call| ReportProtocolBlock::ToolResult {
                        tool_use_id: call.tool_use_id.clone(),
                        status: ReportToolResultStatus::Error,
                        content: serde_json::json!({"error": {
                            "code": "inconsistent_stop_reason",
                            "message": "Your response ended inconsistently: it contained this tool call but did not stop for tool use. Re-issue the tool call in a well-formed response."
                        }}),
                    })
                    .collect()
            };
            let corrective_message = ReportProtocolMessage {
                role: ReportProtocolRole::User,
                content,
                created_at: jiff::Timestamp::now(),
            };
            protocol.push(corrective_message.clone());
            turn_messages.push(corrective_message);
            continue;
        }
        corrective_round_used = false;

        match output.stop_reason {
            // `max_tokens` beside complete tool calls is truncation, not a
            // protocol inconsistency: the calls that made it out are valid
            // work. Execute them like a normal tool round and tell the model
            // its response was cut off so it continues instead of re-paying
            // for everything it already produced.
            ReportStopReason::ToolUse | ReportStopReason::MaxTokens
                if !output.tool_calls.is_empty() =>
            {
                if rounds >= limits.max_tool_rounds
                    || progress.converse_calls >= limits.max_converse_calls
                {
                    let exhausted = if rounds >= limits.max_tool_rounds {
                        ExhaustedWriterLimit::ToolRounds
                    } else {
                        ExhaustedWriterLimit::ConverseCalls
                    };
                    return Err(limit_exhausted_failure(
                        exhausted,
                        rounds,
                        progress.converse_calls,
                        limits,
                    ));
                }
                rounds += 1;
                progress.tool_uses = progress
                    .tool_uses
                    .checked_add(output.tool_calls.len() as u32)
                    .ok_or_else(|| {
                        TurnRunFailure::new(
                            ReportFailureCode::InvalidProtocol,
                            "The report assistant returned too many tool requests.",
                        )
                    })?;

                let mut result_blocks = Vec::with_capacity(output.tool_calls.len());
                for call in &output.tool_calls {
                    let context = match call.name.as_str() {
                        claria_bedrock::report::LIST_RECORD_FILES_TOOL => {
                            Some("Record file list".to_string())
                        }
                        claria_bedrock::report::READ_RECORD_FILE_TOOL => call
                            .input
                            .get("filename")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string),
                        claria_bedrock::report::SET_FULL_DRAFT_TITLE_TOOL => {
                            Some("Complete report title".to_string())
                        }
                        claria_bedrock::report::WRITE_FULL_DRAFT_SECTION_TOOL => {
                            Some("Complete report section".to_string())
                        }
                        claria_bedrock::report::SKIP_FULL_DRAFT_SECTION_TOOL => {
                            Some("Deferred report section".to_string())
                        }
                        claria_bedrock::report::MARK_SECTION_FAILED_TOOL => {
                            Some("Failed report section".to_string())
                        }
                        claria_bedrock::report::FINISH_FULL_DRAFT_TOOL => {
                            Some("Complete working draft".to_string())
                        }
                        _ => None,
                    };
                    request.emit_progress(ReportTurnProgress::ToolStarted {
                        name: call.name.clone(),
                        context: context.clone(),
                    });
                    let result = tool_context.execute(call).await?;
                    request.emit_progress(ReportTurnProgress::ToolFinished {
                        name: call.name.clone(),
                        context,
                        status: result.status,
                    });
                    result_blocks.push(ReportProtocolBlock::ToolResult {
                        tool_use_id: call.tool_use_id.clone(),
                        status: result.status,
                        content: result.content,
                    });
                }
                // The protocol requires a correlated result message to hold
                // only results, so the truncation notice rides inside the
                // last result's JSON instead of a separate text block.
                if matches!(output.stop_reason, ReportStopReason::MaxTokens)
                    && let Some(ReportProtocolBlock::ToolResult { content, .. }) =
                        result_blocks.last_mut()
                    && let Some(object) = content.as_object_mut()
                {
                    object.insert(
                        "response_truncated".to_string(),
                        serde_json::Value::String(
                            "Your previous response hit the per-response output-token limit \
                             after this tool call, so anything you were still generating was \
                             cut off. Continue the remaining work in your next response."
                                .to_string(),
                        ),
                    );
                }
                let result_message = ReportProtocolMessage {
                    role: ReportProtocolRole::User,
                    content: result_blocks,
                    created_at: jiff::Timestamp::now(),
                };
                protocol.push(result_message.clone());
                turn_messages.push(result_message);
            }
            ReportStopReason::EndTurn => {
                if request.is_full_draft() && !tool_context.full_draft_is_finalized() {
                    if completion_reminder_used {
                        return Err(TurnRunFailure::new(
                            ReportFailureCode::InvalidProtocol,
                            "The writer stopped twice without finalizing the complete draft. Nothing was saved.",
                        ));
                    }
                    completion_reminder_used = true;
                    // Name the sections that are actually outstanding. "Every
                    // required section" left the model to re-derive the set
                    // from a plan it had already worked through once.
                    let undecided = tool_context.undecided_plan_sections();
                    let outstanding = if undecided.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " The plan still requires: {}.",
                            undecided
                                .iter()
                                .map(|(id, heading)| format!("{id} ({heading})"))
                                .collect::<Vec<_>>()
                                .join("; ")
                        )
                    };
                    let reminder = ReportProtocolMessage {
                        role: ReportProtocolRole::User,
                        content: vec![ReportProtocolBlock::Text {
                            text: format!(
                                "The full draft is not finalized. Write, skip, or mark failed every \
                                 planned section, then call finish_full_draft. Do not end with prose \
                                 before that tool succeeds.{outstanding}"
                            ),
                        }],
                        created_at: jiff::Timestamp::now(),
                    };
                    protocol.push(reminder.clone());
                    turn_messages.push(reminder);
                    continue;
                }
                terminal_text = response_text;
                break;
            }
            // A cut-off final explanation cannot invalidate structured work
            // that was already staged/finalized by a successful tool call.
            ReportStopReason::MaxTokens if tool_context.staged_proposal.is_some() => {
                terminal_text = if response_text.is_empty() {
                    REPORT_TRUNCATED_NOTICE.to_string()
                } else {
                    format!("{response_text}\n\n{REPORT_TRUNCATED_NOTICE}")
                };
                break;
            }
            ReportStopReason::MaxTokens if tool_context.full_draft_is_finalized() => {
                let notice = "Claria note: the complete working draft was finalized, but the assistant's closing summary was cut short.";
                terminal_text = if response_text.is_empty() {
                    notice.to_string()
                } else {
                    format!("{response_text}\n\n{notice}")
                };
                break;
            }
            reason => return Err(terminal_stop_failure(reason)),
        }
    }

    let staged_proposal = tool_context.staged_proposal.take();
    let finalized_full_draft = tool_context.take_finalized_full_draft();
    drop(tool_context);
    if request.is_full_draft() && finalized_full_draft.is_none() {
        return Err(TurnRunFailure::new(
            ReportFailureCode::InvalidProtocol,
            "The writer did not finalize a complete draft. Nothing was saved.",
        ));
    }

    let completed_at = jiff::Timestamp::now();
    let mut assistant_text = terminal_text;
    let mut finalized_revision = None;
    if let Some(finalized) = finalized_full_draft {
        let expected_revision = loaded.workspace.draft.revision;
        loaded.workspace.draft = loaded
            .workspace
            .draft
            .replace_content(expected_revision, finalized.content, completed_at)
            .map_err(|error| {
                TurnRunFailure::new(ReportFailureCode::InvalidProtocol, error.to_string())
            })?;
        let revision = loaded.workspace.draft.revision;
        if let Some(run) = run.as_deref() {
            stamp_run_authorship(
                &mut loaded.workspace.draft.content,
                &run.run,
                revision,
                completed_at,
            );
        }
        // The run no longer owns the workspace: the revision it was building
        // toward is cut, so edits, retries, and saves are free again.
        loaded.workspace.active_run_id = None;
        loaded.workspace.session.pending_proposal = None;
        loaded.workspace.session.last_agent_revision = Some(revision);
        mark_template_current(&mut loaded.workspace);
        finalized_revision = Some(revision);
        if assistant_text.trim().is_empty() {
            assistant_text = finalized.summary;
        }
    } else {
        loaded.workspace.session.pending_proposal = staged_proposal;
        loaded.workspace.session.last_agent_revision = Some(loaded.workspace.draft.revision);
    }

    let sanitized_messages = sanitize_turn_messages(turn_messages);
    let record_context_files = match request.kind {
        TurnRunKind::FullDraft { record_context, .. } => record_context.files.clone(),
        TurnRunKind::Targeted { .. } => Vec::new(),
    };
    let turn = ReportAuthoringTurn {
        id: Uuid::new_v4(),
        model_id: progress.model_id.clone(),
        messages: sanitized_messages,
        record_context_files,
        usage: progress.usage.clone(),
        usage_complete: progress.usage_complete,
        converse_calls: progress.converse_calls,
        tool_uses: progress.tool_uses,
        created_at: progress.started_at,
        completed_at,
    };
    let turn_id = turn.id;
    let proposal_id = loaded
        .workspace
        .session
        .pending_proposal
        .as_ref()
        .map(|proposal| proposal.id);
    loaded
        .workspace
        .push_turn_with_limit(turn, limits.max_retained_turns as usize)
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

    // Workspace first, run second. The revision is the user's document and the
    // run object is bookkeeping about it, so a run that never records its own
    // completion is healed from the workspace on the next load rather than
    // failing a generation that already landed.
    if let (Some(run), Some(revision)) = (run, finalized_revision) {
        run.run.status = claria_core::models::report_run::DraftRunStatus::Completed;
        run.run.finalized_revision = Some(revision);
        if let Err(error) = save_draft_run(s3, bucket, run).await {
            tracing::warn!(
                run_id = %run.run.run_id,
                error = %error,
                "the drafting run completed but could not record its own finish"
            );
        }
    }

    if let Some(cache) = request.prompt_cache {
        if !request.is_full_draft()
            && loaded.workspace.session.turns.len() == prior_turn_count.saturating_add(1)
        {
            cache.refresh(
                loaded.workspace.report_id,
                &progress.model_id,
                loaded.workspace.session.turns.len(),
                protocol,
            );
        } else {
            // A full-draft system prompt or retained-history prune cannot
            // share the targeted-edit prefix safely.
            cache.invalidate(loaded.workspace.report_id);
        }
    }

    Ok(ReportTurnOutcome {
        workspace: loaded.workspace.clone(),
        turn_id,
        assistant_text,
        proposal_id,
        // Replaced after the append-only final attempt record succeeds.
        attempt: progress.metadata(ReportAttemptStatus::Completed, None),
    })
}

/// Which configured guardrail ended the turn.
///
/// Both are raised in the same Preferences section, but naming the wrong one
/// sends the clinician to a control that cannot change the outcome: every
/// tool round spends a Converse call, so the call ceiling binds first unless
/// it stays at least one above the round ceiling.
#[derive(Debug, Clone, Copy)]
enum ExhaustedWriterLimit {
    ToolRounds,
    ConverseCalls,
}

/// Report an exhausted writer guardrail with the counts actually reached, the
/// ceiling that stopped them, and the one Preferences field that moves it.
///
/// The counts matter more than the verdict: 39 of 40 rounds spent listing
/// files is a different problem from 40 of 40 spent writing sections, and a
/// screenshot of this message should be enough to tell them apart.
fn limit_exhausted_failure(
    exhausted: ExhaustedWriterLimit,
    rounds: u32,
    calls: u32,
    limits: ReportTurnLimits,
) -> TurnRunFailure {
    let (field, configured, ceiling, cost_note) = match exhausted {
        ExhaustedWriterLimit::ToolRounds => (
            TOOL_ROUNDS_FIELD_LABEL,
            limits.max_tool_rounds,
            MAX_CONFIGURABLE_TOOL_ROUNDS,
            "Each added round is another billed model call",
        ),
        ExhaustedWriterLimit::ConverseCalls => (
            CONVERSE_CALLS_FIELD_LABEL,
            limits.max_converse_calls,
            MAX_CONFIGURABLE_CONVERSE_CALLS,
            "Each added call is billed",
        ),
    };
    let advice = if configured >= ceiling {
        format!(
            "\"{field}\" is already at its maximum of {ceiling} in {WRITER_LIMITS_SECTION}, so raising it is not an option. Give the writer less to do in one request: attach fewer records, or fill the report a section at a time."
        )
    } else {
        // Raising rounds alone accomplishes nothing when the call ceiling is
        // only one greater, because the next round would spend the last call.
        let companion = if matches!(exhausted, ExhaustedWriterLimit::ToolRounds)
            && limits.max_converse_calls <= limits.max_tool_rounds.saturating_add(1)
        {
            format!(
                " Raise \"{CONVERSE_CALLS_FIELD_LABEL}\" alongside it — it has to stay at least one higher."
            )
        } else {
            String::new()
        };
        format!(
            "Raise \"{field}\" in {WRITER_LIMITS_SECTION}; it is set to {configured} and goes up to {ceiling}.{companion} {cost_note}, so raise it a step at a time and check the session cost afterwards."
        )
    };
    TurnRunFailure::new(
        ReportFailureCode::ToolRoundLimit,
        format!(
            "The writer stopped before finishing the report after {rounds} of {} tool-use rounds and {calls} of {} Bedrock calls. Nothing from this turn was applied. {advice}",
            limits.max_tool_rounds, limits.max_converse_calls
        ),
    )
}

/// The in-flight Converse call a Bedrock failure interrupted.
///
/// Which call of how many died is the difference between an entitlement
/// problem (call 1 of 50, every time) and a flaky call deep into a
/// whole-report run, so it belongs in the message a clinician screenshots
/// rather than only in the log they will not open.
#[derive(Debug, Clone, Copy)]
struct FailedReportCall<'a> {
    number: u32,
    ceiling: u32,
    model_id: &'a str,
}

/// Retries granted to one Bedrock call whose request never completed —
/// three attempts in total, then the turn fails with a message that reports
/// how long Bedrock went unanswered. Only interruptions before completion
/// (a broken response stream, or a request that never got a response) are
/// retried: they are the failures where nothing was committed and the
/// identical request is known to be safe to re-send. A refused request —
/// throttled, denied, invalid — is answered, not interrupted, and fails
/// the turn immediately as before.
const STREAM_INTERRUPTION_RETRIES: u32 = 2;

/// Pause between interrupted attempts, long enough for a transient network
/// drop to clear and short enough to stay well inside the prompt-cache window
/// the previous completed call wrote. That window is five minutes for a
/// targeted edit and an hour for a drafting run on a model that accepts the
/// extended TTL, so this is sized against the shorter of the two.
const STREAM_INTERRUPTION_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Human phrasing of how long a failed call was retried for, in the
/// user-facing failure message: seconds under two minutes, whole minutes
/// (rounded up) beyond.
fn elapsed_phrase(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 120 {
        format!("{seconds} seconds")
    } else {
        format!("{} minutes", seconds.div_ceil(60))
    }
}

fn map_bedrock_failure(
    error: BedrockError,
    call: FailedReportCall<'_>,
    attempts: u32,
    elapsed: std::time::Duration,
) -> TurnRunFailure {
    if matches!(
        &error,
        BedrockError::SchemaViolation(_) | BedrockError::ResponseParse(_)
    ) {
        // These messages describe protocol structure, never report or record
        // text, and make otherwise opaque provider regressions diagnosable.
        tracing::error!(error = %error, "invalid Bedrock report protocol");
    }
    let FailedReportCall {
        number,
        ceiling,
        model_id,
    } = call;
    let failure = match &error {
        BedrockError::ContextBudgetExceeded { .. } => TurnRunFailure::new(
            ReportFailureCode::ContextBudget,
            "The report and conversation exceed this model's safe token budget. Shorten the report or start a new report workspace before retrying.",
        ),
        BedrockError::SchemaViolation(_) | BedrockError::ResponseParse(_) => TurnRunFailure::new(
            ReportFailureCode::InvalidProtocol,
            format!(
                "Bedrock returned an invalid report-tool protocol on call {number} of {ceiling}. Nothing from this turn was applied."
            ),
        ),
        BedrockError::UnsupportedModel(_) => TurnRunFailure::new(
            ReportFailureCode::Bedrock,
            "The selected model is not verified for report tools. Choose a current Claude model.",
        ),
        // Every attempt at this call went out and got no complete response
        // back — the stream stalled or dropped, or the request never got a
        // response at all — so retrying now is unlikely to help and the
        // wait is the actionable fact. Must precede the `Service` arm,
        // which would otherwise claim the DispatchFailure-coded shape.
        _ if error.is_interrupted_before_completion() => TurnRunFailure::new(
            ReportFailureCode::Bedrock,
            format!(
                "Bedrock appears to be having a hard time: {attempts} attempts at call {number} of {ceiling} got no response over the last {}. Try again later. Usage for the calls that completed was retained.",
                elapsed_phrase(elapsed)
            ),
        )
        .uncertain_usage(),
        // These reached AWS and came back refused. The service code is the
        // only thing separating "retry", "pick another model", and "call
        // support", so it decides the message instead of collapsing into one
        // unactionable sentence.
        BedrockError::Service {
            code, request_id, ..
        } => {
            let cause = match code.as_str() {
                "AccessDeniedException" => format!(
                    "Your AWS account is not permitted to invoke {model_id}. Choose a model the account has Bedrock access to, or request access for this one in the AWS console."
                ),
                "ResourceNotFoundException" => format!(
                    "Bedrock does not offer {model_id} in this account's region. Choose a different model."
                ),
                "ThrottlingException" | "ServiceQuotaExceededException" => {
                    "AWS throttled the request — filling a whole report issues many calls in quick succession. Wait a moment and retry.".to_string()
                }
                "ModelNotReadyException" | "ServiceUnavailableException"
                | "InternalServerException" => {
                    "Bedrock was temporarily unavailable. Retry in a few minutes.".to_string()
                }
                "ModelTimeoutException" => {
                    "The model did not respond in time. Retry, or fill the report a section at a time if it keeps happening.".to_string()
                }
                "ValidationException" => format!(
                    "Bedrock rejected the request as invalid for {model_id}. The Claria Console log holds the service's reason."
                ),
                // Every non-service SDK failure lands here — connect, dispatch,
                // and timeout alike — so the wording must not pin the blame on
                // the network when a long generation simply ran out of time.
                "DispatchFailure" => {
                    "Claria could not complete the call to Bedrock: the connection failed or the request timed out. Check the network and retry."
                        .to_string()
                }
                other => {
                    let request_id = request_id.as_deref().unwrap_or("not reported");
                    format!("Bedrock returned {other} (request ID {request_id}).")
                }
            };
            TurnRunFailure::new(
                ReportFailureCode::Bedrock,
                format!(
                    "Bedrock call {number} of {ceiling} failed. {cause} Usage for the calls that completed was retained."
                ),
            )
            .uncertain_usage()
        }
        _ => TurnRunFailure::new(
            ReportFailureCode::Bedrock,
            format!(
                "Bedrock call {number} of {ceiling} could not be completed. Usage for the calls that completed was retained. The Claria Console log holds the underlying error."
            ),
        )
        .uncertain_usage(),
    };
    failure.with_internal(error)
}

fn terminal_stop_failure(reason: ReportStopReason) -> TurnRunFailure {
    let message = match reason {
        ReportStopReason::ContentFiltered => {
            "The report assistant response was blocked by content filtering."
        }
        ReportStopReason::GuardrailIntervened => {
            "A Bedrock guardrail stopped the report assistant response."
        }
        ReportStopReason::MalformedModelOutput | ReportStopReason::MalformedToolUse => {
            "The model returned malformed tool output. Nothing from this turn was applied."
        }
        ReportStopReason::MaxTokens => {
            "The report assistant reached its output-token limit before finishing. Nothing from this turn was applied."
        }
        ReportStopReason::ModelContextWindowExceeded => {
            "The report-authoring context exceeded this model's context window. Nothing from this turn was applied."
        }
        ReportStopReason::StopSequence | ReportStopReason::Unknown => {
            "The report assistant stopped unexpectedly. Nothing from this turn was applied."
        }
        ReportStopReason::EndTurn | ReportStopReason::ToolUse => {
            "The report assistant stopped unexpectedly."
        }
    };
    TurnRunFailure::new(ReportFailureCode::UnexpectedStop, message)
}

impl TurnRunFailure {
    pub(crate) fn with_internal<T>(mut self, error: T) -> Self
    where
        T: std::error::Error + Send + Sync + 'static,
    {
        // Preserve the typed source for diagnostics while the outer Display
        // remains the PHI-free UI message.
        self.source = Some(Box::new(error));
        self
    }
}

fn empty_usage(model_id: &str) -> TurnUsage {
    TurnUsage {
        model_id: model_id.to_string(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_write_input_tokens: 0,
        cache_ttl: None,
        cost_usd: 0.0,
        pricing_version: 0,
    }
}

fn merge_usage(total: &mut TurnUsage, call: &TurnUsage, first: bool) {
    total.input_tokens = total.input_tokens.saturating_add(call.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(call.output_tokens);
    total.cache_read_input_tokens = total
        .cache_read_input_tokens
        .saturating_add(call.cache_read_input_tokens);
    total.cache_write_input_tokens = total
        .cache_write_input_tokens
        .saturating_add(call.cache_write_input_tokens);
    total.cost_usd += call.cost_usd;
    if first {
        total.pricing_version = call.pricing_version;
    } else if total.pricing_version != call.pricing_version {
        total.pricing_version = 0;
    }
}
