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
    /// Keys that `DeleteObjects` refuses to delete, reporting a per-object
    /// `AccessDenied` in the 200 response instead. A test hook: S3 signals
    /// partial batch failure inside a success response, and that is the one
    /// shape a caller can silently mistake for "everything deleted".
    pub s3_delete_object_failures: HashSet<String>,
    /// Object count of every `DeleteObjects` request received, in arrival
    /// order. Tests read this to tell a batched delete from a per-object one.
    pub s3_delete_objects_batches: Vec<usize>,
    /// Number of GET Object requests received for each key.
    pub s3_get_object_requests: HashMap<String, usize>,
    /// Keys whose GET Object operation returns an injected service failure.
    pub s3_get_object_failures: HashSet<String>,
    /// Fault injection: conditional PUTs for a key return 409 this many times.
    pub s3_conditional_conflicts_remaining: HashMap<String, u32>,
    /// Fault injection: conditional PUTs for a key return 412 this many times.
    pub s3_precondition_failures_remaining: HashMap<String, u32>,

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
    /// FIFO responses consumed only by Converse requests that carry
    /// `toolConfig`. Ordinary chat/extraction requests keep their existing
    /// canned behavior.
    pub bedrock_tool_responses: Vec<ScriptedBedrockResponse>,
    /// FIFO responses for plain (no `toolConfig`) Converse requests. When
    /// empty, the canned chat/extraction response is returned.
    pub bedrock_text_responses: Vec<ScriptedBedrockResponse>,
    /// Raw plain Converse request bodies, in wire order. `ConverseStream`
    /// requests land here too — the streaming endpoint shares the plain
    /// script FIFO and canned response, delivered as event-stream frames.
    pub bedrock_text_requests: Vec<serde_json::Value>,
    /// How many of the captured plain requests arrived via `ConverseStream`.
    pub bedrock_stream_request_count: usize,
    /// Fault injection: the next N `ConverseStream` responses send an
    /// opening frame and one text delta, then hold the connection open
    /// forever without finishing. Reproduces a socket that dies
    /// mid-generation — the one failure the AWS SDK does not cover, since
    /// the generated streaming operations register no stalled-stream
    /// protection interceptor. Each stalled response decrements the
    /// counter, so retry paths can be scripted to stall then recover.
    /// A stalled response still consumes its scripted payload.
    pub bedrock_stream_stalls: u32,
    /// Fault injection: once set, every `ConverseStream` request after the
    /// Nth stalls the same way [`Self::bedrock_stream_stalls`] does, and goes
    /// on doing so. The counter above fires on whichever response comes next,
    /// which a test cannot aim at a particular call without racing the loop
    /// it is testing; this pins the stall to a chosen point in a scripted
    /// conversation — say, the call that would follow the second landed
    /// section.
    pub bedrock_stream_stalls_after: Option<usize>,
    /// Fault injection: the next N `ConverseStream` responses send an
    /// opening frame and one text delta, then sever the connection with a
    /// body error. Reproduces a socket that resets mid-generation, and —
    /// unlike a stall — fails immediately, so retry paths can be exercised
    /// in real time. Checked after `bedrock_stream_stalls`; a dropped
    /// response still consumes its scripted payload.
    pub bedrock_stream_drops: u32,
    /// Raw tool-configured Converse request bodies, in wire order.
    pub bedrock_tool_requests: Vec<serde_json::Value>,
    /// Decoded model IDs for the captured tool-configured requests.
    pub bedrock_tool_model_ids: Vec<String>,
    /// Raw CountTokens requests for report inputs.
    pub bedrock_count_token_requests: Vec<serde_json::Value>,
    /// Decoded model IDs for the captured report CountTokens requests.
    pub bedrock_count_token_model_ids: Vec<String>,
    /// Optional deterministic token count returned by the mock.
    pub bedrock_count_tokens_override: Option<u32>,
    /// Bare model IDs that return CountTokens ValidationException while still
    /// allowing Converse, matching newly launched Bedrock models.
    pub bedrock_count_tokens_unsupported_models: HashSet<String>,

    // Transcribe
    pub transcription_jobs: HashMap<String, TranscriptionJob>,
    /// Every StartTranscriptionJob / StartMedicalTranscriptionJob body the mock
    /// has received, in arrival order. Tests inspect this to assert on the
    /// exact request shape (e.g. `IdentifyMultipleLanguages`, `LanguageOptions`,
    /// `Settings`) the SDK produced.
    pub transcribe_requests: Vec<RecordedTranscribeRequest>,
    /// Pre-loaded transcript JSON to write to S3 as the next job's result.
    /// When set, the mock pops from the head of this queue on each
    /// StartTranscriptionJob call instead of using the hardcoded English stub.
    /// Tests use this as a "cassette" — a recorded AWS Transcribe response
    /// for the input audio file.
    pub transcribe_response_cassette: Vec<serde_json::Value>,

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptedBedrockResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

impl ScriptedBedrockResponse {
    pub fn success(body: serde_json::Value) -> Self {
        Self { status: 200, body }
    }

    pub fn error(status: u16, body: serde_json::Value) -> Self {
        Self { status, body }
    }
}

// ── Transcribe types ──

#[derive(Debug, Clone)]
pub struct RecordedTranscribeRequest {
    /// `"StartTranscriptionJob"` or `"StartMedicalTranscriptionJob"`.
    pub operation: String,
    pub body: serde_json::Value,
}

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
    /// Look up the current object. A latest delete marker makes the key
    /// absent even though older data versions remain restorable.
    pub fn get_latest_object(&self, bucket: &str, key: &str) -> Option<&ObjectVersion> {
        self.objects
            .get(&(bucket.to_string(), key.to_string()))
            .and_then(|versions| versions.last())
            .filter(|version| !version.is_delete_marker)
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
            .and_then(|versions| versions.iter().find(|v| v.version_id == version_id))
    }

    /// List all keys in a bucket under a given prefix (latest version, not deleted).
    pub fn list_keys(&self, bucket: &str, prefix: &str) -> BTreeMap<String, &ObjectVersion> {
        let mut result = BTreeMap::new();
        for ((b, k), versions) in &self.objects {
            if b != bucket || !k.starts_with(prefix) {
                continue;
            }
            if let Some(latest) = versions.last().filter(|version| !version.is_delete_marker) {
                result.insert(k.clone(), latest);
            }
        }
        result
    }
}
