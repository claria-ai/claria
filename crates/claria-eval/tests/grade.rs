//! Fault injection for the writer-eval grader.
//!
//! A grader nobody has broken on purpose is a grader that reports success. Each
//! test here builds a run that is correct except for one planted defect and
//! asserts that exactly that defect is found — and the first test asserts a
//! clean run is graded clean, so the others are not passing because everything
//! fails.
//!
//! Reads the committed fixture, so it also fails if the manifest and the grader
//! stop agreeing on the schema. No Bedrock, no network, no cost.

use std::path::PathBuf;

use claria_eval::{
    grade::{DraftedSection, EvalResult, FindingKind, PlannedSection, grade},
    manifest::{ExpectedIntent, Manifest},
};

fn manifest() -> Manifest {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/writer-eval/manifest.json");
    Manifest::load(&path).expect("the committed fixture manifest loads")
}

/// A run that satisfies every expectation the manifest states: each drafted
/// section carries exactly the tokens it owns, and names its required records.
fn perfect_run(manifest: &Manifest) -> EvalResult {
    let mut plan = Vec::new();
    let mut sections = Vec::new();
    for expectation in &manifest.sections {
        let skip = expectation.expected_intent == ExpectedIntent::Skip;
        plan.push(PlannedSection {
            heading: expectation.heading.clone(),
            intent: if skip { "skip" } else { "draft" }.to_string(),
            scope: format!("What {} must assert.", expectation.heading),
            evidence: expectation.must_evidence.clone(),
        });
        let text = if skip {
            String::new()
        } else {
            // Every token this section owns, and nothing else.
            manifest
                .tokens_owned_by(&expectation.heading)
                .into_iter()
                .collect::<Vec<_>>()
                .join(". ")
        };
        sections.push(DraftedSection {
            heading: expectation.heading.clone(),
            text,
        });
    }
    EvalResult {
        writer_model_id: "us.anthropic.claude-opus-4-6-v1".to_string(),
        planner_model_id: "us.anthropic.claude-sonnet-test".to_string(),
        reasoning_enabled: true,
        plan,
        sections,
    }
}

fn kinds(report: &claria_eval::grade::GradeReport) -> Vec<FindingKind> {
    let mut kinds: Vec<FindingKind> = report.findings.iter().map(|f| f.kind).collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

#[test]
fn a_perfect_run_is_graded_clean() {
    let manifest = manifest();
    let report = grade(&manifest, &perfect_run(&manifest));
    assert!(
        report.findings.is_empty(),
        "a run built from the manifest's own expectations must grade clean, \
         otherwise every other test here passes for the wrong reason: {:?}",
        report.findings
    );
}

#[test]
fn a_perfect_run_scores_full_precision() {
    let manifest = manifest();
    let report = grade(&manifest, &perfect_run(&manifest));
    for score in &report.scope {
        assert_eq!(
            score.foreign_facts_present, 0,
            "{} carried a fact it does not own",
            score.heading
        );
        if score.own_facts_expected > 0 {
            assert_eq!(score.precision, Some(1.0), "{}", score.heading);
            assert_eq!(score.recall, Some(1.0), "{}", score.heading);
        }
    }
}

#[test]
fn a_sentinel_token_in_the_output_is_caught() {
    let manifest = manifest();
    let sentinel = manifest
        .sentinels
        .iter()
        .find(|s| !s.tokens_never_in_output.is_empty())
        .expect("the fixture plants at least one never-in-output token");
    let token = sentinel.tokens_never_in_output[0].clone();

    let mut run = perfect_run(&manifest);
    let victim = run
        .sections
        .iter_mut()
        .find(|section| !section.text.is_empty())
        .expect("some section is drafted");
    victim.text.push_str(&format!(" Also noted: {token}."));

    let report = grade(&manifest, &run);
    assert!(
        kinds(&report).contains(&FindingKind::SentinelTokenInOutput),
        "a planted sentinel token must be found: {:?}",
        report.findings
    );
    assert_eq!(report.totals.sentinel_tokens_in_output, 1);
}

/// The injection sentinel is checked case-insensitively, so a lower-cased echo
/// is still an echo.
#[test]
fn a_lower_cased_sentinel_echo_is_still_caught() {
    let manifest = manifest();
    let sentinel = manifest
        .sentinels
        .iter()
        .find(|s| s.kind == "prompt_injection")
        .expect("the fixture plants an injection sentinel");
    let token = sentinel.tokens_never_in_output[0].to_lowercase();

    let mut run = perfect_run(&manifest);
    run.sections
        .iter_mut()
        .find(|section| !section.text.is_empty())
        .expect("some section is drafted")
        .text
        .push_str(&format!(" {token}"));

    let report = grade(&manifest, &run);
    assert!(kinds(&report).contains(&FindingKind::SentinelTokenInOutput));
}

#[test]
fn naming_a_forbidden_record_as_evidence_is_caught() {
    let manifest = manifest();
    let sentinel = manifest
        .sentinels
        .iter()
        .find(|s| !s.never_evidence_for.is_empty())
        .expect("the fixture forbids at least one record somewhere");
    let heading = sentinel.never_evidence_for[0].clone();

    let mut run = perfect_run(&manifest);
    run.plan
        .iter_mut()
        .find(|entry| entry.heading == heading)
        .expect("the forbidden section is planned")
        .evidence
        .push(sentinel.filename.clone());

    let report = grade(&manifest, &run);
    assert!(
        kinds(&report).contains(&FindingKind::ForbiddenEvidence),
        "{:?}",
        report.findings
    );
}

/// The measurement the whole eval exists for: a section carrying a fact that
/// belongs to another section.
#[test]
fn a_cross_section_leak_is_caught_and_lowers_precision() {
    let manifest = manifest();
    let mut run = perfect_run(&manifest);

    // Take a token owned by one drafted section and put it in another.
    let (donor, recipient) = {
        let drafted: Vec<&str> = manifest
            .sections
            .iter()
            .filter(|s| s.expected_intent == ExpectedIntent::Draft)
            .map(|s| s.heading.as_str())
            .filter(|h| !manifest.tokens_owned_by(h).is_empty())
            .collect();
        (drafted[0].to_string(), drafted[1].to_string())
    };
    let stolen = manifest
        .tokens_owned_by(&donor)
        .into_iter()
        .next()
        .expect("the donor owns a token")
        .to_string();

    run.sections
        .iter_mut()
        .find(|section| section.heading == recipient)
        .expect("the recipient is drafted")
        .text
        .push_str(&format!(" {stolen}"));

    let report = grade(&manifest, &run);
    let score = report
        .scope
        .iter()
        .find(|score| score.heading == recipient)
        .expect("the recipient is scored");
    assert_eq!(
        score.foreign_facts_present, 1,
        "a stolen token must count against the recipient"
    );
    assert!(
        score.precision.expect("the recipient carries facts") < 1.0,
        "precision must fall when a section carries someone else's fact"
    );
}

#[test]
fn a_starved_section_is_caught_by_recall() {
    let manifest = manifest();
    let mut run = perfect_run(&manifest);
    let heading = manifest
        .sections
        .iter()
        .find(|s| {
            s.expected_intent == ExpectedIntent::Draft
                && manifest.tokens_owned_by(&s.heading).len() > 1
        })
        .expect("some section owns more than one fact")
        .heading
        .clone();

    run.sections
        .iter_mut()
        .find(|section| section.heading == heading)
        .expect("the section is drafted")
        .text = "Nothing much to report.".to_string();

    let report = grade(&manifest, &run);
    let score = report
        .scope
        .iter()
        .find(|score| score.heading == heading)
        .expect("scored");
    assert_eq!(score.own_facts_present, 0);
    assert_eq!(score.recall, Some(0.0));
    // An empty section has no precision rather than a perfect one.
    assert_eq!(score.precision, None);
}

#[test]
fn drafting_a_section_the_fixture_expects_skipped_is_caught() {
    let manifest = manifest();
    let heading = manifest
        .sections
        .iter()
        .find(|s| s.expected_intent == ExpectedIntent::Skip)
        .expect("the fixture expects one section skipped")
        .heading
        .clone();

    let mut run = perfect_run(&manifest);
    run.plan
        .iter_mut()
        .find(|entry| entry.heading == heading)
        .expect("planned")
        .intent = "draft".to_string();

    let report = grade(&manifest, &run);
    assert!(
        kinds(&report).contains(&FindingKind::WrongIntent),
        "{:?}",
        report.findings
    );
}

#[test]
fn a_hallucinated_evidence_filename_is_caught() {
    let manifest = manifest();
    let mut run = perfect_run(&manifest);
    run.plan[0]
        .evidence
        .push("2019-02-30-a-record-nobody-uploaded.txt".to_string());

    let report = grade(&manifest, &run);
    assert!(
        kinds(&report).contains(&FindingKind::UnknownEvidenceFile),
        "{:?}",
        report.findings
    );
}

#[test]
fn dropping_one_side_of_a_contradiction_is_caught() {
    let manifest = manifest();
    let conflict = manifest
        .conflicts
        .first()
        .expect("the fixture plants a contradiction")
        .clone();

    let mut run = perfect_run(&manifest);
    let section = run
        .sections
        .iter_mut()
        .find(|section| section.heading == conflict.section)
        .expect("the owning section is drafted");
    // Keep the first value, silently drop the second — the failure mode a
    // clinician would never notice.
    section.text = section.text.replace(&conflict.tokens[1], "");

    let report = grade(&manifest, &run);
    assert!(
        kinds(&report).contains(&FindingKind::ConflictNotSurfaced),
        "{:?}",
        report.findings
    );
}

/// A score rendered differently is still the same score.
#[test]
fn punctuation_around_a_score_does_not_hide_it() {
    let manifest = manifest();
    let mut run = perfect_run(&manifest);

    let heading = manifest
        .sections
        .iter()
        .find(|s| {
            s.expected_intent == ExpectedIntent::Draft
                && manifest
                    .tokens_owned_by(&s.heading)
                    .iter()
                    .any(|t| t.contains(' ') && t.chars().any(char::is_numeric))
        })
        .map(|s| s.heading.clone());
    let Some(heading) = heading else {
        return; // No numeric token to rewrite; nothing to assert.
    };
    let token = manifest
        .tokens_owned_by(&heading)
        .into_iter()
        .find(|t| t.contains(' ') && t.chars().any(char::is_numeric))
        .expect("checked above")
        .to_string();
    let (label, value) = token.rsplit_once(' ').expect("token has a space");

    let section = run
        .sections
        .iter_mut()
        .find(|section| section.heading == heading)
        .expect("drafted");
    section.text = section
        .text
        .replace(&token, &format!("{label} = {value}"));

    let report = grade(&manifest, &run);
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.kind == FindingKind::MissingFact && f.detail == token),
        "a token rendered with an equals sign must still be found: {:?}",
        report.findings
    );
}

/// The manifest must declare itself synthetic, or loading refuses. A corpus of
/// clinical paperwork that is not marked synthetic has no business here.
#[test]
fn a_manifest_not_marked_synthetic_is_refused() {
    let mut value: serde_json::Value = serde_json::to_value(manifest()).expect("re-serialize");
    value["synthetic"] = serde_json::Value::Bool(false);
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&value).expect("bytes")).expect("write");

    let error = Manifest::load(&path).expect_err("a non-synthetic manifest must be refused");
    assert!(
        error.to_string().contains("not marked synthetic"),
        "{error}"
    );
}
