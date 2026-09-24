//! claria-provisioner
//!
//! IaC engine for provisioning and hardening Claria's AWS infrastructure.
//! Library consumed by the Tauri desktop app.
//!
//! Public API:
//! - `assess_credentials()` — classify credentials as root / admin / scoped / insufficient
//! - `bootstrap_account()` — create least-privilege IAM user from root or admin credentials
//! - `build_manifest()` — construct the resource manifest from config
//! - `build_syncers()` — construct all ResourceSyncer impls from an SdkConfig and manifest
//! - `build_orphan_syncers()` — construct syncers for resources state records but the manifest dropped
//! - `build_persistence()` — construct StatePersistence from an SdkConfig and a caller-provided state directory
//! - `plan()` — scan all resources and produce an annotated plan
//! - `reconcile_state()` — record what a whole-manifest scan found, so state can rebuild itself
//! - `execute()` — apply a plan, flushing state after each action
//! - `destroy_all()` — tear down all managed resources

use std::collections::HashSet;

pub mod account_setup;
pub mod addr;
pub mod error;
pub mod manifest;
pub mod orchestrate;
pub mod persistence;
pub mod plan;
pub mod state;
pub mod syncer;
pub mod syncers;

pub use crate::{
    account_setup::{
        AccessKeyInfo, AssumeRoleResult, BootstrapResult, BootstrapStep, CallerIdentity,
        CredentialAssessment, CredentialClass, MAX_ACCESS_KEYS_PER_USER, NewCredentials,
        StepStatus, assess_credentials, assume_role, bootstrap_account, build_role_arn,
        create_access_key, delete_user_access_key, get_caller_identity, list_user_access_keys,
        update_iam_policy, validate_new_credentials, validate_teardown_credentials,
    },
    addr::ResourceAddr,
    error::ProvisionerError,
    manifest::{CredentialScope, FieldDrift, Lifecycle, Manifest, ResourceSpec, Severity},
    orchestrate::{
        build_plan_entry, destroy_all, execute, find_orphans, log_scan_summary, plan,
        reconcile_state, record_applied,
    },
    persistence::StatePersistence,
    plan::{Action, Cause, PlanEntry},
    state::ProvisionerState,
    syncer::ResourceSyncer,
};

/// Construct the resource manifest from runtime config.
pub fn build_manifest(account_id: &str, system_name: &str, region: &str) -> Manifest {
    Manifest::claria(account_id, system_name, region)
}

/// Every IAM action the manifest's resources need, aggregated for the policy diff.
fn required_actions(manifest: &Manifest) -> HashSet<String> {
    manifest
        .specs
        .iter()
        .flat_map(|s| s.iam_actions.iter().cloned())
        .collect()
}

/// The AWS clients one pass of syncer construction shares.
struct SyncerClients<'a> {
    config: &'a aws_config::SdkConfig,
    s3: aws_sdk_s3::Client,
    iam: aws_sdk_iam::Client,
    cloudtrail: aws_sdk_cloudtrail::Client,
    bedrock: aws_sdk_bedrock::Client,
}

impl<'a> SyncerClients<'a> {
    fn new(config: &'a aws_config::SdkConfig) -> Self {
        Self {
            config,
            s3: claria_storage::client::from_config(config),
            iam: aws_sdk_iam::Client::new(config),
            cloudtrail: aws_sdk_cloudtrail::Client::new(config),
            bedrock: aws_sdk_bedrock::Client::new(config),
        }
    }
}

/// Construct the one syncer that manages `spec`, or `None` when this build has
/// no implementation for its resource type.
///
/// The single place resource types are mapped to implementations, so a spec from
/// the manifest and a spec reconstructed from state get the same syncer.
fn syncer_for(
    spec: &ResourceSpec,
    clients: &SyncerClients<'_>,
    required_actions: &HashSet<String>,
    system_name: &str,
    account_id: &str,
) -> Option<Box<dyn ResourceSyncer>> {
    let syncer: Box<dyn ResourceSyncer> = match spec.resource_type.as_str() {
        "iam_user" => Box::new(syncers::iam_user::IamUserSyncer::new(
            spec.clone(),
            clients.iam.clone(),
        )),
        "iam_user_policy" => Box::new(syncers::iam_user_policy::IamUserPolicySyncer::new(
            spec.clone(),
            clients.iam.clone(),
            required_actions.clone(),
            system_name.to_string(),
            account_id.to_string(),
        )),
        "baa_agreement" => Box::new(syncers::baa_agreement::BaaAgreementSyncer::new(
            spec.clone(),
            clients.config,
        )),
        "s3_bucket" => Box::new(syncers::s3_bucket::S3BucketSyncer::new(
            spec.clone(),
            clients.s3.clone(),
        )),
        "s3_bucket_versioning" => Box::new(
            syncers::s3_bucket_versioning::S3BucketVersioningSyncer::new(
                spec.clone(),
                clients.s3.clone(),
            ),
        ),
        "s3_bucket_encryption" => Box::new(
            syncers::s3_bucket_encryption::S3BucketEncryptionSyncer::new(
                spec.clone(),
                clients.s3.clone(),
            ),
        ),
        "s3_bucket_public_access_block" => Box::new(
            syncers::s3_bucket_public_access_block::S3BucketPublicAccessBlockSyncer::new(
                spec.clone(),
                clients.s3.clone(),
            ),
        ),
        "s3_bucket_policy" => Box::new(syncers::s3_bucket_policy::S3BucketPolicySyncer::new(
            spec.clone(),
            clients.s3.clone(),
        )),
        "cloudtrail_trail" => Box::new(syncers::cloudtrail_trail::CloudTrailTrailSyncer::new(
            spec.clone(),
            clients.cloudtrail.clone(),
        )),
        "cloudtrail_trail_logging" => Box::new(
            syncers::cloudtrail_trail_logging::CloudTrailTrailLoggingSyncer::new(
                spec.clone(),
                clients.cloudtrail.clone(),
            ),
        ),
        "bedrock_model_agreement" => Box::new(
            syncers::bedrock_model_agreement::BedrockModelAgreementSyncer::new(
                spec.clone(),
                clients.bedrock.clone(),
            ),
        ),
        "transcribe_access" => Box::new(syncers::transcribe_access::TranscribeAccessSyncer::new(
            spec.clone(),
        )),
        "cost_explorer_access" => {
            Box::new(syncers::cost_explorer_access::CostExplorerAccessSyncer::new(spec.clone()))
        }
        _ => return None,
    };
    Some(syncer)
}

/// Construct all [`ResourceSyncer`] impls from an SDK config and manifest.
///
/// The returned vec is ordered: elevated resources first, then regular resources
/// in dependency order. AWS clients are shared across syncers that use the same service.
///
/// `scope_filter` narrows to only one credential scope (or `None` for all).
pub fn build_syncers(
    config: &aws_config::SdkConfig,
    manifest: &Manifest,
    scope_filter: Option<CredentialScope>,
) -> Vec<Box<dyn ResourceSyncer>> {
    let required_actions = required_actions(manifest);
    let clients = SyncerClients::new(config);

    manifest
        .specs
        .iter()
        .filter(|spec| scope_filter.is_none_or(|scope| spec.credential_scope == scope))
        .map(|spec| {
            syncer_for(
                spec,
                &clients,
                &required_actions,
                &manifest.system_name,
                &manifest.account_id,
            )
            .unwrap_or_else(|| panic!("unknown resource type in manifest: {}", spec.resource_type))
        })
        .collect()
}

/// Construct syncers for the resources state records but the manifest no longer
/// declares, so [`execute`] and [`destroy_all`] can actually tear them down.
///
/// Without these, an orphan's address is absent from the syncer map the delete
/// pass looks in, and the only thing that happens to it is that Claria forgets
/// it exists.
///
/// A resource type this build does not know — state written by a newer Claria —
/// yields nothing. It stays in the plan as an orphan and keeps its state record
/// rather than being silently dropped.
pub fn build_orphan_syncers(
    config: &aws_config::SdkConfig,
    manifest: &Manifest,
    state: &ProvisionerState,
) -> Vec<Box<dyn ResourceSyncer>> {
    let manifest_addrs: HashSet<_> = manifest.specs.iter().map(|s| s.addr()).collect();
    let required_actions = required_actions(manifest);
    let clients = SyncerClients::new(config);

    state
        .resources
        .keys()
        .filter(|addr| !manifest_addrs.contains(addr))
        .filter_map(|addr| {
            let spec = ResourceSpec::orphaned(addr);
            let syncer = syncer_for(
                &spec,
                &clients,
                &required_actions,
                &manifest.system_name,
                &manifest.account_id,
            );
            if syncer.is_none() {
                tracing::warn!(
                    addr = %addr,
                    "orphaned resource has no syncer in this build and cannot be destroyed here"
                );
            }
            syncer
        })
        .collect()
}

/// Construct a [`StatePersistence`] from an SDK config, system name, and the
/// caller-provided local state directory.
///
/// The desktop app owns every local path decision — this crate never derives
/// one. `local_state_dir` is the directory the safety-net copy of the state
/// lives in; the file name within it is this crate's concern.
pub fn build_persistence(
    config: &aws_config::SdkConfig,
    system_name: &str,
    account_id: &str,
    local_state_dir: &std::path::Path,
) -> Result<StatePersistence, ProvisionerError> {
    let s3_client = claria_storage::client::from_config(config);
    let bucket = claria_core::s3_keys::bucket_name(account_id, system_name);
    let s3_key = claria_core::s3_keys::PROVISIONER_STATE.to_string();

    let local_path = local_state_dir.join("provisioner-state.json");

    Ok(StatePersistence {
        s3: s3_client,
        bucket,
        s3_key,
        local_path,
    })
}
