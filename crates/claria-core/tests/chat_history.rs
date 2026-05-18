//! Tests for `ChatHistoryMessage` — verifies legacy chat-history JSON
//! (no `usage` key at all) still deserialises to `usage: None`.

use claria_core::models::chat_history::{ChatHistoryMessage, ChatHistoryRole};

#[test]
fn legacy_message_without_usage_key_deserialises_to_none() {
    // Pre-Phase-1 chat-history JSON has no `usage` key at all.
    let legacy_json = r#"{
        "role": "assistant",
        "content": "Hello!",
        "timestamp": "2026-04-01T12:00:00Z"
    }"#;

    let msg: ChatHistoryMessage =
        serde_json::from_str(legacy_json).expect("legacy message must deserialise");

    assert!(matches!(msg.role, ChatHistoryRole::Assistant));
    assert_eq!(msg.content, "Hello!");
    assert!(
        msg.usage.is_none(),
        "legacy messages with no `usage` key must deserialise to None"
    );
}

#[test]
fn legacy_user_message_deserialises_to_none_usage() {
    let legacy_json = r#"{
        "role": "user",
        "content": "What is X?",
        "timestamp": "2026-04-01T12:00:00Z"
    }"#;

    let msg: ChatHistoryMessage = serde_json::from_str(legacy_json).unwrap();
    assert!(matches!(msg.role, ChatHistoryRole::User));
    assert!(msg.usage.is_none());
}

#[test]
fn explicit_null_usage_deserialises_to_none() {
    let json = r#"{
        "role": "assistant",
        "content": "Hi",
        "timestamp": "2026-04-01T12:00:00Z",
        "usage": null
    }"#;

    let msg: ChatHistoryMessage = serde_json::from_str(json).unwrap();
    assert!(msg.usage.is_none());
}

#[test]
fn message_with_usage_round_trips() {
    let json = r#"{
        "role": "assistant",
        "content": "Hi",
        "timestamp": "2026-04-01T12:00:00Z",
        "usage": {
            "model_id": "us.anthropic.claude-sonnet-4-20250514-v1:0",
            "input_tokens": 1000,
            "output_tokens": 200,
            "cache_read_input_tokens": 0,
            "cache_write_input_tokens": 0,
            "cost_usd": 0.0033,
            "pricing_version": 1
        }
    }"#;

    let msg: ChatHistoryMessage = serde_json::from_str(json).unwrap();
    {
        let usage = msg.usage.as_ref().expect("usage should be present");
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 200);
        assert_eq!(usage.pricing_version, 1);
    }

    // Re-serialise and deserialise to confirm round-trip stability.
    let serialised = serde_json::to_string(&msg).unwrap();
    let back: ChatHistoryMessage = serde_json::from_str(&serialised).unwrap();
    let back_usage = back.usage.unwrap();
    assert_eq!(back_usage.input_tokens, 1000);
    assert_eq!(back_usage.cost_usd, 0.0033);
}
