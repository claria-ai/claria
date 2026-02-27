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
//! ## Inference profile ID structure
//!
//! IDs follow the pattern `{scope}.{provider}.{model}-{version_info}`, but
//! the version suffix is inconsistent across generations:
//!
//! ```text
//! us.anthropic.claude-sonnet-4-6                      (no date, no version)
//! us.anthropic.claude-sonnet-4-20250514-v1:0          (date + version)
//! us.anthropic.claude-haiku-4-5-20251001-v1:0         (date + version)
//! ```
//!
//! Because the suffix format varies, we don't parse these into structured
//! tuples — we treat them as opaque strings and match against the foundation
//! model registry by bare ID.
//!
//! ## Legacy model filtering
//!
//! AWS marks superseded models as `LEGACY` in the foundation model registry,
//! but continues to list their inference profiles as `ACTIVE`. Attempting to
//! invoke a legacy profile returns an error like:
//!
//! > "The model is marked by provider as Legacy."
//!
//! We originally maintained a hardcoded list of legacy model ID fragments,
//! but this broke every time AWS deprecated another generation. The current
//! approach is fully dynamic:
//!
//! 1. Call `ListFoundationModels(provider="anthropic")` and collect all IDs
//!    where `model_lifecycle.status == LEGACY` into a `HashSet`.
//! 2. Call `ListInferenceProfiles` and filter out any profile whose bare
//!    model ID appears in the legacy set.
//!
//! This means Claria automatically excludes newly deprecated models without
//! code changes.
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

use std::collections::HashSet;

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

/// List available Anthropic Claude inference profiles, excluding legacy models.
///
/// 1. Calls `ListFoundationModels` to build a set of model IDs with `LEGACY`
///    lifecycle status. This is the authoritative, dynamic signal from AWS —
///    no hardcoded list needed.
/// 2. Calls `ListInferenceProfiles` filtered to system-defined, active profiles
///    whose ID contains `"anthropic.claude"`.
/// 3. Filters out any profile whose underlying foundation model is legacy.
///
/// Results are sorted by name.
pub async fn list_chat_models(
    config: &aws_config::SdkConfig,
) -> Result<Vec<ChatModel>, BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    // Step 1: Build a set of legacy foundation model IDs.
    let legacy_ids = fetch_legacy_model_ids(&client).await?;

    // Step 2: List inference profiles.
    let response = client
        .list_inference_profiles()
        .type_equals(InferenceProfileType::SystemDefined)
        .max_results(100)
        .send()
        .await
        .map_err(|e| BedrockError::Invocation(e.into_service_error().to_string()))?;

    let mut models: Vec<ChatModel> = response
        .inference_profile_summaries()
        .iter()
        .filter(|p| {
            let id = p.inference_profile_id();
            id.contains("anthropic.claude")
                && *p.status() == InferenceProfileStatus::Active
                && !is_profile_legacy(id, &legacy_ids)
        })
        .map(|p| ChatModel {
            model_id: p.inference_profile_id().to_string(),
            name: p.inference_profile_name().to_string(),
        })
        .collect();

    models.sort_by(|a, b| a.name.cmp(&b.name));

    info!(
        count = models.len(),
        legacy = legacy_ids.len(),
        "discovered chat models"
    );

    Ok(models)
}

/// Fetch all Anthropic foundation model IDs that have `LEGACY` lifecycle status.
async fn fetch_legacy_model_ids(
    client: &aws_sdk_bedrock::Client,
) -> Result<HashSet<String>, BedrockError> {
    let response = client
        .list_foundation_models()
        .by_provider("anthropic")
        .send()
        .await
        .map_err(|e| BedrockError::Invocation(e.into_service_error().to_string()))?;

    let legacy: HashSet<String> = response
        .model_summaries()
        .iter()
        .filter(|m| {
            m.model_lifecycle()
                .map(|lc| *lc.status() == FoundationModelLifecycleStatus::Legacy)
                .unwrap_or(false)
        })
        .map(|m| m.model_id().to_string())
        .collect();

    Ok(legacy)
}

/// Check whether an inference profile's underlying model is in the legacy set.
///
/// Inference profile IDs look like `us.anthropic.claude-sonnet-4-6` or
/// `global.anthropic.claude-opus-4-5-20251101-v1:0`. The foundation model ID
/// is the part after the first dot: `anthropic.claude-...`. Each profile also
/// lists its underlying model ARNs, but we can match more simply by stripping
/// the scope prefix and checking if the bare ID is in the legacy set.
fn is_profile_legacy(inference_profile_id: &str, legacy_ids: &HashSet<String>) -> bool {
    // Strip scope prefix (e.g. "us." or "global.") to get the bare model ID.
    let bare_id = inference_profile_id
        .split_once('.')
        .map(|(_, rest)| rest)
        .unwrap_or(inference_profile_id);

    legacy_ids.contains(bare_id)
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
    let mut seen_bare_ids = std::collections::HashSet::new();

    for model in &models {
        let bare_id = model
            .model_id
            .split_once('.')
            .map(|(_, rest)| rest)
            .unwrap_or(&model.model_id);

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
