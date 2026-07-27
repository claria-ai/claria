//! `AuditEvent::emit` must put the `details` payload on the wire. It used to
//! log the action, resource and user and silently drop the payload that every
//! call site went to the trouble of building.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use claria_audit::events::AuditEvent;
use tracing::field::{Field, Visit};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

type Fields = Arc<Mutex<Vec<(String, String)>>>;

/// Mirrors how `claria-desktop`'s console layer reads events: fields arrive
/// through `record_str`/`record_debug` and are rendered verbatim, so anything
/// asserted here is what actually reaches the exported log.
struct CaptureLayer(Fields);

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
        let mut captured = self.0.lock().expect("capture lock");
        event.record(&mut FieldCollector(&mut captured));
    }
}

fn capture(event: &AuditEvent) -> Vec<(String, String)> {
    let fields: Fields = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureLayer(fields.clone()));
    tracing::subscriber::with_default(subscriber, || event.emit());
    fields.lock().expect("capture lock").clone()
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("field {name} missing from {fields:?}"))
}

#[test]
fn emit_records_the_details_payload_as_json() {
    let event = AuditEvent::new("chat_message", "client", "abc", "123456789012").with_details(
        serde_json::json!({
            "input_tokens": 1234,
            "cost_usd": 0.0042,
            "model_id": "anthropic.claude-sonnet-4",
        }),
    );

    let fields = capture(&event);
    let details = field(&fields, "audit.details");

    // Real JSON, not a Rust debug rendering — an auditor grepping the exported
    // log can parse this line.
    let parsed: serde_json::Value = serde_json::from_str(details).expect("details parse as JSON");
    assert_eq!(parsed["input_tokens"], 1234);
    assert_eq!(parsed["cost_usd"], 0.0042);
    assert_eq!(parsed["model_id"], "anthropic.claude-sonnet-4");
}

#[test]
fn emit_records_identity_timestamp_and_the_original_fields() {
    let event = AuditEvent::new("extract_document_text", "record_file", "scan.pdf", "acct");
    let fields = capture(&event);

    assert_eq!(field(&fields, "audit.action"), "extract_document_text");
    assert_eq!(field(&fields, "audit.resource_type"), "record_file");
    assert_eq!(field(&fields, "audit.resource_id"), "scan.pdf");
    assert_eq!(field(&fields, "audit.user_sub"), "acct");
    assert_eq!(field(&fields, "audit.event_id"), event.event_id.to_string());
    assert_eq!(
        field(&fields, "audit.timestamp"),
        event.timestamp.to_string()
    );
}

#[test]
fn emit_records_an_empty_object_when_there_are_no_details() {
    let event = AuditEvent::new("infra_chat", "infrastructure", "infra", "acct");
    assert_eq!(field(&capture(&event), "audit.details"), "{}");
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
