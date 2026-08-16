//! Durable attempt metadata and per-call usage receipts.

use aws_sdk_s3::Client as S3Client;
use claria_bedrock::report;
use claria_core::models::turn_usage::TurnUsage;
use claria_storage::error::StorageError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ReportFailureCode, turn::AttemptProgress};

// v2 adds stop_reason, latency_ms, max_output_tokens, system_prompt_sha256,
// and app_version to ReportCallUsageRecord; v1 records deserialize with the
// new fields defaulted.
pub(crate) const ATTEMPT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportAttemptStatus {
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportAttemptMetadata {
    pub schema_version: u32,
    pub attempt_id: Uuid,
    pub report_id: Uuid,
    pub client_id: Uuid,
    pub model_id: String,
    pub status: ReportAttemptStatus,
    pub failure_code: Option<ReportFailureCode>,
    pub usage: TurnUsage,
    pub usage_complete: bool,
    pub converse_calls: u32,
    pub tool_uses: u32,
    pub started_at: jiff::Timestamp,
    pub completed_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportCallUsageRecord {
    pub schema_version: u32,
    pub attempt_id: Uuid,
    pub report_id: Uuid,
    pub client_id: Uuid,
    pub model_id: String,
    pub call_number: u32,
    pub usage: Option<TurnUsage>,
    pub usage_complete: bool,
    pub recorded_at: jiff::Timestamp,
    /// Wire stop reason of this Converse call (schema v2).
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// Wall-clock duration of this Converse call (schema v2).
    #[serde(default)]
    pub latency_ms: Option<u64>,
    /// The enforced output ceiling in effect for this call (schema v2).
    #[serde(default)]
    pub max_output_tokens: u32,
    /// SHA-256 of the system prompt in effect, so quality investigations can
    /// tell which prompt produced a turn without storing the prompt text
    /// (schema v2).
    #[serde(default)]
    pub system_prompt_sha256: String,
    /// Workspace crates version in lockstep with the desktop, so this is the
    /// app version that ran the call (schema v2).
    #[serde(default)]
    pub app_version: Option<String>,
}

pub(crate) async fn persist_call_usage(
    s3: &S3Client,
    bucket: &str,
    progress: &AttemptProgress,
    call_number: u32,
    usage: Option<TurnUsage>,
    stop_reason: claria_bedrock::report::ReportStopReason,
    latency_ms: Option<u64>,
) -> Result<(), StorageError> {
    let record = ReportCallUsageRecord {
        schema_version: ATTEMPT_SCHEMA_VERSION,
        attempt_id: progress.attempt_id,
        report_id: progress.report_id,
        client_id: progress.client_id,
        model_id: progress.model_id.clone(),
        call_number,
        usage_complete: usage.is_some(),
        usage,
        recorded_at: jiff::Timestamp::now(),
        stop_reason: Some(stop_reason.as_str().to_string()),
        latency_ms,
        max_output_tokens: report::REPORT_OUTPUT_TOKEN_RESERVE,
        system_prompt_sha256: progress.system_prompt_sha256.clone(),
        // Workspace crates version in lockstep with the desktop binary.
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };
    let key = claria_core::s3_keys::report_call_usage(
        progress.client_id,
        progress.attempt_id,
        call_number,
    );
    claria_storage::state::save_state_if_none_match(s3, bucket, &key, &record)
        .await
        .map(|_| ())
}

pub(crate) async fn persist_attempt(
    s3: &S3Client,
    bucket: &str,
    metadata: &ReportAttemptMetadata,
) -> Result<(), StorageError> {
    let key = claria_core::s3_keys::report_attempt(metadata.client_id, metadata.attempt_id);
    claria_storage::state::save_state_if_none_match(s3, bucket, &key, metadata)
        .await
        .map(|_| ())
}
