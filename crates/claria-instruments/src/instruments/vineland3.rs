use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// Vineland-3: Vineland Adaptive Behavior Scales, Third Edition.
/// Communication, Daily Living Skills, Socialization, Motor Skills domains
/// + Adaptive Behavior Composite. V-scale scores (mean 15, SD 3).
pub struct Vineland3;

impl Instrument for Vineland3 {
    fn id(&self) -> &str {
        "vineland3"
    }

    fn name(&self) -> &str {
        "Vineland-3"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let v_scale = ScoreRange {
                min: 1.0,
                max: 24.0,
                step: Some(1.0),
            };
            let standard = ScoreRange {
                min: 20.0,
                max: 160.0,
                step: Some(1.0),
            };

            vec![
                Domain {
                    id: "communication".to_string(),
                    name: "Communication".to_string(),
                    subscales: vec![
                        Subscale {
                            id: "receptive".to_string(),
                            name: "Receptive".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                        Subscale {
                            id: "expressive".to_string(),
                            name: "Expressive".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                        Subscale {
                            id: "written".to_string(),
                            name: "Written".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "daily_living_skills".to_string(),
                    name: "Daily Living Skills".to_string(),
                    subscales: vec![
                        Subscale {
                            id: "personal".to_string(),
                            name: "Personal".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                        Subscale {
                            id: "domestic".to_string(),
                            name: "Domestic".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                        Subscale {
                            id: "community".to_string(),
                            name: "Community".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "socialization".to_string(),
                    name: "Socialization".to_string(),
                    subscales: vec![
                        Subscale {
                            id: "interpersonal_relationships".to_string(),
                            name: "Interpersonal Relationships".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                        Subscale {
                            id: "play_and_leisure".to_string(),
                            name: "Play and Leisure Time".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                        Subscale {
                            id: "coping_skills".to_string(),
                            name: "Coping Skills".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "motor_skills".to_string(),
                    name: "Motor Skills".to_string(),
                    subscales: vec![
                        Subscale {
                            id: "gross_motor".to_string(),
                            name: "Gross Motor".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                        Subscale {
                            id: "fine_motor".to_string(),
                            name: "Fine Motor".to_string(),
                            score_type: ScoreType::VScale,
                            range: v_scale,
                            description: None,
                        },
                    ],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: None,
                },
                Domain {
                    id: "adaptive_behavior_composite".to_string(),
                    name: "Adaptive Behavior Composite".to_string(),
                    subscales: vec![],
                    composite_score_type: Some(ScoreType::Standard),
                    composite_range: Some(standard),
                    description: Some("Overall composite across all domains".to_string()),
                },
            ]
        });
        &DOMAINS
    }
}
