use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::turn_usage::TurnUsage;

/// A persisted chat session between a user and a Bedrock model.
///
/// Uploaded to S3 after every call/response pair so the conversation
/// is durable and traceable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistory {
    pub id: Uuid,
    pub client_id: Uuid,
    /// User-facing session name. Empty only for histories written before
    /// named chats were introduced; callers assign a numbered fallback.
    #[serde(default)]
    pub name: String,
    pub model_id: String,
    pub messages: Vec<ChatHistoryMessage>,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}

/// A single message in a persisted chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistoryMessage {
    pub role: ChatRole,
    pub content: String,
    pub timestamp: jiff::Timestamp,
    /// `Some` on assistant turns whose Converse response carried a usage
    /// block. `None` on user turns and on assistant turns from history
    /// written before this schema landed. Never synthesised — `None` means
    /// "unknown".
    #[serde(default)]
    pub usage: Option<TurnUsage>,
    /// The wire stop reason of the assistant turn; `None` on user turns and
    /// pre-schema history.
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// Wall-clock duration of the streamed exchange that produced this
    /// assistant turn; `None` when unknown.
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

/// Role of a chat message — the one role enum shared by persisted history,
/// the Bedrock chat calls, and the desktop IPC surface.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

/// One conversation turn — role plus text content. Shared by the Bedrock
/// chat call and the desktop IPC surface so the two cannot drift.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Most messages replayed to Bedrock on one chat request. Counted in
/// messages rather than exchanges because that is what the Converse
/// `messages` array holds; 80 is 40 user/assistant exchanges.
pub const MAX_RETAINED_CHAT_MESSAGES: usize = 80;

/// Most conversation text replayed to Bedrock on one chat request, in
/// bytes. Roughly 64k tokens — about a third of the input budget a 200k
/// model leaves after the output reserve, so the record context in the
/// system prompt keeps the majority of the window.
pub const MAX_CHAT_HISTORY_WIRE_BYTES: usize = 256 * 1024;

/// How much of each ceiling a prune leaves behind.
///
/// Pruning to the ceiling would drop one message per request forever after
/// the first breach, and every drop moves the start of the conversation, so
/// the tail `cachePoint` would miss on every later turn. Cutting back to
/// three quarters means a breach costs one cache miss and then roughly ten
/// exchanges of hits before the next one.
const PRUNE_TARGET_PERCENT: usize = 75;

/// Bytes this message contributes to the Converse `messages` array.
fn message_wire_bytes(message: &ChatMessage) -> usize {
    message.content.len()
}

/// The suffix of `messages` that a chat request replays to Bedrock.
///
/// Oldest messages are dropped once the history crosses either
/// [`MAX_RETAINED_CHAT_MESSAGES`] or [`MAX_CHAT_HISTORY_WIRE_BYTES`]. The
/// persisted history in S3 is never pruned — this bounds only what goes on
/// the wire, so a clinician's record of the conversation stays whole.
///
/// The result always begins on a [`ChatRole::User`] message: Converse
/// rejects a conversation that opens with an assistant turn, so a cut that
/// lands on one advances past it. If no user message survives the cut the
/// history is returned unpruned rather than replaced with something Bedrock
/// would reject.
///
/// Idempotent — bounding an already-bounded history returns it unchanged.
pub fn bounded_chat_history(messages: &[ChatMessage]) -> &[ChatMessage] {
    let total_bytes: usize = messages.iter().map(message_wire_bytes).sum();
    if messages.len() <= MAX_RETAINED_CHAT_MESSAGES && total_bytes <= MAX_CHAT_HISTORY_WIRE_BYTES {
        return messages;
    }

    let target_messages = MAX_RETAINED_CHAT_MESSAGES * PRUNE_TARGET_PERCENT / 100;
    let target_bytes = MAX_CHAT_HISTORY_WIRE_BYTES * PRUNE_TARGET_PERCENT / 100;

    let mut start = 0;
    let mut bytes = total_bytes;
    // The newest message is always kept, even when it alone exceeds the
    // byte target — dropping it would discard the question being asked.
    while messages.len() - start > 1
        && (messages.len() - start > target_messages || bytes > target_bytes)
    {
        bytes -= message_wire_bytes(&messages[start]);
        start += 1;
    }

    while start < messages.len() && messages[start].role != ChatRole::User {
        start += 1;
    }
    if start >= messages.len() {
        return messages;
    }
    &messages[start..]
}
