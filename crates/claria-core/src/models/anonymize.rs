use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// The result of a Bedrock anonymization transaction.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AnonymizationResult {
    pub anonymized_text: String,
    pub replacements: Vec<PiiReplacement>,
    pub transaction_id: Uuid,
}

/// A single PII replacement made during anonymization.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PiiReplacement {
    pub original: String,
    pub replacement: String,
    pub pii_type: PiiType,
    /// Byte offset ranges in the original text: `(start, end)`.
    pub offsets: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PiiType {
    Name,
    DateOfBirth,
    Address,
    Phone,
    Email,
    School,
    Provider,
    Location,
    Other,
}
