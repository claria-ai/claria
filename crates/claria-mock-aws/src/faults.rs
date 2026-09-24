//! Injected operation failures.
//!
//! [`MockState::operation_failures`](crate::state::MockState::operation_failures)
//! names operations that must answer with a service error instead of doing
//! their work. This module is the one place that turns such an entry into a
//! response, so the three wire protocols the mock speaks report a refusal the
//! way their real services do — an SDK that cannot find an error code in the
//! shape it expects hands the caller an unhandled error with no code at all,
//! and a test written against that proves nothing.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::{state::SharedState, xml};

/// How a service reports an error on the wire.
#[derive(Debug, Clone, Copy)]
pub enum Wire {
    /// S3: a bare `Error` root.
    RestXml,
    /// IAM and STS: `Error` nested inside an `ErrorResponse` envelope.
    Query,
    /// CloudTrail, Transcribe, Cost Explorer, Artifact: `__type` in JSON.
    Json,
}

/// The refusal response for `operation`, or `None` when no failure is injected.
///
/// Every injected failure answers 403 — the hook exists for reads the
/// credentials are not allowed to make, and a mock that let a test pick an
/// arbitrary status would invite tests that pass against a shape AWS never
/// sends.
pub async fn injected(operation: &str, wire: Wire, state: &SharedState) -> Option<Response> {
    let code = state
        .read()
        .await
        .operation_failures
        .get(operation)
        .cloned()?;

    let message = format!("User is not authorized to perform: {operation}");
    Some(match wire {
        Wire::RestXml => (StatusCode::FORBIDDEN, xml::error_xml(&code, &message)).into_response(),
        Wire::Query => {
            (StatusCode::FORBIDDEN, xml::query_error_xml(&code, &message)).into_response()
        }
        Wire::Json => (
            StatusCode::FORBIDDEN,
            [("content-type", "application/x-amz-json-1.1")],
            json!({"__type": code, "message": message}).to_string(),
        )
            .into_response(),
    })
}
