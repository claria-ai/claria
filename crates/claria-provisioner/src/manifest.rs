use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::addr::ResourceAddr;
use crate::error::ProvisionerError;
use crate::syncer::BoxFuture;

/// Inject runtime-discovered `ResourceSpec`s into the manifest.
///
/// Implementations live outside the provisioner crate (e.g. in
/// `claria-live-aws` for marketplace agreements) and are passed into
/// `build_manifest_with_contributors`. The provisioner pipeline treats
/// the resulting specs identically to statically declared ones — same
/// plan rendering, same state tracking, same audit trail.
///
/// Implementations MUST be read-only against AWS — they may call
/// `List*`/`Describe*`/`Get*` to derive their specs, but must never
/// mutate AWS state.
pub trait ManifestContributor: Send + Sync {
    fn contribute<'a>(
        &'a self,
        sdk: &'a aws_config::SdkConfig,
    ) -> BoxFuture<'a, Result<Vec<ResourceSpec>, ProvisionerError>>;
}

/// Every resource in the system is declared as a `ResourceSpec`.
///
/// The spec carries both the desired AWS state and the trust metadata
/// (label, description, severity, required IAM actions). This is the
/// single source of truth — the syncer, the plan, and the UI all read
/// from it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ResourceSpec {
    /// e.g. "s3_bucket", "baa_agreement"
    pub resource_type: String,
    /// e.g. "123456789012-claria-data"
    pub resource_name: String,
    /// Data (read-only precondition) or Managed (Claria creates/updates/deletes)
    pub lifecycle: Lifecycle,
    /// The desired AWS state as a JSON value — shape varies per resource type
    pub desired: Value,

    /// Which credential scope this resource belongs to
    pub credential_scope: CredentialScope,

    // ── Trust metadata ──
    /// Short label for the UI, e.g. "S3 Bucket Encryption"
    pub label: String,
    /// Human-readable purpose, e.g. "Server-side encryption — your data is encrypted at rest"
    pub description: String,
    /// How much attention this entry needs
    pub severity: Severity,
    /// IAM actions this resource requires (aggregated for policy diff)
    pub iam_actions: Vec<String>,
}

impl ResourceSpec {
    pub fn addr(&self) -> ResourceAddr {
        ResourceAddr {
            resource_type: self.resource_type.clone(),
            resource_name: self.resource_name.clone(),
        }
    }

    /// Construct a minimal spec for an orphaned resource (display only).
    pub fn orphaned(addr: &ResourceAddr) -> Self {
        Self {
            resource_type: addr.resource_type.clone(),
            resource_name: addr.resource_name.clone(),
            lifecycle: Lifecycle::Managed,
            desired: Value::Null,
            credential_scope: CredentialScope::Regular,
            label: format!("{} (orphaned)", addr.resource_type),
            description: "Resource is no longer managed by Claria and will be removed".into(),
            severity: Severity::Destructive,
            iam_actions: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Data,
    Managed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CredentialScope {
    /// Requires elevated credentials (root/admin) to create or modify.
    /// Can be read with regular (claria-admin) credentials for drift detection.
    Elevated,
    /// Uses the regular claria-admin credentials.
    Regular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Data sources — read-only checks
    Info,
    /// Routine infra (S3 settings, CloudTrail)
    Normal,
    /// Requires acknowledgment (BAA, model agreements)
    Elevated,
    /// Data loss risk (bucket deletion during orphan cleanup)
    Destructive,
}

/// Structured before/after for a single field that doesn't match desired state.
///
/// Returned by `ResourceSyncer::diff()`. The frontend renders these directly
/// as before/after rows.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct FieldDrift {
    /// Machine-readable field name, e.g. "sse_algorithm"
    pub field: String,
    /// Human-readable label, e.g. "Encryption algorithm"
    pub label: String,
    /// What we want
    pub expected: Value,
    /// What AWS has
    pub actual: Value,
}

/// The full manifest: all resource specs for a Claria deployment.
///
/// No version tracking — the reconciler uses structural comparison.
/// Either a resource with name X and properties Y exists, or it doesn't
/// and gets queued for reconciliation.
pub struct Manifest {
    pub specs: Vec<ResourceSpec>,
    /// Account ID — used by syncers that need to construct ARNs.
    pub account_id: String,
    /// System name — used by syncers that need to generate policy documents.
    pub system_name: String,
}

impl Manifest {
    /// Build the default Claria manifest from runtime config.
    pub fn claria(account_id: &str, system_name: &str, region: &str) -> Self {
        let bucket = format!("{account_id}-{system_name}-data");
        let trail = format!("{system_name}-trail");

        Manifest {
            account_id: account_id.to_string(),
            system_name: system_name.to_string(),
            specs: vec![
                // ── elevated resources (require admin/root to create) ────
                ResourceSpec {
                    resource_type: "iam_user".into(),
                    resource_name: "claria-admin".into(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"exists": true}),
                    credential_scope: CredentialScope::Elevated,
                    label: "IAM User".into(),
                    description: "Dedicated least-privilege user that Claria operates as".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "iam:GetUser".into(),
                        "sts:GetCallerIdentity".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "iam_user_policy".into(),
                    resource_name: "claria-admin-policy".into(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!(null), // dynamically set — see IamUserPolicySyncer
                    credential_scope: CredentialScope::Elevated,
                    label: "IAM Policy".into(),
                    description: "Permissions scoped to only what Claria needs".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "iam:ListAttachedUserPolicies".into(),
                        "iam:GetPolicy".into(),
                        "iam:GetPolicyVersion".into(),
                    ],
                },
                // ── regular resources ─────────────────────────────────────
                ResourceSpec {
                    resource_type: "baa_agreement".into(),
                    resource_name: "aws-baa".into(),
                    lifecycle: Lifecycle::Data,
                    desired: json!({"state": "active"}),
                    credential_scope: CredentialScope::Regular,
                    label: "BAA Agreement".into(),
                    description: "Business Associate Agreement — must be accepted in the AWS Artifact console"
                        .into(),
                    severity: Severity::Elevated,
                    iam_actions: vec!["artifact:ListCustomerAgreements".into()],
                },
                ResourceSpec {
                    resource_type: "s3_bucket".into(),
                    resource_name: bucket.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"region": region}),
                    credential_scope: CredentialScope::Regular,
                    label: "S3 Bucket".into(),
                    description: "Encrypted storage for your client records and documents".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:HeadBucket".into(),
                        "s3:CreateBucket".into(),
                        "s3:DeleteBucket".into(),
                        "s3:ListBucket".into(),
                        "s3:ListBucketVersions".into(),
                        "s3:GetObject".into(),
                        "s3:GetObjectVersion".into(),
                        "s3:PutObject".into(),
                        "s3:DeleteObject".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "s3_bucket_versioning".into(),
                    resource_name: bucket.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"status": "Enabled"}),
                    credential_scope: CredentialScope::Regular,
                    label: "S3 Bucket Versioning".into(),
                    description: "S3 version history — protects against accidental deletion".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:GetBucketVersioning".into(),
                        "s3:PutBucketVersioning".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "s3_bucket_encryption".into(),
                    resource_name: bucket.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"sse_algorithm": "AES256"}),
                    credential_scope: CredentialScope::Regular,
                    label: "S3 Bucket Encryption".into(),
                    description: "Server-side encryption — all objects in this bucket are encrypted at rest".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:GetEncryptionConfiguration".into(),
                        "s3:PutEncryptionConfiguration".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "s3_bucket_public_access_block".into(),
                    resource_name: bucket.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({
                        "block_public_acls": true,
                        "ignore_public_acls": true,
                        "block_public_policy": true,
                        "restrict_public_buckets": true,
                    }),
                    credential_scope: CredentialScope::Regular,
                    label: "Public Access Block".into(),
                    description: "Prevents your data from ever being publicly accessible".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:GetBucketPublicAccessBlock".into(),
                        "s3:PutBucketPublicAccessBlock".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "s3_bucket_policy".into(),
                    resource_name: bucket.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({
                        "statements": [
                            {
                                "sid": "AWSCloudTrailAclCheck",
                                "effect": "Allow",
                                "principal": {"service": "cloudtrail.amazonaws.com"},
                                "action": "s3:GetBucketAcl",
                                "resource": format!("arn:aws:s3:::{bucket}"),
                                "condition": {"StringEquals": {"AWS:SourceAccount": account_id}},
                            },
                            {
                                "sid": "AWSCloudTrailWrite",
                                "effect": "Allow",
                                "principal": {"service": "cloudtrail.amazonaws.com"},
                                "action": "s3:PutObject",
                                "resource": format!("arn:aws:s3:::{bucket}/_cloudtrail/AWSLogs/{account_id}/*"),
                                "condition": {"StringEquals": {
                                    "s3:x-amz-acl": "bucket-owner-full-control",
                                    "AWS:SourceAccount": account_id,
                                }},
                            },
                        ]
                    }),
                    credential_scope: CredentialScope::Regular,
                    label: "Bucket Policy".into(),
                    description: "Access policy — controls which AWS services can reach your data"
                        .into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:GetBucketPolicy".into(),
                        "s3:PutBucketPolicy".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "cloudtrail_trail".into(),
                    resource_name: trail.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({
                        "s3_bucket": &bucket,
                        "s3_key_prefix": "_cloudtrail",
                        "is_multi_region": false,
                    }),
                    credential_scope: CredentialScope::Regular,
                    label: "CloudTrail Trail".into(),
                    description: "Audit trail — records all account activity (HIPAA requirement)"
                        .into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "cloudtrail:GetTrail".into(),
                        "cloudtrail:CreateTrail".into(),
                        "cloudtrail:DeleteTrail".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "cloudtrail_trail_logging".into(),
                    resource_name: trail.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"enabled": true}),
                    credential_scope: CredentialScope::Regular,
                    label: "Trail Logging".into(),
                    description: "Audit logging status — must be active for compliance".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "cloudtrail:GetTrailStatus".into(),
                        "cloudtrail:StartLogging".into(),
                        "cloudtrail:StopLogging".into(),
                    ],
                },
                // Marketplace agreements (bedrock_model_agreement) are
                // contributed at runtime by the live-aws framework — they
                // depend on AWS's current Anthropic catalog, not on
                // hardcoded model IDs. See claria-live-aws.
                ResourceSpec {
                    resource_type: "transcribe_access".into(),
                    resource_name: "transcribe".into(),
                    lifecycle: Lifecycle::Data,
                    desired: json!({"enabled": true}),
                    credential_scope: CredentialScope::Regular,
                    label: "Amazon Transcribe".into(),
                    description: "Audio-to-text transcription for uploaded recordings".into(),
                    severity: Severity::Info,
                    iam_actions: vec![
                        "transcribe:StartTranscriptionJob".into(),
                        "transcribe:GetTranscriptionJob".into(),
                        "transcribe:DeleteTranscriptionJob".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "cost_explorer_access".into(),
                    resource_name: "cost-explorer".into(),
                    lifecycle: Lifecycle::Data,
                    desired: json!({"enabled": true}),
                    credential_scope: CredentialScope::Regular,
                    label: "AWS Cost Explorer".into(),
                    description: "Read-only access to view your AWS spending".into(),
                    severity: Severity::Info,
                    iam_actions: vec!["ce:GetCostAndUsage".into()],
                },
            ],
        }
    }
}
