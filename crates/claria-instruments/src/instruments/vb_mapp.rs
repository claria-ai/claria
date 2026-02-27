use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// VB-MAPP: Verbal Behavior Milestones Assessment and Placement Program.
/// 16 skill areas across 3 developmental levels, 170 milestones, 0/0.5/1 scoring.
pub struct VbMapp;

impl Instrument for VbMapp {
    fn id(&self) -> &str {
        "vb_mapp"
    }

    fn name(&self) -> &str {
        "VB-MAPP"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let milestone_range = ScoreRange {
                min: 0.0,
                max: 1.0,
                step: Some(0.5),
            };

            let skill_areas = [
                ("mand", "Mand"),
                ("tact", "Tact"),
                ("echoic", "Echoic"),
                ("intraverbal", "Intraverbal"),
                ("listener_responding", "Listener Responding"),
                ("motor_imitation", "Motor Imitation"),
                ("visual_perceptual", "Visual Perceptual Skills and Match-to-Sample"),
                ("independent_play", "Independent Play"),
                ("social_behavior", "Social Behavior and Social Play"),
                ("spontaneous_vocal", "Spontaneous Vocal Behavior"),
                ("listener_by_function", "Listener Responding by Function, Feature, and Class"),
                ("reading", "Reading"),
                ("writing", "Writing"),
                ("math", "Math"),
                ("group_classroom", "Group and Classroom Skills"),
                ("linguistics", "Linguistic Structure"),
            ];

            let mut domains = Vec::new();

            // Milestones Assessment domain (the 16 skill areas)
            let subscales: Vec<Subscale> = skill_areas
                .iter()
                .map(|(id, name)| Subscale {
                    id: id.to_string(),
                    name: name.to_string(),
                    score_type: ScoreType::Milestone,
                    range: milestone_range,
                    description: None,
                })
                .collect();

            domains.push(Domain {
                id: "milestones".to_string(),
                name: "Milestones Assessment".to_string(),
                subscales,
                composite_score_type: None,
                composite_range: None,
                description: Some(
                    "170 milestones across 16 skill areas, scored 0/0.5/1".to_string(),
                ),
            });

            // Barriers Assessment domain
            domains.push(Domain {
                id: "barriers".to_string(),
                name: "Barriers Assessment".to_string(),
                subscales: vec![Subscale {
                    id: "barriers_total".to_string(),
                    name: "Barriers Total".to_string(),
                    score_type: ScoreType::Rating,
                    range: ScoreRange {
                        min: 0.0,
                        max: 4.0,
                        step: Some(1.0),
                    },
                    description: Some("24 barriers rated 0-4".to_string()),
                }],
                composite_score_type: None,
                composite_range: None,
                description: Some("Assessment of barriers to learning".to_string()),
            });

            domains
        });
        &DOMAINS
    }
}
