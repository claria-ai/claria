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
//! - `build_persistence()` — construct StatePersistence from an SdkConfig
//! - `plan()` — scan all resources and produce an annotated plan
//! - `execute()` — apply a plan, flushing state after each action
//! - `destroy_all()` — tear down all managed resources

use std::collections::HashSet;
use std::path::PathBuf;

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

pub use crate::account_setup::{
    assess_credentials, assume_role, bootstrap_account, build_role_arn, delete_user_access_key,
    list_user_access_keys, update_iam_policy, AccessKeyInfo, AssumeRoleResult, BootstrapResult,
    BootstrapStep, CallerIdentity, CredentialAssessment, CredentialClass, NewCredentials,
    StepStatus,
};
pub use crate::addr::ResourceAddr;
pub use crate::error::ProvisionerError;
pub use crate::manifest::{FieldDrift, Lifecycle, Manifest, ResourceSpec, Severity};
pub use crate::orchestrate::{destroy_all, execute, plan};
pub use crate::persistence::StatePersistence;
pub use crate::plan::{Action, Cause, PlanEntry};
pub use crate::state::ProvisionerState;
pub use crate::syncer::ResourceSyncer;

/// Construct the resource manifest from runtime config.
pub fn build_manifest(account_id: &str, system_name: &str, region: &str) -> Manifest {
    Manifest::claria(account_id, system_name, region)
}

/// Construct all [`ResourceSyncer`] impls from an SDK config and manifest.
///
/// The returned vec is ordered: data sources first, then managed resources in
/// dependency order. AWS clients are shared across syncers that use the same service.
pub fn build_syncers(
    config: &aws_config::SdkConfig,
    manifest: &Manifest,
) -> Vec<Box<dyn ResourceSyncer>> {
    let required_actions: HashSet<String> = manifest
        .specs
        .iter()
        .flat_map(|s| s.iam_actions.iter().cloned())
        .collect();

    let s3 = aws_sdk_s3::Client::new(config);
    let iam = aws_sdk_iam::Client::new(config);
    let cloudtrail = aws_sdk_cloudtrail::Client::new(config);
    let bedrock = aws_sdk_bedrock::Client::new(config);

    manifest
        .specs
        .iter()
        .map(|spec| -> Box<dyn ResourceSyncer> {
            match spec.resource_type.as_str() {
                "iam_user" => Box::new(syncers::iam_user::IamUserSyncer::new(
                    spec.clone(),
                    iam.clone(),
                )),
                "iam_user_policy" => Box::new(syncers::iam_user_policy::IamUserPolicySyncer::new(
                    spec.clone(),
                    iam.clone(),
                    required_actions.clone(),
                )),
                "baa_agreement" => Box::new(syncers::baa_agreement::BaaAgreementSyncer::new(
                    spec.clone(),
                    config,
                )),
                "s3_bucket" => Box::new(syncers::s3_bucket::S3BucketSyncer::new(
                    spec.clone(),
                    s3.clone(),
                )),
                "s3_bucket_versioning" => {
                    Box::new(syncers::s3_bucket_versioning::S3BucketVersioningSyncer::new(
                        spec.clone(),
                        s3.clone(),
                    ))
                }
                "s3_bucket_encryption" => {
                    Box::new(syncers::s3_bucket_encryption::S3BucketEncryptionSyncer::new(
                        spec.clone(),
                        s3.clone(),
                    ))
                }
                "s3_bucket_public_access_block" => Box::new(
                    syncers::s3_bucket_public_access_block::S3BucketPublicAccessBlockSyncer::new(
                        spec.clone(),
                        s3.clone(),
                    ),
                ),
                "s3_bucket_policy" => Box::new(syncers::s3_bucket_policy::S3BucketPolicySyncer::new(
                    spec.clone(),
                    s3.clone(),
                )),
                "cloudtrail_trail" => {
                    Box::new(syncers::cloudtrail_trail::CloudTrailTrailSyncer::new(
                        spec.clone(),
                        cloudtrail.clone(),
                    ))
                }
                "cloudtrail_trail_logging" => Box::new(
                    syncers::cloudtrail_trail_logging::CloudTrailTrailLoggingSyncer::new(
                        spec.clone(),
                        cloudtrail.clone(),
                    ),
                ),
                "bedrock_model_agreement" => Box::new(
                    syncers::bedrock_model_agreement::BedrockModelAgreementSyncer::new(
                        spec.clone(),
                        bedrock.clone(),
                    ),
                ),
                other => panic!("unknown resource type in manifest: {other}"),
            }
        })
        .collect()
}

/// Construct a [`StatePersistence`] from an SDK config and system name.
pub fn build_persistence(
    config: &aws_config::SdkConfig,
    system_name: &str,
    account_id: &str,
) -> Result<StatePersistence, ProvisionerError> {
    let s3_client = aws_sdk_s3::Client::new(config);
    let bucket = format!("{account_id}-{system_name}-data");
    let s3_key = "_state/provisioner.json".to_string();

    let local_dir = dirs::config_dir()
        .ok_or_else(|| ProvisionerError::State("no OS config directory found".into()))?
        .join("com.claria.desktop")
        .join(system_name);

    let local_path = local_dir.join("provisioner-state.json");

    Ok(StatePersistence {
        s3: s3_client,
        bucket,
        s3_key,
        local_path,
    })
}

/// Resolve the local state directory for a given system name.
pub fn local_state_dir(system_name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("com.claria.desktop").join(system_name))
}
