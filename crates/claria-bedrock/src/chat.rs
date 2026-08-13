//! Chat-oriented Bedrock operations: model discovery, conversation, and
//! agreement management.
//!
//! # Bedrock model concepts
//!
//! ## Foundation models vs. inference profiles
//!
//! AWS Bedrock exposes two overlapping layers:
//!
//! - **Foundation models** (`ListFoundationModels`) — the canonical model
//!   registry. Each entry has a `model_id` like `anthropic.claude-sonnet-4-6`
//!   and carries metadata including lifecycle status (`ACTIVE` or `LEGACY`).
//!
//! - **Inference profiles** (`ListInferenceProfiles`) — cross-region routing
//!   wrappers. A single foundation model gets multiple profiles scoped by
//!   region: `us.anthropic.claude-sonnet-4-6`, `eu.anthropic.claude-sonnet-4-6`,
//!   `global.anthropic.claude-sonnet-4-6`, etc. The Converse API accepts an
//!   inference profile ID as its `model_id` parameter.
//!
//! The relationship is: strip the scope prefix (everything before the first
//! dot) from an inference profile ID to get the bare foundation model ID.
//!
//! ## Model discovery strategy
//!
//! Not all active foundation models have inference profiles yet. AWS adds
//! inference profiles over time — newly launched models may only appear in
//! `ListFoundationModels` initially. We use a **hybrid approach**:
//!
//! 1. Call `ListFoundationModels(provider="anthropic")` to get all Claude
//!    models with `ACTIVE` lifecycle status. Skip context-window variants
//!    (suffixed with `:48k`, `:200k`, etc.) — only the base model ID.
//! 2. Call `ListInferenceProfiles` to get cross-region routing wrappers.
//! 3. For each active foundation model, prefer a `us.` inference profile
//!    from the API. If none was returned, construct `us.{model_id}` — the
//!    Converse API requires an inference profile ID (bare model IDs fail
//!    with "on-demand throughput isn't supported").
//!
//! This ensures newly launched models appear immediately, while still
//! preferring inference profiles for established models.
//!
//! ## Legacy model filtering
//!
//! AWS marks superseded models as `LEGACY` in the foundation model registry,
//! but continues to list their inference profiles as `ACTIVE`. Since we
//! start from the foundation model registry filtered to `ACTIVE`, legacy
//! models are excluded automatically. Models that have inference profiles
//! but are absent from `ListFoundationModels` entirely (e.g. Claude 3 Opus)
//! are also excluded since we start from the foundation model list.
//!
//! ## Marketplace agreements
//!
//! Before a model can be invoked, its Marketplace agreement must be accepted.
//! New models (like Claude Sonnet 4 at launch) require this even if earlier
//! Claude models were already accepted. The relevant APIs are:
//!
//! - `GetFoundationModelAvailability(model_id)` — returns
//!   `agreement_availability.status`:
//!   - `Available` → agreement exists but hasn't been accepted yet
//!   - `NotAvailable` → already accepted, or no agreement required
//!
//! - `ListFoundationModelAgreementOffers(model_id)` → returns offer tokens
//! - `CreateFoundationModelAgreement(model_id, offer_token)` → accepts it
//!
//! We run this during onboarding (`accept_all_model_agreements`) so users
//! don't have to manually accept each model in the AWS console. The
//! operation is idempotent — re-accepting an already-accepted model is a
//! no-op.
//!
//! ## Required IAM permissions
//!
//! The scoped `claria-admin` IAM user needs these actions (provisioned by
//! `claria-provisioner::account_setup`):
//!
//! ```text
//! bedrock:ListFoundationModels
//! bedrock:ListInferenceProfiles
//! bedrock:GetFoundationModelAvailability
//! bedrock:ListFoundationModelAgreementOffers
//! bedrock:CreateFoundationModelAgreement
//! bedrock:InvokeModel
//! bedrock:InvokeModelWithResponseStream
//! ```

use std::collections::HashMap;

use aws_sdk_bedrock::types::{
    FoundationModelLifecycleStatus, InferenceProfileStatus, InferenceProfileType,
};
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, ConverseTokensRequest, InferenceConfiguration, Message,
    StopReason, SystemContentBlock,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use claria_core::model_id::CacheTtlChoice;

use crate::{
    converse::{StreamCollector, StreamOutcome},
    error::BedrockError,
};

// ── Types ────────────────────────────────────────────────────────────────────

/// An available chat model (Bedrock inference profile).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatModel {
    /// Inference profile ID, e.g. `us.anthropic.claude-sonnet-4-20250514-v1:0`.
    pub model_id: String,
    /// Human-readable name, e.g. `"US Anthropic Claude Sonnet 4"`.
    pub name: String,
}

// The conversation-turn types are claria-core domain types; re-exported here
// so Bedrock callers keep their `chat::ChatMessage` paths.
pub use claria_core::models::chat_history::{ChatMessage, ChatRole};

// ── Model discovery ──────────────────────────────────────────────────────────

/// List available Anthropic Claude chat models.
///
/// Uses a hybrid approach to ensure both established and newly launched models
/// appear:
///
/// 1. Calls `ListFoundationModels` to get all Claude models with `ACTIVE`
///    lifecycle status (skipping context-window variants like `:48k`).
/// 2. Calls `ListInferenceProfiles` to get cross-region routing wrappers.
/// 3. For each active foundation model, prefers a `us.` inference profile
///    from the API. If one wasn't returned, constructs `us.{model_id}` — the
///    Converse API requires an inference profile ID (bare model IDs fail with
///    "on-demand throughput isn't supported").
///
/// This handles the case where `ListInferenceProfiles` doesn't yet return
/// profiles for a model (e.g. before marketplace agreement acceptance).
/// Legacy models are excluded because we start from the ACTIVE foundation
/// model list. Models absent from the registry (e.g. Claude 3 Opus) are also
/// excluded.
///
/// Results are sorted by name.
pub async fn list_chat_models(
    config: &aws_config::SdkConfig,
) -> Result<Vec<ChatModel>, BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    // Step 1: Get all ACTIVE Claude foundation models (base IDs only).
    let active_models = fetch_active_foundation_models(&client).await?;

    // Step 2: Build a map from bare model ID → US inference profile.
    let us_profiles = fetch_us_inference_profiles(&client).await?;

    // Step 3: For each active foundation model, use the US inference profile.
    // If the API didn't return one, construct it: the Converse API requires an
    // inference profile ID (bare model IDs fail with "on-demand throughput
    // isn't supported"). The profile ID format is `us.{foundation_model_id}`.
    let mut models: Vec<ChatModel> = active_models
        .into_iter()
        .map(|(model_id, model_name)| {
            if let Some((profile_id, profile_name)) = us_profiles.get(&model_id) {
                ChatModel {
                    model_id: profile_id.clone(),
                    name: profile_name.clone(),
                }
            } else {
                ChatModel {
                    model_id: format!("us.{model_id}"),
                    name: model_name,
                }
            }
        })
        .collect();

    models.sort_by(|a, b| a.name.cmp(&b.name));

    info!(count = models.len(), "discovered chat models");

    Ok(models)
}

/// Fetch active Anthropic Claude foundation models, returning (model_id, name).
///
/// Skips context-window variants (IDs ending in `:48k`, `:200k`, etc.) — only
/// the base model ID is included.
async fn fetch_active_foundation_models(
    client: &aws_sdk_bedrock::Client,
) -> Result<Vec<(String, String)>, BedrockError> {
    let response = client
        .list_foundation_models()
        .by_provider("anthropic")
        .send()
        .await
        .map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::warn!(error = %msg, "ListFoundationModels failed");
            BedrockError::Invocation(msg)
        })?;

    let models: Vec<(String, String)> = response
        .model_summaries()
        .iter()
        .filter(|m| {
            let id = m.model_id();
            // Must be a Claude model.
            let is_claude = id.contains("claude");
            // Must have ACTIVE lifecycle.
            let is_active = m
                .model_lifecycle()
                .map(|lc| *lc.status() == FoundationModelLifecycleStatus::Active)
                .unwrap_or(false);
            // Skip context-window variants like `:48k`, `:200k`.
            let is_variant = id.rsplit_once(':').is_some_and(|(_, suffix)| {
                suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) && suffix != "0"
            });
            is_claude && is_active && !is_variant
        })
        .map(|m| {
            let name = m.model_name().unwrap_or(m.model_id()).to_string();
            (m.model_id().to_string(), name)
        })
        .collect();

    Ok(models)
}

/// Fetch US-scoped inference profiles for Claude, returning a map from
/// bare foundation model ID → (inference profile ID, profile name).
async fn fetch_us_inference_profiles(
    client: &aws_sdk_bedrock::Client,
) -> Result<HashMap<String, (String, String)>, BedrockError> {
    let response = client
        .list_inference_profiles()
        .type_equals(InferenceProfileType::SystemDefined)
        .max_results(100)
        .send()
        .await
        .map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::warn!(error = %msg, "ListInferenceProfiles failed");
            BedrockError::Invocation(msg)
        })?;

    let mut map = HashMap::new();

    for p in response.inference_profile_summaries() {
        let id = p.inference_profile_id();
        // Only US-scoped Claude profiles.
        if !id.starts_with("us.") || !id.contains("anthropic.claude") {
            continue;
        }
        if *p.status() != InferenceProfileStatus::Active {
            continue;
        }
        // Strip "us." prefix to get the bare foundation model ID.
        let bare_id = &id[3..];
        map.insert(
            bare_id.to_string(),
            (id.to_string(), p.inference_profile_name().to_string()),
        );
    }

    Ok(map)
}

// ── Chat conversation ────────────────────────────────────────────────────────

/// Output-token ceiling for every chat Converse call. The input budget below
/// is derived from this reserve — the two must move together.
///
/// Matched to the writer's reserve deliberately: clinicians draft
/// report-length sections here to carry into Word, and a long clinical
/// section runs to roughly 25k tokens. The 8k this used to sit at is the
/// same ceiling that measurably degraded the writer before v0.24, and it
/// truncated for the same reason. The input budget it leaves — the model's
/// window minus this reserve — is what the writer has run on since.
pub const CHAT_MAX_OUTPUT_TOKENS: u32 = 32_768;

/// Appended to a chat reply that hit [`CHAT_MAX_OUTPUT_TOKENS`] before
/// finishing. The partial answer is kept — it already streamed to the
/// reader — and this says why it stops where it does.
pub const CHAT_TRUNCATED_NOTICE: &str = "Claria note: the assistant reached its \
response-length limit, so this reply was cut short. Ask it to continue if you need the rest.";

/// Input-token budget for a chat request against `model_id`: the model's
/// context window minus the [`CHAT_MAX_OUTPUT_TOKENS`] output reserve.
pub fn chat_input_token_budget(model_id: &str) -> u32 {
    claria_core::model_id::ModelCapabilities::for_id(model_id)
        .context_window_tokens
        .saturating_sub(CHAT_MAX_OUTPUT_TOKENS)
}

/// Strategy for placing Bedrock prompt-cache `cachePoint` blocks on a chat
/// request: one after the system prefix and one at the conversation tail,
/// so the next turn re-reads the whole history from cache.
///
/// `ttl` selects the cache tier for every cache point in the request —
/// Bedrock rejects mixed TTLs, so one choice covers both markers. The
/// 1-hour TTL is gated on the central capability table
/// (`supports_extended_cache_ttl`); callers on unsupported families fall
/// back to `FiveMinutes` silently.
#[derive(Debug, Clone, Copy)]
pub struct CacheStrategy {
    /// Whether prompt caching is enabled at all (config-flag controlled).
    pub enabled: bool,
    /// Whether the model family is known to support prompt caching.
    /// `claria-bedrock` does not maintain this catalogue itself; the
    /// caller (claria-desktop) decides based on the configured model.
    pub model_supports_caching: bool,
    /// TTL applied to every cache point this strategy emits. `FiveMinutes`
    /// is the server default and stays off the wire.
    pub ttl: CacheTtlChoice,
}

impl CacheStrategy {
    /// Minimum estimated prefix tokens before we emit a `cachePoint`:
    /// Bedrock's 1,024-token floor plus 18% slack for tokenizer estimate
    /// error.
    pub const MIN_PREFIX_TOKENS: u32 = 1200;

    /// Caching ON when `model_supports_caching`, with `ttl` on every cache
    /// point; the token floor still applies per prompt.
    pub fn enabled_for_model(model_supports_caching: bool, ttl: CacheTtlChoice) -> Self {
        Self {
            enabled: true,
            model_supports_caching,
            ttl,
        }
    }

    /// Caching disabled.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            model_supports_caching: false,
            ttl: CacheTtlChoice::FiveMinutes,
        }
    }

    /// Whether to emit a `cachePoint` after the system prefix.
    ///
    /// Uses a cheap char-based estimate of `len/4 ≈ tokens` (English/code/
    /// XML averages). If the estimate falls below [`Self::MIN_PREFIX_TOKENS`]
    /// we skip the marker — Bedrock would silently no-op anyway and the log
    /// line is confusing.
    pub fn cache_system_prefix(&self, system_prompt: &str) -> bool {
        if !self.enabled || !self.model_supports_caching {
            return false;
        }
        let est_tokens = (system_prompt.len() / 4) as u32;
        est_tokens >= Self::MIN_PREFIX_TOKENS
    }

    /// Whether to emit a `cachePoint` at the conversation tail so the next
    /// turn reads the full history from cache.
    ///
    /// Same estimate and floor as [`Self::cache_system_prefix`], but over
    /// everything ahead of the tail marker — system prefix plus every
    /// message — since that whole span is what Bedrock would cache.
    pub fn cache_conversation_tail(&self, system_prompt: &str, messages: &[ChatMessage]) -> bool {
        if !self.enabled || !self.model_supports_caching {
            return false;
        }
        let chars = system_prompt.len()
            + messages
                .iter()
                .map(|message| message.content.len())
                .sum::<usize>();
        (chars / 4) as u32 >= Self::MIN_PREFIX_TOKENS
    }
}

/// Send a multi-turn conversation to Bedrock via `ConverseStream`, forwarding
/// each assistant text delta to `on_delta` as it arrives.
///
/// The caller provides the full message history, a system prompt, and a
/// [`CacheStrategy`] controlling placement of the Bedrock `cachePoint`
/// block. The complete accumulated text, wire stop reason, and usage (from
/// the trailing `metadata` event, `None` preserved when the service omitted
/// it) come back in the returned [`StreamOutcome`]. Truncation and context
/// overflow are typed errors, exactly as on the unary flows.
///
/// This is chat's only Bedrock path — chat is the one surface that renders
/// incrementally. Extraction, translation, and the report writer keep their
/// unary Converse calls.
// Timing span logs model id and turn count — never prompt, message, or
// delta text, which may hold PHI.
#[tracing::instrument(level = "trace", skip_all, fields(model_id = %model_id, turns = messages.len()))]
pub async fn chat_converse_stream<F>(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ChatMessage],
    cache_strategy: CacheStrategy,
    tuning: crate::converse::ModelTuning,
    mut on_delta: F,
) -> Result<StreamOutcome, BedrockError>
where
    F: FnMut(&str),
{
    let client = crate::converse::runtime_client(config);
    let (converse_messages, system_blocks, cache_ttl) = prepare_chat_request(
        config,
        &client,
        model_id,
        system_prompt,
        messages,
        cache_strategy,
    )
    .await?;

    let started = std::time::Instant::now();
    let response = client
        .converse_stream()
        .model_id(model_id)
        .set_system(Some(system_blocks))
        .set_messages(Some(converse_messages))
        .inference_config(
            InferenceConfiguration::builder()
                .max_tokens(CHAT_MAX_OUTPUT_TOKENS as i32)
                .set_temperature(tuning.temperature)
                .build(),
        )
        .set_additional_model_request_fields(crate::converse::additional_request_fields(tuning)?)
        .send()
        .await
        .map_err(|error| crate::converse::classify_error("chat ConverseStream", error))?;

    let mut stream = response.stream;
    let mut collector = StreamCollector::new();
    loop {
        // Mid-stream failures have a different SdkError shape (raw event
        // frame, not an HTTP response); keep the full error chain instead of
        // collapsing to "unhandled error".
        let event = stream.recv().await.map_err(|error| {
            tracing::error!(operation = "chat ConverseStream", "Bedrock stream failed");
            BedrockError::Invocation(
                aws_sdk_bedrockruntime::error::DisplayErrorContext(&error).to_string(),
            )
        })?;
        let Some(event) = event else { break };
        if let Some(delta) = collector.absorb(event) {
            on_delta(&delta);
        }
    }

    let mut outcome = collector.finish(model_id, CHAT_MAX_OUTPUT_TOKENS, cache_ttl)?;
    // The reader has already watched the partial answer arrive, so the notice
    // rides the same delta channel as the rest of it — the live view and the
    // persisted history end up with identical text.
    if outcome.stop_reason == StopReason::MaxTokens.as_str() {
        let notice = format!("\n\n{CHAT_TRUNCATED_NOTICE}");
        on_delta(&notice);
        outcome.text.push_str(&notice);
    }
    outcome.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    crate::converse::log_turn_usage(
        "chat ConverseStream",
        model_id,
        outcome.usage.as_ref(),
        Some(&outcome.stop_reason),
        outcome.latency_ms,
        CHAT_MAX_OUTPUT_TOKENS,
    );
    Ok(outcome)
}

/// Build the Converse message list and system blocks and enforce the input
/// budget — the request preparation shared verbatim by the unary and
/// streaming chat paths so the two can never drift. The returned
/// [`CacheTtlChoice`] is the TTL carried by the request's cache points
/// (`None` when the request went out uncached), for usage capture.
async fn prepare_chat_request(
    config: &aws_config::SdkConfig,
    client: &aws_sdk_bedrockruntime::Client,
    model_id: &str,
    system_prompt: &str,
    messages: &[ChatMessage],
    cache_strategy: CacheStrategy,
) -> Result<
    (
        Vec<Message>,
        Vec<SystemContentBlock>,
        Option<CacheTtlChoice>,
    ),
    BedrockError,
> {
    let mut converse_messages: Vec<Message> = Vec::new();

    for msg in messages {
        let role = match msg.role {
            ChatRole::User => ConversationRole::User,
            ChatRole::Assistant => ConversationRole::Assistant,
        };
        let message = Message::builder()
            .role(role)
            .content(ContentBlock::Text(msg.content.clone()))
            .build()
            .map_err(|e| BedrockError::Invocation(e.to_string()))?;
        converse_messages.push(message);
    }

    // Input budget: the same estimate-then-verify logic as the report path.
    // The conversation is estimated at ~4 chars/token; a real CountTokens
    // call runs only once the estimate comes within ~10% of the budget, and
    // overflow surfaces as the friendly context-window error instead of a
    // raw AWS string. Runs before cache points are attached — they do not
    // change the token count and stay out of the CountTokens request.
    let current_chars = system_prompt.len() as u64
        + messages
            .iter()
            .map(|message| message.content.len() as u64)
            .sum::<u64>();
    let mut budget =
        crate::converse::InputTokenBudget::estimated(chat_input_token_budget(model_id));
    budget
        .ensure_within(
            current_chars,
            || async {
                let bare_model_id = counting_model(config).await?;
                let request = ConverseTokensRequest::builder()
                    .set_messages(Some(converse_messages.clone()))
                    .system(SystemContentBlock::Text(system_prompt.to_string()))
                    .build();
                crate::converse::count_input_tokens(client, bare_model_id, request).await
            },
            |_, _| BedrockError::ContextWindowExceeded,
        )
        .await?;

    // Cache placement: one point after the system prefix, one at the
    // conversation tail so the next turn reads the whole history from
    // cache. Both carry the strategy's TTL — Bedrock rejects mixed TTLs in
    // one request — and the two markers stay well under Bedrock's cache
    // point limit.
    let cache_system = cache_strategy.cache_system_prefix(system_prompt);
    let cache_tail = cache_strategy.cache_conversation_tail(system_prompt, messages);
    let ttl = crate::converse::sdk_cache_ttl(cache_strategy.ttl);

    let system_blocks: Vec<SystemContentBlock> = if cache_system {
        vec![
            SystemContentBlock::Text(system_prompt.to_string()),
            SystemContentBlock::CachePoint(crate::converse::cache_point(ttl.clone())?),
        ]
    } else {
        vec![SystemContentBlock::Text(system_prompt.to_string())]
    };
    let converse_messages = if cache_tail {
        crate::converse::with_tail_cache_point(converse_messages, ttl)?
    } else {
        converse_messages
    };

    let applied_ttl = (cache_system || cache_tail).then_some(cache_strategy.ttl);
    Ok((converse_messages, system_blocks, applied_ttl))
}

// ── Token counting ───────────────────────────────────────────────────────────

/// Model ID used for the `CountTokens` API, resolved once per process.
static COUNTING_MODEL: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// Resolve the model to count tokens against: the newest available Haiku,
/// chosen by `claria_core::model_id::preferred_counting_model` (release-date
/// stamp ordering across both naming shapes).
///
/// Not every model supports `CountTokens` (e.g. Fable rejects it with a
/// ValidationException), so counting never uses the chat model. Haiku has
/// reliably supported the API across generations.
pub(crate) async fn counting_model(
    config: &aws_config::SdkConfig,
) -> Result<&'static str, BedrockError> {
    COUNTING_MODEL
        .get_or_try_init(|| async {
            let client = aws_sdk_bedrock::Client::new(config);
            let models = fetch_active_foundation_models(&client).await?;
            claria_core::model_id::preferred_counting_model(
                models.into_iter().map(|(model_id, _)| model_id),
            )
            .ok_or_else(|| {
                BedrockError::Invocation("no Haiku model available for token counting".to_string())
            })
        })
        .await
        .map(String::as_str)
}

/// Count the input tokens for a context (system prompt) before the user sends
/// a message. Uses the free Bedrock `CountTokens` API.
///
/// The count runs against the newest available Haiku (see [`counting_model`]),
/// never the chat model — so it is an estimate when the chat model uses a
/// different tokenizer. The result does not depend on which model the user
/// is chatting with, so no chat model is accepted here.
///
/// A minimal dummy user message is included because the Converse format
/// requires at least one message.
pub async fn count_context_tokens(
    config: &aws_config::SdkConfig,
    system_prompt: &str,
) -> Result<u32, BedrockError> {
    let client = crate::converse::runtime_client(config);

    // CountTokens requires a bare foundation model ID, not an inference
    // profile ID; `counting_model` returns bare IDs from the foundation
    // model registry.
    let bare_model_id = counting_model(config).await?;
    info!(bare_model_id, "context token count requested");

    let dummy_message = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(".".to_string()))
        .build()
        .map_err(|e| BedrockError::Invocation(e.to_string()))?;

    let converse_input = ConverseTokensRequest::builder()
        .messages(dummy_message)
        .system(SystemContentBlock::Text(system_prompt.to_string()))
        .build();

    let tokens =
        crate::converse::count_input_tokens(&client, bare_model_id, converse_input).await?;
    info!(bare_model_id, tokens, "context token count complete");
    Ok(tokens)
}

// ── Model agreement management ───────────────────────────────────────────────

/// Accept the Marketplace agreement for a foundation model.
///
/// Lists available offers for the model and accepts the first one.
/// Idempotent: if the agreement already exists, this is a no-op.
pub async fn accept_model_agreement(
    config: &aws_config::SdkConfig,
    model_id: &str,
) -> Result<(), BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    // List offers for this model.
    let offers_response = client
        .list_foundation_model_agreement_offers()
        .model_id(model_id)
        .send()
        .await
        .map_err(|e| BedrockError::Agreement(e.into_service_error().to_string()))?;

    let offers = offers_response.offers();
    if offers.is_empty() {
        // No offers means no agreement mechanism — nothing to do.
        return Ok(());
    }

    let offer_token = offers[0].offer_token();

    info!(model_id, offer_token, "accepting model agreement");

    match client
        .create_foundation_model_agreement()
        .model_id(model_id)
        .offer_token(offer_token)
        .send()
        .await
    {
        Ok(_) => {
            info!(model_id, "model agreement accepted");
        }
        Err(e) => {
            let msg = e.into_service_error().to_string();
            if msg.contains("already exists") {
                info!(model_id, "model agreement already accepted");
            } else {
                tracing::warn!(model_id, error = %msg, "failed to accept model agreement");
                return Err(BedrockError::Agreement(msg));
            }
        }
    }

    Ok(())
}

/// Result of attempting to accept agreements for all available models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgreementSummary {
    /// Models that already had agreements accepted.
    pub already_accepted: Vec<String>,
    /// Models whose agreements were accepted during this call.
    pub newly_accepted: Vec<String>,
    /// Models that failed agreement acceptance (model_id, error message).
    pub failed: Vec<(String, String)>,
}

/// Ensure Marketplace agreements are accepted for all available Claude models.
///
/// Lists all Claude inference profiles, deduplicates to bare foundation model
/// IDs, and attempts acceptance for each. Already-accepted agreements are
/// treated as success (idempotent).
pub async fn accept_all_model_agreements(
    config: &aws_config::SdkConfig,
) -> Result<AgreementSummary, BedrockError> {
    let models = list_chat_models(config).await?;

    let mut summary = AgreementSummary {
        already_accepted: Vec::new(),
        newly_accepted: Vec::new(),
        failed: Vec::new(),
    };

    // Deduplicate: inference profiles like `us.anthropic.claude-sonnet-4-...`
    // and `global.anthropic.claude-sonnet-4-...` share the same underlying
    // model. Strip the region prefix to get the bare model ID.
    let mut seen_bare_ids = std::collections::HashSet::new();

    for model in &models {
        let bare_id = claria_core::model_id::strip_scope_prefix(&model.model_id);

        if !seen_bare_ids.insert(bare_id.to_string()) {
            continue;
        }

        // Attempt acceptance directly — this is idempotent and handles
        // already-accepted agreements gracefully.
        match accept_model_agreement(config, bare_id).await {
            Ok(()) => {
                summary.newly_accepted.push(bare_id.to_string());
            }
            Err(e) => {
                summary.failed.push((bare_id.to_string(), e.to_string()));
            }
        }
    }

    info!(
        accepted = summary.newly_accepted.len(),
        failed = summary.failed.len(),
        "model agreement check complete"
    );

    Ok(summary)
}
