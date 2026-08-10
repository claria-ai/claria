use bytes::Bytes;

use crate::state::{
    AccessKeyRecord, BucketState, CallerIdentity, CostGroup, CostPeriod, FoundationModel,
    IamPolicy, IamPolicyVersion, IamUser, InferenceProfile, InferenceProfileModel, MockState,
    ModelLifecycle, ObjectVersion, PublicAccessBlock, Trail, VersioningStatus,
};

const ACCOUNT_ID: &str = "185735714230";
const SYSTEM_NAME: &str = "claria";

fn bucket_name() -> String {
    format!("{ACCOUNT_ID}-{SYSTEM_NAME}-data")
}

pub fn load(name: &str, state: &mut MockState) -> Result<(), String> {
    match name {
        "fresh-account" => fresh_account(state),
        "bootstrapped" => bootstrapped(state),
        "fully-provisioned" => fully_provisioned(state),
        "drifted" => drifted(state),
        _ => return Err(name.to_string()),
    }
    Ok(())
}

/// STS identity exists as root, nothing else provisioned.
fn fresh_account(state: &mut MockState) {
    state.caller_identity = CallerIdentity {
        account: ACCOUNT_ID.to_string(),
        arn: format!("arn:aws:iam::{ACCOUNT_ID}:root"),
        user_id: ACCOUNT_ID.to_string(),
    };
    load_bedrock_models(state);
}

/// IAM user + policy created, but no infrastructure yet.
fn bootstrapped(state: &mut MockState) {
    state.caller_identity = claria_admin_identity();
    create_iam_user(state);
    create_iam_policy(state);
    load_bedrock_models(state);
}

/// All 14 resources in sync, sample data in S3.
fn fully_provisioned(state: &mut MockState) {
    state.caller_identity = claria_admin_identity();
    create_iam_user(state);
    create_iam_policy(state);
    create_bucket_with_all_config(state);
    create_cloudtrail(state);
    accept_bedrock_models(state);
    load_bedrock_models(state);
    state.baa_accepted = true;
    load_sample_data(state);
    load_cost_data(state);
}

/// Like fully-provisioned but with versioning disabled and encryption missing.
fn drifted(state: &mut MockState) {
    fully_provisioned(state);
    let bucket = bucket_name();
    if let Some(b) = state.buckets.get_mut(&bucket) {
        b.versioning = VersioningStatus::Suspended;
        b.encryption_algorithm = None;
    }
}

// ── Building blocks ──

fn claria_admin_identity() -> CallerIdentity {
    CallerIdentity {
        account: ACCOUNT_ID.to_string(),
        arn: format!("arn:aws:iam::{ACCOUNT_ID}:user/claria-admin"),
        user_id: "AIDAMOCKUSERID0001".to_string(),
    }
}

fn create_iam_user(state: &mut MockState) {
    let user = IamUser {
        user_name: "claria-admin".to_string(),
        arn: format!("arn:aws:iam::{ACCOUNT_ID}:user/claria-admin"),
        user_id: "AIDAMOCKUSERID0001".to_string(),
        create_date: "2026-03-01T17:30:00Z".to_string(),
    };
    state.users.insert("claria-admin".to_string(), user);

    // Create an access key for this user
    let key = AccessKeyRecord {
        access_key_id: "AKIAMOCKKEY00000001".to_string(),
        secret_access_key: "mock-secret-key-00000001".to_string(),
        user_name: "claria-admin".to_string(),
        status: "Active".to_string(),
        create_date: "2026-03-01T17:30:00Z".to_string(),
        last_used_date: Some("2026-03-22T10:00:00Z".to_string()),
        last_used_service: Some("s3".to_string()),
    };
    state
        .access_keys
        .insert("AKIAMOCKKEY00000001".to_string(), key);
}

fn create_iam_policy(state: &mut MockState) {
    let policy_arn = format!("arn:aws:iam::{ACCOUNT_ID}:policy/ClariaProvisionerAccess");
    let document = serde_json::json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Effect": "Allow",
                "Action": [
                    "s3:HeadBucket", "s3:CreateBucket", "s3:DeleteBucket",
                    "s3:GetBucketVersioning", "s3:PutBucketVersioning",
                    "s3:GetEncryptionConfiguration", "s3:PutEncryptionConfiguration",
                    "s3:GetBucketPublicAccessBlock", "s3:PutBucketPublicAccessBlock",
                    "s3:GetBucketPolicy", "s3:PutBucketPolicy",
                    "s3:GetObject", "s3:GetObjectVersion", "s3:PutObject",
                    "s3:DeleteObject", "s3:ListBucket", "s3:ListBucketVersions"
                ],
                "Resource": [
                    format!("arn:aws:s3:::{}", bucket_name()),
                    format!("arn:aws:s3:::{}/*", bucket_name()),
                ]
            }
        ]
    })
    .to_string();

    let version = IamPolicyVersion {
        version_id: "v1".to_string(),
        document,
        is_default: true,
        create_date: "2026-03-01T17:30:00Z".to_string(),
    };

    let policy = IamPolicy {
        arn: policy_arn.clone(),
        policy_name: "ClariaProvisionerAccess".to_string(),
        description: "Minimal permissions for the Claria desktop app".to_string(),
        default_version_id: "v1".to_string(),
        versions: vec![version],
    };

    state.policies.insert(policy_arn.clone(), policy);
    state
        .user_attached_policies
        .entry("claria-admin".to_string())
        .or_default()
        .push(policy_arn);
}

fn create_bucket_with_all_config(state: &mut MockState) {
    let bucket = bucket_name();
    let tls_policy = serde_json::json!({
        "Version": "2012-10-17",
        "Statement": [{
            "Effect": "Deny",
            "Principal": "*",
            "Action": "s3:*",
            "Resource": [
                format!("arn:aws:s3:::{bucket}"),
                format!("arn:aws:s3:::{bucket}/*"),
            ],
            "Condition": { "Bool": { "aws:SecureTransport": "false" } }
        }]
    });

    state.buckets.insert(
        bucket,
        BucketState {
            region: "us-east-1".to_string(),
            versioning: VersioningStatus::Enabled,
            encryption_algorithm: Some("AES256".to_string()),
            public_access_block: Some(PublicAccessBlock {
                block_public_acls: true,
                ignore_public_acls: true,
                block_public_policy: true,
                restrict_public_buckets: true,
            }),
            policy: Some(tls_policy.to_string()),
        },
    );
}

fn create_cloudtrail(state: &mut MockState) {
    let trail = Trail {
        name: "claria-audit-trail".to_string(),
        trail_arn: format!("arn:aws:cloudtrail:us-east-1:{ACCOUNT_ID}:trail/claria-audit-trail"),
        s3_bucket_name: bucket_name(),
        s3_key_prefix: Some("_cloudtrail".to_string()),
        is_multi_region: false,
    };
    state.trails.insert("claria-audit-trail".to_string(), trail);
    state
        .trail_logging
        .insert("claria-audit-trail".to_string(), true);
}

fn load_bedrock_models(state: &mut MockState) {
    let models = [
        ("anthropic.claude-opus-4-6-20260301-v1:0", "Claude Opus 4.6"),
        ("anthropic.claude-sonnet-4-20250514-v1:0", "Claude Sonnet 4"),
        (
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            "Claude Haiku 4.5",
        ),
    ];

    state.foundation_models = models
        .iter()
        .map(|(id, name)| FoundationModel {
            model_id: id.to_string(),
            model_name: name.to_string(),
            provider_name: "Anthropic".to_string(),
            model_lifecycle: ModelLifecycle {
                status: "ACTIVE".to_string(),
            },
        })
        .collect();

    state.inference_profiles = models
        .iter()
        .map(|(id, name)| InferenceProfile {
            inference_profile_id: format!("us.{id}"),
            inference_profile_name: format!("US {name}"),
            r#type: "SYSTEM_DEFINED".to_string(),
            status: "ACTIVE".to_string(),
            models: vec![InferenceProfileModel {
                model_arn: format!("arn:aws:bedrock:us-east-1::foundation-model/{id}"),
            }],
        })
        .collect();
}

fn accept_bedrock_models(state: &mut MockState) {
    for model in &state.foundation_models {
        state.model_agreements.insert(model.model_id.clone());
    }
}

fn load_sample_data(state: &mut MockState) {
    let bucket = bucket_name();
    let client_id = "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb";

    // Client record
    let client_json = serde_json::json!({
        "id": client_id,
        "name": "Jane Doe",
        "created_at": "2026-02-15T10:00:00Z",
    });
    put_object(
        state,
        &bucket,
        &format!("clients/{client_id}.json"),
        serde_json::to_vec(&client_json).unwrap(),
        "application/json",
    );

    // Record files
    put_object(
        state,
        &bucket,
        &format!("records/{client_id}/intake-parent-interview.txt"),
        b"Jane Doe \xe2\x80\x94 Parent Interview, 2/15/2026\nReferral: Dr. Reyes (pediatrician)\nHomework takes 2-3 hours with frequent crying and refusal.".to_vec(),
        "text/plain",
    );
    put_object(
        state,
        &bucket,
        &format!("records/{client_id}/teacher-observation.txt"),
        b"Teacher behavioral checklist from Ms. Alvarado. Student is frequently off-task.".to_vec(),
        "text/plain",
    );

    // Provisioner state
    let prov_state = serde_json::json!({
        "manifest_version": 1,
        "resources": {}
    });
    put_object(
        state,
        &bucket,
        "_state/provisioner.json",
        serde_json::to_vec(&prov_state).unwrap(),
        "application/json",
    );
}

fn put_object(state: &mut MockState, bucket: &str, key: &str, body: Vec<u8>, content_type: &str) {
    let now = "2026-03-01T17:30:00Z".to_string();
    let version_id = if state
        .buckets
        .get(bucket)
        .is_some_and(|b| b.versioning == VersioningStatus::Enabled)
    {
        uuid::Uuid::new_v4().to_string()
    } else {
        "null".to_string()
    };

    let obj = ObjectVersion {
        version_id,
        body: Bytes::from(body),
        content_type: content_type.to_string(),
        etag: format!("{:x}", uuid::Uuid::new_v4().as_u128()),
        last_modified: now,
        is_delete_marker: false,
    };

    state
        .objects
        .entry((bucket.to_string(), key.to_string()))
        .or_default()
        .push(obj);
}

fn load_cost_data(state: &mut MockState) {
    let services = [
        ("Amazon Bedrock", 0.15_f64, 0.12_f64),
        ("Amazon Simple Storage Service", 0.035, 0.01),
        ("AWS CloudTrail", 0.04, 0.015),
        ("AWS Cost Explorer", 0.01, 0.01),
    ];

    let mut seed: u64 = 42;
    let mut rand = || -> f64 {
        seed = seed.wrapping_mul(16807).wrapping_add(0) % 2147483647;
        (seed as f64 - 1.0) / 2147483646.0
    };

    for i in (0..30).rev() {
        let date = jiff::civil::date(2026, 3, 3)
            .checked_sub(jiff::SignedDuration::from_hours(24 * i))
            .unwrap();
        let next = date
            .checked_add(jiff::SignedDuration::from_hours(24))
            .unwrap();
        let day_of_week = date.weekday();
        let is_weekend = day_of_week == jiff::civil::Weekday::Saturday
            || day_of_week == jiff::civil::Weekday::Sunday;
        let weekend_factor = if is_weekend { 0.3 } else { 1.0 };

        let groups = services
            .iter()
            .map(|(name, base, variance)| {
                let factor = if *name == "Amazon Bedrock" {
                    weekend_factor
                } else {
                    1.0
                };
                let amount = (base + (rand() - 0.4) * variance) * factor;
                let amount = amount.max(0.0);
                CostGroup {
                    key: name.to_string(),
                    amount: format!("{amount:.4}"),
                    unit: "USD".to_string(),
                }
            })
            .collect();

        state.cost_data.push(CostPeriod {
            start: date.to_string(),
            end: next.to_string(),
            groups,
        });
    }
}
