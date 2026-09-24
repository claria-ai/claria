//! A read Claria could not make is reported as unreadable, never as absent.
//!
//! Every syncer used to turn a refused read into `Ok(None)` or into a
//! fabricated non-conformant value, because a read error ended the whole scan.
//! The plan then told the operator that a bucket holding PHI needed creating,
//! or that encryption they could not see was missing. These tests drive the
//! real syncers — not the `MockSyncer` `plan_test.rs` uses — against a mock
//! that refuses individual operations, which is the only way to reach those
//! arms at all.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::{scenarios, testing::MockServer};
use claria_provisioner::{
    Action, Cause, CredentialScope, Manifest, PlanEntry, ProvisionerState, ResourceAddr,
    ResourceSyncer, StatePersistence, orchestrate,
    state::{ResourceState, ResourceStatus},
};
use serde_json::json;

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

/// A mock serving a fully provisioned account, with `operations` refused.
///
/// The scenario matters: every resource really is there and conformant, so an
/// entry that comes back as anything other than `Ok` can only have come from
/// the refusal.
async fn provisioned_with_denials(operations: &[(&str, &str)]) -> MockServer {
    let server = MockServer::spawn().await;
    {
        let mut st = server.state.write().await;
        scenarios::load("fully-provisioned", &mut st).expect("load scenario");
        for (operation, code) in operations {
            st.operation_failures
                .insert((*operation).to_string(), (*code).to_string());
        }
    }
    server
}

async fn scan(sdk: &aws_config::SdkConfig, manifest: &Manifest) -> Vec<PlanEntry> {
    let syncers = claria_provisioner::build_syncers(sdk, manifest, None);
    orchestrate::plan(&syncers, None)
        .await
        .expect("a refused read must not end the scan")
}

fn entry<'a>(entries: &'a [PlanEntry], resource_type: &str) -> &'a PlanEntry {
    entries
        .iter()
        .find(|e| e.spec.resource_type == resource_type)
        .unwrap_or_else(|| panic!("no {resource_type} entry in the plan"))
}

/// Every syncer in the table: deny the read it makes, and assert the plan says
/// "unknown" rather than inventing an absence or a drift.
///
/// One test per syncer would be nine copies of four lines. The interesting
/// axis is the syncer, so it is the table's first column.
#[tokio::test]
async fn a_refused_read_is_never_reported_as_state() {
    // (denied operation, error code as that service really reports it,
    //  the resource type whose entry must go unknown)
    let cases = [
        ("HeadBucket", "AccessDenied", "s3_bucket"),
        (
            "GetBucketVersioning",
            "AccessDenied",
            "s3_bucket_versioning",
        ),
        (
            "GetBucketEncryption",
            "AccessDenied",
            "s3_bucket_encryption",
        ),
        ("GetBucketPolicy", "AccessDenied", "s3_bucket_policy"),
        (
            "GetPublicAccessBlock",
            "AccessDenied",
            "s3_bucket_public_access_block",
        ),
        ("GetTrail", "AccessDeniedException", "cloudtrail_trail"),
        (
            "GetTrailStatus",
            "AccessDeniedException",
            "cloudtrail_trail_logging",
        ),
        ("GetUser", "AccessDeniedException", "iam_user"),
        (
            "ListAttachedUserPolicies",
            "AccessDeniedException",
            "iam_user_policy",
        ),
    ];

    for (operation, code, resource_type) in cases {
        let server = provisioned_with_denials(&[(operation, code)]).await;
        let sdk = build_sdk_config(&server.endpoint);
        let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

        let entries = scan(&sdk, &manifest).await;
        let found = entry(&entries, resource_type);

        assert_eq!(
            found.action,
            Action::Unknown,
            "{operation} was refused, so {resource_type} must be unknown, not {:?}",
            found.action
        );
        assert_eq!(found.cause, Cause::Unreadable);
        assert!(
            found.actual.is_none(),
            "an unreadable resource has no observed state"
        );
        assert!(
            found.drift.is_empty(),
            "an unreadable resource cannot have drifted"
        );

        let reason = found
            .error
            .as_deref()
            .expect("an unknown entry carries the reason");
        assert!(
            reason.contains(operation),
            "the reason names the refused call: {reason}"
        );
    }
}

/// The failure that cost the most: a bucket of PHI reported as one to create.
#[tokio::test]
async fn a_denied_head_bucket_is_not_a_missing_bucket() {
    let server = provisioned_with_denials(&[("HeadBucket", "AccessDenied")]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;
    let bucket = entry(&entries, "s3_bucket");

    assert_ne!(
        bucket.action,
        Action::Create,
        "a bucket Claria cannot see is not a bucket that needs creating"
    );
    assert_eq!(bucket.action, Action::Unknown);
}

/// The reason every syncer swallowed in the first place. One refused read used
/// to end the scan with `?`, so the operator saw an error string instead of a
/// plan.
#[tokio::test]
async fn a_refused_read_does_not_cost_the_rest_of_the_scan() {
    let server = provisioned_with_denials(&[("HeadBucket", "AccessDenied")]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;

    assert_eq!(
        entries.len(),
        manifest.specs.len(),
        "every resource gets an entry, refused or not"
    );
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.action == Action::Unknown)
            .count(),
        1,
        "only the refused resource is unknown"
    );
}

/// A bucket that genuinely is not there still reads as one to create. The
/// regression guard for the whole change: failing closed is only correct if it
/// did not also break the absence it was distinguishing itself from.
#[tokio::test]
async fn a_genuinely_absent_bucket_still_reads_as_create() {
    let server = MockServer::spawn().await;
    {
        let mut st = server.state.write().await;
        scenarios::load("fresh-account", &mut st).expect("load scenario");
    }
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;
    let bucket = entry(&entries, "s3_bucket");

    assert_eq!(bucket.action, Action::Create);
    assert_eq!(bucket.cause, Cause::Missing);
    assert!(bucket.error.is_none());
}

/// A conformant account still scans clean. The denials are the new behaviour;
/// everything else must be untouched.
#[tokio::test]
async fn a_provisioned_account_that_answers_every_read_has_nothing_unknown() {
    let server = provisioned_with_denials(&[]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;

    let unknown: Vec<&str> = entries
        .iter()
        .filter(|e| e.action == Action::Unknown)
        .map(|e| e.spec.resource_type.as_str())
        .collect();
    assert!(unknown.is_empty(), "unexpectedly unknown: {unknown:?}");
}

/// The state record is what teardown works from, so an unreadable resource must
/// neither be recorded (nothing was observed) nor dropped (it may well exist).
#[tokio::test]
async fn an_unreadable_resource_neither_gains_nor_loses_its_record() {
    let server = provisioned_with_denials(&[("HeadBucket", "AccessDenied")]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let entries = scan(&sdk, &manifest).await;

    // Starting from an empty record, nothing is invented for the bucket.
    let mut fresh = ProvisionerState::new(REGION.to_string(), BUCKET.to_string());
    orchestrate::reconcile_state(&entries, &mut fresh);
    let bucket_addr = ResourceAddr {
        resource_type: "s3_bucket".into(),
        resource_name: BUCKET.into(),
    };
    assert!(
        !fresh.resources.contains_key(&bucket_addr),
        "nothing was observed, so nothing may be recorded"
    );

    // Starting from a record that already knows the bucket, it survives.
    let mut known = ProvisionerState::new(REGION.to_string(), BUCKET.to_string());
    known.resources.insert(
        bucket_addr.clone(),
        ResourceState {
            resource_type: "s3_bucket".into(),
            resource_id: BUCKET.into(),
            status: ResourceStatus::Created,
            properties: json!({}),
        },
    );
    orchestrate::reconcile_state(&entries, &mut known);
    assert!(
        known.resources.contains_key(&bucket_addr),
        "a refused read must never drop a record — that is how a bucket \
         survives a teardown that reported success"
    );
}

/// `execute` acts on create, modify and delete. An unknown entry is none of
/// them, and must stay that way: acting on a resource nobody could read is
/// acting blind.
#[tokio::test]
async fn execute_does_not_act_on_an_unreadable_resource() {
    let server = provisioned_with_denials(&[("GetBucketVersioning", "AccessDenied")]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    // Scope-filtered so the plan and the syncer list cover the same
    // addresses; `execute` refuses a plan naming a resource it was given no
    // syncer for, and that refusal is not what this test is about.
    let syncers =
        claria_provisioner::build_syncers(&sdk, &manifest, Some(CredentialScope::Regular));
    let entries = orchestrate::plan(&syncers, None)
        .await
        .expect("a refused read must not end the scan");
    assert_eq!(
        entry(&entries, "s3_bucket_versioning").action,
        Action::Unknown
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let persistence = StatePersistence {
        s3: claria_storage::client::from_config(&sdk),
        bucket: BUCKET.to_string(),
        s3_key: claria_core::s3_keys::PROVISIONER_STATE.to_string(),
        local_path: dir.path().join("provisioner-state.json"),
    };
    let mut state = ProvisionerState::new(REGION.to_string(), BUCKET.to_string());

    orchestrate::execute(&entries, &syncers, &mut state, &persistence)
        .await
        .expect("execute must not choke on an unknown entry");

    // The mock still reports the versioning it had; nothing wrote over it.
    assert!(
        server.state.read().await.buckets.contains_key(BUCKET),
        "the bucket is still there with its versioning untouched"
    );
    assert!(
        !state.resources.contains_key(&ResourceAddr {
            resource_type: "s3_bucket_versioning".into(),
            resource_name: BUCKET.into(),
        }),
        "nothing was applied, so nothing may be recorded as applied"
    );
}

/// Teardown reads directly rather than through the plan, and a refused read
/// there must never come back as "already gone".
///
/// It surfaces instead: credentials that cannot see the bucket almost
/// certainly cannot delete it either, and the one unacceptable outcome is a
/// teardown that reports success over a bucket of PHI still standing.
#[tokio::test]
async fn teardown_refuses_rather_than_reporting_success_it_cannot_confirm() {
    let server = provisioned_with_denials(&[("HeadBucket", "AccessDenied")]).await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);

    let dir = tempfile::tempdir().expect("tempdir");
    let persistence = StatePersistence {
        s3: claria_storage::client::from_config(&sdk),
        bucket: BUCKET.to_string(),
        s3_key: claria_core::s3_keys::PROVISIONER_STATE.to_string(),
        local_path: dir.path().join("provisioner-state.json"),
    };

    let bucket_addr = ResourceAddr {
        resource_type: "s3_bucket".into(),
        resource_name: BUCKET.into(),
    };
    let mut state = ProvisionerState::new(REGION.to_string(), BUCKET.to_string());
    state.resources.insert(
        bucket_addr.clone(),
        ResourceState {
            resource_type: "s3_bucket".into(),
            resource_id: BUCKET.into(),
            status: ResourceStatus::Created,
            properties: json!({}),
        },
    );

    let syncers: Vec<Box<dyn ResourceSyncer>> =
        claria_provisioner::build_syncers(&sdk, &manifest, Some(CredentialScope::Regular));
    let error = orchestrate::destroy_all(&syncers, &mut state, &persistence)
        .await
        .expect_err("a teardown that cannot read the bucket must not report success");

    assert!(
        error.to_string().contains("s3:HeadBucket"),
        "the operator is told which call was refused: {error}"
    );
    assert!(
        server.state.read().await.buckets.contains_key(BUCKET),
        "the bucket is still standing, which is why the teardown had to fail"
    );
    assert!(
        state.resources.contains_key(&bucket_addr),
        "a bucket that was not destroyed keeps its record"
    );
}
