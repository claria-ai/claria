//! claria-instruments
//!
//! Clinical assessment instrument definitions. Pure data — no AWS dependency.
//! Defines the structure, domains, subscales, and scoring rules for each
//! supported instrument.

pub mod error;
pub mod instruments;
pub mod scoring;

use scoring::{Domain, ScoreEntry, ValidationError};

/// Trait implemented by each clinical assessment instrument.
pub trait Instrument: Send + Sync {
    /// Unique identifier for this instrument (e.g., "vb_mapp", "vineland3").
    fn id(&self) -> &str;

    /// Human-readable name (e.g., "VB-MAPP", "Vineland-3").
    fn name(&self) -> &str;

    /// The domains and subscales this instrument measures.
    fn domains(&self) -> &[Domain];

    /// Validate a set of score entries against this instrument's rules.
    fn validate_scores(&self, scores: &[ScoreEntry]) -> Vec<ValidationError> {
        let all_subscales: Vec<_> = self
            .domains()
            .iter()
            .flat_map(|d| &d.subscales)
            .collect();

        let mut errors = Vec::new();
        for entry in scores {
            if let Some(subscale) = all_subscales.iter().find(|s| s.id == entry.subscale_id)
                && !subscale.range.contains(entry.value)
            {
                errors.push(ValidationError {
                    subscale_id: entry.subscale_id.clone(),
                    value: entry.value,
                    expected_range: subscale.range,
                    score_type: subscale.score_type,
                    message: format!(
                        "{}: {} score {} is outside range [{}, {}]",
                        self.name(),
                        subscale.name,
                        entry.value,
                        subscale.range.min,
                        subscale.range.max,
                    ),
                });
            }
        }
        errors
    }

    /// Format scores as structured text for inclusion in a Bedrock prompt.
    fn to_structured_input(&self, scores: &[ScoreEntry]) -> String {
        let mut output = format!("## {}\n\n", self.name());
        for domain in self.domains() {
            output.push_str(&format!("### {}\n", domain.name));
            for subscale in &domain.subscales {
                if let Some(entry) = scores.iter().find(|e| e.subscale_id == subscale.id) {
                    output.push_str(&format!("- {}: {}\n", subscale.name, entry.value));
                }
            }
            output.push('\n');
        }
        output
    }
}

/// Return all registered instruments.
pub fn all_instruments() -> Vec<Box<dyn Instrument>> {
    vec![
        Box::new(instruments::vb_mapp::VbMapp),
        Box::new(instruments::ablls_r::AbllsR),
        Box::new(instruments::vineland3::Vineland3),
        Box::new(instruments::ados2::Ados2),
        Box::new(instruments::basc3::Basc3),
        Box::new(instruments::wais_iv::WaisIv),
        Box::new(instruments::srs2::Srs2),
        Box::new(instruments::cars2::Cars2),
    ]
}

/// Look up an instrument by ID.
pub fn get_instrument(id: &str) -> Option<Box<dyn Instrument>> {
    all_instruments().into_iter().find(|i| i.id() == id)
}
