use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// WAIS-IV: Wechsler Adult Intelligence Scale, Fourth Edition.
/// Verbal Comprehension, Perceptual Reasoning, Working Memory, Processing Speed indices.
/// Subtests: scaled scores (mean 10, SD 3). Indices: standard scores (mean 100, SD 15).
pub struct WaisIv;

impl Instrument for WaisIv {
    fn id(&self) -> &str {
        "wais_iv"
    }

    fn name(&self) -> &str {
        "WAIS-IV"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let scaled = ScoreRange {
                min: 1.0,
                max: 19.0,
                step: Some(1.0),
            };
            let standard = ScoreRange {
                min: 40.0,
                max: 160.0,
                step: Some(1.0),
            };

            vec![
                Domain {
                    id: "verbal_comprehension".to_string(),
                    name: "Verbal Comprehension Index (VCI)".to_string(),
                    subscales: vec![
                        subtest("similarities", "Similarities", scaled),
                        subtest("vocabulary", "Vocabulary", scaled),
                        subtest("information", "Information", scaled),
                        subtest("comprehension", "Comprehension", scaled),
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "perceptual_reasoning".to_string(),
                    name: "Perceptual Reasoning Index (PRI)".to_string(),
                    subscales: vec![
                        subtest("block_design", "Block Design", scaled),
                        subtest("matrix_reasoning", "Matrix Reasoning", scaled),
                        subtest("visual_puzzles", "Visual Puzzles", scaled),
                        subtest("figure_weights", "Figure Weights", scaled),
                        subtest("picture_completion", "Picture Completion", scaled),
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "working_memory".to_string(),
                    name: "Working Memory Index (WMI)".to_string(),
                    subscales: vec![
                        subtest("digit_span", "Digit Span", scaled),
                        subtest("arithmetic", "Arithmetic", scaled),
                        subtest("letter_number_sequencing", "Letter-Number Sequencing", scaled),
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "processing_speed".to_string(),
                    name: "Processing Speed Index (PSI)".to_string(),
                    subscales: vec![
                        subtest("symbol_search", "Symbol Search", scaled),
                        subtest("coding", "Coding", scaled),
                        subtest("cancellation", "Cancellation", scaled),
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "full_scale_iq".to_string(),
                    name: "Full Scale IQ (FSIQ)".to_string(),
                    subscales: vec![],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: Some("Overall composite across all indices".to_string()),
                },
            ]
        });
        &DOMAINS
    }
}

fn subtest(id: &str, name: &str, range: ScoreRange) -> Subscale {
    Subscale {
        id: id.to_string(),
        name: name.to_string(),
        score_type: ScoreType::Scaled,
        range,
        description: None,
    }
}
