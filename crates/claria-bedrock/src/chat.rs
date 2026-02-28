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
    AgreementStatus, FoundationModelLifecycleStatus, InferenceProfileStatus, InferenceProfileType,
};
use aws_sdk_bedrockruntime::types::{ContentBlock, ConversationRole, Message, SystemContentBlock};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::BedrockError;

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
        .map_err(|e| BedrockError::Invocation(e.into_service_error().to_string()))?;

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
        .map_err(|e| BedrockError::Invocation(e.into_service_error().to_string()))?;

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

/// Send a multi-turn conversation to Bedrock and return the assistant's reply.
///
/// The caller provides the full message history and a system prompt.
/// This is the shared implementation used by the desktop chat command.
pub async fn chat_converse(
    config: &aws_config::SdkConfig,
    model_id: &str,
    system_prompt: &str,
    messages: &[ChatMessage],
) -> Result<String, BedrockError> {
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

    let response = client
        .converse()
        .model_id(model_id)
        .system(SystemContentBlock::Text(system_prompt.to_string()))
        .set_messages(Some(converse_messages))
        .send()
        .await
        .map_err(|e| BedrockError::Invocation(e.into_service_error().to_string()))?;

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

    Ok(response_text)
}

// ── Model agreement management ───────────────────────────────────────────────

/// Accept the Marketplace agreement for a foundation model.
///
/// Lists available offers for the model and accepts the first one.
/// This is a no-op if the model has no agreement requirement.
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
        return Err(BedrockError::Agreement(format!(
            "no agreement offers found for model {model_id}"
        )));
    }

    let offer_token = offers[0].offer_token();

    info!(model_id, offer_token, "accepting model agreement");

    client
        .create_foundation_model_agreement()
        .model_id(model_id)
        .offer_token(offer_token)
        .send()
        .await
        .map_err(|e| BedrockError::Agreement(e.into_service_error().to_string()))?;

    info!(model_id, "model agreement accepted");

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

/// Check and accept Marketplace agreements for all available Claude models.
///
/// Lists all Claude inference profiles, checks each underlying foundation
/// model's agreement status, and accepts any that are pending. Returns a
/// summary of what was done.
pub async fn accept_all_model_agreements(
    config: &aws_config::SdkConfig,
) -> Result<AgreementSummary, BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    // Discover all Claude inference profiles.
    let models = list_chat_models(config).await?;

    let mut summary = AgreementSummary {
        already_accepted: Vec::new(),
        newly_accepted: Vec::new(),
        failed: Vec::new(),
    };

    // Deduplicate: inference profiles like `us.anthropic.claude-sonnet-4-...`
    // and `global.anthropic.claude-sonnet-4-...` share the same underlying
    // model. Strip the region prefix to get the bare model ID.
    // Models without inference profiles already have bare IDs like
    // `anthropic.claude-opus-4-6-v1`.
    let mut seen_bare_ids = std::collections::HashSet::new();

    for model in &models {
        let bare_id = strip_scope_prefix(&model.model_id);

        if !seen_bare_ids.insert(bare_id.to_string()) {
            continue;
        }

        // Check agreement status for this foundation model.
        let availability = client
            .get_foundation_model_availability()
            .model_id(bare_id)
            .send()
            .await;

        let needs_agreement = match &availability {
            Ok(resp) => resp
                .agreement_availability()
                .map(|a| *a.status() == AgreementStatus::Available)
                .unwrap_or(false),
            Err(_) => {
                // Can't check — skip this model.
                continue;
            }
        };

        if !needs_agreement {
            summary.already_accepted.push(bare_id.to_string());
            continue;
        }

        // Accept the agreement.
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
        already_accepted = summary.already_accepted.len(),
        newly_accepted = summary.newly_accepted.len(),
        failed = summary.failed.len(),
        "model agreement check complete"
    );

    Ok(summary)
}
