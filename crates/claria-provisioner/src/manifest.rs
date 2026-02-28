use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::addr::ResourceAddr;

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

/// The full manifest: version + all resource specs.
pub struct Manifest {
    pub version: u32,
    pub specs: Vec<ResourceSpec>,
}

impl Manifest {
    /// Bump when adding, removing, or changing resource specs.
    pub const VERSION: u32 = 1;

    /// Build the default Claria manifest from runtime config.
    pub fn claria(account_id: &str, system_name: &str, region: &str) -> Self {
        let bucket = format!("{account_id}-{system_name}-data");
        let trail = format!("{system_name}-trail");

        Manifest {
            version: Self::VERSION,
            specs: vec![
                // ── data sources (read-only preconditions) ────────────────
                ResourceSpec {
                    resource_type: "iam_user".into(),
                    resource_name: "claria-admin".into(),
                    lifecycle: Lifecycle::Data,
                    desired: json!({"exists": true}),
                    label: "IAM User".into(),
                    description: "Dedicated least-privilege user that Claria operates as".into(),
                    severity: Severity::Info,
                    iam_actions: vec!["iam:GetUser".into()],
                },
                ResourceSpec {
                    resource_type: "iam_user_policy".into(),
                    resource_name: "claria-admin-policy".into(),
                    lifecycle: Lifecycle::Data,
                    desired: json!(null), // dynamically set — see IamUserPolicySyncer
                    label: "IAM Policy".into(),
                    description: "Permissions scoped to only what Claria needs".into(),
                    severity: Severity::Info,
                    iam_actions: vec![
                        "iam:ListAttachedUserPolicies".into(),
                        "iam:GetPolicyVersion".into(),
                    ],
                },
                // ── managed resources ─────────────────────────────────────
                ResourceSpec {
                    resource_type: "baa_agreement".into(),
                    resource_name: "aws-baa".into(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"state": "active"}),
                    label: "BAA Agreement".into(),
                    description: "Business Associate Agreement — your legal HIPAA contract with AWS"
                        .into(),
                    severity: Severity::Elevated,
                    iam_actions: vec!["artifact:ListCustomerAgreements".into()],
                },
                ResourceSpec {
                    resource_type: "s3_bucket".into(),
                    resource_name: bucket.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"region": region}),
                    label: "S3 Bucket".into(),
                    description: "Encrypted storage for your client records and documents".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:HeadBucket".into(),
                        "s3:CreateBucket".into(),
                        "s3:DeleteBucket".into(),
                        "s3:ListObjectsV2".into(),
                        "s3:DeleteObject".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "s3_bucket_versioning".into(),
                    resource_name: bucket.clone(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"status": "Enabled"}),
                    label: "Versioning".into(),
                    description: "Version history — protects against accidental deletion".into(),
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
                    label: "Encryption".into(),
                    description: "Server-side encryption — your data is encrypted at rest".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:GetBucketEncryption".into(),
                        "s3:PutBucketEncryption".into(),
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
                    label: "Public Access Block".into(),
                    description: "Prevents your data from ever being publicly accessible".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "s3:GetPublicAccessBlock".into(),
                        "s3:PutPublicAccessBlock".into(),
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
                    label: "Trail Logging".into(),
                    description: "Audit logging status — must be active for compliance".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        "cloudtrail:GetTrailStatus".into(),
                        "cloudtrail:StartLogging".into(),
                        "cloudtrail:StopLogging".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "bedrock_model_agreement".into(),
                    resource_name: "anthropic.claude-sonnet-4".into(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"agreement": "accepted"}),
                    label: "Claude Sonnet 4 Access".into(),
                    description: "AI model access for report generation".into(),
                    severity: Severity::Elevated,
                    iam_actions: vec![
                        "bedrock:ListFoundationModels".into(),
                        "bedrock:GetFoundationModelAvailability".into(),
                        "bedrock:ListFoundationModelAgreementOffers".into(),
                        "bedrock:CreateFoundationModelAgreement".into(),
                    ],
                },
                ResourceSpec {
                    resource_type: "bedrock_model_agreement".into(),
                    resource_name: "anthropic.claude-opus-4".into(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"agreement": "accepted"}),
                    label: "Claude Opus 4 Access".into(),
                    description: "AI model access for complex analysis".into(),
                    severity: Severity::Elevated,
                    iam_actions: vec![
                        "bedrock:ListFoundationModels".into(),
                        "bedrock:GetFoundationModelAvailability".into(),
                        "bedrock:ListFoundationModelAgreementOffers".into(),
                        "bedrock:CreateFoundationModelAgreement".into(),
                    ],
                },
            ],
        }
    }
}
