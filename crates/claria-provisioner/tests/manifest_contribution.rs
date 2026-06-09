use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use aws_config::SdkConfig;
use serde_json::json;

use claria_provisioner::syncer::BoxFuture;
use claria_provisioner::{
    build_manifest_with_contributors, CredentialScope, Lifecycle, Manifest, ManifestContributor,
    ProvisionerError, ResourceSpec, Severity,
};

const ACCT: &str = "123456789012";
const SYS: &str = "claria";
const REGION: &str = "us-east-1";

fn dummy_sdk_config() -> SdkConfig {
    SdkConfig::builder().build()
}

fn make_marketplace_spec(model_name: &str) -> ResourceSpec {
    ResourceSpec {
        resource_type: "bedrock_model_agreement".into(),
        resource_name: model_name.into(),
        lifecycle: Lifecycle::Managed,
        desired: json!({"agreement": "accepted"}),
        credential_scope: CredentialScope::Regular,
        label: format!("{model_name} Access"),
        description: "AI model access".into(),
        severity: Severity::Elevated,
        iam_actions: vec!["bedrock:CreateFoundationModelAgreement".into()],
    }
}

struct FixedContributor {
    specs: Vec<ResourceSpec>,
    call_count: Arc<AtomicUsize>,
}

impl FixedContributor {
    fn new(specs: Vec<ResourceSpec>) -> Self {
        Self {
            specs,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl ManifestContributor for FixedContributor {
    fn contribute<'a>(
        &'a self,
        _sdk: &'a SdkConfig,
    ) -> BoxFuture<'a, Result<Vec<ResourceSpec>, ProvisionerError>> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let specs = self.specs.clone();
        Box::pin(async move { Ok(specs) })
    }
}

struct ErroringContributor;

impl ManifestContributor for ErroringContributor {
    fn contribute<'a>(
        &'a self,
        _sdk: &'a SdkConfig,
    ) -> BoxFuture<'a, Result<Vec<ResourceSpec>, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::State(
                "synthetic contributor failure".into(),
            ))
        })
    }
}

fn static_manifest() -> Manifest {
    Manifest::claria(ACCT, SYS, REGION)
}

#[tokio::test]
async fn build_manifest_with_no_contributors_matches_legacy_behavior() {
    let sdk = dummy_sdk_config();
    let manifest = build_manifest_with_contributors(&sdk, ACCT, SYS, REGION, &[])
        .await
        .unwrap();

    // Should equal the static-only manifest
    let static_only = static_manifest();
    assert_eq!(manifest.specs.len(), static_only.specs.len());
    for (a, b) in manifest.specs.iter().zip(static_only.specs.iter()) {
        assert_eq!(a.resource_type, b.resource_type);
        assert_eq!(a.resource_name, b.resource_name);
    }
}

#[tokio::test]
async fn build_manifest_includes_contributed_specs() {
    let sdk = dummy_sdk_config();
    let contributor: Box<dyn ManifestContributor> = Box::new(FixedContributor::new(vec![
        make_marketplace_spec("anthropic.claude-sonnet-4"),
        make_marketplace_spec("anthropic.claude-opus-4"),
    ]));
    let contributors = [contributor];

    let manifest = build_manifest_with_contributors(&sdk, ACCT, SYS, REGION, &contributors)
        .await
        .unwrap();

    let static_count = static_manifest().specs.len();
    assert_eq!(manifest.specs.len(), static_count + 2);

    let names: Vec<&str> = manifest
        .specs
        .iter()
        .filter(|s| s.resource_type == "bedrock_model_agreement")
        .map(|s| s.resource_name.as_str())
        .collect();
    assert!(names.contains(&"anthropic.claude-sonnet-4"));
    assert!(names.contains(&"anthropic.claude-opus-4"));
}

#[tokio::test]
async fn build_manifest_propagates_contributor_errors() {
    let sdk = dummy_sdk_config();
    let contributor: Box<dyn ManifestContributor> = Box::new(ErroringContributor);
    let contributors = [contributor];

    let result = build_manifest_with_contributors(&sdk, ACCT, SYS, REGION, &contributors).await;
    match result {
        Err(e) => {
            let s = e.to_string();
            assert!(
                s.contains("synthetic contributor failure"),
                "error did not propagate: {s}"
            );
        }
        Ok(_) => panic!("expected contributor error to propagate"),
    }
}

#[tokio::test]
async fn build_manifest_invokes_each_contributor_once() {
    let sdk = dummy_sdk_config();
    let c1 = FixedContributor::new(vec![make_marketplace_spec("anthropic.claude-sonnet-4")]);
    let c2 = FixedContributor::new(vec![make_marketplace_spec("anthropic.claude-opus-4")]);
    let count_1 = c1.call_count.clone();
    let count_2 = c2.call_count.clone();

    let contributors: Vec<Box<dyn ManifestContributor>> = vec![Box::new(c1), Box::new(c2)];
    let _ = build_manifest_with_contributors(&sdk, ACCT, SYS, REGION, &contributors)
        .await
        .unwrap();

    assert_eq!(count_1.load(Ordering::SeqCst), 1);
    assert_eq!(count_2.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn build_manifest_first_contributor_error_short_circuits_subsequent() {
    let sdk = dummy_sdk_config();
    let c2 = FixedContributor::new(vec![make_marketplace_spec("anthropic.claude-opus-4")]);
    let count_2 = c2.call_count.clone();

    let contributors: Vec<Box<dyn ManifestContributor>> =
        vec![Box::new(ErroringContributor), Box::new(c2)];

    let result = build_manifest_with_contributors(&sdk, ACCT, SYS, REGION, &contributors).await;
    assert!(result.is_err(), "expected error from first contributor");
    // Second contributor must not have run after the first errored
    assert_eq!(count_2.load(Ordering::SeqCst), 0);
}
