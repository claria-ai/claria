use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Assessment {
    pub id: Uuid,
    pub title: String,
    pub instrument_id: String,
    pub client_name: String,
    pub date_administered: jiff::civil::Date,
    pub scores: serde_json::Value,
    pub notes: Option<String>,
    pub anonymized: bool,
    pub s3_key: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}
