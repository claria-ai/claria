//! `AuditEvent::emit` is a one-line tracing mirror of the durable S3 event.
//! It must never carry the details payload or a non-UUID resource id — the
//! console export is a support artifact in a HIPAA app, and record filenames
//! or per-event detail JSON do not belong in it. The full payload lives only
//! in the S3 object.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use claria_storage::audit::AuditEvent;
use tracing::field::{Field, Visit};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

type Captured = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

/// Mirrors how `claria-desktop`'s console layer reads events: fields arrive
/// through `record_str`/`record_debug` and are rendered verbatim, so anything
/// asserted here is what actually reaches the exported log.
struct CaptureLayer(Captured);

struct FieldCollector<'a>(&'a mut Vec<(String, String)>);

impl Visit for FieldCollector<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .push((field.name().to_string(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = Vec::new();
        event.record(&mut FieldCollector(&mut fields));
        self.0
            .lock()
            .expect("capture lock")
            .push((event.metadata().target().to_string(), fields));
    }
}

fn capture(event: &AuditEvent) -> (String, Vec<(String, String)>) {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureLayer(captured.clone()));
    tracing::subscriber::with_default(subscriber, || event.emit());
    let mut events = captured.lock().expect("capture lock").clone();
    assert_eq!(events.len(), 1, "emit produced {events:?}");
    events.remove(0)
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("field {name} missing from {fields:?}"))
}

#[test]
fn emit_is_a_summary_without_the_details_payload() {
    let event = AuditEvent::new(
        "chat_message",
        "client",
        uuid::Uuid::nil().to_string(),
        "123456789012",
    )
    .with_details(serde_json::json!({
        "input_tokens": 1234,
        "cost_usd": 0.0042,
        "model_id": "anthropic.claude-sonnet-4",
    }));

    let (target, fields) = capture(&event);

    assert_eq!(target, "claria_storage::audit::trail");
    assert_eq!(field(&fields, "audit.action"), "chat_message");
    assert_eq!(field(&fields, "audit.resource_type"), "client");
    assert_eq!(field(&fields, "audit.event_id"), event.event_id.to_string());
    assert!(
        !fields.iter().any(|(k, _)| k == "audit.details"),
        "details payload leaked into the tracing mirror: {fields:?}"
    );
    assert!(
        !fields.iter().any(|(_, v)| v.contains("1234")),
        "details values leaked into the tracing mirror: {fields:?}"
    );
}

#[test]
fn emit_keeps_uuid_resource_ids_and_redacts_the_rest() {
    let id = uuid::Uuid::new_v4();
    let (_, fields) = capture(&AuditEvent::new(
        "delete_client",
        "client",
        id.to_string(),
        "acct",
    ));
    assert_eq!(field(&fields, "audit.resource_id"), id.to_string());

    // A filename resource_id (legitimate in the durable trail) must not
    // reach the log line.
    let (_, fields) = capture(&AuditEvent::new(
        "extract_document_text",
        "record_file",
        "Jane Doe scan.pdf",
        "acct",
    ));
    assert_eq!(field(&fields, "audit.resource_id"), "<redacted>");
}

#[test]
fn events_round_trip_through_json() {
    let event = AuditEvent::new("chat_message", "client", "abc", "acct")
        .with_details(serde_json::json!({ "output_tokens": 7 }));

    let encoded = serde_json::to_string(&event).expect("serialize");
    let decoded: AuditEvent = serde_json::from_str(&encoded).expect("deserialize");

    assert_eq!(decoded.event_id, event.event_id);
    assert_eq!(decoded.timestamp, event.timestamp);
    assert_eq!(decoded.action, event.action);
    assert_eq!(decoded.details, event.details);
    // snake_case on the wire, per the project's serialization rule.
    assert!(encoded.contains("\"resource_type\""), "{encoded}");
}
