//! Tests for `ChatHistoryMessage` — verifies legacy chat-history JSON
//! (no `usage` key at all) still deserialises to `usage: None` — and for
//! the bound on how much of a conversation is replayed to Bedrock.

use claria_core::models::chat_history::{
    ChatHistory, ChatHistoryMessage, ChatMessage, ChatRole, MAX_CHAT_HISTORY_WIRE_BYTES,
    MAX_RETAINED_CHAT_MESSAGES, bounded_chat_history,
};

/// A conversation of `exchanges` user/assistant pairs plus a trailing user
/// message — the shape the frontend sends on every turn.
fn conversation(exchanges: usize, content_len: usize) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for _ in 0..exchanges {
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: "u".repeat(content_len),
        });
        messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content: "a".repeat(content_len),
        });
    }
    messages.push(ChatMessage {
        role: ChatRole::User,
        content: "the newest question".to_string(),
    });
    messages
}

fn assert_valid_for_converse(messages: &[ChatMessage]) {
    assert!(
        !messages.is_empty(),
        "Converse rejects an empty conversation"
    );
    assert_eq!(
        messages[0].role,
        ChatRole::User,
        "Converse rejects a conversation that opens on an assistant turn"
    );
    for pair in messages.windows(2) {
        assert_ne!(
            pair[0].role, pair[1].role,
            "Converse rejects two consecutive turns in the same role"
        );
    }
}

#[test]
fn a_short_conversation_is_sent_whole() {
    let messages = conversation(3, 100);
    let bounded = bounded_chat_history(&messages);
    assert_eq!(bounded.len(), messages.len());
}

#[test]
fn a_conversation_past_the_message_ceiling_is_pruned_from_the_front() {
    let messages = conversation(MAX_RETAINED_CHAT_MESSAGES, 10);
    let bounded = bounded_chat_history(&messages);

    assert!(bounded.len() < messages.len(), "nothing was pruned");
    assert!(bounded.len() < MAX_RETAINED_CHAT_MESSAGES);
    assert_valid_for_converse(bounded);
    // The newest message is the question being asked; it is never dropped.
    assert_eq!(
        bounded.last().expect("newest message").content,
        "the newest question"
    );
}

#[test]
fn a_conversation_past_the_byte_ceiling_is_pruned_even_when_short() {
    // Ten exchanges is far under the message ceiling, but each message is
    // large enough that the conversation blows the byte ceiling.
    let messages = conversation(10, MAX_CHAT_HISTORY_WIRE_BYTES / 8);
    let bounded = bounded_chat_history(&messages);

    assert!(bounded.len() < messages.len(), "nothing was pruned");
    let bytes: usize = bounded.iter().map(|message| message.content.len()).sum();
    assert!(
        bytes <= MAX_CHAT_HISTORY_WIRE_BYTES,
        "{bytes} bytes still exceeds the wire ceiling"
    );
    assert_valid_for_converse(bounded);
}

/// A cut that lands on an assistant turn has to advance past it: Bedrock
/// rejects a conversation whose first message is an assistant reply, so
/// pruning that ignored this would turn a long chat into a hard failure.
#[test]
fn pruning_always_leaves_a_conversation_bedrock_accepts() {
    for exchanges in MAX_RETAINED_CHAT_MESSAGES..MAX_RETAINED_CHAT_MESSAGES + 12 {
        let messages = conversation(exchanges, 10);
        assert_valid_for_converse(bounded_chat_history(&messages));
    }
}

#[test]
fn bounding_an_already_bounded_conversation_changes_nothing() {
    let messages = conversation(MAX_RETAINED_CHAT_MESSAGES, 10);
    let once = bounded_chat_history(&messages).to_vec();
    let twice = bounded_chat_history(&once);
    assert_eq!(twice.len(), once.len());
}

/// Pruning cuts back below the ceiling rather than to it, so the next
/// several turns fit without pruning again. A prune on every turn would
/// move the start of the conversation each time and miss the tail cache
/// point forever after.
#[test]
fn pruning_leaves_headroom_so_later_turns_reuse_the_same_prefix() {
    let messages = conversation(MAX_RETAINED_CHAT_MESSAGES, 10);
    let pruned = bounded_chat_history(&messages).to_vec();

    // Append several more exchanges; the retained prefix must not move.
    let mut grown = pruned.clone();
    for _ in 0..4 {
        grown.push(ChatMessage {
            role: ChatRole::Assistant,
            content: "reply".to_string(),
        });
        grown.push(ChatMessage {
            role: ChatRole::User,
            content: "follow-up".to_string(),
        });
    }
    let bounded = bounded_chat_history(&grown);
    assert_eq!(
        bounded.len(),
        grown.len(),
        "a prune on every turn would invalidate the cached prefix each time"
    );
}

/// The last message alone can exceed the byte ceiling — a clinician pasting
/// a long document into the box. Dropping it would delete the question.
#[test]
fn an_oversized_newest_message_is_kept() {
    let messages = vec![ChatMessage {
        role: ChatRole::User,
        content: "x".repeat(MAX_CHAT_HISTORY_WIRE_BYTES * 2),
    }];
    let bounded = bounded_chat_history(&messages);
    assert_eq!(bounded.len(), 1);
}

#[test]
fn legacy_history_without_name_gets_an_empty_migration_sentinel() {
    let legacy_json = r#"{
        "id": "11111111-1111-4111-8111-111111111111",
        "client_id": "22222222-2222-4222-8222-222222222222",
        "model_id": "test-model",
        "messages": [],
        "created_at": "2026-04-01T12:00:00Z",
        "updated_at": "2026-04-01T12:00:00Z"
    }"#;

    let history: ChatHistory = serde_json::from_str(legacy_json).expect("legacy history");
    assert!(history.name.is_empty());
}

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

    assert!(matches!(msg.role, ChatRole::Assistant));
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
    assert!(matches!(msg.role, ChatRole::User));
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
