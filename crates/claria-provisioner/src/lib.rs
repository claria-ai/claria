//! claria-provisioner
//!
//! IaC engine for provisioning and hardening Claria's AWS infrastructure.
//! Library consumed by the Tauri desktop app.
//!
//! Public API:
//! - `assess_credentials()` — classify credentials as root / admin / scoped / insufficient
//! - `bootstrap_account()` — create least-privilege IAM user from root or admin credentials
//! - `build_resources()` — construct all managed Resource impls from an SdkConfig
//! - `build_persistence()` — construct StatePersistence from an SdkConfig
//! - `scan()` — query current state of all resources
//! - `build_plan()` — compare scan results against state, produce four-bucket plan
//! - `execute()` — apply a plan, flushing state after each action
//! - `provision()` — convenience: scan → plan → execute
//! - `destroy()` — tear down all managed resources

use std::path::PathBuf;

pub mod account_setup;
pub mod drift;
pub mod error;
pub mod persistence;
pub mod plan;
pub mod resource;
pub mod resources;
pub mod scan;
pub mod state;
pub mod sync;

pub use crate::account_setup::{
    assess_credentials, assume_role, bootstrap_account, build_role_arn, delete_user_access_key,
    list_user_access_keys, AccessKeyInfo, AssumeRoleResult, BootstrapResult, BootstrapStep,
    CallerIdentity, CredentialAssessment, CredentialClass, NewCredentials, StepStatus,
};
pub use crate::drift::build_plan;
pub use crate::error::ProvisionerError;
pub use crate::persistence::StatePersistence;
pub use crate::plan::{Plan, PlanEntry};
pub use crate::resource::Resource;
pub use crate::scan::{scan, ScanResult, ScanStatus};
pub use crate::state::ProvisionerState;
pub use crate::sync::execute_plan;

// ── Resource + persistence factories ─────────────────────────────────────────

/// Specific Bedrock model ID prefixes we check for availability.
///
/// We verify that both Sonnet (fast) and Opus (capable) families are enabled,
/// since Claria uses them for different tasks.
const DEFAULT_MODEL_IDS: &[&str] = &[
    "anthropic.claude-sonnet-4",
    "anthropic.claude-opus-4",
];

/// Construct all managed [`Resource`] impls from an SDK config and system name.
///
/// The returned vec is ordered: S3 first (other resources depend on the bucket),
/// then CloudTrail, then Bedrock verification. The desktop app passes this vec
/// directly to [`scan`], [`build_plan`], and [`execute_plan`].
pub fn build_resources(
    config: &aws_config::SdkConfig,
    system_name: &str,
) -> Vec<Box<dyn Resource>> {
    let region = config
        .region()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());

    let bucket_name = format!("{system_name}-data");
    let trail_name = format!("{system_name}-trail");

    let s3_client = aws_sdk_s3::Client::new(config);
    let cloudtrail_client = aws_sdk_cloudtrail::Client::new(config);
    let bedrock_client = aws_sdk_bedrock::Client::new(config);

    let model_ids: Vec<String> = DEFAULT_MODEL_IDS.iter().map(|s| (*s).to_string()).collect();

    vec![
        Box::new(resources::s3_bucket::S3BucketResource::new(
            s3_client,
            bucket_name.clone(),
            region,
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
    ]
}

/// Construct a [`StatePersistence`] from an SDK config and system name.
///
/// State is dual-written: local disk (safety net) + S3 (authoritative).
/// The local path is under the OS config directory at
/// `com.claria.desktop/{system_name}/provisioner-state.json`.
pub fn build_persistence(
    config: &aws_config::SdkConfig,
    system_name: &str,
) -> Result<StatePersistence, ProvisionerError> {
    let s3_client = aws_sdk_s3::Client::new(config);
    let bucket = format!("{system_name}-data");
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
///
/// Useful for the desktop app to pre-create the directory or check for
/// existing local state without constructing a full `StatePersistence`.
pub fn local_state_dir(system_name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("com.claria.desktop").join(system_name))
}

// ── Convenience orchestrators ────────────────────────────────────────────────

/// Full provisioning: scan → plan → execute.
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

/// Destroy all managed resources in reverse order.
pub async fn destroy(
    persistence: &StatePersistence,
    resources: &[Box<dyn Resource>],
) -> Result<(), ProvisionerError> {
    let mut state = persistence.load().await?;

    for resource in resources.iter().rev() {
        let resource_type = resource.resource_type();
        if let Some(rs) = state.resources.get(resource_type) {
            tracing::info!(resource_type = %resource_type, "destroying resource");
            resource.delete(&rs.resource_id).await?;
        }
    }

    state.resources.clear();
    persistence.flush(&state).await?;

    Ok(())
}
