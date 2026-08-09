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
    pub role: ChatHistoryRole,
    pub content: String,
    pub timestamp: jiff::Timestamp,
    /// `Some` on assistant turns whose Converse response carried a usage
    /// block. `None` on user turns and on assistant turns from history
    /// written before this schema landed. Never synthesised — `None` means
    /// "unknown".
    #[serde(default)]
    pub usage: Option<TurnUsage>,
}

/// Role of a chat history message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatHistoryRole {
    User,
    Assistant,
}
