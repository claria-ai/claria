use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use aws_config::SdkConfig;
use serde_json::{json, Value};

use claria_live_aws::{
    Candidate, ChangeReason, LiveAwsValueProvider, LiveAwsValueRegistry, ProviderKind,
    Reconciliation,
};
use claria_live_aws::error::ProviderError;
use claria_live_aws::provider::BoxFuture;
use claria_provisioner::{CredentialScope, Lifecycle, ResourceSpec, Severity};

fn dummy_sdk() -> SdkConfig {
    SdkConfig::builder().build()
}

fn marketplace_spec(name: &str) -> ResourceSpec {
    ResourceSpec {
        resource_type: "bedrock_model_agreement".into(),
        resource_name: name.into(),
        lifecycle: Lifecycle::Managed,
        desired: json!({"agreement": "accepted"}),
        credential_scope: CredentialScope::Regular,
        label: name.into(),
        description: "test".into(),
        severity: Severity::Elevated,
        iam_actions: vec!["bedrock:CreateFoundationModelAgreement".into()],
    }
}

struct FixedProvider {
    key: &'static str,
    label: &'static str,
    kind: ProviderKind,
    result: Reconciliation,
    invocations: Arc<AtomicUsize>,
}

impl FixedProvider {
    fn in_sync(key: &'static str) -> Self {
        Self {
            key,
            label: key,
            kind: ProviderKind::UserChoice,
            result: Reconciliation::InSync,
            invocations: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn user_choice_drift(key: &'static str, candidates: Vec<Candidate>) -> Self {
        Self {
            key,
            label: key,
            kind: ProviderKind::UserChoice,
            result: Reconciliation::UserChoiceDrift {
                reason: ChangeReason::NeverConfigured,
                candidates,
                current: None,
                summary: "needs config".into(),
            },
            invocations: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn contribution(key: &'static str, specs: Vec<ResourceSpec>) -> Self {
        Self {
            key,
            label: key,
            kind: ProviderKind::ProvisionerContributor,
            result: Reconciliation::Contribution {
                reason: ChangeReason::NewUpstreamResource {
                    name: "test".into(),
                },
                specs,
                summary: "test contribution".into(),
            },
            invocations: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl LiveAwsValueProvider for FixedProvider {
    fn key(&self) -> &'static str {
        self.key
    }
    fn label(&self) -> &'static str {
        self.label
    }
    fn description(&self) -> &'static str {
        "test provider"
    }
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    fn reconcile<'a>(
        &'a self,
        _sdk: &'a SdkConfig,
        _current: &'a Value,
    ) -> BoxFuture<'a, Result<Reconciliation, ProviderError>> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let r = self.result.clone();
        Box::pin(async move { Ok(r) })
    }
}

struct FailingProvider {
    key: &'static str,
}

impl LiveAwsValueProvider for FailingProvider {
    fn key(&self) -> &'static str {
        self.key
    }
    fn label(&self) -> &'static str {
        self.key
    }
    fn description(&self) -> &'static str {
        "fails"
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::UserChoice
    }
    fn reconcile<'a>(
        &'a self,
        _sdk: &'a SdkConfig,
        _current: &'a Value,
    ) -> BoxFuture<'a, Result<Reconciliation, ProviderError>> {
        Box::pin(async {
            Err(ProviderError::AwsApi("synthetic api failure".into()))
        })
    }
}

fn dummy_candidate() -> Candidate {
    Candidate {
        value: json!("us.anthropic.claude-sonnet-4-20250514-v1:0"),
        label: "Claude Sonnet 4".into(),
        description: "test".into(),
        recommended: true,
        metadata: Value::Null,
    }
}

#[tokio::test]
async fn reconcile_all_returns_in_sync_for_fresh_config_with_no_drift() {
    let registry = LiveAwsValueRegistry::new()
        .with_provider(Box::new(FixedProvider::in_sync("a")))
        .with_provider(Box::new(FixedProvider::in_sync("b")));
    let values = HashMap::new();

    let (report, contributors) = registry.reconcile_all(&dummy_sdk(), &values).await;

    assert_eq!(report.in_sync, vec!["a", "b"]);
    assert!(report.user_choice_drift.is_empty());
    assert!(report.contribution_summaries.is_empty());
    assert!(report.errors.is_empty());
    assert_eq!(contributors.len(), 0);
}

#[tokio::test]
async fn reconcile_all_collects_user_choice_drift() {
    let registry = LiveAwsValueRegistry::new().with_provider(Box::new(
        FixedProvider::user_choice_drift("chat", vec![dummy_candidate()]),
    ));

    let (report, contributors) = registry.reconcile_all(&dummy_sdk(), &HashMap::new()).await;

    assert!(report.in_sync.is_empty());
    assert_eq!(report.user_choice_drift.len(), 1);
    assert_eq!(report.user_choice_drift[0].key, "chat");
    assert_eq!(report.user_choice_drift[0].candidates.len(), 1);
    assert_eq!(contributors.len(), 0);
}

#[tokio::test]
async fn reconcile_all_collects_contributions_into_contributors_vec() {
    let registry = LiveAwsValueRegistry::new().with_provider(Box::new(
        FixedProvider::contribution(
            "marketplace",
            vec![
                marketplace_spec("anthropic.claude-sonnet-4"),
                marketplace_spec("anthropic.claude-opus-4"),
            ],
        ),
    ));

    let (report, contributors) = registry.reconcile_all(&dummy_sdk(), &HashMap::new()).await;

    assert!(report.in_sync.is_empty());
    assert!(report.user_choice_drift.is_empty());
    assert_eq!(report.contribution_summaries.len(), 1);
    assert_eq!(report.contribution_summaries[0].key, "marketplace");
    assert_eq!(report.contribution_summaries[0].spec_count, 2);
    assert_eq!(contributors.len(), 1);

    // Verify contributor returns the same specs (no re-querying AWS)
    let specs = contributors[0].contribute(&dummy_sdk()).await.unwrap();
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].resource_name, "anthropic.claude-sonnet-4");
    assert_eq!(specs[1].resource_name, "anthropic.claude-opus-4");
}

#[tokio::test]
async fn reconcile_all_collects_provider_errors_without_failing_others() {
    let healthy = FixedProvider::in_sync("healthy");
    let healthy_invocations = healthy.invocations.clone();

    let registry = LiveAwsValueRegistry::new()
        .with_provider(Box::new(FailingProvider { key: "broken" }))
        .with_provider(Box::new(healthy));

    let (report, _contributors) = registry.reconcile_all(&dummy_sdk(), &HashMap::new()).await;

    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].key, "broken");
    assert!(report.errors[0].message.contains("synthetic api failure"));
    // Healthy provider still ran and was reported
    assert_eq!(report.in_sync, vec!["healthy"]);
    assert_eq!(healthy_invocations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn reconcile_all_passes_current_value_to_provider() {
    struct InspectingProvider {
        captured: Arc<std::sync::Mutex<Option<Value>>>,
    }
    impl LiveAwsValueProvider for InspectingProvider {
        fn key(&self) -> &'static str { "inspector" }
        fn label(&self) -> &'static str { "inspector" }
        fn description(&self) -> &'static str { "" }
        fn kind(&self) -> ProviderKind { ProviderKind::UserChoice }
        fn reconcile<'a>(
            &'a self,
            _sdk: &'a SdkConfig,
            current: &'a Value,
        ) -> BoxFuture<'a, Result<Reconciliation, ProviderError>> {
            *self.captured.lock().unwrap() = Some(current.clone());
            Box::pin(async { Ok(Reconciliation::InSync) })
        }
    }

    let captured = Arc::new(std::sync::Mutex::new(None));
    let registry = LiveAwsValueRegistry::new().with_provider(Box::new(InspectingProvider {
        captured: captured.clone(),
    }));

    let mut values = HashMap::new();
    values.insert(
        "inspector".to_string(),
        json!("us.anthropic.claude-sonnet-4-20250514-v1:0"),
    );

    let _ = registry.reconcile_all(&dummy_sdk(), &values).await;

    let seen = captured.lock().unwrap().clone();
    assert_eq!(
        seen,
        Some(json!("us.anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[tokio::test]
async fn reconcile_all_uses_null_when_no_value_stored() {
    struct NullChecker {
        saw_null: Arc<AtomicUsize>,
    }
    impl LiveAwsValueProvider for NullChecker {
        fn key(&self) -> &'static str { "checker" }
        fn label(&self) -> &'static str { "checker" }
        fn description(&self) -> &'static str { "" }
        fn kind(&self) -> ProviderKind { ProviderKind::UserChoice }
        fn reconcile<'a>(
            &'a self,
            _sdk: &'a SdkConfig,
            current: &'a Value,
        ) -> BoxFuture<'a, Result<Reconciliation, ProviderError>> {
            if current.is_null() {
                self.saw_null.fetch_add(1, Ordering::SeqCst);
            }
            Box::pin(async { Ok(Reconciliation::InSync) })
        }
    }
    let saw_null = Arc::new(AtomicUsize::new(0));
    let registry = LiveAwsValueRegistry::new().with_provider(Box::new(NullChecker {
        saw_null: saw_null.clone(),
    }));

    let _ = registry.reconcile_all(&dummy_sdk(), &HashMap::new()).await;
    assert_eq!(saw_null.load(Ordering::SeqCst), 1);
}

#[test]
fn apply_user_choice_writes_to_values_map() {
    let registry = LiveAwsValueRegistry::new()
        .with_provider(Box::new(FixedProvider::in_sync("bedrock.chat_model")));
    let mut values = HashMap::new();

    registry
        .apply_user_choice(
            "bedrock.chat_model",
            json!("us.anthropic.claude-sonnet-4-20250514-v1:0"),
            &mut values,
        )
        .unwrap();

    assert_eq!(
        values.get("bedrock.chat_model"),
        Some(&json!("us.anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[test]
fn apply_user_choice_rejects_unknown_key() {
    let registry = LiveAwsValueRegistry::new();
    let mut values = HashMap::new();
    let result = registry.apply_user_choice("not.a.key", json!("x"), &mut values);
    assert!(matches!(result, Err(ProviderError::UnknownKey { .. })));
}

#[test]
fn apply_user_choice_rejects_provisioner_contributor_key() {
    let registry = LiveAwsValueRegistry::new().with_provider(Box::new(
        FixedProvider::contribution("marketplace", vec![]),
    ));
    let mut values = HashMap::new();
    let result = registry.apply_user_choice("marketplace", json!("x"), &mut values);
    assert!(matches!(result, Err(ProviderError::InvalidApply { .. })));
}
