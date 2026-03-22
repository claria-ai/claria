use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub type SharedState = Arc<RwLock<MockState>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(RwLock::new(MockState::default()))
}

#[derive(Debug, Default)]
pub struct MockState {
    // S3
    pub buckets: HashMap<String, BucketState>,
    /// (bucket, key) → version stack (newest last).
    pub objects: HashMap<(String, String), Vec<ObjectVersion>>,

    // IAM
    pub users: HashMap<String, IamUser>,
    /// policy ARN → policy record
    pub policies: HashMap<String, IamPolicy>,
    /// user name → attached policy ARNs
    pub user_attached_policies: HashMap<String, Vec<String>>,
    /// (user, policy_name) → policy document JSON
    pub user_inline_policies: HashMap<(String, String), String>,
    /// access_key_id → record
    pub access_keys: HashMap<String, AccessKeyRecord>,

    // STS
    pub caller_identity: CallerIdentity,

    // CloudTrail
    pub trails: HashMap<String, Trail>,
    pub trail_logging: HashMap<String, bool>,

    // Bedrock
    pub foundation_models: Vec<FoundationModel>,
    pub model_agreements: HashSet<String>,
    pub inference_profiles: Vec<InferenceProfile>,

    // Transcribe
    pub transcription_jobs: HashMap<String, TranscriptionJob>,

    // Cost Explorer
    pub cost_data: Vec<CostPeriod>,

    // Artifact
    pub baa_accepted: bool,
}

// ── S3 types ──

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct BucketState {
    pub region: String,
    pub versioning: VersioningStatus,
    pub encryption_algorithm: Option<String>,
    pub public_access_block: Option<PublicAccessBlock>,
    pub policy: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum VersioningStatus {
    #[default]
    Unset,
    Enabled,
    Suspended,
}

#[derive(Debug, Clone)]
pub struct PublicAccessBlock {
    pub block_public_acls: bool,
    pub ignore_public_acls: bool,
    pub block_public_policy: bool,
    pub restrict_public_buckets: bool,
}

#[derive(Debug, Clone)]
pub struct ObjectVersion {
    pub version_id: String,
    pub body: bytes::Bytes,
    pub content_type: String,
    pub etag: String,
    pub last_modified: String,
    pub is_delete_marker: bool,
}

// ── IAM types ──

#[derive(Debug, Clone)]
pub struct IamUser {
    pub user_name: String,
    pub arn: String,
    pub user_id: String,
    pub create_date: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IamPolicy {
    pub arn: String,
    pub policy_name: String,
    pub description: String,
    pub versions: Vec<IamPolicyVersion>,
    pub default_version_id: String,
}

#[derive(Debug, Clone)]
pub struct IamPolicyVersion {
    pub version_id: String,
    pub document: String,
    pub is_default: bool,
    pub create_date: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AccessKeyRecord {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub user_name: String,
    pub status: String,
    pub create_date: String,
    pub last_used_date: Option<String>,
    pub last_used_service: Option<String>,
}

// ── STS types ──

#[derive(Debug, Clone)]
pub struct CallerIdentity {
    pub account: String,
    pub arn: String,
    pub user_id: String,
}

impl Default for CallerIdentity {
    fn default() -> Self {
        Self {
            account: "123456789012".to_string(),
            arn: "arn:aws:iam::123456789012:root".to_string(),
            user_id: "123456789012".to_string(),
        }
    }
}

// ── CloudTrail types ──

#[derive(Debug, Clone)]
pub struct Trail {
    pub name: String,
    pub trail_arn: String,
    pub s3_bucket_name: String,
    pub s3_key_prefix: Option<String>,
    pub is_multi_region: bool,
}

// ── Bedrock types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundationModel {
    pub model_id: String,
    pub model_name: String,
    pub provider_name: String,
    pub model_lifecycle: ModelLifecycle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLifecycle {
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceProfile {
    pub inference_profile_id: String,
    pub inference_profile_name: String,
    pub r#type: String,
    pub status: String,
    pub models: Vec<InferenceProfileModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceProfileModel {
    pub model_arn: String,
}

// ── Transcribe types ──

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TranscriptionJob {
    pub job_name: String,
    pub status: String,
    pub media_uri: String,
    pub output_bucket: String,
    pub output_key: String,
    pub language_code: String,
}

// ── Cost Explorer types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPeriod {
    pub start: String,
    pub end: String,
    pub groups: Vec<CostGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostGroup {
    pub key: String,
    pub amount: String,
    pub unit: String,
}

// ── State helpers ──

impl MockState {
    /// Look up the latest non-deleted version of an object.
    pub fn get_latest_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Option<&ObjectVersion> {
        self.objects
            .get(&(bucket.to_string(), key.to_string()))
            .and_then(|versions| {
                versions.iter().rev().find(|v| !v.is_delete_marker)
            })
    }

    /// Get a specific version of an object.
    pub fn get_object_version(
        &self,
        bucket: &str,
        key: &str,
        version_id: &str,
    ) -> Option<&ObjectVersion> {
        self.objects
            .get(&(bucket.to_string(), key.to_string()))
            .and_then(|versions| {
                versions.iter().find(|v| v.version_id == version_id)
            })
    }

    /// List all keys in a bucket under a given prefix (latest version, not deleted).
    pub fn list_keys(&self, bucket: &str, prefix: &str) -> BTreeMap<String, &ObjectVersion> {
        let mut result = BTreeMap::new();
        for ((b, k), versions) in &self.objects {
            if b != bucket || !k.starts_with(prefix) {
                continue;
            }
            if let Some(latest) = versions.iter().rev().find(|v| !v.is_delete_marker) {
                result.insert(k.clone(), latest);
            }
        }
        result
    }
}
