use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use claria_provisioner::{
    Action, Cause, Lifecycle, Manifest, PlanEntry, ProvisionerError, ProvisionerState,
    ResourceAddr, ResourceSpec, ResourceSyncer, orchestrate,
    state::{ResourceState, ResourceStatus},
    syncer::BoxFuture,
    syncers::s3_bucket_policy::S3BucketPolicySyncer,
};

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

/// The action names the IAM policy syncer compares against, as `desired_state`
/// builds them: sorted and deduplicated.
fn collect_required_actions(manifest: &Manifest) -> Vec<String> {
    let mut actions: Vec<String> = manifest
        .specs
        .iter()
        .flat_map(|s| s.iam_actions.iter().map(|a| a.action.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    actions.sort();
    actions
}

/// The bucket policy the real syncer would render for this spec.
///
/// Calls the syncer rather than restating it: a second rendering here would
/// drift from what Claria writes, and the test would keep passing while doing
/// so. The client is never used — `render_policy_document` reads only the
/// spec.
fn render_policy_document(spec: &ResourceSpec) -> Value {
    let config = aws_config::SdkConfig::builder()
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build();
    S3BucketPolicySyncer::new(spec.clone(), claria_storage::client::from_config(&config))
        .render_policy_document()
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
    let mut state = ProvisionerState::new(REGION.into(), BUCKET.into());
    state.resources = resources;
    state
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
    let state = ProvisionerState::new(REGION.into(), BUCKET.into());

    let syncers = mock_syncers(
        &m,
        &[
            (
                "iam_user.claria-admin",
                Some(json!({
                    "exists": true,
                    "user_arn": format!("arn:aws:iam::{ACCT}:user/claria-admin"),
                })),
            ),
            ("iam_user_policy.claria-admin-policy", None),
            ("baa_agreement.aws-baa", None),
            (
                "bedrock_model_agreement.anthropic.claude",
                Some(json!({"agreement": "pending"})),
            ),
            (
                "transcribe_access.transcribe",
                Some(json!({"access": "granted"})),
            ),
            (
                "cost_explorer_access.cost-explorer",
                Some(json!({"access": "granted_by_policy"})),
            ),
        ],
    );

    let result = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();

    // Structural comparison: missing resources → Create/Missing,
    // drifted resources → Modify/Drift, in-sync → Ok/InSync.
    assert_plan(
        &result,
        &[
            e("iam_user.claria-admin", Action::Ok, Cause::InSync),
            e(
                "iam_user_policy.claria-admin-policy",
                Action::Create,
                Cause::Missing,
            ),
            e(
                "baa_agreement.aws-baa",
                Action::PreconditionFailed,
                Cause::Missing,
            ),
            e(
                &format!("s3_bucket.{BUCKET}"),
                Action::Create,
                Cause::Missing,
            ),
            e(
                &format!("s3_bucket_versioning.{BUCKET}"),
                Action::Create,
                Cause::Missing,
            ),
            e(
                &format!("s3_bucket_encryption.{BUCKET}"),
                Action::Create,
                Cause::Missing,
            ),
            e(
                &format!("s3_bucket_public_access_block.{BUCKET}"),
                Action::Create,
                Cause::Missing,
            ),
            e(
                &format!("s3_bucket_policy.{BUCKET}"),
                Action::Create,
                Cause::Missing,
            ),
            e(
                &format!("cloudtrail_trail.{TRAIL}"),
                Action::Create,
                Cause::Missing,
            ),
            e(
                &format!("cloudtrail_trail_logging.{TRAIL}"),
                Action::Create,
                Cause::Missing,
            ),
            e(
                "bedrock_model_agreement.anthropic.claude",
                Action::Modify,
                Cause::Drift,
            ),
            e("transcribe_access.transcribe", Action::Ok, Cause::InSync),
            e(
                "cost_explorer_access.cost-explorer",
                Action::Ok,
                Cause::InSync,
            ),
        ],
    );
}

/// Happy path — everything provisioned and in sync, zero actions needed.
#[tokio::test]
async fn plan_fully_provisioned() {
    let m = manifest();
    let state = fully_provisioned_state(&m);
    let reads = all_in_sync_reads(&m);
    let syncers = mock_syncers(&m, &reads);

    let result = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();

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
    let result = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();

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

/// IAM policy is missing an action — structural drift triggers Modify.
///
/// The policy is missing `ce:GetCostAndUsage` from the current manifest.
/// No version tracking needed — the structural diff detects the gap.
#[tokio::test]
async fn plan_policy_escalation() {
    let m = manifest();
    let state = fully_provisioned_state(&m);

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
    let result = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();

    for entry in &result {
        let addr = entry.spec.addr().to_string();
        if addr == policy_addr {
            assert_eq!(entry.action, Action::Modify);
            assert_eq!(entry.cause, Cause::Drift);
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

/// Removing a formerly granted action is also policy drift, so an existing
/// installation asks for elevated credentials and actually revokes it.
#[tokio::test]
async fn plan_policy_reduction_escalation() {
    let m = manifest();
    let state = fully_provisioned_state(&m);
    let mut reads = all_in_sync_reads(&m);
    let mut old_actions = collect_required_actions(&m);
    old_actions.push("s3:DeleteObjectVersion".to_string());

    let policy_addr = "iam_user_policy.claria-admin-policy".to_string();
    for (addr, value) in &mut reads {
        if *addr == policy_addr {
            *value = Some(json!({
                "policy_attached": true,
                "policy_document": {},
                "current_actions": old_actions,
            }));
            break;
        }
    }

    let syncers = mock_syncers(&m, &reads);
    let result = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();
    let policy = result
        .iter()
        .find(|entry| entry.spec.addr().to_string() == policy_addr)
        .expect("IAM policy plan entry");

    assert_eq!(policy.action, Action::Modify);
    assert_eq!(policy.cause, Cause::Drift);
}

// ── The ownership record ──────────────────────────────────────────────
//
// State answers one question: which resources is Claria responsible for.
// Nothing reads it to decide drift. These pin the two ways it used to end up
// wrong — a scan that recorded nothing, and an update that recorded nothing
// unless the resource was already known.

/// A scan of a healthy account is the only thing that can rebuild the record.
///
/// Before this, `Action::Ok` wrote nothing, so a reset state file stayed empty
/// forever and teardown skipped every resource it no longer knew about.
#[tokio::test]
async fn reconcile_records_resources_a_scan_found() {
    let m = manifest();
    let state = ProvisionerState::new(REGION.into(), BUCKET.into());
    let reads = all_in_sync_reads(&m);
    let syncers = mock_syncers(&m, &reads);

    let entries = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();
    assert!(
        entries.iter().all(|e| e.action == Action::Ok),
        "fixture should be fully conformant"
    );

    let mut rebuilt = ProvisionerState::new(REGION.into(), BUCKET.into());
    assert!(
        orchestrate::reconcile_state(&entries, &mut rebuilt),
        "a scan of a provisioned account should have something to record"
    );

    for spec in &m.specs {
        let recorded = rebuilt.resources.contains_key(&spec.addr());
        match spec.lifecycle {
            // Preconditions Claria reads are not resources it owns.
            Lifecycle::Data => assert!(
                !recorded,
                "{} is a data source and should not be recorded",
                spec.addr()
            ),
            Lifecycle::Managed => assert!(recorded, "{} went unrecorded", spec.addr()),
        }
    }
}

/// A second scan of an unchanged account must not ask for a flush.
#[tokio::test]
async fn reconcile_is_idempotent() {
    let m = manifest();
    let state = fully_provisioned_state(&m);
    let reads = all_in_sync_reads(&m);
    let syncers = mock_syncers(&m, &reads);

    let entries = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();
    let mut again = state.clone();
    assert!(
        !orchestrate::reconcile_state(&entries, &mut again),
        "nothing changed, so nothing should be flushed"
    );
}

/// A read that could not be completed reports "absent", and several syncers do
/// exactly that on AccessDenied. Dropping the record on that basis is how a
/// bucket survives a teardown that reported success, so reconcile only adds.
#[tokio::test]
async fn reconcile_never_drops_a_record_for_an_absent_read() {
    let m = manifest();
    let state = fully_provisioned_state(&m);
    // Every read fails closed: the whole account looks missing.
    let syncers = mock_syncers::<&str>(&m, &[]);

    let entries = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();
    let mut kept = state.clone();
    assert!(!orchestrate::reconcile_state(&entries, &mut kept));
    assert_eq!(
        kept.resources.len(),
        state.resources.len(),
        "an unreadable account must not erase the ownership record"
    );
}

/// An orphan is pending deletion. Recording it as managed would put it back
/// under the manifest's care and lose the delete.
#[tokio::test]
async fn reconcile_leaves_orphans_alone() {
    let m = manifest();
    let mut state = fully_provisioned_state(&m);
    let orphan = ResourceAddr {
        resource_type: "s3_bucket".into(),
        resource_name: "123456789012-oldname-data".into(),
    };
    state.resources.insert(
        orphan.clone(),
        ResourceState {
            resource_type: orphan.resource_type.clone(),
            resource_id: orphan.resource_name.clone(),
            status: ResourceStatus::Created,
            properties: json!({}),
        },
    );

    let reads = all_in_sync_reads(&m);
    let syncers = mock_syncers(&m, &reads);
    let entries = orchestrate::plan(&syncers, Some((&m, &state)))
        .await
        .unwrap();

    let orphans: Vec<_> = entries
        .iter()
        .filter(|e| e.cause == Cause::Orphaned)
        .collect();
    assert_eq!(
        orphans.len(),
        1,
        "the stale bucket should be the one orphan"
    );
    assert_eq!(orphans[0].action, Action::Delete);
    assert_eq!(orphans[0].spec.addr(), orphan);

    let mut after = state.clone();
    assert!(!orchestrate::reconcile_state(&entries, &mut after));
}

/// Orphans are decided against the manifest, not against whichever syncers a
/// pass happened to build. A scope-filtered pass used to call every resource
/// outside its scope an orphan — which queued the data bucket for deletion.
#[test]
fn orphans_are_decided_against_the_manifest() {
    let m = manifest();
    let state = fully_provisioned_state(&m);

    assert!(
        orchestrate::find_orphans(&m, &state).is_empty(),
        "everything in state is in the manifest"
    );
}

/// An update to a resource the state file had never heard of has to land, or
/// the resource stays invisible to teardown.
#[test]
fn record_applied_upserts() {
    let m = manifest();
    let spec = m
        .specs
        .iter()
        .find(|s| s.resource_type == "s3_bucket_encryption")
        .expect("manifest has bucket encryption");
    let mut state = ProvisionerState::new(REGION.into(), BUCKET.into());

    orchestrate::record_applied(
        &mut state,
        spec,
        json!({"sse_algorithm": "AES256"}),
        ResourceStatus::Updated,
    );

    let recorded = state
        .resources
        .get(&spec.addr())
        .expect("an update on an unknown resource must still be recorded");
    assert_eq!(recorded.resource_id, spec.resource_name);
    assert_eq!(recorded.properties, json!({"sse_algorithm": "AES256"}));
}
