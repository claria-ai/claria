use aws_sdk_bedrock::{
    Client,
    types::{AgreementStatus, FoundationModelLifecycleStatus},
};
use serde_json::{Value, json};

use crate::{
    error::{ProvisionerError, format_err_chain},
    manifest::ResourceSpec,
    syncer::{BoxFuture, ResourceSyncer},
};

/// Check whether a model ID is a context-window variant (e.g. `:48k`, `:200k`).
fn is_context_window_variant(model_id: &str) -> bool {
    model_id.rsplit_once(':').is_some_and(|(_, suffix)| {
        suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) && suffix != "0"
    })
}

/// Outcome of a zero-cost invocation probe against the Bedrock runtime.
///
/// `GetFoundationModelAvailability` can report a model as fully available
/// (agreement AVAILABLE, entitlement AVAILABLE, AUTHORIZED) while the runtime
/// still rejects invocation — AWS enables the newest models per-account on a
/// gradual rollout. The only ground truth is asking the runtime itself.
///
/// The probe sends `Converse` with an empty message list. Access checks run
/// before request validation, so an accessible model answers with a
/// `ValidationException` ("conversation must start with a user message")
/// while a gated model answers with an `AccessDeniedException`. No tokens are
/// billed either way.
enum InvocationProbe {
    /// Runtime accepted the request past the access check.
    Invokable,
    /// Runtime refused with an access error (message included).
    Denied(String),
}

pub struct BedrockModelAgreementSyncer {
    spec: ResourceSpec,
    client: Client,
    runtime_client: aws_sdk_bedrockruntime::Client,
}

impl BedrockModelAgreementSyncer {
    pub fn new(
        spec: ResourceSpec,
        client: Client,
        runtime_client: aws_sdk_bedrockruntime::Client,
    ) -> Self {
        Self {
            spec,
            client,
            runtime_client,
        }
    }

    fn model_prefix(&self) -> &str {
        &self.spec.resource_name
    }

    /// Enumerate ACTIVE foundation models matching this spec's prefix.
    ///
    /// LEGACY models are excluded: the chat UI never offers them, and once
    /// AWS begins decommissioning one its availability flips in ways the
    /// account can't fix — including it here would create permanent drift.
    async fn matching_model_ids(&self) -> Result<Vec<String>, ProvisionerError> {
        let models = self
            .client
            .list_foundation_models()
            .send()
            .await
            .map_err(|e| {
                let msg = format_err_chain(&e);
                tracing::error!(error = %msg, "ListFoundationModels failed");
                ProvisionerError::Aws(msg)
            })?;

        Ok(models
            .model_summaries()
            .iter()
            .filter(|m| {
                m.model_lifecycle()
                    .map(|lc| *lc.status() == FoundationModelLifecycleStatus::Active)
                    .unwrap_or(false)
            })
            .map(|m| m.model_id().to_string())
            .filter(|id| id.contains(self.model_prefix()) && !is_context_window_variant(id))
            .collect())
    }

    /// Zero-cost runtime probe: can this account actually invoke the model?
    ///
    /// Probes via the `us.{model_id}` inference profile — the same ID the
    /// chat path invokes. Errors other than access denial (throttling,
    /// nonexistent profile for a brand-new model, ...) count as invokable so
    /// transient failures don't produce spurious drift.
    async fn probe_invocation(&self, model_id: &str) -> InvocationProbe {
        let profile_id = format!("us.{model_id}");
        match self
            .runtime_client
            .converse()
            .model_id(&profile_id)
            .set_messages(Some(Vec::new()))
            .send()
            .await
        {
            // Can't happen with no messages, but if it did, access is proven.
            Ok(_) => InvocationProbe::Invokable,
            Err(e) => {
                let service_err = e.into_service_error();
                if service_err.is_access_denied_exception() {
                    let msg = service_err
                        .meta()
                        .message()
                        .unwrap_or("access denied")
                        .to_string();
                    tracing::info!(model_id, error = %msg, "invocation probe denied");
                    InvocationProbe::Denied(msg)
                } else {
                    InvocationProbe::Invokable
                }
            }
        }
    }

    /// Classify each matching model's invocation availability for this
    /// account and region.
    ///
    /// `GetFoundationModelAvailability` alone is not trustworthy: every
    /// field can read available/authorized while the runtime still denies
    /// invocation (AWS rolls the newest models out per-account), and
    /// `entitlement_availability` reads `AVAILABLE` even before the model's
    /// marketplace agreement has been accepted. So the runtime probe decides
    /// invokability, and the availability fields only inform the blocked
    /// reason.
    async fn classify_models(&self) -> Result<ModelAvailability, ProvisionerError> {
        let ids = self.matching_model_ids().await?;
        let mut invokable: Vec<String> = Vec::new();
        let mut blocked: Vec<BlockedModel> = Vec::new();

        for model_id in ids {
            match self.probe_invocation(&model_id).await {
                InvocationProbe::Invokable => invokable.push(model_id),
                InvocationProbe::Denied(denial) => {
                    let reason = self.describe_block(&model_id, &denial).await;
                    blocked.push(BlockedModel { model_id, reason });
                }
            }
        }

        Ok(ModelAvailability { invokable, blocked })
    }

    /// Compose a human-readable reason a model isn't invokable.
    ///
    /// Called only after the runtime probe was denied. Reads the agreement
    /// status to distinguish the auto-fixable case (agreement not accepted
    /// yet, offer available — Apply accepts it) from the AWS-side gate
    /// (everything green but AWS hasn't enabled the model for this account
    /// yet — only AWS Support can expedite that).
    async fn describe_block(&self, model_id: &str, denial: &str) -> String {
        let availability = match self
            .client
            .get_foundation_model_availability()
            .model_id(model_id)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                let msg = format_err_chain(&e);
                tracing::warn!(model_id, error = %msg, "GetFoundationModelAvailability failed");
                return format!("not invokable ({denial}); availability probe failed: {msg}");
            }
        };

        let agreement_status = availability
            .agreement_availability()
            .map(|a| a.status().clone());

        match agreement_status {
            Some(AgreementStatus::Pending) => {
                "marketplace agreement acceptance is in progress — re-scan in a moment".into()
            }
            Some(AgreementStatus::Error) => format!(
                "marketplace agreement errored: {}",
                availability
                    .agreement_availability()
                    .and_then(|a| a.error_message())
                    .unwrap_or("no detail from AWS")
            ),
            Some(AgreementStatus::NotAvailable) | None => {
                // Agreement not accepted. If AWS lists an offer, Apply can
                // accept it automatically; otherwise the console is the only
                // path.
                let has_offer = self
                    .client
                    .list_foundation_model_agreement_offers()
                    .model_id(model_id)
                    .send()
                    .await
                    .map(|resp| !resp.offers().is_empty())
                    .unwrap_or(false);
                if has_offer {
                    "marketplace agreement not accepted yet — click Apply to accept it \
                     automatically"
                        .into()
                } else {
                    "no marketplace agreement offer for this model — request access on the \
                     Bedrock \"Model access\" console page"
                        .into()
                }
            }
            // Agreement in place (or an unrecognized future status) yet the
            // runtime still refuses: AWS hasn't enabled this model for the
            // account. Nothing Claria or the Bedrock console can do.
            _ => format!(
                "AWS is rolling this model out gradually and hasn't enabled it for this \
                 account yet — contact AWS Support to expedite (AWS said: {denial})"
            ),
        }
    }

    /// Accept all available marketplace agreements for matching models.
    /// Idempotent — re-accepting an existing agreement is treated as success.
    async fn accept_agreements(&self) -> Result<(), ProvisionerError> {
        let ids = self.matching_model_ids().await?;
        let mut failures: Vec<String> = Vec::new();

        for model_id in &ids {
            let offers = match self
                .client
                .list_foundation_model_agreement_offers()
                .model_id(model_id)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    let msg = format_err_chain(&e);
                    tracing::warn!(model_id, error = %msg, "failed to list agreement offers");
                    failures.push(format!("{model_id}: {msg}"));
                    continue;
                }
            };

            if offers.offers().is_empty() {
                continue;
            }

            let offer_token = offers.offers()[0].offer_token();

            match self
                .client
                .create_foundation_model_agreement()
                .model_id(model_id)
                .offer_token(offer_token)
                .send()
                .await
            {
                Ok(_) => {
                    tracing::info!(model_id, "model agreement accepted");
                }
                Err(e) => {
                    let msg = format_err_chain(&e);
                    if msg.contains("already exists") {
                        tracing::debug!(model_id, "model agreement already accepted");
                    } else {
                        tracing::warn!(model_id, error = %msg, "failed to accept model agreement");
                        failures.push(format!("{model_id}: {msg}"));
                    }
                }
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(ProvisionerError::Aws(failures.join("; ")))
        }
    }
}

/// One unavailable model and the reason it can't be invoked.
struct BlockedModel {
    model_id: String,
    reason: String,
}

/// Result of classifying matching models against actual runtime access.
struct ModelAvailability {
    invokable: Vec<String>,
    blocked: Vec<BlockedModel>,
}

impl ModelAvailability {
    fn into_json(self) -> Value {
        let agreement = if self.blocked.is_empty() {
            "accepted"
        } else {
            "blocked"
        };
        json!({
            "agreement": agreement,
            "invokable_models": self.invokable,
            "blocked_models": self
                .blocked
                .into_iter()
                .map(|b| json!({"model_id": b.model_id, "reason": b.reason}))
                .collect::<Vec<_>>(),
        })
    }
}

impl ResourceSyncer for BedrockModelAgreementSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let availability = self.classify_models().await?;
            Ok(Some(availability.into_json()))
        })
    }

    /// Reduce `read()`'s rich state to a single comparable field so drift
    /// detection trips when any model is blocked.
    fn current_state(&self, actual: &Value) -> Value {
        json!({
            "agreement": actual
                .get("agreement")
                .cloned()
                .unwrap_or(Value::Null),
        })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            // Try to accept any pending marketplace agreements — this is the
            // only mutation we can do without user intervention.
            self.accept_agreements().await?;

            // Re-classify. If any model is still blocked, surface the reasons
            // so the UI explains what the user must do manually.
            let availability = self.classify_models().await?;
            if !availability.blocked.is_empty() {
                let summary = availability
                    .blocked
                    .iter()
                    .map(|b| format!("{}: {}", b.model_id, b.reason))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(ProvisionerError::Aws(format!(
                    "Bedrock models still blocked after accepting agreements — {summary}"
                )));
            }
            Ok(availability.into_json())
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        // Can't un-accept a model agreement
        Box::pin(async { Ok(()) })
    }
}
