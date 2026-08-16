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
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    CONVERSE_CALLS_FIELD_LABEL, FullReportGenerationOutcome, MAX_CONFIGURABLE_CONVERSE_CALLS,
    MAX_CONFIGURABLE_TOOL_ROUNDS, MAX_INSTRUCTION_CHARACTERS, MAX_REPORT_REFERENCES,
    REPORT_CONFLICT_MESSAGE, REPORT_TRUNCATED_NOTICE, ReportAttemptMetadata, ReportAttemptStatus,
    ReportAuthoringError, ReportBlockReference, ReportFailureCode, ReportPromptCache,
    ReportTurnLimits, ReportTurnOutcome, TOOL_ROUNDS_FIELD_LABEL, WRITER_LIMITS_SECTION,
    attempts::{ATTEMPT_SCHEMA_VERSION, persist_attempt, persist_call_usage},
    context::{
        build_untrusted_context, flatten_protocol_history, sanitize_turn_messages, sha256_hex,
    },
    prompts::{full_report_system_prompt, report_system_prompt},
    record_context::{FullRecordContext, load_full_record_context, load_record_inventory},
    tools::ToolExecutionContext,
    workspace::{
        LoadedWorkspace, ensure_revision, load_for_report, load_or_create, mark_template_current,
        save_loaded,
    },
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

    fn emit_progress(self, event: ReportTurnProgress) {
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
) -> Result<ReportTurnOutcome, ReportAuthoringError> {
    let instruction = message.instruction.trim();
    let references = message.references;
    validate_instruction(instruction)?;
    validate_model_choice(model_id)?;
    if references.len() > MAX_REPORT_REFERENCES {
        return Err(ReportAuthoringError::InvalidInput(format!(
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
) -> Result<ReportTurnOutcome, ReportAuthoringError> {
    let instruction = message.instruction.trim();
    validate_instruction(instruction)?;
    validate_model_choice(model_id)?;
    if message.references.len() > MAX_REPORT_REFERENCES {
        return Err(ReportAuthoringError::InvalidInput(format!(
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
) -> Result<ReportTurnOutcome, ReportAuthoringError> {
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
    )
    .await
}

/// Generate and directly save one complete working-draft revision from every
/// readable client-record file. The model may use many internal tool rounds,
/// but no partial candidate reaches the workspace and no proposal acceptance
/// is required.
pub async fn generate_full_report(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    expected_revision: u64,
    model_id: &str,
    request: FullReportRequest<'_>,
) -> Result<FullReportGenerationOutcome, ReportAuthoringError> {
    let guidance = request.guidance.trim();
    if guidance.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportAuthoringError::InvalidInput(format!(
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
) -> Result<FullReportGenerationOutcome, ReportAuthoringError> {
    let guidance = request.guidance.trim();
    if guidance.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportAuthoringError::InvalidInput(format!(
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
) -> Result<FullReportGenerationOutcome, ReportAuthoringError> {
    ensure_ready_for_turn(&loaded.workspace, expected_revision)?;
    let client_id = loaded.workspace.client_id;
    let inventory = load_record_inventory(s3, bucket, client_id)
        .await
        .map_err(|source| ReportAuthoringError::storage("listing full-draft records", source))?;
    let record_context = load_full_record_context(s3, bucket, model_id, &inventory).await?;
    request.emit_progress(ReportTurnProgress::RecordContextPrepared {
        included_files: record_context.summary.included_files,
        unavailable_files: record_context.summary.unavailable_files,
        total_characters: record_context.summary.total_characters,
    });

    let outcome = execute_turn(
        sdk_config,
        s3,
        bucket,
        model_id,
        TurnRunRequest::full_draft(
            request.guidance.trim(),
            &record_context,
            request.limits,
            request.progress,
            request.prompt_cache,
            request.system_prompt_body,
            request.model_tuning,
        ),
        &mut loaded,
    )
    .await?;
    Ok(FullReportGenerationOutcome {
        workspace: outcome.workspace,
        turn_id: outcome.turn_id,
        assistant_text: outcome.assistant_text,
        record_context: record_context.summary,
        attempt: outcome.attempt,
    })
}

fn validate_instruction(instruction: &str) -> Result<(), ReportAuthoringError> {
    if instruction.is_empty() {
        return Err(ReportAuthoringError::InvalidInput(
            "Enter an instruction for the report assistant.".to_string(),
        ));
    }
    if instruction.chars().count() > MAX_INSTRUCTION_CHARACTERS {
        return Err(ReportAuthoringError::InvalidInput(format!(
            "The instruction exceeds {MAX_INSTRUCTION_CHARACTERS} characters."
        )));
    }
    Ok(())
}

fn validate_model_choice(model_id: &str) -> Result<(), ReportAuthoringError> {
    if model_id.trim().is_empty() {
        Err(ReportAuthoringError::InvalidInput(
            "Choose a model before starting the writer.".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn ensure_ready_for_turn(
    workspace: &ReportWorkspace,
    expected_revision: u64,
) -> Result<(), ReportAuthoringError> {
    ensure_revision(workspace, expected_revision)?;
    if workspace.session.pending_proposal.is_some() {
        return Err(ReportAuthoringError::InvalidInput(
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
) -> Result<ReportTurnOutcome, ReportAuthoringError> {
    let started_at = jiff::Timestamp::now();
    let mut progress = AttemptProgress::new(&loaded.workspace, model_id, started_at);
    let result = run_turn(sdk_config, s3, bucket, request, loaded, &mut progress).await;

    match result {
        Ok(mut outcome) => {
            let metadata = progress.metadata(ReportAttemptStatus::Completed, None);
            persist_attempt(s3, bucket, &metadata).await.map_err(|source| {
                ReportAuthoringError::TurnFailed {
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
            Err(ReportAuthoringError::TurnFailed {
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

    fn full_draft(
        guidance: &'a str,
        record_context: &'a FullRecordContext,
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

async fn run_turn(
    sdk_config: &aws_config::SdkConfig,
    s3: &S3Client,
    bucket: &str,
    request: TurnRunRequest<'_>,
    loaded: &mut LoadedWorkspace,
    progress: &mut AttemptProgress,
) -> Result<ReportTurnOutcome, TurnRunFailure> {
    let limits = request.limits;
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
        } => {
            let report_context =
                build_untrusted_context(&loaded.workspace, &[]).map_err(|message| {
                    TurnRunFailure::new(ReportFailureCode::InvalidProtocol, message)
                })?;
            let guidance_text = if guidance.is_empty() {
                "No additional user guidance was supplied.".to_string()
            } else {
                format!("Additional user guidance:\n{guidance}")
            };
            protocol_content = vec![
                ReportProtocolBlock::Text {
                    text: report_context,
                },
                ReportProtocolBlock::Text {
                    text: record_context.prompt.clone(),
                },
                ReportProtocolBlock::Text {
                    text: format!(
                        "Whole-document request: fill the complete working report from the supplied readable-record snapshot.\n{guidance_text}"
                    ),
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
    let mut tool_context = ToolExecutionContext::new(
        s3,
        bucket,
        &inventory,
        &loaded.workspace,
        &progress.model_id,
        request.is_full_draft(),
    );
    let terminal_text: String;
    // Named once for the failure messages: `progress` is borrowed mutably
    // across the loop body, and the model is fixed for the whole attempt.
    let model_id = progress.model_id.clone();

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
            progress,
            call_number,
            output.usage.clone(),
            output.stop_reason,
            output.latency_ms,
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
                    let reminder = ReportProtocolMessage {
                        role: ReportProtocolRole::User,
                        content: vec![ReportProtocolBlock::Text {
                            text: "The full draft is not finalized. Continue writing every required section, then call finish_full_draft. Do not end with prose before that tool succeeds."
                                .to_string(),
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
    if let Some(finalized) = finalized_full_draft {
        let expected_revision = loaded.workspace.draft.revision;
        loaded.workspace.draft = loaded
            .workspace
            .draft
            .replace_content(expected_revision, finalized.content, completed_at)
            .map_err(|error| {
                TurnRunFailure::new(ReportFailureCode::InvalidProtocol, error.to_string())
            })?;
        loaded.workspace.session.pending_proposal = None;
        loaded.workspace.session.last_agent_revision = Some(loaded.workspace.draft.revision);
        mark_template_current(&mut loaded.workspace);
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
        ReportAuthoringError::Conflict => TurnRunFailure::new(
            ReportFailureCode::WorkspaceConflict,
            REPORT_CONFLICT_MESSAGE,
        ),
        other => TurnRunFailure::new(
            ReportFailureCode::RecordStorage,
            "Claria could not save the completed report turn to managed storage. Reload before retrying.",
        )
        .with_internal(other),
    })?;

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
/// drop to clear and short enough to stay inside the 5-minute window of the
/// prompt cache written by the previous completed call.
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
