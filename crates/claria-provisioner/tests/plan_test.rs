use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use claria_provisioner::{
    orchestrate, Action, Cause, Manifest, PlanEntry, ProvisionerError, ProvisionerState,
    ResourceSpec, ResourceSyncer,
};
use claria_provisioner::state::{ResourceState, ResourceStatus};
use claria_provisioner::syncer::BoxFuture;

// ── Constants ─────────────────────────────────────────────────────────

const ACCT: &str = "123456789012";
const SYS: &str = "claria";
const REGION: &str = "us-east-1";
const BUCKET: &str = "123456789012-claria-data";
const TRAIL: &str = "claria-trail";

fn manifest() -> Manifest {
    Manifest::claria(ACCT, SYS, REGION)
}

// ── MockSyncer ────────────────────────────────────────────────────────

type CurrentStateFn = Box<dyn Fn(&Value) -> Value + Send + Sync>;

struct MockSyncer {
    spec: ResourceSpec,
    read_result: Option<Value>,
    desired_override: Option<Value>,
    current_fn: Option<CurrentStateFn>,
}

impl ResourceSyncer for MockSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<Value>, ProvisionerError>> {
        let result = Ok(self.read_result.clone());
        Box::pin(async move { result })
    }

    fn desired_state(&self) -> Value {
        self.desired_override
            .clone()
            .unwrap_or_else(|| self.spec.desired.clone())
    }

    fn current_state(&self, actual: &Value) -> Value {
        match &self.current_fn {
            Some(f) => f(actual),
            None => actual.clone(),
        }
    }

    fn create(&self) -> BoxFuture<'_, Result<Value, ProvisionerError>> {
        panic!("MockSyncer::create should not be called by plan()")
    }

    fn update(&self) -> BoxFuture<'_, Result<Value, ProvisionerError>> {
        panic!("MockSyncer::update should not be called by plan()")
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        panic!("MockSyncer::destroy should not be called by plan()")
    }
}

// ── Builders ──────────────────────────────────────────────────────────

fn collect_required_actions(manifest: &Manifest) -> Vec<String> {
    let mut actions: Vec<String> = manifest
        .specs
        .iter()
        .flat_map(|s| s.iam_actions.iter().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    actions.sort();
    actions
}

/// Replicate `S3BucketPolicySyncer::render_policy_document` for the mock.
fn render_policy_document(spec: &ResourceSpec) -> Value {
    let statements = spec.desired.get("statements").cloned().unwrap_or(json!([]));
    let stmts = statements.as_array().cloned().unwrap_or_default();
    let aws_stmts: Vec<Value> = stmts
        .into_iter()
        .map(|s| {
            json!({
                "Sid": s.get("sid").and_then(|v| v.as_str()).unwrap_or(""),
                "Effect": s.get("effect").and_then(|v| v.as_str()).unwrap_or("Allow"),
                "Principal": s.get("principal").map(|p| {
                    if let Some(svc) = p.get("service").and_then(|v| v.as_str()) {
                        json!({"Service": svc})
                    } else {
                        p.clone()
                    }
                }).unwrap_or(json!("*")),
                "Action": s.get("action").cloned().unwrap_or(json!("")),
                "Resource": s.get("resource").cloned().unwrap_or(json!("")),
                "Condition": s.get("condition").cloned().unwrap_or(json!({})),
            })
        })
        .collect();

    json!({
        "Version": "2012-10-17",
        "Statement": aws_stmts,
    })
}

/// Build mock syncers from manifest + sparse read map.
///
/// Keys are `"resource_type.resource_name"` (the `ResourceAddr` Display format).
/// Missing keys default to `None` (resource doesn't exist in AWS).
/// The four special syncers (`iam_user`, `iam_user_policy`, `baa_agreement`,
/// `s3_bucket_policy`) get their `desired_override` / `current_fn` wired
/// automatically.
fn mock_syncers<S: AsRef<str>>(
    manifest: &Manifest,
    reads: &[(S, Option<Value>)],
) -> Vec<Box<dyn ResourceSyncer>> {
    let reads_map: HashMap<String, Option<Value>> = reads
        .iter()
        .map(|(k, v)| (k.as_ref().to_string(), v.clone()))
        .collect();

    let required_actions = collect_required_actions(manifest);

    manifest
        .specs
        .iter()
        .map(|spec| {
            let addr = spec.addr().to_string();
            let read_result = reads_map.get(&addr).cloned().unwrap_or(None);

            let (desired_override, current_fn): (Option<Value>, Option<CurrentStateFn>) =
                match spec.resource_type.as_str() {
                "iam_user" => {
                    let desired = spec.desired.clone();
                    (None, Some(Box::new(move |_| desired.clone())))
                }
                "iam_user_policy" => {
                    let desired = json!({"actions": &required_actions});
                    (
                        Some(desired),
                        Some(Box::new(|actual: &Value| {
                            let mut actions: Vec<String> = actual
                                .get("current_actions")
                                .and_then(|a| a.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default();
                            actions.sort();
                            json!({"actions": actions})
                        })),
                    )
                }
                "baa_agreement" => (
                    None,
                    Some(Box::new(|actual: &Value| {
                        let state = actual.get("state").cloned().unwrap_or(json!("unknown"));
                        json!({"state": state})
                    })),
                ),
                "s3_bucket_policy" => {
                    let rendered = render_policy_document(spec);
                    (Some(rendered), None)
                }
                _ => (None, None),
            };

            Box::new(MockSyncer {
                spec: spec.clone(),
                read_result,
                desired_override,
                current_fn,
            }) as Box<dyn ResourceSyncer>
        })
        .collect()
}

// ── Helpers ───────────────────────────────────────────────────────────

fn fully_provisioned_state(manifest: &Manifest) -> ProvisionerState {
    let mut resources = HashMap::new();
    for spec in &manifest.specs {
        resources.insert(
            spec.addr(),
            ResourceState {
                resource_type: spec.resource_type.clone(),
                resource_id: spec.resource_name.clone(),
                status: ResourceStatus::Created,
                properties: json!({}),
            },
        );
    }
    ProvisionerState {
        resources,
        manifest_version: Some(Manifest::VERSION),
        region: REGION.into(),
        bucket: BUCKET.into(),
    }
}

fn all_in_sync_reads(manifest: &Manifest) -> Vec<(String, Option<Value>)> {
    let required_actions = collect_required_actions(manifest);

    manifest
        .specs
        .iter()
        .map(|spec| {
            let addr = spec.addr().to_string();
            let value = match spec.resource_type.as_str() {
                "iam_user" => Some(json!({
                    "exists": true,
                    "user_arn": format!("arn:aws:iam::{ACCT}:user/claria-admin"),
                })),
                "iam_user_policy" => Some(json!({
                    "policy_attached": true,
                    "policy_document": {},
                    "current_actions": &required_actions,
                })),
                "baa_agreement" => Some(json!({
                    "state": "active",
                    "agreement_name": "BAA",
                    "effective_start": "2024-01-01",
                })),
                "s3_bucket_policy" => Some(render_policy_document(spec)),
                _ => Some(spec.desired.clone()),
            };
            (addr, value)
        })
        .collect()
}

fn assert_plan(plan: &[PlanEntry], expected: &[(String, Action, Cause)]) {
    let actual: Vec<(String, Action, Cause)> = plan
        .iter()
        .map(|e| (e.spec.addr().to_string(), e.action, e.cause))
        .collect();

    assert_eq!(
        actual.len(),
        expected.len(),
        "plan length mismatch: got {}, expected {}\nactual:\n{}\nexpected:\n{}",
        actual.len(),
        expected.len(),
        format_entries(&actual),
        format_entries(expected),
    );

    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            a, e,
            "mismatch at index {i}:\n  actual:   ({}, {:?}, {:?})\n  expected: ({}, {:?}, {:?})",
            a.0, a.1, a.2, e.0, e.1, e.2,
        );
    }
}

fn format_entries(entries: &[(String, Action, Cause)]) -> String {
    entries
        .iter()
        .enumerate()
        .map(|(i, (addr, action, cause))| format!("  [{i}] {addr} => {action:?}, {cause:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn e(addr: &str, action: Action, cause: Cause) -> (String, Action, Cause) {
    (addr.to_string(), action, cause)
}

// ── Test scenarios ────────────────────────────────────────────────────

/// First onboarding — empty state, no manifest version.
///
/// IAM user exists, policy and BAA are missing, all managed resources
/// need creation, bedrock models need agreement acceptance.
#[tokio::test]
async fn plan_fresh_account() {
    let m = manifest();
    let state = ProvisionerState {
        resources: HashMap::new(),
        manifest_version: None,
        region: REGION.into(),
        bucket: BUCKET.into(),
    };

    let syncers = mock_syncers(&m, &[
        ("iam_user.claria-admin", Some(json!({
            "exists": true,
            "user_arn": format!("arn:aws:iam::{ACCT}:user/claria-admin"),
        }))),
        ("iam_user_policy.claria-admin-policy", None),
        ("baa_agreement.aws-baa", None),
        ("bedrock_model_agreement.anthropic.claude-sonnet-4", Some(json!({"agreement": "pending"}))),
        ("bedrock_model_agreement.anthropic.claude-opus-4", Some(json!({"agreement": "pending"}))),
        ("transcribe_access.transcribe", Some(json!({"enabled": true}))),
        ("cost_explorer_access.cost-explorer", Some(json!({"enabled": true}))),
    ]);

    let result = orchestrate::plan(&syncers, &state).await.unwrap();

    // manifest_version is None → manifest_upgraded = true → managed creates
    // get ManifestChanged (not FirstProvision, because no prior version exists).
    assert_plan(&result, &[
        e("iam_user.claria-admin",                             Action::Ok,                 Cause::InSync),
        e("iam_user_policy.claria-admin-policy",               Action::PreconditionFailed, Cause::Drift),
        e("baa_agreement.aws-baa",                             Action::PreconditionFailed, Cause::Drift),
        e(&format!("s3_bucket.{BUCKET}"),                      Action::Create,             Cause::ManifestChanged),
        e(&format!("s3_bucket_versioning.{BUCKET}"),           Action::Create,             Cause::ManifestChanged),
        e(&format!("s3_bucket_encryption.{BUCKET}"),           Action::Create,             Cause::ManifestChanged),
        e(&format!("s3_bucket_public_access_block.{BUCKET}"),  Action::Create,             Cause::ManifestChanged),
        e(&format!("s3_bucket_policy.{BUCKET}"),               Action::Create,             Cause::ManifestChanged),
        e(&format!("cloudtrail_trail.{TRAIL}"),                Action::Create,             Cause::ManifestChanged),
        e(&format!("cloudtrail_trail_logging.{TRAIL}"),        Action::Create,             Cause::ManifestChanged),
        e("bedrock_model_agreement.anthropic.claude-sonnet-4", Action::Modify,             Cause::ManifestChanged),
        e("bedrock_model_agreement.anthropic.claude-opus-4",   Action::Modify,             Cause::ManifestChanged),
        e("transcribe_access.transcribe",                      Action::Ok,                 Cause::InSync),
        e("cost_explorer_access.cost-explorer",                Action::Ok,                 Cause::InSync),
    ]);
}

/// Happy path — everything provisioned and in sync, zero actions needed.
#[tokio::test]
async fn plan_fully_provisioned() {
    let m = manifest();
    let state = fully_provisioned_state(&m);
    let reads = all_in_sync_reads(&m);
    let syncers = mock_syncers(&m, &reads);

    let result = orchestrate::plan(&syncers, &state).await.unwrap();

    assert_eq!(result.len(), m.specs.len());
    for entry in &result {
        assert_eq!(
            entry.action,
            Action::Ok,
            "{} expected Ok, got {:?}",
            entry.spec.addr(),
            entry.action,
        );
        assert_eq!(
            entry.cause,
            Cause::InSync,
            "{} expected InSync, got {:?}",
            entry.spec.addr(),
            entry.cause,
        );
        assert!(
            entry.drift.is_empty(),
            "{} has unexpected drift: {:?}",
            entry.spec.addr(),
            entry.drift,
        );
    }
}

/// Simple config drift — someone disabled versioning.
///
/// Only the versioning entry should show Modify/Drift with a field-level diff.
#[tokio::test]
async fn plan_versioning_drifted() {
    let m = manifest();
    let state = fully_provisioned_state(&m);

    let versioning_addr = format!("s3_bucket_versioning.{BUCKET}");
    let mut reads = all_in_sync_reads(&m);
    for (addr, value) in &mut reads {
        if *addr == versioning_addr {
            *value = Some(json!({"status": "Suspended"}));
        }
    }

    let syncers = mock_syncers(&m, &reads);
    let result = orchestrate::plan(&syncers, &state).await.unwrap();

    for entry in &result {
        let addr = entry.spec.addr().to_string();
        if addr == versioning_addr {
            assert_eq!(entry.action, Action::Modify);
            assert_eq!(entry.cause, Cause::Drift);
            assert_eq!(entry.drift.len(), 1);
            assert_eq!(entry.drift[0].field, "status");
            assert_eq!(entry.drift[0].expected, json!("Enabled"));
            assert_eq!(entry.drift[0].actual, json!("Suspended"));
        } else {
            assert_eq!(
                entry.action,
                Action::Ok,
                "{addr} expected Ok, got {:?}",
                entry.action,
            );
        }
    }
}

/// Claria upgrade added new resources — IAM policy needs new actions.
///
/// State has `manifest_version = VERSION - 1`, simulating a Claria upgrade.
/// The IAM policy is missing `ce:GetCostAndUsage` from the latest manifest.
#[tokio::test]
async fn plan_policy_escalation() {
    let m = manifest();
    let mut state = fully_provisioned_state(&m);
    state.manifest_version = Some(Manifest::VERSION - 1);

    let mut reads = all_in_sync_reads(&m);

    // Remove ce:GetCostAndUsage from current_actions to simulate old policy.
    let all_actions = collect_required_actions(&m);
    let trimmed: Vec<String> = all_actions
        .into_iter()
        .filter(|a| a != "ce:GetCostAndUsage")
        .collect();
    let policy_addr = "iam_user_policy.claria-admin-policy".to_string();
    for (addr, value) in &mut reads {
        if *addr == policy_addr {
            *value = Some(json!({
                "policy_attached": true,
                "policy_document": {},
                "current_actions": trimmed,
            }));
            break;
        }
    }

    let syncers = mock_syncers(&m, &reads);
    let result = orchestrate::plan(&syncers, &state).await.unwrap();

    for entry in &result {
        let addr = entry.spec.addr().to_string();
        if addr == policy_addr {
            assert_eq!(entry.action, Action::PreconditionFailed);
            assert_eq!(entry.cause, Cause::ManifestChanged);
        } else {
            assert_eq!(
                entry.action,
                Action::Ok,
                "{addr} expected Ok, got {:?}",
                entry.action,
            );
            assert_eq!(
                entry.cause,
                Cause::InSync,
                "{addr} expected InSync, got {:?}",
                entry.cause,
            );
        }
    }
}
