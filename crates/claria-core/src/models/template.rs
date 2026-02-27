use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Template {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub s3_key: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}
