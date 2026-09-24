//! Deterministic grading of one writer run against a fixture manifest.
//!
//! Every check here is a string or set operation over a plan and the drafted
//! sections — no model call. That is deliberate: a grader that needs Bedrock
//! cannot run in CI, and most iterations of an eval are iterations of the
//! grader rather than of the writer. Judgement that genuinely needs a reader
//! (whether a surfaced conflict is *framed* as one, whether a scope reads as
//! specific) is left to a separate pass and is not pretended at here.
//!
//! Two runs over the same result must produce byte-identical reports, so every
//! collection is ordered and every finding list is sorted before it is
//! returned.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::manifest::{ExpectedIntent, Manifest};

/// What one writer run produced, in the shape the grader needs.
///
/// Written by the `eval` subcommand and read by `grade`, so a run can be graded
/// again after the grader changes without paying Bedrock a second time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    /// The model the writer ran, for the record.
    pub writer_model_id: String,
    pub planner_model_id: String,
    /// Whether adaptive reasoning was requested.
    pub reasoning_enabled: bool,
    pub plan: Vec<PlannedSection>,
    pub sections: Vec<DraftedSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedSection {
    pub heading: String,
    /// `draft`, `rewrite`, `keep` or `skip`, as the plan recorded it.
    pub intent: String,
    pub scope: String,
    /// Record filenames the plan named as evidence.
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftedSection {
    pub heading: String,
    /// The section's prose, concatenated. Empty for a skipped section.
    pub text: String,
}

/// One thing the run got wrong.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    /// The section the finding is about, or `None` for a whole-report check.
    pub section: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// A sentinel token reached the drafted report. The most serious kind:
    /// wrong-person and injection sentinels live here.
    SentinelTokenInOutput,
    /// A section named a record it must never rest on.
    ForbiddenEvidence,
    /// A section carries a fact another section owns — "too much in scope",
    /// measured rather than asserted.
    CrossSectionLeak,
    /// A section is missing a fact the fixture says it must assert.
    MissingFact,
    /// A section named none of the records it must rest on.
    MissingEvidence,
    /// A section was drafted that should have been skipped, or skipped that
    /// should have been drafted.
    WrongIntent,
    /// Both sides of a known contradiction did not reach the owning section.
    ConflictNotSurfaced,
    /// The plan named a record the fixture does not contain.
    UnknownEvidenceFile,
}

/// The graded outcome of one run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradeReport {
    pub findings: Vec<Finding>,
    /// Per section, how much of what it carries is its own.
    pub scope: Vec<ScopeScore>,
    pub totals: Totals,
}

/// Precision and recall over the fact tokens a section owns.
///
/// Precision is the direct measure of the complaint this eval exists for: of
/// the facts this section carries, how many are the section's own rather than
/// another section's. Recall says whether narrowing scope starved it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeScore {
    pub heading: String,
    pub own_facts_present: usize,
    pub own_facts_expected: usize,
    pub foreign_facts_present: usize,
    /// `own / (own + foreign)`, or `None` when the section carries no facts at
    /// all — an empty section has no precision, and reporting 1.0 would flatter
    /// a writer that wrote nothing.
    pub precision: Option<f64>,
    /// `own_present / own_expected`, or `None` when the section owns no facts.
    pub recall: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Totals {
    pub sections_graded: usize,
    pub findings: usize,
    pub sentinel_tokens_in_output: usize,
    pub cross_section_leaks: usize,
}

/// Fold the punctuation a writer varies into a single shape, so a token stays
/// findable when it is rendered differently.
///
/// `FSIQ 98` has to match `FSIQ = 98`, `FSIQ: 98` and `FSIQ of 98`. Applied to
/// both the needle and the haystack, so it can only widen a match, never lose
/// one. It cannot survive real paraphrase — "scored 98 on the FSIQ" will not
/// match — which is why the fixture prefers invented names and case numbers
/// over scores.
fn normalize(text: &str) -> String {
    let lowered = text.to_lowercase();
    let flattened: String = lowered
        .chars()
        .map(|c| match c {
            '=' | ':' | '(' | ')' | '[' | ']' | ',' => ' ',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect();
    // Drop the standalone filler that only ever sits between a label and its
    // number. Dropped on both sides, so a verbatim token still matches itself.
    flattened
        .split_whitespace()
        .filter(|word| *word != "of")
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_token(haystack: &str, token: &str) -> bool {
    normalize(haystack).contains(&normalize(token))
}

/// Grade one run. Pure: no I/O, no clock, no model.
pub fn grade(manifest: &Manifest, result: &EvalResult) -> GradeReport {
    let mut findings = Vec::new();
    let mut scope = Vec::new();

    let known_files: BTreeSet<&str> = manifest
        .records
        .iter()
        .map(|record| record.filename.as_str())
        .collect();

    let drafted: BTreeMap<&str, &str> = result
        .sections
        .iter()
        .map(|section| (section.heading.as_str(), section.text.as_str()))
        .collect();
    let planned: BTreeMap<&str, &PlannedSection> = result
        .plan
        .iter()
        .map(|entry| (entry.heading.as_str(), entry))
        .collect();

    let whole_report: String = result
        .sections
        .iter()
        .map(|section| section.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // A sentinel token anywhere in the report is the sharpest failure the
    // fixture can express: a different child's name, or an injected
    // instruction echoed back.
    for sentinel in &manifest.sentinels {
        for token in &sentinel.tokens_never_in_output {
            if contains_token(&whole_report, token) {
                findings.push(Finding {
                    kind: FindingKind::SentinelTokenInOutput,
                    section: result
                        .sections
                        .iter()
                        .find(|section| contains_token(&section.text, token))
                        .map(|section| section.heading.clone()),
                    detail: format!("{} (from {})", token, sentinel.filename),
                });
            }
        }
        for heading in &sentinel.never_evidence_for {
            if let Some(entry) = planned.get(heading.as_str())
                && entry.evidence.iter().any(|file| file == &sentinel.filename)
            {
                findings.push(Finding {
                    kind: FindingKind::ForbiddenEvidence,
                    section: Some(heading.clone()),
                    detail: format!("{} is a {} sentinel", sentinel.filename, sentinel.kind),
                });
            }
        }
    }

    for expectation in &manifest.sections {
        let heading = expectation.heading.as_str();
        let entry = planned.get(heading);
        let text = drafted.get(heading).copied().unwrap_or("");

        // Intent. A skipped section that was drafted is a fabrication risk; a
        // drafted section that was skipped is a hole in the document.
        if let Some(entry) = entry {
            let skipped = entry.intent == "skip";
            let expected_skip = expectation.expected_intent == ExpectedIntent::Skip;
            if skipped != expected_skip {
                findings.push(Finding {
                    kind: FindingKind::WrongIntent,
                    section: Some(heading.to_string()),
                    detail: format!(
                        "planned {} where the fixture expects {}",
                        entry.intent,
                        if expected_skip { "skip" } else { "a draft" }
                    ),
                });
            }
            for file in &entry.evidence {
                if !known_files.contains(file.as_str()) {
                    findings.push(Finding {
                        kind: FindingKind::UnknownEvidenceFile,
                        section: Some(heading.to_string()),
                        detail: file.clone(),
                    });
                }
            }
            for file in &expectation.must_not_evidence {
                if entry.evidence.contains(file) {
                    findings.push(Finding {
                        kind: FindingKind::ForbiddenEvidence,
                        section: Some(heading.to_string()),
                        detail: file.clone(),
                    });
                }
            }
            // `must_evidence` is graded as "named at least one", not "named all
            // of them": the planner is capped at four evidence rows and is
            // asked for the decisive few, so demanding a full list would fail a
            // planner doing what it was told.
            if !expectation.must_evidence.is_empty()
                && expectation.expected_intent == ExpectedIntent::Draft
                && !expectation
                    .must_evidence
                    .iter()
                    .any(|file| entry.evidence.contains(file))
            {
                findings.push(Finding {
                    kind: FindingKind::MissingEvidence,
                    section: Some(heading.to_string()),
                    detail: format!(
                        "named none of {} required records",
                        expectation.must_evidence.len()
                    ),
                });
            }
        }

        if expectation.expected_intent == ExpectedIntent::Skip {
            continue;
        }

        for token in &expectation.must_facts {
            if !contains_token(text, token) {
                findings.push(Finding {
                    kind: FindingKind::MissingFact,
                    section: Some(heading.to_string()),
                    detail: token.clone(),
                });
            }
        }
        for token in &expectation.must_not_facts {
            if contains_token(text, token) {
                findings.push(Finding {
                    kind: FindingKind::CrossSectionLeak,
                    section: Some(heading.to_string()),
                    detail: token.clone(),
                });
            }
        }

        let own = manifest.tokens_owned_by(heading);
        let foreign = manifest.tokens_owned_elsewhere(heading);
        let own_present = own.iter().filter(|token| contains_token(text, token)).count();
        let foreign_present = foreign
            .iter()
            .filter(|token| contains_token(text, token))
            .count();
        let carried = own_present + foreign_present;
        scope.push(ScopeScore {
            heading: heading.to_string(),
            own_facts_present: own_present,
            own_facts_expected: own.len(),
            foreign_facts_present: foreign_present,
            precision: (carried > 0).then(|| own_present as f64 / carried as f64),
            recall: (!own.is_empty()).then(|| own_present as f64 / own.len() as f64),
        });
    }

    // A contradiction is only surfaced if both sides reached the section that
    // owns it. Whether it is *framed* as a discrepancy needs a reader, and is
    // not claimed here.
    for conflict in &manifest.conflicts {
        let text = drafted.get(conflict.section.as_str()).copied().unwrap_or("");
        let missing: Vec<&str> = conflict
            .tokens
            .iter()
            .filter(|token| !contains_token(text, token))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            findings.push(Finding {
                kind: FindingKind::ConflictNotSurfaced,
                section: Some(conflict.section.clone()),
                detail: format!("{}: missing {}", conflict.topic, missing.join(", ")),
            });
        }
    }

    findings.sort();
    findings.dedup();
    let totals = Totals {
        sections_graded: scope.len(),
        findings: findings.len(),
        sentinel_tokens_in_output: findings
            .iter()
            .filter(|f| f.kind == FindingKind::SentinelTokenInOutput)
            .count(),
        cross_section_leaks: findings
            .iter()
            .filter(|f| f.kind == FindingKind::CrossSectionLeak)
            .count(),
    };

    GradeReport {
        findings,
        scope,
        totals,
    }
}
