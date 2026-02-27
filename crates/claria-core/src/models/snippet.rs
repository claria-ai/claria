use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// A "bad" text snippet — an example of language or phrasing to avoid in reports.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextSnippet {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub s3_key: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}
