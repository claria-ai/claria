use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Client {
    pub id: Uuid,
    pub name: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}
