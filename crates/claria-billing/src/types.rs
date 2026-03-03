use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CostGranularity {
    Hourly,
    Daily,
    Monthly,
}

/// Input query — validated before sending to AWS.
#[derive(Debug, Clone)]
pub struct CostQuery {
    pub start_date: String,
    pub end_date: String,
    pub granularity: CostGranularity,
    pub group_by_service: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CostResultGroup {
    pub key: String,
    pub amount: String,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CostTimePeriod {
    pub start: String,
    pub end: String,
    pub groups: Vec<CostResultGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CostAndUsageResult {
    pub periods: Vec<CostTimePeriod>,
}
