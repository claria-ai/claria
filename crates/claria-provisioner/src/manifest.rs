use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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

    /// Which credential scope this resource belongs to
    pub credential_scope: CredentialScope,

    // ── Trust metadata ──
    /// Short label for the UI, e.g. "S3 Bucket Encryption"
    pub label: String,
    /// Human-readable purpose, e.g. "Server-side encryption — your data is encrypted at rest"
    pub description: String,
    /// How much attention this entry needs
    pub severity: Severity,
    /// IAM actions this resource requires.
    ///
    /// The single declaration of what Claria's policy grants: the diff
    /// compares against it and [`crate::account_setup::claria_policy_document`]
    /// renders it. Widening an action list here widens the policy.
    pub iam_actions: Vec<IamAction>,
}

/// What a granted IAM action may be used on.
///
/// Carried per action rather than per resource because one resource's actions
/// do not share a scope — `iam_user` needs `iam:GetUser` against its own user
/// ARN and `sts:GetCallerIdentity` against the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IamScope {
    /// Only buckets under this deployment's `{account}-{system}-` prefix.
    ClariaBuckets,
    /// Only Claria's own IAM user and policy.
    ClariaIdentity,
    /// Account-wide. For actions AWS does not scope to a resource — trails
    /// are looked up by name, foundation models and agreements are not
    /// account resources, `ce:GetCostAndUsage` and `sts:GetCallerIdentity`
    /// are account-wide by definition.
    Account,
}

/// One IAM action and what it may be used on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct IamAction {
    /// The IAM action name, which is not always the API operation name —
    /// `s3:GetEncryptionConfiguration`, not `s3:GetBucketEncryption`.
    pub action: String,
    pub scope: IamScope,
}

impl IamAction {
    fn new(action: &str, scope: IamScope) -> Self {
        Self {
            action: action.to_string(),
            scope,
        }
    }

    /// Scoped to this deployment's buckets.
    pub fn bucket(action: &str) -> Self {
        Self::new(action, IamScope::ClariaBuckets)
    }

    /// Scoped to Claria's own IAM user and policy.
    pub fn identity(action: &str) -> Self {
        Self::new(action, IamScope::ClariaIdentity)
    }

    /// Account-wide.
    pub fn account(action: &str) -> Self {
        Self::new(action, IamScope::Account)
    }
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

/// A service precondition the IAM policy grants but nothing has exercised.
///
/// Honest rather than reassuring: Claria asked for the permission and does not
/// know whether the account will honour it. The first real call proves it.
pub const GRANTED_BY_POLICY: &str = "granted_by_policy";

/// A service precondition a live call has proved.
pub const VERIFIED: &str = "verified";

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
        let bucket = claria_core::s3_keys::bucket_name(account_id, system_name);
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
                    // The ARN is part of what is wanted, not decoration: it
                    // pins the user to this account. Without it the syncer
                    // had nothing to compare and reported "exists" against
                    // "exists" forever.
                    desired: json!({
                        "exists": true,
                        "user_arn": format!("arn:aws:iam::{account_id}:user/claria-admin"),
                    }),
                    credential_scope: CredentialScope::Elevated,
                    label: "IAM User".into(),
                    description: "Dedicated least-privilege user that Claria operates as".into(),
                    severity: Severity::Normal,
                    iam_actions: vec![
                        IamAction::identity("iam:GetUser"),
                        IamAction::account("sts:GetCallerIdentity"),
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
                        IamAction::identity("iam:ListAttachedUserPolicies"),
                        IamAction::identity("iam:GetPolicy"),
                        IamAction::identity("iam:GetPolicyVersion"),
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
                    iam_actions: vec![IamAction::account("artifact:ListCustomerAgreements")],
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
                    // Permanent version deletion belongs exclusively to the
                    // temporary elevated credentials used for full teardown.
                    iam_actions: vec![
                        IamAction::bucket("s3:HeadBucket"),
                        IamAction::bucket("s3:CreateBucket"),
                        IamAction::bucket("s3:DeleteBucket"),
                        IamAction::bucket("s3:ListBucket"),
                        IamAction::bucket("s3:ListBucketVersions"),
                        IamAction::bucket("s3:GetObject"),
                        IamAction::bucket("s3:GetObjectVersion"),
                        IamAction::bucket("s3:PutObject"),
                        IamAction::bucket("s3:DeleteObject"),
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
                        IamAction::bucket("s3:GetBucketVersioning"),
                        IamAction::bucket("s3:PutBucketVersioning"),
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
                        IamAction::bucket("s3:GetEncryptionConfiguration"),
                        IamAction::bucket("s3:PutEncryptionConfiguration"),
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
                        IamAction::bucket("s3:GetBucketPublicAccessBlock"),
                        IamAction::bucket("s3:PutBucketPublicAccessBlock"),
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
                        IamAction::bucket("s3:GetBucketPolicy"),
                        IamAction::bucket("s3:PutBucketPolicy"),
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
                        IamAction::account("cloudtrail:GetTrail"),
                        IamAction::account("cloudtrail:CreateTrail"),
                        IamAction::account("cloudtrail:DeleteTrail"),
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
                        IamAction::account("cloudtrail:GetTrailStatus"),
                        IamAction::account("cloudtrail:StartLogging"),
                        IamAction::account("cloudtrail:StopLogging"),
                    ],
                },
                ResourceSpec {
                    resource_type: "bedrock_model_agreement".into(),
                    resource_name: "anthropic.claude".into(),
                    lifecycle: Lifecycle::Managed,
                    desired: json!({"agreement": "accepted"}),
                    credential_scope: CredentialScope::Regular,
                    label: "Anthropic Claude Access".into(),
                    description: "AI model access for chat, report generation, and analysis"
                        .into(),
                    severity: Severity::Elevated,
                    iam_actions: vec![
                        IamAction::account("bedrock:ListFoundationModels"),
                        IamAction::account("bedrock:ListInferenceProfiles"),
                        IamAction::account("bedrock:GetFoundationModelAvailability"),
                        IamAction::account("bedrock:ListFoundationModelAgreementOffers"),
                        IamAction::account("bedrock:CreateFoundationModelAgreement"),
                        IamAction::account("bedrock:InvokeModel"),
                        IamAction::account("bedrock:InvokeModelWithResponseStream"),
                        IamAction::account("bedrock:CountTokens"),
                        IamAction::account("aws-marketplace:ViewSubscriptions"),
                        IamAction::account("aws-marketplace:Subscribe"),
                    ],
                },
                ResourceSpec {
                    resource_type: "transcribe_access".into(),
                    resource_name: "transcribe".into(),
                    lifecycle: Lifecycle::Data,
                    desired: json!({"access": "granted"}),
                    credential_scope: CredentialScope::Regular,
                    label: "Amazon Transcribe".into(),
                    description: "Audio-to-text transcription for uploaded recordings".into(),
                    severity: Severity::Info,
                    iam_actions: vec![
                        IamAction::account("transcribe:StartTranscriptionJob"),
                        IamAction::account("transcribe:GetTranscriptionJob"),
                        IamAction::account("transcribe:DeleteTranscriptionJob"),
                        IamAction::account("transcribe:StartMedicalTranscriptionJob"),
                        IamAction::account("transcribe:GetMedicalTranscriptionJob"),
                        IamAction::account("transcribe:DeleteMedicalTranscriptionJob"),
                    ],
                },
                ResourceSpec {
                    resource_type: "cost_explorer_access".into(),
                    resource_name: "cost-explorer".into(),
                    lifecycle: Lifecycle::Data,
                    desired: json!({"access": GRANTED_BY_POLICY}),
                    credential_scope: CredentialScope::Regular,
                    label: "AWS Cost Explorer".into(),
                    description:
                        "Permission to read your AWS spending, granted by Claria's IAM policy"
                            .into(),
                    severity: Severity::Info,
                    iam_actions: vec![IamAction::account("ce:GetCostAndUsage")],
                },
            ],
        }
    }

    /// Every IAM action this deployment's resources declare.
    ///
    /// The whole point of the type: the policy document and the drift
    /// comparison are both rendered from this one list, so they cannot come
    /// to disagree. They were separate declarations, and a divergence in
    /// either direction made the plan report a drift that applying could
    /// never resolve — the diff read the manifest, the write emitted a
    /// hand-kept JSON document.
    ///
    /// Region does not reach a policy, so this does not ask for one.
    pub fn iam_actions(account_id: &str, system_name: &str) -> Vec<IamAction> {
        Manifest::claria(account_id, system_name, "us-east-1")
            .specs
            .into_iter()
            .flat_map(|spec| spec.iam_actions)
            .collect()
    }

    /// Ask the Cost Explorer precondition to prove itself with a live call.
    ///
    /// Off by default, and the only resource in the manifest with a switch,
    /// because Cost Explorer's one granted action — `ce:GetCostAndUsage` — is
    /// billed at $0.01 per request and the Provision page scans on mount. An
    /// operator who has not turned Cost Explorer on would be paying a cent a
    /// visit to verify a feature they do not use; one who has is already
    /// paying per request on the Cost page, where the same call runs for real.
    ///
    /// Without it the precondition reports [`GRANTED_BY_POLICY`] — what the
    /// policy asked for, stated as such, rather than a check dressed up as
    /// one.
    pub fn with_cost_explorer_probe(mut self, probe: bool) -> Self {
        if !probe {
            return self;
        }
        for spec in &mut self.specs {
            if spec.resource_type == "cost_explorer_access" {
                spec.desired = json!({"access": VERIFIED});
                spec.description = "Read-only access to view your AWS spending".into();
            }
        }
        self
    }
}
