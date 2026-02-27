use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Goal {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub recommendations: Vec<Recommendation>,
    pub s3_key: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Recommendation {
    pub title: String,
    pub description: String,
}
