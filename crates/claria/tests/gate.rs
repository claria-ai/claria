//! The completion gate: what code can decide about a finished report, and
//! what it refuses to hold against one.
//!
//! Every case here is built out of durable state alone — no Bedrock call is
//! scripted in this file, and none is needed, which is the property under
//! test.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria::{self as pipeline, CompletionCheck, CompletionCheckKind};
use claria_core::models::{
    findings::{Finding, FindingAnchor, FindingStatus, ReportFindings, ReviewPass},
    report::{ReportBlock, ReportContent, ReportSection},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, PlanEntry, RecordCitation,
        RunInstruction, RunPlan, RunSection, RunSectionState, SectionIntent,
    },
};
use claria_mock_aws::testing::MockServer;
use claria_report_store as store;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-completion-gate-test";
const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";
const PLANNER_MODEL_ID: &str = "us.anthropic.claude-haiku-test";

const REFERRAL_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const BACKGROUND_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SUMMARY_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

/// Wrapped on purpose: a citation that resolves across this line break is what
/// proves the gate normalizes whitespace before searching.
const INTAKE_TEXT: &str = "Referred by pediatrician for attention concerns.\nParent reports \
     homework avoidance at the kitchen table.";
/// Copied verbatim out of one line of the record.
const EXACT_QUOTE: &str = "Referred by pediatrician for attention concerns.";
/// The same words as the record, reflowed: a newline collapsed to a space and
/// a run of spaces collapsed to one.
const REFLOWED_QUOTE: &str = "attention concerns.    Parent reports\nhomework avoidance";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn setup() -> (MockServer, aws_sdk_s3::Client) {
    let server = MockServer::spawn().await;
    let s3 = client::from_config(&sdk_config(&server.endpoint));
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
    (server, s3)
}

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph {
        text: text.to_string(),
    }
}

fn section(id: &str, heading: &str, text: &str) -> ReportSection {
    ReportSection {
        id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        blocks: vec![paragraph(text)],
        skipped: false,
        template_blocks: None,
        template_directives: Vec::new(),
        authorship: None,
    }
}

fn skipped_section(id: &str, heading: &str) -> ReportSection {
    ReportSection {
        id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        blocks: Vec::new(),
        skipped: true,
        template_blocks: Some(vec![paragraph("Template boilerplate.")]),
        template_directives: Vec::new(),
        authorship: None,
    }
}

/// The written report every clean case in this file evaluates.
fn drafted_content() -> ReportContent {
    ReportContent {
        title: "Psychoeducational Evaluation".to_string(),
        sections: vec![
            section(
                REFERRAL_ID,
                "Reason for Referral",
                "The client was referred by his pediatrician for attention concerns.",
            ),
            section(
                BACKGROUND_ID,
                "Background",
                "The client attends fourth grade and avoids homework at home.",
            ),
            section(
                SUMMARY_ID,
                "Summary and Clinical Interpretation",
                "Findings are consistent with the referral question.",
            ),
        ],
    }
}

/// A client with one readable record and a three-section report saved as
/// revision 1.
async fn seed_report(s3: &aws_sdk_s3::Client, client_id: Uuid, content: ReportContent) -> Uuid {
    let now = jiff::Timestamp::now();
    let client = claria_core::models::client::Client {
        id: client_id,
        name: "Test client".to_string(),
        created_at: now,
        updated_at: now,
    };
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::client(client_id),
        serde_json::to_vec(&client).expect("client JSON"),
        Some("application/json"),
    )
    .await
    .expect("put client");
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::client_record_file(client_id, "intake.txt"),
        INTAKE_TEXT.as_bytes().to_vec(),
        Some("text/plain"),
    )
    .await
    .expect("put record");
    let workspace = store::start_report_workspace(s3, BUCKET, client_id)
        .await
        .expect("start writing session");
    store::save_report_draft_for_report(s3, BUCKET, client_id, workspace.report_id, 0, content)
        .await
        .expect("seed the drafted report");
    workspace.report_id
}

fn plan_entry(id: &str, heading: &str, intent: SectionIntent, required: bool) -> PlanEntry {
    PlanEntry {
        section_id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        intent,
        required,
        scope: String::new(),
        evidence: Vec::new(),
        instruction: None,
        curated_records: None,
    }
}

/// A plan a planning model produced and a clinician approved: the kind the
/// gate is entitled to hold a missing citation against.
fn planned(entries: Vec<PlanEntry>) -> RunPlan {
    let now = jiff::Timestamp::now();
    RunPlan {
        model_id: PLANNER_MODEL_ID.to_string(),
        entries,
        user_edited: true,
        synthetic: false,
        approved_at: Some(now),
        plan_warnings: Vec::new(),
        created_at: now,
    }
}

/// The plan the un-gated whole-report command manufactures: one `Draft` row
/// per section the report already had, nobody's decision.
fn synthetic(entries: Vec<PlanEntry>) -> RunPlan {
    RunPlan {
        synthetic: true,
        user_edited: false,
        model_id: MODEL_ID.to_string(),
        ..planned(entries)
    }
}

/// Every required row, drafted, in document order.
fn full_plan() -> RunPlan {
    planned(vec![
        plan_entry(
            REFERRAL_ID,
            "Reason for Referral",
            SectionIntent::Draft,
            true,
        ),
        plan_entry(BACKGROUND_ID, "Background", SectionIntent::Draft, true),
        plan_entry(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            SectionIntent::Draft,
            true,
        ),
    ])
}

fn run_section(
    id: &str,
    heading: &str,
    position: u32,
    state: RunSectionState,
    citations: Vec<RecordCitation>,
) -> RunSection {
    RunSection {
        section_id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        position,
        state,
        blocks: if state == RunSectionState::Drafted {
            vec![paragraph("Landed section content.")]
        } else {
            Vec::new()
        },
        citations,
        attempts: 1,
        error: None,
        records: None,
        updated_at: jiff::Timestamp::now(),
    }
}

fn cites(quote: &str) -> Vec<RecordCitation> {
    vec![RecordCitation {
        filename: "intake.txt".to_string(),
        quote: quote.to_string(),
    }]
}

/// Every section drafted, every required one citing the record verbatim.
fn drafted_sections() -> Vec<RunSection> {
    vec![
        run_section(
            REFERRAL_ID,
            "Reason for Referral",
            0,
            RunSectionState::Drafted,
            cites(EXACT_QUOTE),
        ),
        run_section(
            BACKGROUND_ID,
            "Background",
            1,
            RunSectionState::Drafted,
            cites(EXACT_QUOTE),
        ),
        run_section(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            2,
            RunSectionState::Drafted,
            cites(EXACT_QUOTE),
        ),
    ]
}

async fn put_run(s3: &aws_sdk_s3::Client, run: &DraftRun) {
    run.validate().expect("the seeded run is valid");
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::report_draft_run(run.client_id, run.report_id, run.run_id),
        serde_json::to_vec(run).expect("run JSON"),
        Some("application/json"),
    )
    .await
    .expect("seed the drafting run");
}

/// A run that already cut revision 1 — the shape the gate reads for a report
/// the writer has finished.
async fn seed_completed_run(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    plan: RunPlan,
    sections: Vec<RunSection>,
) -> Uuid {
    let now = jiff::Timestamp::now();
    let run = DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id: Uuid::new_v4(),
        report_id,
        client_id,
        base_revision: 0,
        status: DraftRunStatus::Completed,
        plan: Some(plan),
        title: Some("Psychoeducational Evaluation".to_string()),
        sections,
        instructions: vec![RunInstruction {
            text: "Write the whole evaluation.".to_string(),
            added_at: now,
        }],
        writer_model_id: MODEL_ID.to_string(),
        finalized_revision: Some(1),
        partial: false,
        record_snapshot: None,
        created_at: now,
        updated_at: now,
    };
    put_run(s3, &run).await;
    run.run_id
}

/// An interrupted run that still owns the workspace, which is how a stopped
/// draft looks to the gate.
async fn seed_stopped_run(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    plan: RunPlan,
    sections: Vec<RunSection>,
) -> Uuid {
    let now = jiff::Timestamp::now();
    let run = DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id: Uuid::new_v4(),
        report_id,
        client_id,
        base_revision: 1,
        status: DraftRunStatus::Stopped,
        plan: Some(plan),
        title: Some("Psychoeducational Evaluation".to_string()),
        sections,
        instructions: Vec::new(),
        writer_model_id: MODEL_ID.to_string(),
        finalized_revision: None,
        partial: false,
        record_snapshot: None,
        created_at: now,
        updated_at: now,
    };
    put_run(s3, &run).await;

    let mut loaded = store::load_for_report(s3, BUCKET, client_id, report_id)
        .await
        .expect("load the workspace");
    loaded.workspace.active_run_id = Some(run.run_id);
    loaded.workspace.updated_at = jiff::Timestamp::now();
    store::save_loaded(s3, BUCKET, &mut loaded)
        .await
        .expect("hand the workspace to the run");
    run.run_id
}

fn finding(section_id: &str, property: &str, revision: u64, status: FindingStatus) -> Finding {
    let now = jiff::Timestamp::now();
    Finding {
        id: Uuid::new_v4(),
        pass: ReviewPass::Consistency,
        property: property.to_string(),
        model_id: PLANNER_MODEL_ID.to_string(),
        anchor: FindingAnchor {
            section_id: section_id.parse().expect("section UUID"),
            revision,
        },
        description: "The claim is not supported by the records.".to_string(),
        span: None,
        conflicting: None,
        record_citation: None,
        proposal: None,
        status,
        applied_revision: None,
        resolved_at: (status != FindingStatus::Open).then_some(now),
        created_at: now,
    }
}

async fn seed_findings(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    findings: Vec<Finding>,
) {
    let mut loaded = store::load_report_findings(s3, BUCKET, client_id, report_id)
        .await
        .expect("load findings");
    loaded.findings = ReportFindings {
        findings,
        ..loaded.findings
    };
    loaded.findings.updated_at = jiff::Timestamp::now();
    store::save_report_findings(s3, BUCKET, &mut loaded)
        .await
        .expect("seed findings");
}

async fn evaluate(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
) -> pipeline::CompletionReport {
    pipeline::evaluate_report_completion(s3, BUCKET, client_id, report_id)
        .await
        .expect("evaluate completion")
}

/// The checks as `(kind, section, detail)`, which is what every assertion here
/// is actually about.
fn summarize(checks: &[CompletionCheck]) -> Vec<(CompletionCheckKind, Option<String>, String)> {
    checks
        .iter()
        .map(|check| {
            (
                check.kind,
                check.section_id.map(|id| id.to_string()),
                check.detail.clone(),
            )
        })
        .collect()
}

fn kinds(checks: &[CompletionCheck]) -> Vec<CompletionCheckKind> {
    checks.iter().map(|check| check.kind).collect()
}

#[tokio::test]
async fn a_finished_run_with_resolved_citations_is_complete() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    seed_completed_run(&s3, client_id, report_id, full_plan(), drafted_sections()).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&report.checks),
        Vec::new(),
        "no check should fail"
    );
    assert!(report.complete);
    assert_eq!(report.evaluated_revision, 1);
}

#[tokio::test]
async fn a_report_with_no_run_skips_the_run_checks() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert!(
        report.complete,
        "a hand-written report has no run to hold anything against: {:?}",
        report.checks
    );
}

#[tokio::test]
async fn a_quote_that_is_not_in_the_record_fails() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    let mut sections = drafted_sections();
    sections[1].citations = cites("Referred by the school psychologist for reading concerns.");
    seed_completed_run(&s3, client_id, report_id, full_plan(), sections).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert!(!report.complete);
    assert_eq!(
        summarize(&report.checks),
        vec![(
            CompletionCheckKind::UnresolvedCitation,
            Some(BACKGROUND_ID.to_string()),
            "unmatched_quote:intake.txt".to_string()
        )]
    );
}

#[tokio::test]
async fn a_quote_against_a_record_that_is_gone_fails() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    let mut sections = drafted_sections();
    sections[2].citations = vec![RecordCitation {
        filename: "testing-scores.txt".to_string(),
        quote: EXACT_QUOTE.to_string(),
    }];
    seed_completed_run(&s3, client_id, report_id, full_plan(), sections).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&report.checks),
        vec![(
            CompletionCheckKind::UnresolvedCitation,
            Some(SUMMARY_ID.to_string()),
            "missing_file:testing-scores.txt".to_string()
        )]
    );
}

#[tokio::test]
async fn a_reflowed_quote_still_resolves() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    let mut sections = drafted_sections();
    sections[0].citations = cites(REFLOWED_QUOTE);
    seed_completed_run(&s3, client_id, report_id, full_plan(), sections).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert!(
        report.complete,
        "a quote differing only in whitespace must resolve: {:?}",
        report.checks
    );
}

#[tokio::test]
async fn a_required_section_left_skipped_is_reported() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let mut content = drafted_content();
    content.sections[2] = skipped_section(SUMMARY_ID, "Summary and Clinical Interpretation");
    let report_id = seed_report(&s3, client_id, content).await;

    let mut sections = drafted_sections();
    sections[2] = run_section(
        SUMMARY_ID,
        "Summary and Clinical Interpretation",
        2,
        RunSectionState::Skipped,
        Vec::new(),
    );
    seed_completed_run(&s3, client_id, report_id, full_plan(), sections).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&report.checks),
        vec![(
            CompletionCheckKind::RequiredSectionEmpty,
            Some(SUMMARY_ID.to_string()),
            "skipped".to_string()
        )]
    );
}

#[tokio::test]
async fn a_required_section_the_writer_never_cited_is_reported() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    let mut sections = drafted_sections();
    sections[1].citations = Vec::new();
    seed_completed_run(&s3, client_id, report_id, full_plan(), sections).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&report.checks),
        vec![(
            CompletionCheckKind::MissingCitation,
            Some(BACKGROUND_ID.to_string()),
            "no_citations".to_string()
        )]
    );
}

#[tokio::test]
async fn a_synthetic_plan_is_never_asked_for_citations() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    let sections = drafted_sections()
        .into_iter()
        .map(|section| RunSection {
            citations: Vec::new(),
            ..section
        })
        .collect();
    let plan = synthetic(full_plan().entries);
    seed_completed_run(&s3, client_id, report_id, plan, sections).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert!(
        report.complete,
        "a run that predates evidence assignment cannot fail for citing nothing: {:?}",
        report.checks
    );
}

#[tokio::test]
async fn a_placeholder_left_in_a_section_is_reported() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let mut content = drafted_content();
    content.sections[0].blocks = vec![paragraph(
        "[CLIENT NAME] was referred by his pediatrician on [DATE].",
    )];
    let report_id = seed_report(&s3, client_id, content).await;
    seed_completed_run(&s3, client_id, report_id, full_plan(), drafted_sections()).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&report.checks),
        vec![(
            CompletionCheckKind::PlaceholderText,
            Some(REFERRAL_ID.to_string()),
            "2".to_string()
        )]
    );
}

#[tokio::test]
async fn a_skipped_sections_placeholders_are_not_reported() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let mut content = drafted_content();
    // Its body is empty and its template copy — which carries the marker —
    // never reaches the export.
    content.sections[2] = ReportSection {
        template_blocks: Some(vec![paragraph("Prepared for [CLIENT NAME].")]),
        ..skipped_section(SUMMARY_ID, "Summary and Clinical Interpretation")
    };
    let report_id = seed_report(&s3, client_id, content).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert!(
        report.complete,
        "a skipped section's template copy is not carryover: {:?}",
        report.checks
    );
}

#[tokio::test]
async fn an_open_finding_on_a_fresh_anchor_blocks_completion() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    seed_completed_run(&s3, client_id, report_id, full_plan(), drafted_sections()).await;
    seed_findings(
        &s3,
        client_id,
        report_id,
        vec![finding(
            BACKGROUND_ID,
            "unsupported_claim",
            1,
            FindingStatus::Open,
        )],
    )
    .await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&report.checks),
        vec![(
            CompletionCheckKind::UnresolvedFinding,
            Some(BACKGROUND_ID.to_string()),
            "unsupported_claim".to_string()
        )]
    );
}

#[tokio::test]
async fn an_open_finding_on_a_stale_anchor_does_not() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    seed_completed_run(&s3, client_id, report_id, full_plan(), drafted_sections()).await;
    // Revision 1 stamped every section's authorship, so a finding anchored to
    // revision 0 describes text the report has already moved past.
    seed_findings(
        &s3,
        client_id,
        report_id,
        vec![
            finding(BACKGROUND_ID, "unsupported_claim", 0, FindingStatus::Open),
            finding(SUMMARY_ID, "terminology", 1, FindingStatus::Dismissed),
        ],
    )
    .await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert!(
        report.complete,
        "neither a stale open finding nor a dismissed one is unresolved: {:?}",
        report.checks
    );
}

#[tokio::test]
async fn a_stopped_runs_undecided_sections_are_reported() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_report(&s3, client_id, drafted_content()).await;
    let sections = vec![
        run_section(
            REFERRAL_ID,
            "Reason for Referral",
            0,
            RunSectionState::Drafted,
            cites(EXACT_QUOTE),
        ),
        run_section(
            BACKGROUND_ID,
            "Background",
            1,
            RunSectionState::Pending,
            Vec::new(),
        ),
        run_section(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            2,
            RunSectionState::Failed,
            Vec::new(),
        ),
    ];
    seed_stopped_run(&s3, client_id, report_id, full_plan(), sections).await;

    let report = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&report.checks),
        vec![
            (
                CompletionCheckKind::SectionNotTerminal,
                Some(BACKGROUND_ID.to_string()),
                "pending".to_string()
            ),
            (
                CompletionCheckKind::SectionNotTerminal,
                Some(SUMMARY_ID.to_string()),
                "failed".to_string()
            ),
        ],
        "an unfinished run's citations are not checked; its undecided sections are"
    );
}

#[tokio::test]
async fn the_checklist_is_ordered_by_kind_then_by_document_position() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let mut content = drafted_content();
    content.sections[0].blocks = vec![paragraph("Prepared for [CLIENT NAME].")];
    content.sections[1] = skipped_section(BACKGROUND_ID, "Background");
    content.sections[2].blocks = vec![paragraph("Signed on [DATE].")];
    let report_id = seed_report(&s3, client_id, content).await;

    let sections = vec![
        run_section(
            REFERRAL_ID,
            "Reason for Referral",
            0,
            RunSectionState::Drafted,
            cites("A quote nobody wrote in any record here."),
        ),
        run_section(
            BACKGROUND_ID,
            "Background",
            1,
            RunSectionState::Skipped,
            Vec::new(),
        ),
        run_section(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            2,
            RunSectionState::Drafted,
            Vec::new(),
        ),
    ];
    seed_completed_run(&s3, client_id, report_id, full_plan(), sections).await;
    seed_findings(
        &s3,
        client_id,
        report_id,
        vec![finding(
            SUMMARY_ID,
            "unsupported_claim",
            1,
            FindingStatus::Open,
        )],
    )
    .await;

    let first = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        kinds(&first.checks),
        vec![
            CompletionCheckKind::RequiredSectionEmpty,
            CompletionCheckKind::UnresolvedCitation,
            CompletionCheckKind::MissingCitation,
            CompletionCheckKind::PlaceholderText,
            CompletionCheckKind::PlaceholderText,
            CompletionCheckKind::UnresolvedFinding,
        ]
    );
    // Two placeholder rows, in document order rather than section-ID order.
    assert_eq!(
        first.checks[3].section_id.map(|id| id.to_string()),
        Some(REFERRAL_ID.to_string())
    );
    assert_eq!(
        first.checks[4].section_id.map(|id| id.to_string()),
        Some(SUMMARY_ID.to_string())
    );

    let second = evaluate(&s3, client_id, report_id).await;
    assert_eq!(
        summarize(&first.checks),
        summarize(&second.checks),
        "two evaluations of unchanged state must agree row for row"
    );
    assert!(!first.complete && !second.complete);
}
