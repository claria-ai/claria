//! Application audit events and the durable S3 sink they are written to.
//!
//! This lives in `claria-storage` because the trail is nothing but S3 writes
//! and this crate owns those; every consumer of the events (the desktop's
//! never-fail `audit::record` wrapper) already depends on storage.
//!
//! S3 has no append, so the trail is one immutable object per event rather
//! than a growing log file. Events are human-paced (a chat turn, a document
//! extraction), so the per-event PUT is cheap next to the Bedrock call that
//! produced it, and nothing is ever buffered in memory waiting to be written —
//! a record either exists in S3 or the write failed loudly.

use aws_config::SdkConfig;
use claria_core::s3_keys;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;

/// A structured, application-level audit event.
///
/// CloudTrail records AWS API calls; these events record what the application
/// was *doing* — which record was extracted, what a chat turn cost, which
/// model answered it. Every event carries its own identity and UTC timestamp
/// so the persisted object is self-describing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub timestamp: Timestamp,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub user_sub: String,
    pub details: Option<serde_json::Value>,
}

impl AuditEvent {
    pub fn new(
        action: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        user_sub: impl Into<String>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Timestamp::now(),
            action: action.into(),
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            user_sub: user_sub.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Mirror this audit event into the tracing stream as a one-line summary.
    ///
    /// Deliberately terse: the durable S3 object carries the full payload —
    /// the details JSON and resource identifiers that may be client-chosen
    /// filenames. The tracing mirror only records that the event happened,
    /// because the console export is a support artifact in a HIPAA app: a
    /// `resource_id` is logged only when it is UUID-shaped, and `details`
    /// never is. The dedicated target keeps the trail greppable.
    pub fn emit(&self) {
        let resource_id = if self.resource_id.parse::<Uuid>().is_ok() {
            self.resource_id.as_str()
        } else {
            "<redacted>"
        };

        // `event!` rather than `info!`: tracing 0.1.44's `info!` macro cannot
        // parse `target:` together with dotted field names.
        tracing::event!(
            target: "claria_storage::audit::trail",
            tracing::Level::INFO,
            audit.event_id = %self.event_id,
            audit.action = %self.action,
            audit.resource_type = %self.resource_type,
            audit.resource_id = resource_id,
            "audit event"
        );
    }
}

/// Write one audit event to S3 as its own object. Returns the key written.
///
/// The key embeds the event's own timestamp, so retries and out-of-order
/// writes still land in the right time slice.
#[tracing::instrument(level = "trace", skip_all, fields(bucket = %bucket, action = %event.action))]
pub async fn write_event(
    sdk_config: &SdkConfig,
    bucket: &str,
    event: &AuditEvent,
) -> Result<String, StorageError> {
    let key = s3_keys::audit_event(event.timestamp, event.event_id);
    let body = serde_json::to_vec(event)?;

    let s3 = crate::client::from_config(sdk_config);
    crate::objects::put_object(&s3, bucket, &key, body, Some("application/json")).await?;

    Ok(key)
}

/// Read every audit event recorded on `date` (UTC), oldest first.
///
/// The day prefix is what makes "what happened on this date" a single
/// `ListObjectsV2`, and the key layout makes S3's listing order chronological,
/// so no sorting is needed after the fact. The per-event GETs fan out with
/// bounded concurrency; `buffered` yields them back in listing order.
pub async fn read_day(
    sdk_config: &SdkConfig,
    bucket: &str,
    date: jiff::civil::Date,
) -> Result<Vec<AuditEvent>, StorageError> {
    use futures::stream::StreamExt;

    let s3 = crate::client::from_config(sdk_config);
    let prefix = s3_keys::audit_day_prefix(date);
    let keys = crate::objects::list_objects(&s3, bucket, &prefix).await?;

    let s3 = &s3;
    let fetches = keys.iter().map(|key| async move {
        let object = crate::objects::get_object(s3, bucket, key).await?;
        serde_json::from_slice::<AuditEvent>(&object.body).map_err(StorageError::from)
    });
    futures::stream::iter(fetches)
        .buffered(crate::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<Result<AuditEvent, StorageError>>>()
        .await
        .into_iter()
        .collect()
}
