use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::token_count::TokenUsage;

/// A Bedrock Transaction — an auditable unit of work.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BedrockTransaction {
    pub id: Uuid,
    pub transaction_type: TransactionType,
    pub model_id: String,
    pub usage: TokenUsage,
    pub status: TransactionStatus,
    pub s3_key: String,
    pub created_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TransactionType {
    ReportGeneration,
    Anonymization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TransactionStatus {
    Pending,
    Complete,
    Failed,
}
