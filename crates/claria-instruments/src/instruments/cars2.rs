use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// CARS-2: Childhood Autism Rating Scale, Second Edition.
/// 15 items, each rated 1–4 (half-point increments allowed). Total 15–60.
pub struct Cars2;

impl Instrument for Cars2 {
    fn id(&self) -> &str {
        "cars2"
    }

    fn name(&self) -> &str {
        "CARS-2"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let item_range = ScoreRange {
                min: 1.0,
                max: 4.0,
                step: Some(0.5),
            };

            let items = [
                ("relating_to_people", "Relating to People"),
                ("imitation", "Imitation"),
                ("emotional_response", "Emotional Response"),
                ("body_use", "Body Use"),
                ("object_use", "Object Use"),
                ("adaptation_to_change", "Adaptation to Change"),
                ("visual_response", "Visual Response"),
                ("listening_response", "Listening Response"),
                ("taste_smell_touch", "Taste, Smell, and Touch Response and Use"),
                ("fear_or_nervousness", "Fear or Nervousness"),
                ("verbal_communication", "Verbal Communication"),
                ("nonverbal_communication", "Nonverbal Communication"),
                ("activity_level", "Activity Level"),
                ("intellectual_response", "Level and Consistency of Intellectual Response"),
                ("general_impressions", "General Impressions"),
            ];

            let subscales: Vec<Subscale> = items
                .iter()
                .map(|(id, name)| Subscale {
                    id: id.to_string(),
                    name: name.to_string(),
                    score_type: ScoreType::Rating,
                    range: item_range,
                    description: None,
                })
                .collect();

            vec![Domain {
                id: "cars2_items".to_string(),
                name: "CARS-2 Items".to_string(),
                subscales,
                composite_score_type: Some(ScoreType::Raw),
                composite_range: Some(ScoreRange {
                    min: 15.0,
                    max: 60.0,
                    step: Some(0.5),
                }),
                description: Some(
                    "15-29.5: minimal-to-no symptoms, 30-36.5: mild-to-moderate, 37+: severe"
                        .to_string(),
                ),
            }]
        });
        &DOMAINS
    }
}
