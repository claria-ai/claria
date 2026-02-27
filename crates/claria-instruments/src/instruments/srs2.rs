use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// SRS-2: Social Responsiveness Scale, Second Edition.
/// Social Awareness, Cognition, Communication, Motivation, RRBs subscales.
/// T-scores: mean 50, SD 10. Higher = more difficulty.
pub struct Srs2;

impl Instrument for Srs2 {
    fn id(&self) -> &str {
        "srs2"
    }

    fn name(&self) -> &str {
        "SRS-2"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let t_score = ScoreRange {
                min: 30.0,
                max: 100.0,
                step: Some(1.0),
            };

            vec![Domain {
                id: "treatment_subscales".to_string(),
                name: "Treatment Subscales".to_string(),
                subscales: vec![
                    subscale("social_awareness", "Social Awareness", t_score),
                    subscale("social_cognition", "Social Cognition", t_score),
                    subscale("social_communication", "Social Communication", t_score),
                    subscale("social_motivation", "Social Motivation", t_score),
                    subscale("rrb", "Restricted Interests and Repetitive Behavior", t_score),
                    subscale("sci", "Social Communication and Interaction (SCI)", t_score),
                    subscale("total", "SRS-2 Total", t_score),
                ],
                composite_score_type: Some(ScoreType::TScore),
                composite_range: Some(t_score),
                description: Some("Higher T-scores indicate greater difficulty".to_string()),
            }]
        });
        &DOMAINS
    }
}

fn subscale(id: &str, name: &str, range: ScoreRange) -> Subscale {
    Subscale {
        id: id.to_string(),
        name: name.to_string(),
        score_type: ScoreType::TScore,
        range,
        description: None,
    }
}
