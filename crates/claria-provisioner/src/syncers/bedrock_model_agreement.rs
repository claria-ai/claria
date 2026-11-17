use aws_sdk_bedrock::{
    Client,
    types::{AgreementAvailability, EntitlementAvailability},
};
use aws_smithy_types::error::display::DisplayErrorContext;
use serde_json::{Value, json};

use crate::{
    error::ProvisionerError,
    manifest::ResourceSpec,
    syncer::{BoxFuture, ResourceSyncer},
};

/// Check whether a model ID is a context-window variant (e.g. `:48k`, `:200k`).
fn is_context_window_variant(model_id: &str) -> bool {
    model_id.rsplit_once(':').is_some_and(|(_, suffix)| {
        suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) && suffix != "0"
    })
}

pub struct BedrockModelAgreementSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl BedrockModelAgreementSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn model_prefix(&self) -> &str {
        &self.spec.resource_name
    }

    /// Enumerate foundation models matching this spec's prefix.
    async fn matching_model_ids(&self) -> Result<Vec<String>, ProvisionerError> {
        let models = self
            .client
            .list_foundation_models()
            .send()
            .await
            .map_err(|e| {
                let msg = DisplayErrorContext(&e).to_string();
                tracing::error!(error = %msg, "ListFoundationModels failed");
                ProvisionerError::Aws(msg)
            })?;

        Ok(models
            .model_summaries()
            .iter()
            .map(|m| m.model_id().to_string())
            .filter(|id| id.contains(self.model_prefix()) && !is_context_window_variant(id))
            .collect())
    }

    /// Classify each matching model's invocation availability for this account
    /// and region. Uses `GetFoundationModelAvailability` rather than relying
    /// on "no pending marketplace offers" — an empty offer list does not
    /// imply the agreement was ever accepted (it could mean the model has
    /// no marketplace mechanism at all and access is gated elsewhere).
    async fn classify_models(&self) -> Result<ModelAvailability, ProvisionerError> {
        let ids = self.matching_model_ids().await?;
        let mut invokable: Vec<String> = Vec::new();
        let mut blocked: Vec<BlockedModel> = Vec::new();

        for model_id in ids {
            match self
                .client
                .get_foundation_model_availability()
                .model_id(&model_id)
                .send()
                .await
            {
                Ok(resp) => {
                    if matches!(
                        resp.entitlement_availability(),
                        EntitlementAvailability::Available
                    ) {
                        invokable.push(model_id);
                    } else {
                        let reason = describe_block(
                            resp.agreement_availability(),
                            resp.entitlement_availability(),
                        );
                        blocked.push(BlockedModel { model_id, reason });
                    }
                }
                Err(e) => {
                    // Don't fail the whole sync on one model — record the
                    // probe failure so it shows in the UI.
                    let msg = DisplayErrorContext(&e).to_string();
                    tracing::warn!(
                        model_id,
                        error = %msg,
                        "GetFoundationModelAvailability failed"
                    );
                    blocked.push(BlockedModel {
                        model_id,
                        reason: format!("availability probe failed: {msg}"),
                    });
                }
            }
        }

        Ok(ModelAvailability { invokable, blocked })
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
                    let msg = DisplayErrorContext(&e).to_string();
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
                    let msg = DisplayErrorContext(&e).to_string();
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

/// Result of classifying matching models against the account's entitlement
/// state.
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

/// Compose a human-readable reason a model isn't invokable, from the two
/// availability fields the Bedrock API returns.
///
/// `AVAILABLE` / `PENDING` agreement statuses describe a marketplace flow
/// Claria can accept automatically on Apply (see `accept_agreements`). Only
/// `NOT_AVAILABLE` and absent-agreement cases need the user to open the AWS
/// console themselves.
fn describe_block(
    agreement: Option<&AgreementAvailability>,
    entitlement: &EntitlementAvailability,
) -> String {
    let agreement_status = agreement.map(|a| a.status().as_str()).unwrap_or("absent");
    match agreement_status {
        "AVAILABLE" => {
            "marketplace agreement available — click Apply to accept automatically".into()
        }
        "PENDING" => {
            "marketplace agreement acceptance is in progress — re-scan in a moment".into()
        }
        "ERROR" => format!(
            "marketplace agreement errored: {}",
            agreement
                .and_then(|a| a.error_message())
                .unwrap_or("no detail from AWS")
        ),
        // NOT_AVAILABLE or absent: no marketplace flow for this model, so
        // Claria can't accept anything on the user's behalf. Access must be
        // requested via the Bedrock "Model access" console page.
        _ => match entitlement {
            EntitlementAvailability::NotAvailable => {
                "no marketplace agreement available — request access on the Bedrock \"Model access\" console page"
                    .into()
            }
            _ => format!(
                "entitlement {} / agreement {}",
                entitlement.as_str(),
                agreement_status
            ),
        },
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
