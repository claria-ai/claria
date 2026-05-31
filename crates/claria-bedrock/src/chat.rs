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
//! Before a model can be invoked, its Marketplace agreement must be executed
//! and (for Anthropic) the account's first-time-use form submitted. That flow
//! is explicit and user-driven — see the [`crate::agreements`] module and the
//! desktop app's enrollment page. This module only *consumes* the result: a
//! Converse failure caused by missing access is classified by
//! [`classify_converse_error`] into [`crate::error::BedrockError::ModelAccess`]
//! so the UI can route the user to enrollment for the offending model.
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
//! bedrock:DeleteFoundationModelAgreement
//! bedrock:PutUseCaseForModelAccess
//! bedrock:GetUseCaseForModelAccess
//! bedrock:InvokeModel
//! bedrock:InvokeModelWithResponseStream
//! bedrock:CountTokens
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

use crate::{
    error::{BedrockError, ModelAccessReason},
    tokens,
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
pub(crate) async fn fetch_active_foundation_models(
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
pub(crate) async fn fetch_us_inference_profiles(
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
pub(crate) fn strip_scope_prefix(id: &str) -> &str {
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
            classify_converse_error(model_id, &msg)
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

/// Classify a Converse service error into a structured [`BedrockError`].
///
/// When the failure is an access/agreement problem we emit
/// [`BedrockError::ModelAccess`] carrying the *bare* foundation model id (so the
/// UI can deep-link to model enrollment) and a coarse reason. Anything else
/// stays a plain [`BedrockError::Invocation`].
///
/// `model_id` is the inference profile id passed to Converse; the marker strips
/// it to the bare foundation id. Matching is on the error message text because
/// Bedrock collapses several distinct conditions onto `AccessDeniedException`.
pub fn classify_converse_error(model_id: &str, msg: &str) -> BedrockError {
    let lower = msg.to_lowercase();
    let bare = strip_scope_prefix(model_id).to_string();

    // Anthropic first-time-use form not submitted for the account.
    if lower.contains("ftuformnotfilled")
        || lower.contains("usecaseform")
        || lower.contains("use case details")
        || lower.contains("use-case")
        || lower.contains("first-time")
    {
        return BedrockError::ModelAccess {
            model_id: bare,
            reason: ModelAccessReason::UseCaseFormRequired,
        };
    }

    // Bare model id invoked where an inference profile is required.
    if lower.contains("on-demand throughput isn't supported")
        || lower.contains("on-demand throughput isn’t supported")
        || lower.contains("inference profile")
    {
        return BedrockError::ModelAccess {
            model_id: bare,
            reason: ModelAccessReason::NeedsInferenceProfile,
        };
    }

    // Access denied → almost always a missing marketplace agreement/subscription.
    if lower.contains("accessdenied")
        || lower.contains("access to the model")
        || lower.contains("not authorized")
        || lower.contains("marketplace")
        || lower.contains("not subscribed")
        || lower.contains("entitlement")
    {
        return BedrockError::ModelAccess {
            model_id: bare,
            reason: ModelAccessReason::NotSubscribed,
        };
    }

    BedrockError::Invocation(msg.to_string())
}

// ── Token counting ───────────────────────────────────────────────────────────

/// Count the input tokens for a context (system prompt) before the user sends
/// a message. Uses the free Bedrock `CountTokens` API.
///
/// A minimal dummy user message is included because the Converse format
/// requires at least one message.
pub async fn count_context_tokens(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
) -> Result<u32, BedrockError> {
    let client = aws_sdk_bedrockruntime::Client::new(config);

    // CountTokens requires a bare foundation model ID, not an inference
    // profile ID (e.g. `anthropic.claude-sonnet-4-6` not
    // `us.anthropic.claude-sonnet-4-6`).
    let bare_model_id = strip_scope_prefix(model_id);

    let dummy_message = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(".".to_string()))
        .build()
        .map_err(|e| BedrockError::Invocation(e.to_string()))?;

    let converse_input = ConverseTokensRequest::builder()
        .messages(dummy_message)
        .system(SystemContentBlock::Text(system_prompt.to_string()))
        .build();

    info!(bare_model_id, "counting context tokens");

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

// Model-access agreements now live in `crate::agreements` (explicit, user-driven
// enrollment) — chat no longer accepts them on the user's behalf.
