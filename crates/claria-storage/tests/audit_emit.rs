//! `AuditEvent::emit` is a one-line tracing mirror of the durable S3 event,
//! and its field list is a fixed allow-list rather than a filter.
//!
//! Schema v2 moved the boundary: `details` is console-safe by contract and is
//! emitted, while `resource_id` and `phi` are dropped wholesale — the former
//! because it can hold a filename, the latter because it is where free text
//! is required to live. The console export is a support artifact in a HIPAA
//! app; the full payload lives only in the S3 object.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use claria_storage::audit::{AuditEvent, EventCategory, actions};
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
fn emit_carries_the_console_safe_subset() {
    let client = uuid::Uuid::new_v4();
    let event = AuditEvent::new(
        actions::CHAT_TURN,
        "client",
        uuid::Uuid::nil().to_string(),
        "123456789012",
    )
    .with_client_id(client)
    .with_details(serde_json::json!({
        "input_tokens": 1234,
        "cost_usd": 0.0042,
        "model_id": "anthropic.claude-sonnet-4",
    }));

    let (target, fields) = capture(&event);

    assert_eq!(target, "claria_storage::audit::trail");
    assert_eq!(field(&fields, "audit.action"), "chat.turn");
    assert_eq!(field(&fields, "audit.category"), "ai");
    assert_eq!(field(&fields, "audit.resource_type"), "client");
    assert_eq!(field(&fields, "audit.client_id"), client.to_string());
    assert_eq!(field(&fields, "audit.event_id"), event.event_id.to_string());

    // Cost attribution is the reason details is console-safe: an exported log
    // has to be able to answer "what did this turn cost".
    let details = field(&fields, "audit.details");
    assert!(details.contains("1234"), "{details}");
    assert!(details.contains("anthropic.claude-sonnet-4"), "{details}");
}

#[test]
fn emit_never_carries_the_resource_id() {
    // Even a UUID-shaped resource id stays out. A uniform rule leaves no
    // per-event judgment for a future call site to get wrong.
    let (_, fields) = capture(&AuditEvent::new(
        actions::CLIENT_DELETE,
        "client",
        uuid::Uuid::new_v4().to_string(),
        "acct",
    ));
    assert!(
        !fields.iter().any(|(k, _)| k == "audit.resource_id"),
        "resource_id leaked into the tracing mirror: {fields:?}"
    );

    // A filename resource id is legitimate in the durable trail and must not
    // reach the log line in any form.
    let (_, fields) = capture(&AuditEvent::new(
        actions::RECORD_EXTRACT_TEXT,
        "record_file",
        "Jane Doe scan.pdf",
        "acct",
    ));
    assert!(
        !fields.iter().any(|(_, v)| v.contains("Jane Doe")),
        "a filename reached the tracing mirror: {fields:?}"
    );
}

#[test]
fn emit_never_carries_the_phi_payload() {
    let event = AuditEvent::new(actions::RECORD_SEARCH, "client", "abc", "acct")
        .with_details(serde_json::json!({ "match_count": 3 }))
        .with_phi(serde_json::json!({ "query": "Jane Doe custody evaluation" }));

    let (_, fields) = capture(&event);

    assert!(
        !fields.iter().any(|(k, _)| k == "audit.phi"),
        "phi field leaked into the tracing mirror: {fields:?}"
    );
    assert!(
        !fields.iter().any(|(_, v)| v.contains("Jane Doe")),
        "phi values leaked into the tracing mirror: {fields:?}"
    );
    // The safe half still gets through.
    assert!(field(&fields, "audit.details").contains("match_count"));
}

#[test]
fn an_action_cannot_disagree_with_its_category() {
    // The category is not a separate argument any call site can pass, so the
    // only way to get one is through the action constant that owns it.
    assert_eq!(
        AuditEvent::new(actions::RECORD_FILE_READ, "record_file", "x", "acct").category,
        EventCategory::Access
    );
    assert_eq!(
        AuditEvent::new(actions::CLIENT_CREATE, "client", "x", "acct").category,
        EventCategory::Mutation
    );
    assert_eq!(
        AuditEvent::new(actions::REPORT_TOOL_TURN, "report", "x", "acct").category,
        EventCategory::Ai
    );
    assert_eq!(
        AuditEvent::new(actions::PROMPT_SAVE, "prompt", "x", "acct").category,
        EventCategory::Admin
    );
}

#[test]
fn events_round_trip_through_json() {
    let client = uuid::Uuid::new_v4();
    let event = AuditEvent::new(actions::CHAT_TURN, "client", "abc", "acct")
        .with_client_id(client)
        .with_credential_id(Some("AKIAIOSFODNN7EXAMPLE".to_string()))
        .with_app_version("9.9.9")
        .with_details(serde_json::json!({ "output_tokens": 7 }))
        .with_phi(serde_json::json!({ "query": "secret" }));

    let encoded = serde_json::to_string(&event).expect("serialize");
    let decoded: AuditEvent = serde_json::from_str(&encoded).expect("deserialize");

    assert_eq!(decoded.event_id, event.event_id);
    assert_eq!(decoded.timestamp, event.timestamp);
    assert_eq!(decoded.action, "chat.turn");
    assert_eq!(decoded.category, EventCategory::Ai);
    assert_eq!(decoded.client_id, Some(client));
    assert_eq!(
        decoded.credential_id.as_deref(),
        Some("AKIAIOSFODNN7EXAMPLE")
    );
    assert_eq!(decoded.app_version.as_deref(), Some("9.9.9"));
    assert_eq!(decoded.details, event.details);
    // phi reaches S3 in full — it is only the console that never sees it.
    assert_eq!(decoded.phi, event.phi);
    // snake_case on the wire, per the project's serialization rule.
    assert!(encoded.contains("\"resource_type\""), "{encoded}");
}

#[test]
fn absent_optionals_are_omitted_from_the_wire_form() {
    let encoded = serde_json::to_string(&AuditEvent::new(
        actions::PROMPT_SAVE,
        "prompt",
        "x",
        "acct",
    ))
    .expect("serialize");

    for absent in ["client_id", "credential_id", "phi"] {
        assert!(
            !encoded.contains(absent),
            "{absent} should be omitted when unset: {encoded}"
        );
    }
}

#[test]
fn a_pre_v2_object_is_rejected_rather_than_partially_read() {
    // v2 is a clean break: `category` is required. A pre-v2 object fails to
    // deserialize instead of degrading into an event with a made-up category
    // and an action name no v2 query matches. Deploying v2 means purging
    // `_audit/` — see the audit module docs.
    let raw = serde_json::json!({
        "event_id": uuid::Uuid::nil(),
        "timestamp": "2026-07-01T00:00:00Z",
        "action": "chat_message",
        "resource_type": "client",
        "resource_id": "abc",
        "user_sub": "123456789012",
        "details": { "input_tokens": 12 },
    });

    let decoded = serde_json::from_value::<AuditEvent>(raw);

    assert!(
        decoded.is_err(),
        "a pre-v2 object must not read as a v2 event: {decoded:?}"
    );
}

#[test]
fn every_category_round_trips_through_its_wire_spelling() {
    // No catch-all variant exists, so this is the whole set.
    for (category, wire) in [
        (EventCategory::Access, "access"),
        (EventCategory::Mutation, "mutation"),
        (EventCategory::Ai, "ai"),
        (EventCategory::Admin, "admin"),
    ] {
        assert_eq!(category.as_str(), wire);
        assert_eq!(
            serde_json::from_str::<EventCategory>(&format!("\"{wire}\"")).expect("deserialize"),
            category
        );
    }
}

/// Locking and unlocking are operational events over PHI, never reads of it.
/// A session action that drifted into `Access` would put every walk-away lock
/// into the "who read this client's records" answer.
#[test]
fn every_session_action_is_administrative() {
    for action in [
        actions::SESSION_LOCK,
        actions::SESSION_UNLOCK,
        actions::SESSION_UNLOCK_FAILED,
        actions::SESSION_AUTO_LOCK_ENABLE,
        actions::SESSION_AUTO_LOCK_DISABLE,
        actions::SESSION_PIN_CHANGE,
        actions::SESSION_AUTO_LOCK_UPDATE,
    ] {
        assert_eq!(
            action.category,
            EventCategory::Admin,
            "{} is not administrative",
            action.name
        );
        assert!(
            action.name.starts_with("session."),
            "{} is not a session action",
            action.name
        );
    }
}
