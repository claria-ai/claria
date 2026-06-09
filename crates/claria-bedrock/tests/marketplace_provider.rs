//! Unit tests for the pure parts of the marketplace agreement provider.
//!
//! Integration tests that exercise actual `ListFoundationModels` API calls
//! through `claria-mock-aws` belong in a separate test file when the mock
//! AWS infrastructure is ready. These tests focus on the spec-shaping logic
//! that's purely a function of model IDs.

use claria_bedrock::live_aws::BedrockMarketplaceAgreementProvider;
use claria_live_aws::{LiveAwsValueProvider, ProviderKind};

#[test]
fn provider_key_is_stable() {
    let provider = BedrockMarketplaceAgreementProvider::new();
    assert_eq!(provider.key(), "bedrock.marketplace_agreements");
}

#[test]
fn provider_kind_is_provisioner_contributor() {
    let provider = BedrockMarketplaceAgreementProvider::new();
    assert!(matches!(
        provider.kind(),
        ProviderKind::ProvisionerContributor
    ));
}

#[test]
fn provider_has_meaningful_label_and_description() {
    let provider = BedrockMarketplaceAgreementProvider::new();
    assert!(!provider.label().is_empty());
    assert!(provider.description().len() > 30);
}
