use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// The type of score a subscale or domain produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ScoreType {
    /// Unscaled count or sum.
    Raw,
    /// Normed score, typically mean=100, SD=15.
    Standard,
    /// Normed score, typically mean=10, SD=3.
    Scaled,
    /// Vineland-style, mean=15, SD=3.
    VScale,
    /// Percentile rank (0–100).
    Percentile,
    /// T-score, mean=50, SD=10.
    TScore,
    /// Milestone scoring (e.g., 0/0.5/1).
    Milestone,
    /// Likert-style rating (e.g., 0–4).
    Rating,
}

/// Defines the valid range for a score.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScoreRange {
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
}

impl ScoreRange {
    pub fn contains(&self, value: f64) -> bool {
        if value < self.min || value > self.max {
            return false;
        }
        if let Some(step) = self.step {
            let offset = value - self.min;
            let remainder = offset % step;
            // Allow floating point tolerance
            remainder < 1e-9 || (step - remainder) < 1e-9
        } else {
            true
        }
    }
}

/// A domain or subscale definition within an instrument.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Subscale {
    pub id: String,
    pub name: String,
    pub score_type: ScoreType,
    pub range: ScoreRange,
    pub description: Option<String>,
}

/// A top-level domain within an instrument, containing subscales.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Domain {
    pub id: String,
    pub name: String,
    pub subscales: Vec<Subscale>,
    pub composite_score_type: Option<ScoreType>,
    pub composite_range: Option<ScoreRange>,
    pub description: Option<String>,
}

/// A score entry provided by the user for validation.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScoreEntry {
    pub subscale_id: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Error)]
#[ts(export)]
#[error("{message}")]
pub struct ValidationError {
    pub subscale_id: String,
    pub value: f64,
    pub expected_range: ScoreRange,
    pub score_type: ScoreType,
    pub message: String,
}
