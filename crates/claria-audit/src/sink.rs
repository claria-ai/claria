//! Durable S3 sink for application audit events.
//!
//! S3 has no append, so the trail is one immutable object per event rather
//! than a growing log file. Events are human-paced (a chat turn, a document
//! extraction), so the per-event PUT is cheap next to the Bedrock call that
//! produced it, and nothing is ever buffered in memory waiting to be written —
//! a record either exists in S3 or the write failed loudly.

use aws_config::SdkConfig;
use claria_core::s3_keys;

use crate::{error::AuditError, events::AuditEvent};

/// Write one audit event to S3 as its own object. Returns the key written.
///
/// The key embeds the event's own timestamp, so retries and out-of-order
/// writes still land in the right time slice.
#[tracing::instrument(level = "trace", skip_all, fields(bucket = %bucket, action = %event.action))]
pub async fn write_event(
    sdk_config: &SdkConfig,
    bucket: &str,
    event: &AuditEvent,
) -> Result<String, AuditError> {
    let key = s3_keys::audit_event(event.timestamp, event.event_id);
    let body = serde_json::to_vec(event)?;

    let s3 = claria_storage::client::from_config(sdk_config);
    claria_storage::objects::put_object(&s3, bucket, &key, body, Some("application/json")).await?;

    Ok(key)
}

/// Read every audit event recorded on `date` (UTC), oldest first.
///
/// The day prefix is what makes "what happened on this date" a single
/// `ListObjectsV2`, and the key layout makes S3's listing order chronological,
/// so no sorting is needed after the fact.
pub async fn read_day(
    sdk_config: &SdkConfig,
    bucket: &str,
    date: jiff::civil::Date,
) -> Result<Vec<AuditEvent>, AuditError> {
    let s3 = claria_storage::client::from_config(sdk_config);
    let prefix = s3_keys::audit_day_prefix(date);
    let keys = claria_storage::objects::list_objects(&s3, bucket, &prefix).await?;

    let mut events = Vec::with_capacity(keys.len());
    for key in &keys {
        let object = claria_storage::objects::get_object(&s3, bucket, key).await?;
        events.push(serde_json::from_slice(&object.body)?);
    }

    Ok(events)
}
