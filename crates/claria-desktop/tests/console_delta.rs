//! Sequence-cursor semantics of the console ring buffer: polls ship only new
//! lines, and a cursor invalidated by rotation or an app restart triggers a
//! full resend flagged `reset`.

use tracing_subscriber::prelude::*;

use claria_desktop::console::{ConsoleBuffer, ConsoleLayer, frontend_event_fields};

#[test]
fn frontend_event_fields_never_include_provider_text() {
    let raw = "Writer draft planning failed: ValidationException for Jane Doe intake.docx";
    let (event, bytes) = frontend_event_fields(raw);
    assert_eq!(event, "Writer draft planning failed");
    assert_eq!(bytes, raw.len());
    assert!(!event.contains("Jane Doe"));
    assert_eq!(
        frontend_event_fields("PIN unlock failed: bad PIN").0,
        "PIN unlock failed"
    );
    assert_eq!(
        frontend_event_fields("React render crash: Jane Doe").0,
        "React render crash"
    );
    assert_eq!(
        frontend_event_fields("hasConfig failed: Jane Doe").0,
        "hasConfig failed"
    );
    assert_eq!(
        frontend_event_fields("Failed to register drag-drop listener: Jane Doe").0,
        "Failed to register drag-drop listener"
    );
}

fn scoped_buffer() -> (ConsoleBuffer, impl tracing::Subscriber) {
    let buffer = ConsoleBuffer::new();
    let subscriber = tracing_subscriber::registry().with(ConsoleLayer::new(buffer.clone()));
    (buffer, subscriber)
}

#[test]
fn delta_polls_ship_only_new_lines() {
    let (buffer, subscriber) = scoped_buffer();
    tracing::subscriber::with_default(subscriber, || {
        // First poll on an empty buffer: nothing, cursor stays at 0.
        let delta = buffer.entries_since(0);
        assert!(delta.entries.is_empty());
        assert_eq!(delta.next_seq, 0);
        assert!(!delta.reset);

        tracing::info!("first");
        tracing::info!("second");

        let delta = buffer.entries_since(0);
        assert_eq!(delta.entries.len(), 2);
        assert_eq!(delta.next_seq, 2);
        assert!(!delta.reset);

        // Nothing new: empty delta, cursor unchanged.
        let delta = buffer.entries_since(2);
        assert!(delta.entries.is_empty());
        assert_eq!(delta.next_seq, 2);
        assert!(!delta.reset);

        tracing::info!("third");
        let delta = buffer.entries_since(2);
        assert_eq!(delta.entries.len(), 1);
        assert!(delta.entries[0].message.contains("third"));
        assert_eq!(delta.next_seq, 3);
        assert!(!delta.reset);
    });
}

#[test]
fn stale_cursor_from_a_previous_run_resets() {
    let (buffer, subscriber) = scoped_buffer();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("only line");
        // A cursor beyond the buffer end (previous app run) resends all.
        let delta = buffer.entries_since(999);
        assert_eq!(delta.entries.len(), 1);
        assert_eq!(delta.next_seq, 1);
        assert!(delta.reset);
    });
}

#[test]
fn rotation_past_the_cursor_resets() {
    let (buffer, subscriber) = scoped_buffer();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("seen line");
        let seen = buffer.entries_since(0);
        assert!(!seen.reset);

        // Push the buffer past its byte cap so the seen line rotates out.
        let filler = "x".repeat(1024 * 1024);
        for _ in 0..12 {
            tracing::info!("{filler}");
        }

        let delta = buffer.entries_since(seen.next_seq);
        assert!(delta.reset, "rotated-out cursor must trigger a reset");
        assert!(!delta.entries.is_empty());
        assert!(
            delta
                .entries
                .iter()
                .all(|e| !e.message.contains("seen line")),
            "rotated entries must be gone from the resend"
        );
    });
}
