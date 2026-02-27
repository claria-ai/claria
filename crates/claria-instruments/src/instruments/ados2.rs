use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// ADOS-2: Autism Diagnostic Observation Schedule, Second Edition.
/// 5 modules (Toddler, 1–4), Communication + Social Interaction + RRB domains.
pub struct Ados2;

impl Instrument for Ados2 {
    fn id(&self) -> &str {
        "ados2"
    }

    fn name(&self) -> &str {
        "ADOS-2"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let algorithm_range = ScoreRange {
                min: 0.0,
                max: 28.0,
                step: Some(1.0),
            };
            let comparison_range = ScoreRange {
                min: 1.0,
                max: 10.0,
                step: Some(1.0),
            };

            vec![
                Domain {
                    id: "social_affect".to_string(),
                    name: "Social Affect".to_string(),
                    subscales: vec![
                        Subscale {
                            id: "sa_algorithm".to_string(),
                            name: "Social Affect Algorithm Total".to_string(),
                            score_type: ScoreType::Raw,
                            range: algorithm_range,
                            description: None,
                        },
                        Subscale {
                            id: "sa_comparison".to_string(),
                            name: "Social Affect Comparison Score".to_string(),
                            score_type: ScoreType::Scaled,
                            range: comparison_range,
                            description: None,
                        },
                    ],
                    composite_score_type: None,
                    composite_range: None,
                    description: Some("Communication and reciprocal social interaction".to_string()),
                },
                Domain {
                    id: "rrb".to_string(),
                    name: "Restricted and Repetitive Behavior".to_string(),
                    subscales: vec![
                        Subscale {
                            id: "rrb_algorithm".to_string(),
                            name: "RRB Algorithm Total".to_string(),
                            score_type: ScoreType::Raw,
                            range: ScoreRange {
                                min: 0.0,
                                max: 16.0,
                                step: Some(1.0),
                            },
                            description: None,
                        },
                    ],
                    composite_score_type: None,
                    composite_range: None,
                    description: None,
                },
                Domain {
                    id: "overall".to_string(),
                    name: "Overall".to_string(),
                    subscales: vec![
                        Subscale {
                            id: "overall_total".to_string(),
                            name: "Overall Total (SA + RRB)".to_string(),
                            score_type: ScoreType::Raw,
                            range: ScoreRange {
                                min: 0.0,
                                max: 44.0,
                                step: Some(1.0),
                            },
                            description: None,
                        },
                        Subscale {
                            id: "overall_comparison".to_string(),
                            name: "Comparison Score".to_string(),
                            score_type: ScoreType::Scaled,
                            range: comparison_range,
                            description: Some("1-10, higher = more symptoms".to_string()),
                        },
                    ],
                    composite_score_type: None,
                    composite_range: None,
                    description: None,
                },
            ]
        });
        &DOMAINS
    }
}
