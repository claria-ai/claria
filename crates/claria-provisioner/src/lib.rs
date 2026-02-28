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

// Keep old modules alive temporarily — they're still imported by claria-desktop commands
// until Phase 4 swaps to the new API. Cleanup in Phase 6.
pub mod check_baa;
pub mod drift;
pub mod resource;
pub mod resources;
pub mod scan;
pub mod sync;

pub use crate::account_setup::{
    assess_credentials, assume_role, bootstrap_account, build_role_arn, delete_user_access_key,
    list_user_access_keys, AccessKeyInfo, AssumeRoleResult, BootstrapResult, BootstrapStep,
    CallerIdentity, CredentialAssessment, CredentialClass, NewCredentials, StepStatus,
};
pub use crate::addr::ResourceAddr;
pub use crate::check_baa::{check_baa, BaaStatus};
pub use crate::error::ProvisionerError;
pub use crate::manifest::{FieldDrift, Lifecycle, Manifest, ResourceSpec, Severity};
pub use crate::orchestrate::{destroy_all, execute, plan};
pub use crate::persistence::StatePersistence;
pub use crate::plan::{Action, Cause, PlanEntry};
pub use crate::state::ProvisionerState;
pub use crate::syncer::ResourceSyncer;

// Re-export old types for backward compat until Phase 4
pub use crate::drift::{build_plan, OldPlan as Plan, OldPlanEntry as OldPlanEntry};
pub use crate::resource::Resource;
pub use crate::scan::{scan, ScanResult, ScanStatus};
pub use crate::sync::execute_plan;

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

// ── Resource + persistence factories (old API, kept for backward compat) ─────

/// Specific Bedrock model ID prefixes we check for availability.
const DEFAULT_MODEL_IDS: &[&str] = &[
    "anthropic.claude-sonnet-4",
    "anthropic.claude-opus-4",
];

/// Construct all managed [`Resource`] impls from an SDK config and system name.
pub fn build_resources(
    config: &aws_config::SdkConfig,
    system_name: &str,
    account_id: &str,
) -> Vec<Box<dyn Resource>> {
    let region = config
        .region()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());

    let bucket_name = format!("{account_id}-{system_name}-data");
    let trail_name = format!("{system_name}-trail");

    let s3_client = aws_sdk_s3::Client::new(config);
    let cloudtrail_client = aws_sdk_cloudtrail::Client::new(config);
    let bedrock_client = aws_sdk_bedrock::Client::new(config);
    let iam_client = aws_sdk_iam::Client::new(config);

    let model_ids: Vec<String> = DEFAULT_MODEL_IDS.iter().map(|s| (*s).to_string()).collect();

    vec![
        Box::new(resources::s3_bucket::S3BucketResource::new(
            s3_client,
            bucket_name.clone(),
            region,
            account_id.to_string(),
        )),
        Box::new(resources::cloudtrail::CloudTrailResource::new(
            cloudtrail_client,
            trail_name,
            bucket_name,
        )),
        Box::new(resources::bedrock_access::BedrockAccessResource::new(
            bedrock_client,
            model_ids,
        )),
        Box::new(resources::iam_user::IamUserResource::new(iam_client)),
    ]
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

/// Full provisioning: scan → plan → execute (old API).
pub async fn provision(
    persistence: &StatePersistence,
    resources: &[Box<dyn Resource>],
) -> Result<(), ProvisionerError> {
    let mut state = persistence.load().await?;
    let scan_results = scan::scan(resources).await;
    let plan = drift::build_plan(&state, &scan_results, resources);

    if plan.has_changes() {
        tracing::info!(
            creates = plan.create.len(),
            modifies = plan.modify.len(),
            deletes = plan.delete.len(),
            "executing provisioning plan"
        );
        sync::execute_plan(&plan, resources, &mut state, persistence).await?;
    } else {
        tracing::info!("all resources in sync, no changes needed");
    }

    Ok(())
}

/// Destroy all managed resources in reverse order (old API).
pub async fn destroy(
    persistence: &StatePersistence,
    resources: &[Box<dyn Resource>],
) -> Result<(), ProvisionerError> {
    let mut state = persistence.load().await?;

    for resource in resources.iter().rev() {
        let resource_type = resource.resource_type();
        // Find state entry matching this resource type
        let rs = state
            .resources
            .iter()
            .find(|(addr, _)| addr.resource_type == resource_type)
            .map(|(_, rs)| rs.clone());
        if let Some(rs) = rs {
            tracing::info!(resource_type = %resource_type, "destroying resource");
            resource.delete(&rs.resource_id).await?;
        }
    }

    state.resources.clear();
    persistence.flush(&state).await?;

    Ok(())
}
