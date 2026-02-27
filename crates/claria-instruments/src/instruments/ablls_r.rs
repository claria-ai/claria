use crate::scoring::{Domain, ScoreRange, ScoreType, Subscale};
use crate::Instrument;

/// ABLLS-R: Assessment of Basic Language and Learning Skills – Revised.
/// 25 lettered domains (A–Y), 544 skills, scored 0–4.
pub struct AbllsR;

impl Instrument for AbllsR {
    fn id(&self) -> &str {
        "ablls_r"
    }

    fn name(&self) -> &str {
        "ABLLS-R"
    }

    fn domains(&self) -> &[Domain] {
        static DOMAINS: std::sync::LazyLock<Vec<Domain>> = std::sync::LazyLock::new(|| {
            let rating_range = ScoreRange {
                min: 0.0,
                max: 4.0,
                step: Some(1.0),
            };

            let domain_defs = [
                // Basic Learner Skills
                ("A", "Cooperation and Reinforcer Effectiveness"),
                ("B", "Visual Performance"),
                ("C", "Receptive Language"),
                ("D", "Motor Imitation"),
                ("E", "Vocal Imitation"),
                ("F", "Requests"),
                ("G", "Labeling"),
                ("H", "Intraverbals"),
                ("I", "Spontaneous Vocalizations"),
                ("J", "Syntax and Grammar"),
                ("K", "Play and Leisure"),
                ("L", "Social Interaction"),
                ("M", "Group Instruction"),
                ("N", "Classroom Routines"),
                // Academic Skills
                ("O", "Generalized Responding"),
                ("P", "Reading"),
                ("Q", "Math"),
                ("R", "Writing"),
                ("S", "Spelling"),
                // Self-Help Skills
                ("T", "Dressing"),
                ("U", "Eating"),
                ("V", "Grooming"),
                ("W", "Toileting"),
                // Motor Skills
                ("X", "Gross Motor"),
                ("Y", "Fine Motor"),
            ];

            domain_defs
                .iter()
                .map(|(letter, name)| {
                    let id = format!("domain_{}", letter.to_lowercase());
                    Domain {
                        id: id.clone(),
                        name: format!("{letter}. {name}"),
                        subscales: vec![Subscale {
                            id: format!("{id}_score"),
                            name: name.to_string(),
                            score_type: ScoreType::Rating,
                            range: rating_range,
                            description: None,
                        }],
                        composite_score_type: None,
                        composite_range: None,
                        description: None,
                    }
                })
                .collect()
        });
        &DOMAINS
    }
}
