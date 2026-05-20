//! Detecting which Bedrock foundation models the current account can invoke.
//!
//! AWS Bedrock exposes a half-dozen catalog APIs that look like they should
//! tell us this — `ListFoundationModels`, `GetFoundationModelAvailability`,
//! `ListInferenceProfiles`, `GetInferenceProfile`, `ListFoundationModel-
//! AgreementOffers` — but empirically none of them is authoritative. They
//! happily report healthy state for half-published preview entries (e.g.
//! `anthropic.claude-opus-4-7`) that the runtime then rejects with
//! `AccessDeniedException: <model> is not available for this account`.
//!
//! The only honest signal is to ask the runtime itself. `CountTokens` is
//! free (no token billing), fast (~150 ms per call), and uses the same
//! auth path as `Converse`. If `CountTokens` accepts a model, real
//! invocation will too; if it rejects with `ValidationException` or
//! `AccessDeniedException`, the model is not invokable for this account.
//!
//! Both `list_chat_models` (chat dropdown) and `BedrockModelAgreement-
//! Syncer` (provisioner) call into this module so they answer the same
//! question with the same signal.

use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, ConverseTokensRequest, CountTokensInput, Message,
};

/// Outcome of a CountTokens-based invokability probe.
#[derive(Debug, Clone)]
pub enum InvocabilityProbe {
    /// The runtime accepted the call — the model is invokable for this account.
    Invokable,
    /// The runtime rejected the call. Contains the AWS error message for display.
    Rejected(String),
}

impl InvocabilityProbe {
    pub fn is_invokable(&self) -> bool {
        matches!(self, InvocabilityProbe::Invokable)
    }
}

/// Probe one bare foundation-model ID with a free CountTokens call.
///
/// `bare_model_id` must be a bare foundation model ID (e.g.
/// `anthropic.claude-sonnet-4-6`), not an inference profile ID. Strip the
/// `us.` / `eu.` / `global.` prefix beforehand if you have a profile ID.
pub async fn probe_invokable(runtime: &Client, bare_model_id: &str) -> InvocabilityProbe {
    let dummy = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(".".into()))
        .build();
    let message = match dummy {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(bare_model_id, error = %e, "CountTokens probe: dummy message build failed");
            return InvocabilityProbe::Rejected(e.to_string());
        }
    };
    let req = ConverseTokensRequest::builder().messages(message).build();
    match runtime
        .count_tokens()
        .model_id(bare_model_id)
        .input(CountTokensInput::Converse(req))
        .send()
        .await
    {
        Ok(_) => InvocabilityProbe::Invokable,
        Err(e) => {
            let msg = e.into_service_error().to_string();
            tracing::info!(bare_model_id, error = %msg, "CountTokens probe rejected — model not invokable");
            InvocabilityProbe::Rejected(msg)
        }
    }
}

/// Probe many bare foundation-model IDs in parallel.
///
/// Returns one `InvocabilityProbe` per input, in the same order. Callers
/// that want a map keyed on model ID can zip the inputs with the outputs.
pub async fn probe_many(runtime: &Client, bare_ids: &[String]) -> Vec<InvocabilityProbe> {
    let futures = bare_ids
        .iter()
        .map(|id| probe_invokable(runtime, id.as_str()));
    futures::future::join_all(futures).await
}
