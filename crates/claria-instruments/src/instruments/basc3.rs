use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// BASC-3: Behavior Assessment System for Children, Third Edition.
/// TRS, PRS, SRP, SOS forms with clinical and adaptive subscales.
/// T-scores: mean 50, SD 10.
pub struct Basc3;

impl Instrument for Basc3 {
    fn id(&self) -> &str {
        "basc3"
    }

    fn name(&self) -> &str {
        "BASC-3"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let t_score = ScoreRange {
                min: 10.0,
                max: 120.0,
                step: Some(1.0),
            };

            vec![
                Domain {
                    id: "clinical".to_string(),
                    name: "Clinical Scales".to_string(),
                    subscales: vec![
                        subscale("hyperactivity", "Hyperactivity", t_score),
                        subscale("aggression", "Aggression", t_score),
                        subscale("conduct_problems", "Conduct Problems", t_score),
                        subscale("anxiety", "Anxiety", t_score),
                        subscale("depression", "Depression", t_score),
                        subscale("somatization", "Somatization", t_score),
                        subscale("attention_problems", "Attention Problems", t_score),
                        subscale("atypicality", "Atypicality", t_score),
                        subscale("withdrawal", "Withdrawal", t_score),
                    ],
                    composite_score_type: None,
                    composite_range: None,
                    description: Some("Higher scores indicate greater problems".to_string()),
                },
                Domain {
                    id: "adaptive".to_string(),
                    name: "Adaptive Scales".to_string(),
                    subscales: vec![
                        subscale("adaptability", "Adaptability", t_score),
                        subscale("social_skills", "Social Skills", t_score),
                        subscale("leadership", "Leadership", t_score),
                        subscale("activities_of_daily_living", "Activities of Daily Living", t_score),
                        subscale("functional_communication", "Functional Communication", t_score),
                    ],
                    composite_score_type: None,
                    composite_range: None,
                    description: Some("Higher scores indicate better functioning".to_string()),
                },
                Domain {
                    id: "composites".to_string(),
                    name: "Composite Indices".to_string(),
                    subscales: vec![
                        subscale("externalizing_problems", "Externalizing Problems", t_score),
                        subscale("internalizing_problems", "Internalizing Problems", t_score),
                        subscale("behavioral_symptoms_index", "Behavioral Symptoms Index", t_score),
                        subscale("adaptive_skills", "Adaptive Skills", t_score),
                    ],
                    composite_score_type: Some(ScoreType::TScore),
                    composite_range: Some(t_score),
                    description: None,
                },
            ]
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
