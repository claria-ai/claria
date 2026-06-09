//! CloudTrail `LookupEvents` reader.
//!
//! Returns the last 90 days of management events for the region the
//! supplied client is configured for. The API is free and decoupled
//! from the underlying S3 trail bucket — bucket retention does not
//! affect what `LookupEvents` returns.

use aws_sdk_cloudtrail::Client;
use aws_sdk_cloudtrail::types::{LookupAttribute, LookupAttributeKey};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::error::AuditError;

/// Filters for a single `list_events` call. All fields are optional;
/// omitted filters mean "no restriction on this dimension".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LookupQuery {
    pub start_time: Option<Timestamp>,
    pub end_time: Option<Timestamp>,
    pub event_name: Option<String>,
    pub event_source: Option<String>,
    pub username: Option<String>,
    pub read_only: Option<bool>,
    pub next_token: Option<String>,
    pub max_results: Option<i32>,
}

/// One page of CloudTrail events plus the continuation token, if any.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupResult {
    pub events: Vec<LookedUpEvent>,
    pub next_token: Option<String>,
}

/// A flattened CloudTrail event. The raw payload is preserved as a string
/// in `cloudtrail_event_json` for callers that want to render the full event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookedUpEvent {
    pub event_id: Option<String>,
    pub event_time: Option<Timestamp>,
    pub event_name: Option<String>,
    pub event_source: Option<String>,
    pub username: Option<String>,
    pub read_only: Option<bool>,
    pub access_key_id: Option<String>,
    pub resources: Vec<LookedUpResource>,
    pub cloudtrail_event_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookedUpResource {
    pub resource_type: Option<String>,
    pub resource_name: Option<String>,
}

const PAGE_SIZE_CAP: i32 = 50;

pub async fn list_events(
    client: &Client,
    query: LookupQuery,
) -> Result<LookupResult, AuditError> {
    let mut req = client.lookup_events();

    for attr in build_attributes(&query) {
        req = req.lookup_attributes(attr);
    }

    if let Some(start) = query.start_time {
        req = req.start_time(timestamp_to_smithy(start));
    }
    if let Some(end) = query.end_time {
        req = req.end_time(timestamp_to_smithy(end));
    }
    if let Some(token) = query.next_token {
        req = req.next_token(token);
    }
    req = req.max_results(query.max_results.unwrap_or(PAGE_SIZE_CAP).min(PAGE_SIZE_CAP));

    let resp = req
        .send()
        .await
        .map_err(|e| AuditError::LookupEvents(e.into_service_error().to_string()))?;

    let next_token = resp.next_token().map(|s| s.to_string());
    let events = resp.events().iter().map(flatten_event).collect();

    Ok(LookupResult { events, next_token })
}

fn build_attributes(query: &LookupQuery) -> Vec<LookupAttribute> {
    let mut attrs = Vec::new();

    if let Some(name) = &query.event_name {
        attrs.push(attr(LookupAttributeKey::EventName, name));
    }
    if let Some(source) = &query.event_source {
        attrs.push(attr(LookupAttributeKey::EventSource, source));
    }
    if let Some(username) = &query.username {
        attrs.push(attr(LookupAttributeKey::Username, username));
    }
    if let Some(read_only) = query.read_only {
        attrs.push(attr(
            LookupAttributeKey::ReadOnly,
            if read_only { "true" } else { "false" },
        ));
    }

    attrs
}

fn attr(key: LookupAttributeKey, value: &str) -> LookupAttribute {
    LookupAttribute::builder()
        .attribute_key(key)
        .attribute_value(value)
        .build()
        .expect("LookupAttribute fields are both set")
}

fn flatten_event(event: &aws_sdk_cloudtrail::types::Event) -> LookedUpEvent {
    LookedUpEvent {
        event_id: event.event_id().map(str::to_string),
        event_time: event.event_time().and_then(smithy_to_timestamp),
        event_name: event.event_name().map(str::to_string),
        event_source: event.event_source().map(str::to_string),
        username: event.username().map(str::to_string),
        read_only: event.read_only().and_then(|s| s.parse::<bool>().ok()),
        access_key_id: event.access_key_id().map(str::to_string),
        resources: event
            .resources()
            .iter()
            .map(|r| LookedUpResource {
                resource_type: r.resource_type().map(str::to_string),
                resource_name: r.resource_name().map(str::to_string),
            })
            .collect(),
        cloudtrail_event_json: event.cloud_trail_event().map(str::to_string),
    }
}

fn timestamp_to_smithy(ts: Timestamp) -> aws_smithy_types::DateTime {
    aws_smithy_types::DateTime::from_secs_and_nanos(ts.as_second(), ts.subsec_nanosecond() as u32)
}

fn smithy_to_timestamp(dt: &aws_smithy_types::DateTime) -> Option<Timestamp> {
    Timestamp::new(dt.secs(), dt.subsec_nanos() as i32).ok()
}
