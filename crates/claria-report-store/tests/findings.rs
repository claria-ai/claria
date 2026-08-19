//! Durable review findings against mock AWS: the object protocol, the sweep
//! write-back, and the apply / undo / dismiss actions.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_core::models::{
    findings::{
        Finding, FindingAnchor, FindingStatus, MAX_REPORT_FINDINGS, ReportFindings, ReviewCoverage,
        ReviewPass, StyleProposal,
    },
    report::{AuthorshipKind, ReportBlock, ReportContent, ReportSection},
};
use claria_mock_aws::testing::MockServer;
use claria_report_store::{self as store, REPORT_CONFLICT_MESSAGE};
use claria_storage::client;
use jiff::Timestamp;
use uuid::Uuid;

const BUCKET: &str = "claria-report-findings-test";
const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";
const ORIGINAL: &str = "The client is reported to have attended.";

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

async fn put_client(s3: &aws_sdk_s3::Client, client_id: Uuid) {
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
}

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph {
        text: text.to_string(),
    }
}

/// A client with one saved report whose only section holds [`ORIGINAL`].
/// Returns the client, report, and section IDs plus the saved revision.
async fn reviewed_report(s3: &aws_sdk_s3::Client) -> (Uuid, Uuid, Uuid, u64) {
    let client_id = Uuid::new_v4();
    put_client(s3, client_id).await;
    let started = store::start_report_workspace(s3, BUCKET, client_id)
        .await
        .expect("start session");
    let section_id = Uuid::new_v4();
    let saved = store::save_report_draft_for_report(
        s3,
        BUCKET,
        client_id,
        started.report_id,
        0,
        ReportContent {
            title: "Assessment".to_string(),
            sections: vec![ReportSection {
                id: section_id,
                heading: "History".to_string(),
                blocks: vec![paragraph(ORIGINAL), paragraph("A second paragraph.")],
                skipped: false,
                template_blocks: None,
                template_directives: Vec::new(),
                authorship: None,
            }],
        },
    )
    .await
    .expect("save draft");
    (
        client_id,
        started.report_id,
        section_id,
        saved.draft.revision,
    )
}

fn style_finding(section_id: Uuid, revision: u64) -> Finding {
    Finding {
        id: Uuid::new_v4(),
        pass: ReviewPass::Style,
        property: "tense_drift".to_string(),
        model_id: MODEL_ID.to_string(),
        anchor: FindingAnchor {
            section_id,
            revision,
        },
        description: "The sentence drifts into the present tense.".to_string(),
        span: None,
        conflicting: None,
        record_citation: None,
        proposal: Some(StyleProposal {
            block_index: 0,
            original_text: "is reported to have attended".to_string(),
            replacement_text: "attended".to_string(),
        }),
        status: FindingStatus::Open,
        applied_revision: None,
        resolved_at: None,
        created_at: jiff::Timestamp::now(),
    }
}

fn consistency_finding(section_id: Uuid, revision: u64) -> Finding {
    Finding {
        pass: ReviewPass::Consistency,
        property: "internal_contradiction".to_string(),
        proposal: None,
        ..style_finding(section_id, revision)
    }
}

fn coverage(property: &str, revision: u64) -> ReviewCoverage {
    ReviewCoverage {
        pass: ReviewPass::Style,
        property: property.to_string(),
        model_id: MODEL_ID.to_string(),
        revision,
        sections_reviewed: 1,
        findings: 1,
        completed_at: jiff::Timestamp::now(),
    }
}

/// Write one sweep's worth of findings the way the review pass will.
async fn seed(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    fresh: Vec<Finding>,
) -> ReportFindings {
    let mut loaded = store::load_report_findings(s3, BUCKET, client_id, report_id)
        .await
        .expect("load findings");
    store::replace_findings_for_revision(
        &mut loaded.findings,
        revision,
        fresh,
        vec![coverage("tense_drift", revision)],
        jiff::Timestamp::now(),
    )
    .expect("stage sweep");
    store::save_report_findings(s3, BUCKET, &mut loaded)
        .await
        .expect("save findings");
    loaded.findings
}

async fn paragraph_text(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    index: usize,
) -> String {
    let workspace = store::load_report_workspace_by_id(s3, BUCKET, client_id, report_id)
        .await
        .expect("load workspace");
    match &workspace.draft.content.sections[0].blocks[index] {
        ReportBlock::Paragraph { text } => text.clone(),
        other => panic!("expected a paragraph, got {other:?}"),
    }
}

#[tokio::test]
async fn a_report_that_was_never_reviewed_has_empty_findings_and_no_object() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, _, _) = reviewed_report(&s3).await;

    let loaded = store::load_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("load findings");
    assert!(loaded.findings.findings.is_empty());
    assert!(loaded.findings.coverage.is_empty());
    assert_eq!(loaded.etag, None);
    assert_eq!(loaded.findings.client_id, client_id);
    assert_eq!(loaded.findings.report_id, report_id);
    assert!(matches!(
        claria_storage::objects::get_object(
            &s3,
            BUCKET,
            &claria_core::s3_keys::report_findings(client_id, report_id)
        )
        .await,
        Err(claria_storage::error::StorageError::NotFound { .. })
    ));

    assert!(
        store::list_report_findings(&s3, BUCKET, client_id, report_id)
            .await
            .expect("list findings")
            .findings
            .is_empty()
    );
}

#[tokio::test]
async fn findings_save_conditionally_and_a_stale_writer_conflicts() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, section_id, revision) = reviewed_report(&s3).await;

    let mut first = store::load_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("load findings");
    let mut second = store::load_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("load findings again");
    first.findings.findings = vec![style_finding(section_id, revision)];
    first.findings.updated_at = jiff::Timestamp::now();
    store::save_report_findings(&s3, BUCKET, &mut first)
        .await
        .expect("first save creates the object");
    assert!(first.etag.is_some());

    // The second holder still believes the object does not exist.
    second.findings.findings = vec![style_finding(section_id, revision)];
    second.findings.updated_at = jiff::Timestamp::now();
    let error = store::save_report_findings(&s3, BUCKET, &mut second)
        .await
        .expect_err("concurrent create");
    assert_eq!(error.to_string(), REPORT_CONFLICT_MESSAGE);

    // A reload sees the first writer's object and can then save on its ETag.
    let mut reloaded = store::load_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload findings");
    assert_eq!(reloaded.findings.findings.len(), 1);
    reloaded.findings.updated_at = jiff::Timestamp::now();
    store::save_report_findings(&s3, BUCKET, &mut reloaded)
        .await
        .expect("conditional update");

    // …and the stale ETag from before that update no longer writes.
    let error = store::save_report_findings(&s3, BUCKET, &mut first)
        .await
        .expect_err("stale ETag");
    assert_eq!(error.to_string(), REPORT_CONFLICT_MESSAGE);
}

#[tokio::test]
async fn a_sweep_replaces_open_findings_and_keeps_resolved_history() {
    let mut findings = ReportFindings::new(Uuid::new_v4(), Uuid::new_v4(), Timestamp::now());
    let section_id = Uuid::new_v4();

    let mut applied = style_finding(section_id, 1);
    applied.status = FindingStatus::Applied;
    applied.applied_revision = Some(2);
    applied.resolved_at = Some(Timestamp::now());
    let mut dismissed = style_finding(section_id, 1);
    dismissed.status = FindingStatus::Dismissed;
    dismissed.resolved_at = Some(Timestamp::now());
    let mut invalidated = style_finding(section_id, 1);
    invalidated.status = FindingStatus::Invalidated;
    invalidated.resolved_at = Some(Timestamp::now());
    let superseded = style_finding(section_id, 1);
    findings.findings = vec![
        applied.clone(),
        dismissed.clone(),
        invalidated.clone(),
        superseded.clone(),
    ];

    let fresh = style_finding(section_id, 2);
    store::replace_findings_for_revision(
        &mut findings,
        2,
        vec![fresh.clone()],
        vec![coverage("tense_drift", 2)],
        Timestamp::now(),
    )
    .expect("fold sweep");

    let ids: Vec<Uuid> = findings.findings.iter().map(|finding| finding.id).collect();
    assert!(ids.contains(&applied.id));
    assert!(ids.contains(&dismissed.id));
    assert!(ids.contains(&fresh.id));
    // The previous sweep's unresolved rows were just re-derived.
    assert!(!ids.contains(&superseded.id));
    assert!(!ids.contains(&invalidated.id));
    assert_eq!(findings.coverage.len(), 1);

    // A finding anchored to a revision the sweep did not read is refused.
    let error = store::replace_findings_for_revision(
        &mut findings,
        3,
        vec![style_finding(section_id, 2)],
        vec![],
        Timestamp::now(),
    )
    .expect_err("mismatched anchor");
    assert!(error.to_string().contains("did not read"));
}

#[tokio::test]
async fn the_findings_cap_drops_the_oldest_resolved_rows_first() {
    let mut findings = ReportFindings::new(Uuid::new_v4(), Uuid::new_v4(), Timestamp::now());
    let section_id = Uuid::new_v4();
    let base: Timestamp = "2026-01-01T00:00:00Z".parse().expect("timestamp");
    findings.findings = (0..MAX_REPORT_FINDINGS)
        .map(|index| {
            let mut finding = style_finding(section_id, 1);
            finding.status = FindingStatus::Dismissed;
            finding.created_at =
                base + jiff::Span::new().seconds(i64::try_from(index).expect("fits"));
            finding.resolved_at = Some(Timestamp::now());
            finding
        })
        .collect();
    let oldest = findings.findings[0].id;
    let newest = findings.findings[MAX_REPORT_FINDINGS - 1].id;

    let fresh: Vec<Finding> = (0..3).map(|_| style_finding(section_id, 2)).collect();
    let fresh_ids: Vec<Uuid> = fresh.iter().map(|finding| finding.id).collect();
    store::replace_findings_for_revision(
        &mut findings,
        2,
        fresh,
        vec![coverage("tense_drift", 2)],
        Timestamp::now(),
    )
    .expect("fold sweep");

    assert_eq!(findings.findings.len(), MAX_REPORT_FINDINGS);
    let ids: Vec<Uuid> = findings.findings.iter().map(|finding| finding.id).collect();
    assert!(!ids.contains(&oldest));
    assert!(ids.contains(&newest));
    for id in fresh_ids {
        assert!(ids.contains(&id));
    }
}

#[tokio::test]
async fn applying_a_style_finding_cuts_a_revision_and_credits_the_reviewer() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, section_id, revision) = reviewed_report(&s3).await;
    let finding = style_finding(section_id, revision);
    let finding_id = finding.id;
    seed(&s3, client_id, report_id, revision, vec![finding]).await;

    let outcome = store::apply_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect("apply");

    assert_eq!(outcome.workspace.draft.revision, revision + 1);
    assert_eq!(
        paragraph_text(&s3, client_id, report_id, 0).await,
        "The client attended."
    );
    // The untouched paragraph is left exactly as it was.
    assert_eq!(
        paragraph_text(&s3, client_id, report_id, 1).await,
        "A second paragraph."
    );
    let stamp = outcome.workspace.draft.content.sections[0]
        .authorship
        .as_ref()
        .expect("authorship");
    assert_eq!(stamp.kind, AuthorshipKind::ModelRevised);
    assert_eq!(stamp.revision, revision + 1);
    assert_eq!(stamp.model_id.as_deref(), Some(MODEL_ID));
    assert_eq!(stamp.run_id, None);

    let applied = outcome.findings.find(finding_id).expect("finding");
    assert_eq!(applied.status, FindingStatus::Applied);
    assert_eq!(applied.applied_revision, Some(revision + 1));
    assert!(applied.resolved_at.is_some());

    // Applying twice is refused rather than doubled.
    let error = store::apply_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect_err("second apply");
    assert!(error.to_string().contains("Only an open review finding"));

    // The applied finding now reads as stale, because applying it stamped the
    // section past the revision the review read. It stays applied — only open
    // findings are refreshed.
    let listed = store::list_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("list");
    assert_eq!(
        listed.find(finding_id).expect("finding").status,
        FindingStatus::Applied
    );
}

#[tokio::test]
async fn undoing_an_applied_finding_restores_the_text_as_the_users_own_edit() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, section_id, revision) = reviewed_report(&s3).await;
    let finding = style_finding(section_id, revision);
    let finding_id = finding.id;
    seed(&s3, client_id, report_id, revision, vec![finding]).await;
    store::apply_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect("apply");

    let outcome = store::undo_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect("undo");

    assert_eq!(outcome.workspace.draft.revision, revision + 2);
    assert_eq!(paragraph_text(&s3, client_id, report_id, 0).await, ORIGINAL);
    let stamp = outcome.workspace.draft.content.sections[0]
        .authorship
        .as_ref()
        .expect("authorship");
    assert_eq!(stamp.kind, AuthorshipKind::HumanEdited);
    assert_eq!(stamp.revision, revision + 2);
    assert_eq!(stamp.model_id, None);

    let reopened = outcome.findings.find(finding_id).expect("finding");
    assert_eq!(reopened.status, FindingStatus::Open);
    assert_eq!(reopened.applied_revision, None);
    assert_eq!(reopened.resolved_at, None);
    // Reopened means actionable: the undo re-anchored the finding to the
    // revision it cut, so listing does not invalidate it on the way back.
    assert_eq!(reopened.anchor.revision, revision + 2);
    assert_eq!(
        store::list_report_findings(&s3, BUCKET, client_id, report_id)
            .await
            .expect("list")
            .find(finding_id)
            .expect("finding")
            .status,
        FindingStatus::Open
    );

    let error = store::undo_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect_err("nothing left to undo");
    assert!(error.to_string().contains("Only an applied review finding"));

    // …and re-applying works, since the reviewed text is provably back.
    let reapplied = store::apply_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect("re-apply");
    assert_eq!(reapplied.workspace.draft.revision, revision + 3);
}

#[tokio::test]
async fn a_finding_whose_text_has_changed_is_invalidated_rather_than_guessed_at() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, section_id, revision) = reviewed_report(&s3).await;
    let finding = style_finding(section_id, revision);
    let finding_id = finding.id;
    seed(&s3, client_id, report_id, revision, vec![finding]).await;

    // The user rewrites the sentence the review anchored to.
    store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        revision,
        ReportContent {
            title: "Assessment".to_string(),
            sections: vec![ReportSection {
                id: section_id,
                heading: "History".to_string(),
                blocks: vec![
                    paragraph("The client came to every appointment."),
                    paragraph("A second paragraph."),
                ],
                skipped: false,
                template_blocks: None,
                template_directives: Vec::new(),
                authorship: None,
            }],
        },
    )
    .await
    .expect("hand edit");

    let error = store::apply_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect_err("stale anchor");
    assert!(error.to_string().contains("rewritten since the review ran"));

    let listed = store::list_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("list");
    assert_eq!(
        listed.find(finding_id).expect("finding").status,
        FindingStatus::Invalidated
    );
    // The refusal did not touch the report.
    assert_eq!(
        store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
            .await
            .expect("workspace")
            .draft
            .revision,
        revision + 1
    );
}

#[tokio::test]
async fn an_ambiguous_anchor_is_refused_and_invalidated() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let started = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("start session");
    let report_id = started.report_id;
    let section_id = Uuid::new_v4();
    // The same phrase twice in one paragraph: nothing in the anchor says
    // which occurrence the reviewer meant.
    let saved = store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        0,
        ReportContent {
            title: "Assessment".to_string(),
            sections: vec![ReportSection {
                id: section_id,
                heading: "History".to_string(),
                blocks: vec![paragraph(
                    "The client is reported to have attended, and the parent \
                     is reported to have attended.",
                )],
                skipped: false,
                template_blocks: None,
                template_directives: Vec::new(),
                authorship: None,
            }],
        },
    )
    .await
    .expect("save draft");
    let revision = saved.draft.revision;
    let finding = style_finding(section_id, revision);
    let finding_id = finding.id;
    seed(&s3, client_id, report_id, revision, vec![finding]).await;

    let error = store::apply_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect_err("ambiguous anchor");
    assert!(error.to_string().contains("anchor is ambiguous"));

    let loaded = store::load_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("load findings");
    assert_eq!(
        loaded.findings.find(finding_id).expect("finding").status,
        FindingStatus::Invalidated
    );
    assert_eq!(
        store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
            .await
            .expect("workspace")
            .draft
            .revision,
        revision
    );
}

#[tokio::test]
async fn a_consistency_finding_has_no_apply_path() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, section_id, revision) = reviewed_report(&s3).await;
    let finding = consistency_finding(section_id, revision);
    let finding_id = finding.id;
    seed(&s3, client_id, report_id, revision, vec![finding]).await;

    let error = store::apply_style_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect_err("consistency findings are read-only");
    assert!(error.to_string().contains("read-only"));
    assert_eq!(
        store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
            .await
            .expect("workspace")
            .draft
            .revision,
        revision
    );
}

#[tokio::test]
async fn dismissing_a_finding_resolves_it_without_touching_the_report() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, section_id, revision) = reviewed_report(&s3).await;
    let finding = consistency_finding(section_id, revision);
    let finding_id = finding.id;
    seed(&s3, client_id, report_id, revision, vec![finding]).await;

    let outcome = store::dismiss_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect("dismiss");
    assert_eq!(outcome.workspace.draft.revision, revision);
    let dismissed = outcome.findings.find(finding_id).expect("finding");
    assert_eq!(dismissed.status, FindingStatus::Dismissed);
    assert!(dismissed.resolved_at.is_some());

    let error = store::dismiss_finding(&s3, BUCKET, client_id, report_id, finding_id)
        .await
        .expect_err("second dismiss");
    assert!(error.to_string().contains("Only an open review finding"));
}

#[tokio::test]
async fn listing_stamps_stale_open_findings_and_persists_the_answer() {
    let (_server, s3) = setup().await;
    let (client_id, report_id, section_id, revision) = reviewed_report(&s3).await;
    let stale = style_finding(section_id, revision);
    let gone = style_finding(Uuid::new_v4(), revision);
    let (stale_id, gone_id) = (stale.id, gone.id);
    seed(&s3, client_id, report_id, revision, vec![stale, gone]).await;

    // A hand edit stamps the section past the reviewed revision.
    store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        revision,
        ReportContent {
            title: "Assessment".to_string(),
            sections: vec![ReportSection {
                id: section_id,
                heading: "History".to_string(),
                blocks: vec![paragraph("Rewritten."), paragraph("A second paragraph.")],
                skipped: false,
                template_blocks: None,
                template_directives: Vec::new(),
                authorship: None,
            }],
        },
    )
    .await
    .expect("hand edit");

    let listed = store::list_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("list");
    for id in [stale_id, gone_id] {
        assert_eq!(
            listed.find(id).expect("finding").status,
            FindingStatus::Invalidated
        );
    }

    // The write-back is a cache of the derived answer, so a plain load agrees.
    let loaded = store::load_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("load findings");
    assert_eq!(
        loaded.findings.find(stale_id).expect("finding").status,
        FindingStatus::Invalidated
    );

    let error = store::apply_style_finding(&s3, BUCKET, client_id, report_id, stale_id)
        .await
        .expect_err("invalidated findings cannot be applied");
    assert!(error.to_string().contains("Only an open review finding"));
}
