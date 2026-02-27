use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::goal::Recommendation;

/// The structured output from a report generation Bedrock Transaction.
/// Every field is addressable by name in a Jinja2 template.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SchematizedAnswer {
    // Identifying
    pub client_name: String,
    pub date_of_evaluation: String,
    pub evaluator_name: String,

    // Clinical content
    pub background_information: String,
    pub behavioral_observations: String,
    pub assessment_results: Vec<AssessmentResult>,
    pub clinical_impressions: String,
    pub diagnostic_summary: String,
    pub strengths: Vec<String>,
    pub areas_of_concern: Vec<String>,
    pub recommendations: Vec<Recommendation>,
    pub treatment_goals: Vec<TreatmentGoal>,

    // Extensible
    #[serde(default)]
    pub custom_sections: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AssessmentResult {
    pub instrument_name: String,
    pub summary: String,
    pub scores: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TreatmentGoal {
    pub title: String,
    pub description: String,
    pub objectives: Vec<String>,
}
