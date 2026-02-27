use serde::{Deserialize, Serialize};

/// Document styling configuration for exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentStyles {
    /// Font for body text (e.g. "Times New Roman", "Calibri").
    pub body_font: String,

    /// Font for headings (e.g. "Arial", "Calibri").
    pub heading_font: String,

    /// Body text font size in points.
    pub body_size: usize,

    /// Heading 1 font size in points.
    pub heading1_size: usize,

    /// Heading 2 font size in points.
    pub heading2_size: usize,

    /// Heading 3 font size in points.
    pub heading3_size: usize,

    /// Page margin in inches (applied uniformly).
    pub margin_inches: f64,
}

impl Default for DocumentStyles {
    fn default() -> Self {
        Self {
            body_font: "Times New Roman".to_string(),
            heading_font: "Arial".to_string(),
            body_size: 12,
            heading1_size: 16,
            heading2_size: 14,
            heading3_size: 12,
            margin_inches: 1.0,
        }
    }
}
