//! BAA (Business Associate Addendum) check via AWS Artifact.
//!
//! HIPAA requires that a BAA is in place with AWS before processing PHI.
//! This module queries the AWS Artifact `ListCustomerAgreements` API to
//! verify whether the account has an active BAA.

use aws_sdk_artifact::types::CustomerAgreementState;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::ProvisionerError;

/// Result of checking whether the AWS account has an active BAA.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BaaStatus {
    /// Whether an active BAA agreement was found on the account.
    pub has_baa: bool,
    /// Name of the agreement, if found.
    pub agreement_name: Option<String>,
    /// When the agreement became effective (ISO 8601), if found.
    pub effective_start: Option<String>,
    /// State of the agreement (e.g. "active"), if found.
    pub state: Option<String>,
}

/// Check whether the AWS account has an active BAA via AWS Artifact.
///
/// This is a read-only operation. It calls `ListCustomerAgreements` and
/// looks for an active agreement whose name contains "BAA" or
/// "Business Associate" (case-insensitive).
pub async fn check_baa(config: &aws_config::SdkConfig) -> Result<BaaStatus, ProvisionerError> {
    let client = aws_sdk_artifact::Client::new(config);

    let resp = client
        .list_customer_agreements()
        .send()
        .await
        .map_err(|e| ProvisionerError::Aws(format!("artifact:ListCustomerAgreements failed: {e}")))?;

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
            return Ok(BaaStatus {
                has_baa: true,
                agreement_name: Some(name.to_string()),
                effective_start: agreement.effective_start().map(|d| d.to_string()),
                state: agreement.state().map(|s| s.as_str().to_string()),
            });
        }
    }

    Ok(BaaStatus {
        has_baa: false,
        agreement_name: None,
        effective_start: None,
        state: None,
    })
}
