//! The desktop side of the audit trail: when an event is recorded, and what
//! happens when recording it fails.

use aws_config::SdkConfig;
use claria_audit::events::AuditEvent;

/// Log an audit event and persist it to the durable trail in S3.
///
/// Deliberately infallible from the caller's point of view. The operation
/// being audited has already succeeded by the time this runs, so failing the
/// command here would throw away the user's actual work — a saved transcript,
/// a chat turn already paid for — because its receipt could not be filed.
///
/// A failed write is reported at `error` level instead, which puts it in the
/// Claria Console the user can open and export. The event's full payload is
/// still in that log, so a write failure degrades the trail rather than
/// erasing the event.
pub async fn record(sdk_config: &SdkConfig, bucket: &str, event: AuditEvent) {
    event.emit();

    if let Err(e) = claria_audit::sink::write_event(sdk_config, bucket, &event).await {
        tracing::error!(
            audit.event_id = %event.event_id,
            audit.action = %event.action,
            error = %e,
            "audit event was logged but not persisted to the audit trail"
        );
    }
}
