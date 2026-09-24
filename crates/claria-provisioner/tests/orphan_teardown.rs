//! Orphan deletion and teardown — the two operations the state file gates.
//!
//! Everything else in the provisioner reasons from a live read. These two ask
//! state a question only it can answer ("is this resource Claria's?"), which is
//! why a record that cannot be rebuilt turns both of them into no-ops that
//! report success.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::testing::MockServer;
use claria_provisioner::{
    Action, CredentialScope, Lifecycle, PlanEntry, ProvisionerError, ProvisionerState,
    ResourceAddr, ResourceSpec, ResourceSyncer, Severity, StatePersistence, orchestrate,
    state::{ResourceState, ResourceStatus},
    syncer::BoxFuture,
};
use serde_json::{Value, json};

const ACCT: &str = "123456789012";
const SYS: &str = "claria";
const REGION: &str = "us-east-1";
const BUCKET: &str = "123456789012-claria-data";
/// The bucket a previous `system_name` left behind: in state, not in the manifest.
const STALE_BUCKET: &str = "123456789012-oldname-data";

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

fn addr(resource_type: &str, resource_name: &str) -> ResourceAddr {
    ResourceAddr {
        resource_type: resource_type.into(),
        resource_name: resource_name.into(),
    }
}

fn recorded(addr: &ResourceAddr) -> ResourceState {
    ResourceState {
        resource_type: addr.resource_type.clone(),
        resource_id: addr.resource_name.clone(),
        status: ResourceStatus::Created,
        properties: json!({}),
    }
}

fn state_recording(addrs: &[ResourceAddr]) -> ProvisionerState {
    let mut state = ProvisionerState::new(REGION.into(), BUCKET.into());
    for a in addrs {
        state.resources.insert(a.clone(), recorded(a));
    }
    state
}

/// Persistence pointed at the mock. The S3 half of a flush is best-effort, so a
/// bucket that does not exist there costs a warning and nothing else.
fn persistence(sdk: &aws_config::SdkConfig, dir: &tempfile::TempDir) -> StatePersistence {
    claria_provisioner::build_persistence(sdk, SYS, ACCT, dir.path()).expect("build persistence")
}

async fn bucket_exists(s3: &aws_sdk_s3::Client, bucket: &str) -> bool {
    s3.head_bucket().bucket(bucket).send().await.is_ok()
}

// ── Orphan deletion, end to end against the mock S3 ───────────────────

/// A resource the manifest stopped declaring has to be torn down, not merely
/// dropped from the record.
///
/// The plan calls it `Delete` with `Severity::Destructive` and tells the
/// operator it "will be removed". Nothing built a destroyer for it, so the only
/// thing that used to happen was that Claria forgot it, leaving a bucket of PHI
/// running in the account.
#[tokio::test]
async fn an_orphan_is_destroyed_not_just_forgotten() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);
    s3.create_bucket()
        .bucket(STALE_BUCKET)
        .send()
        .await
        .expect("seed the stale bucket");

    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);
    let dir = tempfile::tempdir().expect("tempdir");
    let persistence = persistence(&sdk, &dir);

    let stale = addr("s3_bucket", STALE_BUCKET);
    let mut state = state_recording(std::slice::from_ref(&stale));

    let entries = orchestrate::find_orphans(&manifest, &state);
    assert_eq!(entries.len(), 1, "the stale bucket is the only orphan");
    assert_eq!(entries[0].action, Action::Delete);

    let syncers = claria_provisioner::build_orphan_syncers(&sdk, &manifest, &state);
    assert_eq!(
        syncers.len(),
        1,
        "an orphan needs a destroyer built from state"
    );

    orchestrate::execute(&entries, &syncers, &mut state, &persistence)
        .await
        .expect("execute the delete");

    assert!(
        !bucket_exists(&s3, STALE_BUCKET).await,
        "the orphaned bucket is still standing"
    );
    assert!(
        !state.resources.contains_key(&stale),
        "a destroyed orphan should leave the record"
    );
}

/// State written by a newer Claria can name a resource type this build has never
/// heard of. It cannot be destroyed here, so it must stay in the record —
/// dropping it would strand it in the account with nothing pointing at it.
#[tokio::test]
async fn an_orphan_this_build_cannot_destroy_keeps_its_record() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);
    let dir = tempfile::tempdir().expect("tempdir");
    let persistence = persistence(&sdk, &dir);

    let unknown = addr("sqs_queue", "claria-ingest");
    let mut state = state_recording(std::slice::from_ref(&unknown));

    let entries = orchestrate::find_orphans(&manifest, &state);
    assert_eq!(entries.len(), 1);

    let syncers = claria_provisioner::build_orphan_syncers(&sdk, &manifest, &state);
    assert!(
        syncers.is_empty(),
        "this build has no syncer for that resource type"
    );

    orchestrate::execute(&entries, &syncers, &mut state, &persistence)
        .await
        .expect("an undestroyable orphan is not a failure");

    assert!(
        state.resources.contains_key(&unknown),
        "forgetting it is how it gets stranded"
    );
}

// ── Teardown ──────────────────────────────────────────────────────────

/// A syncer that reports a fixed read and counts how often it was destroyed.
struct CountingSyncer {
    spec: ResourceSpec,
    read_result: Result<Option<Value>, ()>,
    destroys: Arc<AtomicUsize>,
}

impl CountingSyncer {
    fn new(
        lifecycle: Lifecycle,
        read_result: Result<Option<Value>, ()>,
    ) -> (Self, Arc<AtomicUsize>) {
        let destroys = Arc::new(AtomicUsize::new(0));
        let spec = ResourceSpec {
            resource_type: "s3_bucket".into(),
            resource_name: BUCKET.into(),
            lifecycle,
            desired: json!({"region": REGION}),
            credential_scope: CredentialScope::Regular,
            label: "S3 Bucket".into(),
            description: "Encrypted storage".into(),
            severity: Severity::Normal,
            iam_actions: vec![],
        };
        (
            Self {
                spec,
                read_result,
                destroys: Arc::clone(&destroys),
            },
            destroys,
        )
    }
}

impl ResourceSyncer for CountingSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<Value>, ProvisionerError>> {
        let result = match &self.read_result {
            Ok(v) => Ok(v.clone()),
            Err(()) => Err(ProvisionerError::Aws("s3:HeadBucket failed".into())),
        };
        Box::pin(async move { result })
    }

    fn create(&self) -> BoxFuture<'_, Result<Value, ProvisionerError>> {
        panic!("destroy_all must not create")
    }

    fn update(&self) -> BoxFuture<'_, Result<Value, ProvisionerError>> {
        panic!("destroy_all must not update")
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        self.destroys.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

async fn run_destroy_all(
    syncer: CountingSyncer,
    state: &mut ProvisionerState,
) -> Result<(), ProvisionerError> {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let dir = tempfile::tempdir().expect("tempdir");
    let persistence = persistence(&sdk, &dir);
    let syncers: Vec<Box<dyn ResourceSyncer>> = vec![Box::new(syncer)];
    orchestrate::destroy_all(&syncers, state, &persistence).await
}

/// The reset-state case. "Reset Provisioner State" is what the incompatible-state
/// error tells the operator to do, and a scan of a healthy account then finds
/// everything conformant — which never wrote a record. Teardown has to work from
/// the live read, or it reports success and leaves the account intact.
#[tokio::test]
async fn teardown_destroys_what_a_reset_state_file_forgot() {
    let (syncer, destroys) = CountingSyncer::new(Lifecycle::Managed, Ok(Some(json!({}))));
    let mut state = ProvisionerState::default();

    run_destroy_all(syncer, &mut state)
        .await
        .expect("teardown should succeed");

    assert_eq!(
        destroys.load(Ordering::SeqCst),
        1,
        "a resource the record never knew about still has to be destroyed"
    );
}

/// The mirror case: several syncers report a read they could not complete as
/// "absent". Teardown falls back to the record rather than walking away from a
/// bucket it simply could not see.
#[tokio::test]
async fn teardown_falls_back_to_the_record_when_a_read_fails() {
    let (syncer, destroys) = CountingSyncer::new(Lifecycle::Managed, Err(()));
    let mut state = state_recording(&[addr("s3_bucket", BUCKET)]);

    run_destroy_all(syncer, &mut state)
        .await
        .expect("teardown should succeed");

    assert_eq!(destroys.load(Ordering::SeqCst), 1);
}

/// Nothing there and nothing recorded is nothing to do.
#[tokio::test]
async fn teardown_skips_a_resource_that_is_neither_present_nor_recorded() {
    let (syncer, destroys) = CountingSyncer::new(Lifecycle::Managed, Ok(None));
    let mut state = ProvisionerState::default();

    run_destroy_all(syncer, &mut state)
        .await
        .expect("teardown should succeed");

    assert_eq!(destroys.load(Ordering::SeqCst), 0);
}

/// Data entries are preconditions Claria reads, not resources it created. Their
/// syncers refuse to destroy anything, and `transcribe_access` reports itself
/// present unconditionally — so a teardown that did not skip them would abort on
/// the first one and never reach the bucket.
#[tokio::test]
async fn teardown_never_destroys_a_precondition() {
    let (syncer, destroys) = CountingSyncer::new(Lifecycle::Data, Ok(Some(json!({}))));
    let mut state = state_recording(&[addr("s3_bucket", BUCKET)]);

    run_destroy_all(syncer, &mut state)
        .await
        .expect("teardown should succeed");

    assert_eq!(
        destroys.load(Ordering::SeqCst),
        0,
        "a data source is not Claria's to destroy"
    );
}

/// `find_orphans` reads the manifest, never whichever syncers a pass happened to
/// build. A scope-filtered pass used to compare state against its own partial
/// list, which made every resource outside that scope an orphan — so a day-2
/// escalated apply queued the data bucket for deletion.
#[test]
fn a_scope_filtered_pass_cannot_invent_orphans() {
    let manifest = claria_provisioner::build_manifest(ACCT, SYS, REGION);
    let managed: Vec<ResourceAddr> = manifest.specs.iter().map(|s| s.addr()).collect();
    let state = state_recording(&managed);

    let orphans: Vec<PlanEntry> = orchestrate::find_orphans(&manifest, &state);
    assert!(
        orphans.is_empty(),
        "every recorded resource is in the manifest, whatever scope a pass filtered to"
    );
}
