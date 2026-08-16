use claria_core::models::{
    findings::{
        ConflictingRef, Finding, FindingAnchor, FindingStatus, MAX_FINDING_DESCRIPTION_CHARACTERS,
        MAX_REPORT_FINDINGS, REPORT_FINDINGS_SCHEMA_VERSION, RecordCitation, ReportFindings,
        ReviewCoverage, ReviewPass, StyleProposal, TextSpan, decode_report_findings,
        finding_is_stale,
    },
    report::{AuthorshipKind, ReportBlock, ReportContent, ReportSection, SectionAuthorship},
};
use jiff::Timestamp;
use uuid::Uuid;

fn timestamp(value: &str) -> Timestamp {
    value.parse().expect("timestamp")
}

const CREATED: &str = "2026-08-01T12:00:00Z";
const RESOLVED: &str = "2026-08-01T12:05:00Z";

fn findings() -> ReportFindings {
    ReportFindings::new(Uuid::new_v4(), Uuid::new_v4(), timestamp(RESOLVED))
}

fn style_finding(section_id: Uuid, revision: u64) -> Finding {
    Finding {
        id: Uuid::new_v4(),
        pass: ReviewPass::Style,
        property: "tense_drift".to_string(),
        model_id: "us.anthropic.claude-sonnet-test".to_string(),
        anchor: FindingAnchor {
            section_id,
            revision,
        },
        description: "The paragraph switches from past to present tense.".to_string(),
        span: Some(TextSpan {
            block_index: 0,
            start_char: 4,
            end_char: 12,
        }),
        conflicting: None,
        record_citation: None,
        proposal: Some(StyleProposal {
            block_index: 0,
            original_text: "is reported".to_string(),
            replacement_text: "was reported".to_string(),
        }),
        status: FindingStatus::Open,
        applied_revision: None,
        resolved_at: None,
        created_at: timestamp(CREATED),
    }
}

fn consistency_finding(section_id: Uuid, revision: u64) -> Finding {
    Finding {
        pass: ReviewPass::Consistency,
        property: "internal_contradiction".to_string(),
        proposal: None,
        conflicting: Some(ConflictingRef {
            section_id: None,
            quote: "the client declined the assessment".to_string(),
            span: None,
        }),
        record_citation: Some(RecordCitation {
            filename: "intake.pdf".to_string(),
            quote: "the client completed the assessment".to_string(),
        }),
        ..style_finding(section_id, revision)
    }
}

fn coverage(property: &str, pass: ReviewPass) -> ReviewCoverage {
    ReviewCoverage {
        pass,
        property: property.to_string(),
        model_id: "us.anthropic.claude-sonnet-test".to_string(),
        revision: 3,
        sections_reviewed: 4,
        findings: 1,
        completed_at: timestamp(RESOLVED),
    }
}

fn content(sections: Vec<ReportSection>) -> ReportContent {
    ReportContent {
        title: "Assessment".to_string(),
        sections,
    }
}

fn section(id: Uuid, authorship: Option<SectionAuthorship>) -> ReportSection {
    ReportSection {
        id,
        heading: "History".to_string(),
        blocks: vec![ReportBlock::Paragraph {
            text: "It is reported that the client attended.".to_string(),
        }],
        skipped: false,
        template_blocks: None,
        authorship,
    }
}

fn stamp(revision: u64) -> SectionAuthorship {
    SectionAuthorship {
        kind: AuthorshipKind::ModelRevised,
        revision,
        model_id: Some("us.anthropic.claude-sonnet-test".to_string()),
        run_id: None,
        updated_at: timestamp(CREATED),
    }
}

#[test]
fn empty_findings_round_trip_through_decode() {
    let mut findings = findings();
    findings.findings = vec![style_finding(Uuid::new_v4(), 3)];
    findings.coverage = vec![
        coverage("tense_drift", ReviewPass::Style),
        coverage("internal_contradiction", ReviewPass::Consistency),
    ];
    let bytes = serde_json::to_vec(&findings).expect("encode");
    let decoded = decode_report_findings(&bytes).expect("decode");
    assert_eq!(decoded, findings);
    assert_eq!(decoded.schema_version, REPORT_FINDINGS_SCHEMA_VERSION);
}

#[test]
fn wire_format_is_snake_case() {
    let mut findings = findings();
    findings.findings = vec![consistency_finding(Uuid::new_v4(), 3)];
    let value = serde_json::to_value(&findings).expect("encode");
    assert_eq!(
        value["findings"][0]["pass"],
        serde_json::json!("consistency")
    );
    assert_eq!(value["findings"][0]["status"], serde_json::json!("open"));
    assert_eq!(
        value["findings"][0]["record_citation"]["filename"],
        serde_json::json!("intake.pdf")
    );
}

#[test]
fn a_newer_schema_version_is_refused_rather_than_partially_read() {
    let mut findings = findings();
    findings.schema_version = REPORT_FINDINGS_SCHEMA_VERSION + 1;
    let bytes = serde_json::to_vec(&findings).expect("encode");
    let error = decode_report_findings(&bytes).unwrap_err();
    assert!(error.to_string().contains("newer than supported version"));
}

#[test]
fn a_consistency_finding_may_never_carry_a_replacement() {
    let mut findings = findings();
    let mut finding = consistency_finding(Uuid::new_v4(), 3);
    finding.proposal = Some(StyleProposal {
        block_index: 0,
        original_text: "attended".to_string(),
        replacement_text: "did not attend".to_string(),
    });
    findings.findings = vec![finding];

    let error = findings.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must not carry a replacement proposal")
    );
}

#[test]
fn only_a_style_finding_can_be_applied_and_only_with_a_revision() {
    let mut findings = findings();
    let mut applied = style_finding(Uuid::new_v4(), 3);
    applied.status = FindingStatus::Applied;
    applied.resolved_at = Some(timestamp(RESOLVED));
    findings.findings = vec![applied.clone()];
    let error = findings.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("record the revision it produced")
    );

    applied.applied_revision = Some(4);
    findings.findings = vec![applied.clone()];
    findings
        .validate()
        .expect("an applied style finding is valid");

    let mut consistency = consistency_finding(Uuid::new_v4(), 3);
    consistency.status = FindingStatus::Applied;
    consistency.applied_revision = Some(4);
    consistency.resolved_at = Some(timestamp(RESOLVED));
    findings.findings = vec![consistency];
    let error = findings.validate().unwrap_err();
    assert!(error.to_string().contains("only a style finding"));
}

#[test]
fn an_open_finding_carries_neither_resolution_field() {
    let mut findings = findings();
    let mut finding = style_finding(Uuid::new_v4(), 3);
    finding.resolved_at = Some(timestamp(RESOLVED));
    findings.findings = vec![finding.clone()];
    let error = findings.validate().unwrap_err();
    assert!(error.to_string().contains("must not record a resolution"));

    finding.resolved_at = None;
    finding.applied_revision = Some(4);
    findings.findings = vec![finding.clone()];
    let error = findings.validate().unwrap_err();
    assert!(error.to_string().contains("only an applied finding"));

    // Dismissal is a resolution, so it has to say when.
    finding.applied_revision = None;
    finding.status = FindingStatus::Dismissed;
    findings.findings = vec![finding];
    let error = findings.validate().unwrap_err();
    assert!(error.to_string().contains("must record when"));
}

#[test]
fn findings_are_capped_and_uniquely_identified() {
    let mut findings = findings();
    let section_id = Uuid::new_v4();
    findings.findings = (0..=MAX_REPORT_FINDINGS)
        .map(|_| style_finding(section_id, 3))
        .collect();
    let error = findings.validate().unwrap_err();
    assert!(error.to_string().contains("at most 200 review findings"));

    let duplicate = style_finding(section_id, 3);
    findings.findings = vec![duplicate.clone(), duplicate];
    let error = findings.validate().unwrap_err();
    assert!(error.to_string().contains("duplicate finding ID"));
}

#[test]
fn text_ceilings_are_enforced_on_every_quote_bearing_field() {
    let mut findings = findings();
    let section_id = Uuid::new_v4();

    let mut long_description = style_finding(section_id, 3);
    long_description.description = "x".repeat(MAX_FINDING_DESCRIPTION_CHARACTERS + 1);
    findings.findings = vec![long_description];
    assert!(
        findings
            .validate()
            .unwrap_err()
            .to_string()
            .contains("finding description exceeds 1000 characters")
    );

    let mut long_property = style_finding(section_id, 3);
    long_property.property = "x".repeat(65);
    findings.findings = vec![long_property];
    assert!(
        findings
            .validate()
            .unwrap_err()
            .to_string()
            .contains("finding property exceeds 64 characters")
    );

    let mut long_quote = consistency_finding(section_id, 3);
    long_quote.conflicting.as_mut().expect("conflicting").quote = "x".repeat(301);
    findings.findings = vec![long_quote];
    assert!(
        findings
            .validate()
            .unwrap_err()
            .to_string()
            .contains("conflicting quote exceeds 300 characters")
    );

    let mut noop_replacement = style_finding(section_id, 3);
    noop_replacement.proposal = Some(StyleProposal {
        block_index: 0,
        original_text: "attended".to_string(),
        replacement_text: "attended".to_string(),
    });
    findings.findings = vec![noop_replacement];
    assert!(
        findings
            .validate()
            .unwrap_err()
            .to_string()
            .contains("must change the text")
    );
}

#[test]
fn coverage_rows_are_one_per_property() {
    let mut findings = findings();
    findings.coverage = vec![
        coverage("tense_drift", ReviewPass::Style),
        coverage("tense_drift", ReviewPass::Style),
    ];
    let error = findings.validate().unwrap_err();
    assert!(error.to_string().contains("duplicate review coverage"));
}

#[test]
fn staleness_is_derived_from_the_sections_authorship() {
    let section_id = Uuid::new_v4();
    let finding = style_finding(section_id, 3);

    // Never stamped: nothing has rewritten the section since the review.
    assert!(!finding_is_stale(
        &finding,
        &content(vec![section(section_id, None)])
    ));
    // Stamped at or before the reviewed revision: still the text reviewed.
    assert!(!finding_is_stale(
        &finding,
        &content(vec![section(section_id, Some(stamp(3)))])
    ));
    assert!(!finding_is_stale(
        &finding,
        &content(vec![section(section_id, Some(stamp(2)))])
    ));
    // Rewritten since.
    assert!(finding_is_stale(
        &finding,
        &content(vec![section(section_id, Some(stamp(4)))])
    ));
    // Section deleted outright.
    assert!(finding_is_stale(
        &finding,
        &content(vec![section(Uuid::new_v4(), Some(stamp(9)))])
    ));
    assert!(finding_is_stale(&finding, &content(vec![])));
}
