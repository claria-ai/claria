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
    CachePointBlock, CachePointType, ContentBlock, ConversationRole, ConverseTokensRequest,
    Message, SystemContentBlock,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use claria_core::models::turn_usage::TurnUsage;

use crate::{error::BedrockError, tokens};

// ── Types ────────────────────────────────────────────────────────────────────

/// An available chat model (Bedrock inference profile).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatModel {
    /// Inference profile ID, e.g. `us.anthropic.claude-sonnet-4-20250514-v1:0`.
    pub model_id: String,
    /// Human-readable name, e.g. `"US Anthropic Claude Sonnet 4"`.
    pub name: String,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Role of a chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

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
                suffix.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && suffix != "0"
            });
            is_claude && is_active && !is_variant
        })
        .map(|m| {
            let name = m
                .model_name()
                .unwrap_or(m.model_id())
                .to_string();
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

/// Strip a scope prefix (e.g. `us.`, `global.`, `eu.`) from an inference
/// profile ID to get the bare foundation model ID. If the ID is already a bare
/// foundation model ID (starts with a provider like `anthropic.`), returns it
/// unchanged.
fn strip_scope_prefix(id: &str) -> &str {
    if let Some((prefix, rest)) = id.split_once('.') {
        // Scope prefixes are short region tags; provider names contain letters
        // and are longer. A simple heuristic: scope prefixes are ≤6 chars and
        // all-lowercase-alpha (e.g. "us", "eu", "global").
        let is_scope = prefix.len() <= 6 && prefix.chars().all(|c| c.is_ascii_lowercase());
        if is_scope && rest.contains('.') {
            return rest;
        }
    }
    id
}

// ── Chat conversation ────────────────────────────────────────────────────────

/// Strategy for placing a Bedrock prompt-cache `cachePoint` block on the
/// system prefix.
///
/// Phase 2 places a single `CachePoint` after the system block when the
/// estimated prefix is large enough and the model family supports caching.
/// The 5-min TTL is the only TTL Sonnet 4 / Opus 4 support per AWS docs;
/// the 1h TTL is gated to the 4.5 generation (out of scope here).
#[derive(Debug, Clone, Copy)]
pub struct CacheStrategy {
    /// Whether prompt caching is enabled at all (config-flag controlled).
    pub enabled: bool,
    /// Minimum estimated prefix tokens before we emit a `cachePoint`.
    /// Default `1200` — Bedrock's 1,024-token floor plus 18% slack for
    /// tokenizer estimate error.
    pub min_prefix_tokens: u32,
    /// Whether the model family is known to support prompt caching.
    /// `claria-bedrock` does not maintain this catalogue itself; the
    /// caller (claria-desktop) decides based on the configured model.
    pub model_supports_caching: bool,
}

impl CacheStrategy {
    /// Default — caching ON with a 1,200-token floor. The caller still
    /// has to set `model_supports_caching` to `true` for the model to
    /// actually receive a cache point.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            min_prefix_tokens: 1200,
            model_supports_caching: false,
        }
    }

    /// Caching disabled.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            min_prefix_tokens: 1200,
            model_supports_caching: false,
        }
    }

    /// Whether to emit a `cachePoint` after the system prefix.
    ///
    /// Uses a cheap char-based estimate of `len/4 ≈ tokens` (English/code/
    /// XML averages). If the estimate falls below `min_prefix_tokens` we
    /// skip the marker — Bedrock would silently no-op anyway and the log
    /// line is confusing.
    pub fn cache_system_prefix(&self, system_prompt: &str) -> bool {
        if !self.enabled || !self.model_supports_caching {
            return false;
        }
        let est_tokens = (system_prompt.len() / 4) as u32;
        est_tokens >= self.min_prefix_tokens
    }
}

/// Send a multi-turn conversation to Bedrock and return the assistant's
/// reply along with per-turn token usage.
///
/// The caller provides the full message history, a system prompt, and a
/// [`CacheStrategy`] controlling placement of the Bedrock `cachePoint`
/// block. This is the shared implementation used by the desktop chat
/// command.
///
/// Returns a `(String, TurnUsage)` tuple. If the Bedrock response carries
/// no `usage` block (shouldn't happen on Converse, but the SDK type is
/// `Option`), the returned `TurnUsage` has zero token counts and zero cost.
// Timing span logs model id and turn count — never prompt or message text,
// which may hold PHI.
#[tracing::instrument(level = "trace", skip_all, fields(model_id = %model_id, turns = messages.len()))]
pub async fn chat_converse(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ChatMessage],
    cache_strategy: CacheStrategy,
) -> Result<(String, TurnUsage), BedrockError> {
    let client = aws_sdk_bedrockruntime::Client::new(config);

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

    // Build the system block — single Text today, Text + CachePoint when
    // caching is active and the prefix is large enough.
    let system_blocks: Vec<SystemContentBlock> =
        if cache_strategy.cache_system_prefix(system_prompt) {
            let cache_block = CachePointBlock::builder()
                .r#type(CachePointType::Default)
                .build()
                .map_err(|e| BedrockError::Invocation(e.to_string()))?;
            vec![
                SystemContentBlock::Text(system_prompt.to_string()),
                SystemContentBlock::CachePoint(cache_block),
            ]
        } else {
            vec![SystemContentBlock::Text(system_prompt.to_string())]
        };

    let response = client
        .converse()
        .model_id(model_id)
        .set_system(Some(system_blocks))
        .set_messages(Some(converse_messages))
        .send()
        .await
        .map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::error!(model_id, error = %msg, "Bedrock Converse call failed");
            BedrockError::Invocation(msg)
        })?;

    let output_message = response
        .output()
        .and_then(|o| o.as_message().ok())
        .ok_or_else(|| BedrockError::ResponseParse("no message in response".to_string()))?;

    let response_text = output_message
        .content()
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text(text) = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    let usage = match response.usage() {
        Some(u) => tokens::extract_turn_usage(u, model_id),
        None => tokens::empty_turn_usage(model_id),
    };

    // Emit a structured tracing event for cache observability. `hit_rate`
    // is the fraction of input tokens served from cache this turn.
    let total_input =
        usage.input_tokens + usage.cache_read_input_tokens + usage.cache_write_input_tokens;
    let hit_rate = if total_input > 0 {
        usage.cache_read_input_tokens as f64 / total_input as f64
    } else {
        0.0
    };
    tracing::info!(
        target: "claria_bedrock::cache",
        model_id,
        input_tokens = usage.input_tokens,
        cache_read = usage.cache_read_input_tokens,
        cache_write = usage.cache_write_input_tokens,
        hit_rate,
        cost_usd = usage.cost_usd,
        "chat turn complete"
    );

    Ok((response_text, usage))
}

// ── Token counting ───────────────────────────────────────────────────────────

/// Model ID used for the `CountTokens` API, resolved once per process.
static COUNTING_MODEL: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// Resolve the model to count tokens against: the newest available Haiku.
///
/// Not every model supports `CountTokens` (e.g. Fable rejects it with a
/// ValidationException), so counting never uses the chat model. Haiku has
/// reliably supported the API across generations; "newest" is decided by
/// the release date stamp embedded in the model ID, which orders correctly
/// across both naming shapes (`claude-3-5-haiku-20241022`,
/// `claude-haiku-4-5-20251001`).
pub(crate) async fn counting_model(
    config: &aws_config::SdkConfig,
) -> Result<&'static str, BedrockError> {
    COUNTING_MODEL
        .get_or_try_init(|| async {
            let client = aws_sdk_bedrock::Client::new(config);
            let models = fetch_active_foundation_models(&client).await?;
            models
                .into_iter()
                .map(|(model_id, _)| model_id)
                .filter(|id| id.contains("haiku"))
                .max_by_key(|id| release_date_stamp(id))
                .ok_or_else(|| {
                    BedrockError::Invocation(
                        "no Haiku model available for token counting".to_string(),
                    )
                })
        })
        .await
        .map(String::as_str)
}

/// The 8-digit release date stamp embedded in a foundation model ID
/// (`anthropic.claude-haiku-4-5-20251001-v1:0` → 20251001), or 0 if absent.
fn release_date_stamp(model_id: &str) -> u32 {
    let bytes = model_id.as_bytes();
    let mut run = 0;
    // Iterate one past the end so a trailing digit run is still checked.
    for i in 0..=bytes.len() {
        if i < bytes.len() && bytes[i].is_ascii_digit() {
            run += 1;
        } else {
            // Exactly 8 digits is a date stamp; shorter runs are version numbers.
            if run == 8 {
                return model_id[i - 8..i].parse().unwrap_or(0);
            }
            run = 0;
        }
    }
    0
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
    let client = aws_sdk_bedrockruntime::Client::new(config);

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

    let response = client
        .count_tokens()
        .model_id(bare_model_id)
        .input(
            aws_sdk_bedrockruntime::types::CountTokensInput::Converse(converse_input),
        )
        .send()
        .await
        .map_err(|e| {
            let err = e.into_service_error().to_string();
            tracing::warn!(bare_model_id, error = %err, "CountTokens failed");
            BedrockError::Invocation(err)
        })?;

    let tokens = response.input_tokens() as u32;
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
        let bare_id = strip_scope_prefix(&model.model_id);

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
