//! The preconditions Claria reports are ones it actually checked.
//!
//! Four resources could not report drift. Two — Transcribe and Cost Explorer —
//! made no AWS call at all and answered `enabled: true` unconditionally, so a
//! clinician with neither permission saw both reported as satisfied. The other
//! two made a call and then threw the answer away: `iam_user` compared the
//! desired state against itself, and `s3_bucket` read its "observed" region
//! back out of the spec that said what the region should be.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::{scenarios, testing::MockServer};
use claria_provisioner::{
    Action, Cause, Manifest, PlanEntry, orchestrate, syncers::cost_explorer_access,
};

/// The account the `fully-provisioned` scenario builds.
const ACCT: &str = "185735714230";
const SYS: &str = "claria";
const REGION: &str = "us-east-1";
const BUCKET: &str = "185735714230-claria-data";

fn build_sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let creds = Credentials::new(
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        None,
        None,
        "claria-test",
    );
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new(REGION))
        .credentials_provider(SharedCredentialsProvider::new(creds))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn provisioned(denials: &[(&str, &str)]) -> MockServer {
    let server = MockServer::spawn().await;
    {
        let mut st = server.state.write().await;
        scenarios::load("fully-provisioned", &mut st).expect("load scenario");
        for (operation, code) in denials {
            st.operation_failures
                .insert((*operation).to_string(), (*code).to_string());
        }
    }
    server
}

async fn scan(sdk: &aws_config::SdkConfig, manifest: &Manifest) -> Vec<PlanEntry> {
    let syncers = claria_provisioner::build_syncers(sdk, manifest, None);
    orchestrate::plan(&syncers, None).await.expect("scan")
}

fn entry<'a>(entries: &'a [PlanEntry], resource_type: &str) -> &'a PlanEntry {
    entries
        .iter()
        .find(|e| e.spec.resource_type == resource_type)
        .unwrap_or_else(|| panic!("no {resource_type} entry in the plan"))
}

// ── Transcribe ───────────────────────────────────────────────────────────

/// The probe asks about a job that cannot exist. AWS answering at all is the
/// proof, and it bills nothing for the answer.
#[tokio::test]
async fn transcribe_reads_a_job_that_is_not_there_as_access() {
    let server = provisioned(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;
    let transcribe = entry(&entries, "transcribe_access");

    assert_eq!(transcribe.action, Action::Ok);
    assert_eq!(transcribe.cause, Cause::InSync);
    assert_eq!(
        transcribe.actual.as_ref().and_then(|a| a.get("access")),
        Some(&serde_json::json!("granted"))
    );
}

/// The failure the old syncer could not represent: no permission, and the
/// precondition reported as satisfied anyway.
#[tokio::test]
async fn transcribe_without_the_permission_is_not_reported_as_satisfied() {
    let server = provisioned(&[("GetTranscriptionJob", "AccessDeniedException")]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;
    let transcribe = entry(&entries, "transcribe_access");

    assert_eq!(transcribe.action, Action::Unknown);
    assert_eq!(transcribe.cause, Cause::Unreadable);
    assert!(
        transcribe
            .error
            .as_deref()
            .is_some_and(|e| e.contains("transcribe:GetTranscriptionJob")),
        "the reason names the refused call: {:?}",
        transcribe.error
    );
}

/// It really does call AWS — the whole point, given the syncer it replaces
/// held no client at all.
#[tokio::test]
async fn the_transcribe_precondition_makes_a_real_call() {
    let server = provisioned(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    scan(&sdk, &manifest).await;

    let operations: Vec<String> = server
        .state
        .read()
        .await
        .transcribe_requests
        .iter()
        .map(|r| r.operation.clone())
        .collect();
    assert!(
        operations.iter().any(|o| o == "GetTranscriptionJob"),
        "expected a GetTranscriptionJob probe, saw {operations:?}"
    );
}

// ── Cost Explorer ────────────────────────────────────────────────────────

/// `ce:GetCostAndUsage` is billed per request and the Provision page scans on
/// mount, so an operator who does not use Cost Explorer must not be charged to
/// verify it. Zero requests, and the report says what it is.
#[tokio::test]
async fn cost_explorer_is_not_called_unless_the_operator_uses_it() {
    let server = provisioned(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest =
        claria_provisioner::build_manifest(ACCT, SYS, REGION).with_cost_explorer_probe(false);

    let entries = scan(&sdk, &manifest).await;
    let cost = entry(&entries, "cost_explorer_access");

    assert_eq!(cost.action, Action::Ok);
    assert_eq!(
        cost.actual.as_ref().and_then(|a| a.get("access")),
        Some(&serde_json::json!("granted_by_policy")),
        "an unprobed precondition says so rather than claiming to be verified"
    );
    assert_eq!(
        server.state.read().await.cost_explorer_requests,
        0,
        "not one billable request may be made on an operator who has not opted in"
    );
}

#[tokio::test]
async fn cost_explorer_is_probed_once_when_the_operator_uses_it() {
    let server = provisioned(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest =
        claria_provisioner::build_manifest(ACCT, SYS, REGION).with_cost_explorer_probe(true);

    let entries = scan(&sdk, &manifest).await;
    let cost = entry(&entries, "cost_explorer_access");

    assert_eq!(cost.action, Action::Ok);
    assert_eq!(
        cost.actual.as_ref().and_then(|a| a.get("access")),
        Some(&serde_json::json!("verified"))
    );
    assert_eq!(
        server.state.read().await.cost_explorer_requests,
        1,
        "one request per scan, no more — each one costs the clinician a cent"
    );
}

#[tokio::test]
async fn a_refused_cost_explorer_probe_is_not_reported_as_access() {
    let server = provisioned(&[("GetCostAndUsage", "AccessDeniedException")]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest =
        claria_provisioner::build_manifest(ACCT, SYS, REGION).with_cost_explorer_probe(true);

    let entries = scan(&sdk, &manifest).await;
    let cost = entry(&entries, "cost_explorer_access");

    assert_eq!(cost.action, Action::Unknown);
    assert_eq!(cost.cause, Cause::Unreadable);
}

/// The precondition is read-only whichever way it is configured — it is a
/// thing Claria checks, never a thing it makes.
#[tokio::test]
async fn the_cost_explorer_precondition_is_never_created_or_destroyed() {
    let manifest =
        claria_provisioner::build_manifest(ACCT, SYS, REGION).with_cost_explorer_probe(true);
    let spec = manifest
        .specs
        .iter()
        .find(|s| s.resource_type == "cost_explorer_access")
        .expect("the manifest declares it")
        .clone();

    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let syncer = cost_explorer_access::CostExplorerAccessSyncer::new(spec, &sdk);

    use claria_provisioner::ResourceSyncer;
    assert!(syncer.create().await.is_err());
    assert!(syncer.update().await.is_err());
    assert!(syncer.destroy().await.is_err());
}

// ── The two that made a call and discarded the answer ─────────────────────

/// `current_state` used to return the desired state, so this comparison could
/// not fail however wrong the account was.
#[tokio::test]
async fn an_iam_user_in_another_account_is_drift() {
    let server = provisioned(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);

    // Same system, a different account than the one the mock serves.
    let manifest = claria_provisioner::build_manifest("999999999999", SYS, REGION);
    let entries = scan(&sdk, &manifest).await;
    let user = entry(&entries, "iam_user");

    assert_eq!(user.action, Action::Modify);
    assert_eq!(user.cause, Cause::Drift);
    assert!(
        user.drift.iter().any(|d| d.field == "user_arn"),
        "the drift names the ARN that did not match: {:?}",
        user.drift
    );
}

#[tokio::test]
async fn the_expected_iam_user_is_in_sync() {
    let server = provisioned(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;
    let user = entry(&entries, "iam_user");

    assert_eq!(user.action, Action::Ok, "drift: {:?}", user.drift);
}

/// The bucket's region came from the spec that said what it should be, so a
/// bucket in the wrong region compared equal to the right one. It now comes
/// from `x-amz-bucket-region` on the head Claria already makes.
#[tokio::test]
async fn a_bucket_in_the_wrong_region_is_drift() {
    let server = provisioned(&[]).await;
    {
        let mut st = server.state.write().await;
        let bucket = st
            .buckets
            .get_mut(BUCKET)
            .expect("the scenario created the bucket");
        bucket.region = "eu-west-1".to_string();
    }
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;
    let bucket = entry(&entries, "s3_bucket");

    assert_eq!(bucket.action, Action::Modify);
    assert_eq!(bucket.cause, Cause::Drift);
    assert_eq!(
        bucket.actual.as_ref().and_then(|a| a.get("region")),
        Some(&serde_json::json!("eu-west-1")),
        "the reported region is the one AWS gave, not the one that was wanted"
    );
}

#[tokio::test]
async fn a_bucket_in_the_right_region_is_in_sync() {
    let server = provisioned(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;
    let bucket = entry(&entries, "s3_bucket");

    assert_eq!(bucket.action, Action::Ok, "drift: {:?}", bucket.drift);
}
