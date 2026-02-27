use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Report {
    pub id: Uuid,
    pub title: String,
    pub status: ReportStatus,
    pub template_id: Uuid,
    pub transaction_id: Option<Uuid>,
    pub s3_key: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReportStatus {
    Draft,
    Generating,
    Complete,
    Failed,
}
