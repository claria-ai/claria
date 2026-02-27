//! AWS account setup and credential bootstrapping.
//!
//! This module is the "brains" behind getting an AWS account ready for Claria.
//! It classifies credentials, creates least-privilege IAM users and policies,
//! and handles the transition from broad credentials (root / admin) to scoped
//! Claria-only credentials.
//!
//! # Credential flows
//!
//! | Detected class | Action |
//! |----------------|--------|
//! | **Root** | Create IAM policy + user, swap to new creds, delete root access key |
//! | **IamAdmin** | Create IAM policy + user, swap to new creds (leave admin key intact) |
//! | **ScopedClaria** | Ready for resource provisioning — no bootstrap needed |
//! | **Insufficient** | Cannot proceed — tell the operator what's missing |
//!
//! # Sub-account role assumption
//!
//! For operators using AWS Organizations, [`assume_role`] takes parent-account
//! credentials and assumes the `OrganizationAccountAccessRole` (or a custom
//! role) in a sub-account. The returned temporary credentials can then be fed
//! into [`assess_credentials`] and [`bootstrap_account`] to set up a dedicated
//! IAM user in the sub-account.
//!
//! The desktop app (controller/view) calls into this module and receives
//! structured results. It never needs to know *how* IAM works — only *what
//! happened* and *what to do next*.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::ProvisionerError;

// ── Constants ────────────────────────────────────────────────────────────────

pub(crate) const IAM_USER_NAME: &str = "claria-admin";
pub(crate) const IAM_POLICY_NAME: &str = "ClariaProvisionerAccess";

// ── Public types ─────────────────────────────────────────────────────────────

/// Identity information returned by STS `GetCallerIdentity`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CallerIdentity {
    pub account_id: String,
    pub arn: String,
    pub user_id: String,
    pub is_root: bool,
}

/// Classification of the credentials the operator provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CredentialClass {
    /// The AWS account root user. Has God-mode access but should never be
    /// used for day-to-day operations. Bootstrap required.
    Root,

    /// An IAM principal (user or role) with broad permissions — at minimum
    /// the ability to manage IAM users and policies. Bootstrap required to
    /// create a scoped Claria user.
    IamAdmin,

    /// An IAM principal that already has the minimal Claria permissions.
    /// Ready for resource provisioning.
    ScopedClaria,

    /// The credentials lack the permissions Claria needs and also lack the
    /// IAM permissions required to self-bootstrap.
    Insufficient,
}

/// The result of `assess_credentials`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CredentialAssessment {
    pub identity: CallerIdentity,
    pub credential_class: CredentialClass,
    /// Human-readable explanation of why this class was chosen.
    pub reason: String,
}

/// Fresh credentials created during the bootstrap flow.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct NewCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub iam_user_arn: String,
}

/// Metadata for an existing IAM access key.
///
/// Returned by [`list_user_access_keys`] so the operator can decide which
/// key to delete when the IAM 2-key limit is reached.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AccessKeyInfo {
    /// The access key ID (e.g. `AKIA...`).
    pub access_key_id: String,
    /// `"Active"` or `"Inactive"`.
    pub status: String,
    /// When the key was created (ISO 8601).
    pub created_at: Option<String>,
    /// When the key was last used (ISO 8601), or `None` if never used.
    pub last_used_at: Option<String>,
    /// The AWS service the key was last used with (e.g. `"s3"`, `"iam"`),
    /// or `None` if never used.
    pub last_used_service: Option<String>,
}

/// Temporary credentials obtained by assuming a role in a sub-account.
///
/// These are short-lived (typically 1 hour) and include a session token.
/// They should **never** be persisted to disk — they exist only to bootstrap
/// a dedicated IAM user in the sub-account.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AssumeRoleResult {
    /// Temporary access key ID for the assumed role.
    pub access_key_id: String,
    /// Temporary secret access key for the assumed role.
    pub secret_access_key: String,
    /// Session token — required for all API calls made with these credentials.
    pub session_token: String,
    /// When these temporary credentials expire (ISO 8601).
    pub expiration: Option<String>,
    /// The ARN of the assumed role (e.g.
    /// `arn:aws:sts::690641653532:assumed-role/OrganizationAccountAccessRole/claria-setup`).
    pub assumed_role_arn: String,
    /// The account ID of the sub-account we assumed into.
    pub account_id: String,
}

/// A single step in the bootstrap sequence, reported for UI rendering.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BootstrapStep {
    pub name: String,
    pub status: StepStatus,
    pub detail: Option<String>,
}

/// Status of an individual bootstrap step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Succeeded,
    Failed,
}

/// The result of a full bootstrap attempt.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BootstrapResult {
    pub success: bool,
    pub steps: Vec<BootstrapStep>,
    pub account_id: Option<String>,
    /// The new, scoped credentials. `None` on failure.
    pub new_credentials: Option<NewCredentials>,
    pub error: Option<String>,
}

// ── Role assumption ──────────────────────────────────────────────────────────

/// Assume a role in a sub-account using the provided (parent-account)
/// credentials.
///
/// This is the entry point for the **Persona A** (sub-account) flow. The
/// operator provides their parent-account credentials and the sub-account
/// details, and we call STS `AssumeRole` to obtain temporary credentials
/// in the sub-account.
///
/// # Arguments
///
/// * `config` — SDK config built from the parent-account credentials.
/// * `role_arn` — Full ARN of the role to assume, e.g.
///   `arn:aws:iam::690641653532:role/OrganizationAccountAccessRole`.
/// * `session_name` — Optional session name for CloudTrail attribution.
///   Defaults to `"claria-setup"`.
///
/// # Returns
///
/// An [`AssumeRoleResult`] containing temporary credentials that can be
/// used to call [`assess_credentials`] and [`bootstrap_account`].
pub async fn assume_role(
    config: &aws_config::SdkConfig,
    role_arn: &str,
    session_name: Option<&str>,
) -> Result<AssumeRoleResult, ProvisionerError> {
    let sts = aws_sdk_sts::Client::new(config);
    let session = session_name.unwrap_or("claria-setup");

    tracing::info!(role_arn = %role_arn, session = %session, "assuming role in sub-account");

    let resp = sts
        .assume_role()
        .role_arn(role_arn)
        .role_session_name(session)
        .send()
        .await
        .map_err(|e| ProvisionerError::Aws(format!("STS AssumeRole failed: {e}")))?;

    let creds = resp.credentials().ok_or_else(|| {
        ProvisionerError::Aws("AssumeRole returned no credentials".into())
    })?;

    let access_key_id = creds.access_key_id().to_string();
    let secret_access_key = creds.secret_access_key().to_string();
    let session_token = creds.session_token().to_string();
    let expiration = Some(creds.expiration().to_string());

    let assumed_role_arn = resp
        .assumed_role_user()
        .map(|u| u.arn().to_string())
        .unwrap_or_default();

    // Extract account ID from the assumed role ARN
    // Format: arn:aws:sts::ACCOUNT_ID:assumed-role/ROLE/SESSION
    let account_id = assumed_role_arn
        .split(':')
        .nth(4)
        .unwrap_or("")
        .to_string();

    tracing::info!(
        assumed_role_arn = %assumed_role_arn,
        account_id = %account_id,
        "successfully assumed role in sub-account"
    );

    Ok(AssumeRoleResult {
        access_key_id,
        secret_access_key,
        session_token,
        expiration,
        assumed_role_arn,
        account_id,
    })
}

/// Build a role ARN from an account ID and role name.
///
/// Convenience helper so the frontend doesn't need to construct ARNs.
///
/// ```text
/// build_role_arn("690641653532", "OrganizationAccountAccessRole")
/// // => "arn:aws:iam::690641653532:role/OrganizationAccountAccessRole"
/// ```
pub fn build_role_arn(account_id: &str, role_name: &str) -> String {
    format!("arn:aws:iam::{account_id}:role/{role_name}")
}

// ── Credential assessment ────────────────────────────────────────────────────

/// Probe the provided SDK config to determine what kind of credentials the
/// operator supplied.
///
/// This is a read-only operation — it never mutates any AWS state.
pub async fn assess_credentials(
    config: &aws_config::SdkConfig,
) -> Result<CredentialAssessment, ProvisionerError> {
    // Step 1: Who are we?
    let identity = get_caller_identity(config).await?;

    if identity.is_root {
        return Ok(CredentialAssessment {
            identity,
            credential_class: CredentialClass::Root,
            reason: "Credentials belong to the AWS account root user.".into(),
        });
    }

    // Step 2: Can we manage IAM? (probe with a cheap read-only call)
    let iam_client = aws_sdk_iam::Client::new(config);
    let has_iam = iam_client
        .list_users()
        .max_items(1)
        .send()
        .await
        .is_ok();

    if has_iam {
        return Ok(CredentialAssessment {
            identity,
            credential_class: CredentialClass::IamAdmin,
            reason: "Credentials have IAM management permissions.".into(),
        });
    }

    // Step 3: Can we do the things Claria actually needs?
    //
    // Probe a representative action from each service. We don't need all of
    // them to succeed — `HeadBucket` on a non-existent bucket returns 404
    // (not 403) when we have `s3:HeadBucket`, and `ListFoundationModels`
    // is a simple read.
    let s3_ok = probe_s3(config).await;
    let bedrock_ok = probe_bedrock(config).await;

    if s3_ok && bedrock_ok {
        return Ok(CredentialAssessment {
            identity,
            credential_class: CredentialClass::ScopedClaria,
            reason: "Credentials have the required Claria permissions.".into(),
        });
    }

    Ok(CredentialAssessment {
        identity,
        credential_class: CredentialClass::Insufficient,
        reason: format!(
            "Credentials lack required permissions (S3: {}, Bedrock: {}). \
             Provide credentials with IAM admin access so Claria can \
             create a properly scoped user, or attach the \
             ClariaProvisionerAccess policy manually.",
            if s3_ok { "ok" } else { "denied" },
            if bedrock_ok { "ok" } else { "denied" },
        ),
    })
}

// ── Bootstrap ────────────────────────────────────────────────────────────────

/// Create a least-privilege IAM user for Claria using the provided
/// (broad) credentials.
///
/// # Arguments
///
/// * `config` — SDK config built from the operator's current credentials.
/// * `system_name` — S3 bucket prefix / resource naming (e.g. `"claria"`).
/// * `source_access_key_id` — The access key ID of the credentials being
///   used. Needed so we can delete it when the source is root.
/// * `credential_class` — Must be `Root` or `IamAdmin`. Determines cleanup
///   behaviour (root keys are deleted; admin keys are left alone).
///
/// # Returns
///
/// A `BootstrapResult` containing step-by-step status and, on success,
/// the new scoped credentials. The caller (desktop app) is responsible
/// for persisting the new credentials to its config file.
///
/// **Root credentials are never returned or persisted.** They exist only
/// in the SDK config for the duration of this call.
pub async fn bootstrap_account(
    config: &aws_config::SdkConfig,
    system_name: &str,
    source_access_key_id: &str,
    credential_class: CredentialClass,
) -> BootstrapResult {
    assert!(
        credential_class == CredentialClass::Root
            || credential_class == CredentialClass::IamAdmin,
        "bootstrap_account should only be called for Root or IamAdmin credentials"
    );

    let mut steps: Vec<BootstrapStep> = Vec::with_capacity(7);
    let mut result = BootstrapResult {
        success: false,
        steps: Vec::new(),
        account_id: None,
        new_credentials: None,
        error: None,
    };

    // Grab the account ID.
    match get_caller_identity(config).await {
        Ok(identity) => {
            result.account_id = Some(identity.account_id);
        }
        Err(e) => {
            result.error = Some(format!("Failed to validate credentials: {e}"));
            result.steps = steps;
            return result;
        }
    }

    let iam_client = aws_sdk_iam::Client::new(config);

    // ── Step 1: Create IAM policy ────────────────────────────────────────
    push_step(&mut steps, "create_policy", StepStatus::InProgress, None);

    let policy_arn = match create_policy(&iam_client, system_name, &result.account_id).await {
        Ok(arn) => {
            set_step_status(&mut steps, "create_policy", StepStatus::Succeeded, None);
            arn
        }
        Err(e) => {
            set_step_status(
                &mut steps,
                "create_policy",
                StepStatus::Failed,
                Some(e.to_string()),
            );
            result.error = Some(format!("Failed to create IAM policy: {e}"));
            result.steps = steps;
            return result;
        }
    };

    // ── Step 2: Create IAM user ──────────────────────────────────────────
    push_step(&mut steps, "create_user", StepStatus::InProgress, None);

    let user_arn = match create_user(&iam_client).await {
        Ok(arn) => {
            set_step_status(&mut steps, "create_user", StepStatus::Succeeded, None);
            arn
        }
        Err(e) => {
            set_step_status(
                &mut steps,
                "create_user",
                StepStatus::Failed,
                Some(e.to_string()),
            );
            result.error = Some(format!("Failed to create IAM user: {e}"));
            result.steps = steps;
            return result;
        }
    };

    // ── Step 3: Attach policy to user ────────────────────────────────────
    push_step(&mut steps, "attach_policy", StepStatus::InProgress, None);

    if let Err(e) = attach_policy(&iam_client, &policy_arn).await {
        set_step_status(
            &mut steps,
            "attach_policy",
            StepStatus::Failed,
            Some(e.to_string()),
        );
        result.error = Some(format!("Failed to attach IAM policy: {e}"));
        result.steps = steps;
        return result;
    }
    set_step_status(&mut steps, "attach_policy", StepStatus::Succeeded, None);

    // ── Step 4: Create access key for IAM user ───────────────────────────
    //
    // AWS allows at most 2 access keys per IAM user. Check first so we
    // can return a structured error the desktop app can act on (show the
    // existing keys and let the operator delete one).
    push_step(
        &mut steps,
        "create_access_key",
        StepStatus::InProgress,
        None,
    );

    // Pre-check: how many keys does the user already have?
    let existing_key_count = iam_client
        .list_access_keys()
        .user_name(IAM_USER_NAME)
        .send()
        .await
        .map(|r| r.access_key_metadata().len())
        .unwrap_or(0);

    if existing_key_count >= 2 {
        set_step_status(
            &mut steps,
            "create_access_key",
            StepStatus::Failed,
            Some("key_limit_exceeded".into()),
        );
        result.error = Some(format!(
            "The {IAM_USER_NAME} user already has {existing_key_count} access keys \
             (the AWS maximum of 2). Delete an existing key to make room."
        ));
        result.steps = steps;
        return result;
    }

    let (new_key_id, new_secret) = match create_access_key(&iam_client).await {
        Ok(keys) => {
            set_step_status(
                &mut steps,
                "create_access_key",
                StepStatus::Succeeded,
                None,
            );
            keys
        }
        Err(e) => {
            set_step_status(
                &mut steps,
                "create_access_key",
                StepStatus::Failed,
                Some(e.to_string()),
            );
            result.error = Some(format!("Failed to create access key: {e}"));
            result.steps = steps;
            return result;
        }
    };

    // ── Step 5: Validate new credentials ─────────────────────────────────
    push_step(
        &mut steps,
        "validate_new_credentials",
        StepStatus::InProgress,
        None,
    );

    match validate_new_credentials(&new_key_id, &new_secret, config).await {
        Ok(()) => {
            set_step_status(
                &mut steps,
                "validate_new_credentials",
                StepStatus::Succeeded,
                None,
            );
        }
        Err(e) => {
            set_step_status(
                &mut steps,
                "validate_new_credentials",
                StepStatus::Failed,
                Some(e.to_string()),
            );
            result.error = Some(format!(
                "New IAM credentials failed validation after retries: {e}"
            ));
            result.steps = steps;
            return result;
        }
    }

    // ── Step 6: Delete source access key (root only) ─────────────────────
    if credential_class == CredentialClass::Root {
        push_step(
            &mut steps,
            "delete_source_key",
            StepStatus::InProgress,
            None,
        );

        if let Err(e) = delete_access_key(&iam_client, source_access_key_id, None).await {
            // Non-fatal: the IAM user is ready. Warn the operator to clean
            // up the root key manually.
            set_step_status(
                &mut steps,
                "delete_source_key",
                StepStatus::Failed,
                Some(format!(
                    "Could not delete root access key. Please delete it \
                     manually in the IAM console. Error: {e}"
                )),
            );
            tracing::warn!("failed to delete root access key: {e}");
        } else {
            set_step_status(
                &mut steps,
                "delete_source_key",
                StepStatus::Succeeded,
                None,
            );
        }
    } else {
        push_step(
            &mut steps,
            "delete_source_key",
            StepStatus::Succeeded,
            Some("Skipped — source credentials are not root.".into()),
        );
    }

    // ── Done ─────────────────────────────────────────────────────────────
    result.success = true;
    result.new_credentials = Some(NewCredentials {
        access_key_id: new_key_id,
        secret_access_key: new_secret,
        iam_user_arn: user_arn,
    });
    result.steps = steps;
    result
}

// ── Access key management ────────────────────────────────────────────────────

/// List all access keys for the `claria-admin` IAM user, enriched with
/// last-used metadata.
///
/// The desktop app calls this when bootstrap fails due to the 2-key limit,
/// so the operator can choose which key to delete.
pub async fn list_user_access_keys(
    config: &aws_config::SdkConfig,
) -> Result<Vec<AccessKeyInfo>, ProvisionerError> {
    let client = aws_sdk_iam::Client::new(config);

    let resp = client
        .list_access_keys()
        .user_name(IAM_USER_NAME)
        .send()
        .await
        .map_err(|e| ProvisionerError::Aws(format!("iam:ListAccessKeys failed: {e}")))?;

    let mut keys = Vec::new();

    for meta in resp.access_key_metadata() {
        let key_id = meta
            .access_key_id()
            .unwrap_or_default()
            .to_string();
        let status = meta
            .status()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default();
        let created_at = meta.create_date().map(|d| d.to_string());

        // Enrich with last-used info.
        let (last_used_at, last_used_service) =
            match client.get_access_key_last_used().access_key_id(&key_id).send().await {
                Ok(lu_resp) => {
                    let lu = lu_resp.access_key_last_used();
                    (
                        lu.and_then(|l| l.last_used_date()).map(|d| d.to_string()),
                        lu.map(|l| l.service_name().to_string())
                            .filter(|s| !s.is_empty()),
                    )
                }
                Err(_) => (None, None),
            };

        keys.push(AccessKeyInfo {
            access_key_id: key_id,
            status,
            created_at,
            last_used_at,
            last_used_service,
        });
    }

    // Sort by creation date (oldest first).
    keys.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    Ok(keys)
}

/// Delete one access key belonging to the `claria-admin` IAM user.
///
/// The desktop app calls this when the operator picks a key to remove
/// to make room for a fresh one during bootstrap.
pub async fn delete_user_access_key(
    config: &aws_config::SdkConfig,
    access_key_id: &str,
) -> Result<(), ProvisionerError> {
    let client = aws_sdk_iam::Client::new(config);
    delete_access_key(&client, access_key_id, Some(IAM_USER_NAME)).await
}

// ── Policy document ──────────────────────────────────────────────────────────

/// Build the Claria minimal IAM policy document.
///
/// S3 actions are scoped to buckets matching `{system_name}-*`.
/// IAM read actions are scoped to the `claria-admin` user and
/// `ClariaProvisionerAccess` policy so the dashboard can verify its own setup.
fn claria_policy_document(system_name: &str, account_id: &str) -> String {
    serde_json::json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Sid": "ClariaS3",
                "Effect": "Allow",
                "Action": [
                    "s3:HeadBucket",
                    "s3:CreateBucket",
                    "s3:DeleteBucket",
                    "s3:GetBucketVersioning",
                    "s3:PutBucketVersioning",
                    "s3:GetEncryptionConfiguration",
                    "s3:PutEncryptionConfiguration",
                    "s3:GetBucketPublicAccessBlock",
                    "s3:PutBucketPublicAccessBlock",
                    "s3:GetBucketPolicy",
                    "s3:PutBucketPolicy",
                    "s3:GetObject",
                    "s3:PutObject",
                    "s3:DeleteObject",
                    "s3:ListBucket"
                ],
                "Resource": [
                    format!("arn:aws:s3:::{account_id}-{system_name}-*"),
                    format!("arn:aws:s3:::{account_id}-{system_name}-*/*")
                ]
            },
            {
                "Sid": "ClariaCloudTrail",
                "Effect": "Allow",
                "Action": [
                    "cloudtrail:GetTrail",
                    "cloudtrail:GetTrailStatus",
                    "cloudtrail:CreateTrail",
                    "cloudtrail:StartLogging",
                    "cloudtrail:StopLogging",
                    "cloudtrail:DeleteTrail"
                ],
                "Resource": "*"
            },
            {
                "Sid": "ClariaBedrock",
                "Effect": "Allow",
                "Action": [
                    "bedrock:ListFoundationModels",
                    "bedrock:ListInferenceProfiles",
                    "bedrock:GetFoundationModelAvailability",
                    "bedrock:ListFoundationModelAgreementOffers",
                    "bedrock:CreateFoundationModelAgreement",
                    "bedrock:InvokeModel",
                    "bedrock:InvokeModelWithResponseStream"
                ],
                "Resource": "*"
            },
            {
                "Sid": "ClariaMarketplace",
                "Effect": "Allow",
                "Action": [
                    "aws-marketplace:ViewSubscriptions",
                    "aws-marketplace:Subscribe"
                ],
                "Resource": "*"
            },
            {
                "Sid": "ClariaIAMReadSelf",
                "Effect": "Allow",
                "Action": [
                    "iam:GetUser",
                    "iam:ListAttachedUserPolicies",
                    "iam:GetPolicy",
                    "iam:GetPolicyVersion"
                ],
                "Resource": [
                    format!("arn:aws:iam::{account_id}:user/{IAM_USER_NAME}"),
                    format!("arn:aws:iam::{account_id}:policy/{IAM_POLICY_NAME}")
                ]
            },
            {
                "Sid": "ClariaSTS",
                "Effect": "Allow",
                "Action": [
                    "sts:GetCallerIdentity"
                ],
                "Resource": "*"
            }
        ]
    })
    .to_string()
}

// ── STS helpers ──────────────────────────────────────────────────────────────

/// Call STS `GetCallerIdentity` and return structured identity info.
async fn get_caller_identity(
    config: &aws_config::SdkConfig,
) -> Result<CallerIdentity, ProvisionerError> {
    let sts = aws_sdk_sts::Client::new(config);
    let resp = sts
        .get_caller_identity()
        .send()
        .await
        .map_err(|e| ProvisionerError::Aws(format!("STS GetCallerIdentity failed: {e}")))?;

    let arn = resp.arn().unwrap_or_default().to_string();
    let is_root = arn.ends_with(":root");

    Ok(CallerIdentity {
        account_id: resp.account().unwrap_or_default().to_string(),
        arn,
        user_id: resp.user_id().unwrap_or_default().to_string(),
        is_root,
    })
}

// ── Service probes (read-only permission checks) ─────────────────────────────

/// Check if the credentials have basic S3 access.
///
/// We call `ListBuckets` with a max of 1. A successful response (even if
/// empty) means S3 permissions are present. An access-denied error means
/// they're not.
async fn probe_s3(config: &aws_config::SdkConfig) -> bool {
    let client = aws_sdk_s3::Client::new(config);
    client
        .list_buckets()
        .send()
        .await
        .is_ok()
}

/// Check if the credentials have basic Bedrock access.
async fn probe_bedrock(config: &aws_config::SdkConfig) -> bool {
    let client = aws_sdk_bedrock::Client::new(config);
    client
        .list_foundation_models()
        .send()
        .await
        .is_ok()
}

// ── IAM helpers ──────────────────────────────────────────────────────────────

/// Create the Claria minimal IAM policy. Returns the policy ARN.
///
/// Idempotent: if the policy already exists, returns the existing ARN.
async fn create_policy(
    client: &aws_sdk_iam::Client,
    system_name: &str,
    account_id: &Option<String>,
) -> Result<String, ProvisionerError> {
    let acct = account_id.as_deref().unwrap_or("*");
    let document = claria_policy_document(system_name, acct);

    match client
        .create_policy()
        .policy_name(IAM_POLICY_NAME)
        .policy_document(&document)
        .description("Minimal permissions for the Claria desktop app")
        .send()
        .await
    {
        Ok(resp) => {
            let arn = resp
                .policy()
                .and_then(|p| p.arn())
                .ok_or_else(|| {
                    ProvisionerError::Aws("CreatePolicy returned no ARN".into())
                })?
                .to_string();
            tracing::info!(policy_arn = %arn, "created IAM policy");
            Ok(arn)
        }
        Err(e) => {
            let is_conflict = e
                .as_service_error()
                .map(|se| se.is_entity_already_exists_exception())
                .unwrap_or(false);

            if is_conflict
                && let Some(acct) = account_id
            {
                let arn = format!("arn:aws:iam::{acct}:policy/{IAM_POLICY_NAME}");
                tracing::info!(policy_arn = %arn, "IAM policy already exists, updating document");

                // Update the policy document to ensure it matches the current version.
                // This handles the case where code adds new permissions (e.g. IAM read-self)
                // after the policy was originally created.
                update_policy_document(client, &arn, &document).await?;

                return Ok(arn);
            }

            Err(ProvisionerError::Aws(format!(
                "iam:CreatePolicy failed: {e}"
            )))
        }
    }
}

/// Create a new default policy version with the given document.
///
/// AWS allows up to 5 policy versions. If the limit is reached, we delete
/// the oldest non-default version before creating the new one.
async fn update_policy_document(
    client: &aws_sdk_iam::Client,
    policy_arn: &str,
    document: &str,
) -> Result<(), ProvisionerError> {
    match client
        .create_policy_version()
        .policy_arn(policy_arn)
        .policy_document(document)
        .set_as_default(true)
        .send()
        .await
    {
        Ok(resp) => {
            let vid = resp
                .policy_version()
                .and_then(|v| v.version_id())
                .unwrap_or("unknown");
            tracing::info!(policy_arn = %policy_arn, version = %vid, "updated policy document");
            Ok(())
        }
        Err(e) => {
            // If we hit the 5-version limit, prune the oldest non-default
            // version and retry once.
            let is_limit = e
                .as_service_error()
                .map(|se| se.is_limit_exceeded_exception())
                .unwrap_or(false);

            if !is_limit {
                return Err(ProvisionerError::Aws(format!(
                    "iam:CreatePolicyVersion failed: {e}"
                )));
            }

            tracing::info!("policy version limit reached, pruning oldest non-default version");

            let versions = client
                .list_policy_versions()
                .policy_arn(policy_arn)
                .send()
                .await
                .map_err(|e| {
                    ProvisionerError::Aws(format!("iam:ListPolicyVersions failed: {e}"))
                })?;

            // Find oldest non-default version.
            let oldest = versions
                .versions()
                .iter()
                .filter(|v| !v.is_default_version())
                .min_by_key(|v| v.create_date().map(|d| d.to_string()));

            if let Some(v) = oldest {
                let vid = v.version_id().unwrap_or("unknown");
                client
                    .delete_policy_version()
                    .policy_arn(policy_arn)
                    .version_id(vid)
                    .send()
                    .await
                    .map_err(|e| {
                        ProvisionerError::Aws(format!(
                            "iam:DeletePolicyVersion failed: {e}"
                        ))
                    })?;
                tracing::info!(version = %vid, "deleted old policy version");
            }

            // Retry the create.
            client
                .create_policy_version()
                .policy_arn(policy_arn)
                .policy_document(document)
                .set_as_default(true)
                .send()
                .await
                .map_err(|e| {
                    ProvisionerError::Aws(format!(
                        "iam:CreatePolicyVersion (retry) failed: {e}"
                    ))
                })?;

            tracing::info!(policy_arn = %policy_arn, "updated policy document after pruning");
            Ok(())
        }
    }
}

/// Create the `claria-admin` IAM user. Returns the user ARN.
///
/// Idempotent: if the user already exists, returns the existing ARN.
async fn create_user(
    client: &aws_sdk_iam::Client,
) -> Result<String, ProvisionerError> {
    match client
        .create_user()
        .user_name(IAM_USER_NAME)
        .send()
        .await
    {
        Ok(resp) => {
            let arn = resp
                .user()
                .map(|u| u.arn().to_string())
                .ok_or_else(|| {
                    ProvisionerError::Aws("CreateUser returned no user".into())
                })?;
            tracing::info!(user_arn = %arn, "created IAM user");
            Ok(arn)
        }
        Err(e) => {
            let is_conflict = e
                .as_service_error()
                .map(|se| se.is_entity_already_exists_exception())
                .unwrap_or(false);

            if is_conflict {
                let get_resp = client
                    .get_user()
                    .user_name(IAM_USER_NAME)
                    .send()
                    .await
                    .map_err(|e| {
                        ProvisionerError::Aws(format!("iam:GetUser failed: {e}"))
                    })?;

                let arn = get_resp
                    .user()
                    .map(|u| u.arn().to_string())
                    .ok_or_else(|| {
                        ProvisionerError::Aws("iam:GetUser returned no user".into())
                    })?;

                tracing::info!(user_arn = %arn, "IAM user already exists, reusing");
                return Ok(arn);
            }

            Err(ProvisionerError::Aws(format!(
                "iam:CreateUser failed: {e}"
            )))
        }
    }
}

/// Attach a managed policy to the Claria IAM user.
///
/// Idempotent: attaching an already-attached policy is a no-op in IAM.
async fn attach_policy(
    client: &aws_sdk_iam::Client,
    policy_arn: &str,
) -> Result<(), ProvisionerError> {
    client
        .attach_user_policy()
        .user_name(IAM_USER_NAME)
        .policy_arn(policy_arn)
        .send()
        .await
        .map_err(|e| {
            ProvisionerError::Aws(format!("iam:AttachUserPolicy failed: {e}"))
        })?;

    tracing::info!(
        user = IAM_USER_NAME,
        policy_arn = policy_arn,
        "attached policy to user"
    );
    Ok(())
}

/// Create an access key pair for the Claria IAM user.
///
/// Returns `(access_key_id, secret_access_key)`.
async fn create_access_key(
    client: &aws_sdk_iam::Client,
) -> Result<(String, String), ProvisionerError> {
    let resp = client
        .create_access_key()
        .user_name(IAM_USER_NAME)
        .send()
        .await
        .map_err(|e| {
            ProvisionerError::Aws(format!("iam:CreateAccessKey failed: {e}"))
        })?;

    let ak = resp.access_key().ok_or_else(|| {
        ProvisionerError::Aws("CreateAccessKey returned no key".into())
    })?;

    let key_id = ak.access_key_id().to_string();
    let secret = ak.secret_access_key().to_string();

    tracing::info!(access_key_id = %key_id, "created access key for IAM user");
    Ok((key_id, secret))
}

/// Delete an access key from AWS.
///
/// When `user_name` is `None`, the key is assumed to belong to the caller
/// (i.e. the root user), so no `UserName` parameter is sent.
async fn delete_access_key(
    client: &aws_sdk_iam::Client,
    access_key_id: &str,
    user_name: Option<&str>,
) -> Result<(), ProvisionerError> {
    let mut req = client.delete_access_key().access_key_id(access_key_id);

    if let Some(name) = user_name {
        req = req.user_name(name);
    }

    req.send().await.map_err(|e| {
        ProvisionerError::Aws(format!("iam:DeleteAccessKey failed: {e}"))
    })?;

    tracing::info!(access_key_id = %access_key_id, "deleted access key from AWS");
    Ok(())
}

/// Build a temporary SDK config from the new IAM user's credentials and
/// verify they work. Retries up to 10 times with a 2-second backoff
/// because IAM credential propagation is eventually consistent.
async fn validate_new_credentials(
    access_key_id: &str,
    secret_access_key: &str,
    source_config: &aws_config::SdkConfig,
) -> Result<(), ProvisionerError> {
    let region = source_config
        .region()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());

    let new_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region))
        .credentials_provider(aws_sdk_sts::config::Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "claria-bootstrap",
        ))
        .load()
        .await;

    let sts = aws_sdk_sts::Client::new(&new_config);

    for attempt in 0..10 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        match sts.get_caller_identity().send().await {
            Ok(_) => {
                tracing::info!(
                    attempt,
                    "new IAM credentials validated successfully"
                );
                return Ok(());
            }
            Err(e) if attempt < 9 => {
                tracing::debug!(attempt, "new IAM credentials not yet active: {e}");
            }
            Err(e) => {
                return Err(ProvisionerError::Aws(format!(
                    "new credentials failed validation after 10 attempts: {e}"
                )));
            }
        }
    }

    // Unreachable, but satisfy the compiler.
    Err(ProvisionerError::Aws(
        "credential validation loop exited unexpectedly".into(),
    ))
}

// ── Step tracking helpers ────────────────────────────────────────────────────

fn push_step(
    steps: &mut Vec<BootstrapStep>,
    name: &str,
    status: StepStatus,
    detail: Option<String>,
) {
    steps.push(BootstrapStep {
        name: name.to_string(),
        status,
        detail,
    });
}

fn set_step_status(
    steps: &mut [BootstrapStep],
    name: &str,
    status: StepStatus,
    detail: Option<String>,
) {
    if let Some(step) = steps.iter_mut().rfind(|s| s.name == name) {
        step.status = status;
        step.detail = detail;
    }
}