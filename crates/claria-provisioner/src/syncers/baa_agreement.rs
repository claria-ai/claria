use aws_sdk_artifact::types::CustomerAgreementState;
use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct BaaAgreementSyncer {
    spec: ResourceSpec,
    client: aws_sdk_artifact::Client,
}

impl BaaAgreementSyncer {
    pub fn new(spec: ResourceSpec, config: &aws_config::SdkConfig) -> Self {
        Self {
            spec,
            client: aws_sdk_artifact::Client::new(config),
        }
    }
}

impl ResourceSyncer for BaaAgreementSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let resp = self
                .client
                .list_customer_agreements()
                .send()
                .await
                .map_err(|e| {
                    ProvisionerError::Aws(format!(
                        "artifact:ListCustomerAgreements failed: {e}"
                    ))
                })?;

            for agreement in resp.customer_agreements() {
                let is_active = agreement
                    .state()
                    .is_some_and(|s| *s == CustomerAgreementState::Active);

                if !is_active {
                    continue;
                }

                let name = agreement.name().unwrap_or_default();
                let name_lower = name.to_lowercase();

                if name_lower.contains("baa") || name_lower.contains("business associate") {
                    return Ok(Some(json!({
                        "state": "active",
                        "agreement_name": name,
                        "effective_start": agreement.effective_start().map(|d| d.to_string()),
                    })));
                }
            }

            Ok(None)
        })
    }

    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift> {
        let state = actual
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");

        if state == "active" {
            vec![]
        } else {
            vec![FieldDrift {
                field: "state".into(),
                label: "Agreement status".into(),
                expected: json!("active"),
                actual: json!(state),
            }]
        }
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        // BAA agreement acceptance requires manual action in the AWS console
        // or additional API calls that aren't yet in the SDK.
        // For now, mark as needing manual action.
        Box::pin(async {
            Err(ProvisionerError::CreateFailed(
                "BAA agreement must be accepted in the AWS console. Go to AWS Artifact \
                 and accept the Business Associate Addendum."
                    .into(),
            ))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::UpdateFailed(
                "BAA agreement state cannot be modified programmatically".into(),
            ))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            // BAA termination is a significant legal action — don't automate it.
            tracing::warn!("BAA termination skipped — must be done manually in AWS Artifact");
            Ok(())
        })
    }
}
