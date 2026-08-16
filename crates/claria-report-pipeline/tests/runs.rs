//! Durable whole-report drafting: sections survive a run that dies mid-flight,
//! a resumed run finishes the same document without redoing landed work, and
//! the workspace is never left locked behind a run that ended.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_core::models::{
    report::{AuthorshipKind, ReportBlock, ReportContent, ReportSection},
    report_run::{DraftRun, DraftRunStatus, RunSectionState},
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_pipeline as pipeline;
use claria_report_store as store;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-drafting-run-test";
const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";

const REFERRAL_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const BACKGROUND_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SUMMARY_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn setup() -> (MockServer, aws_config::SdkConfig, aws_sdk_s3::Client) {
    let server = MockServer::spawn().await;
    let sdk = sdk_config(&server.endpoint);
    let s3 = client::from_config(&sdk);
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
    (server, sdk, s3)
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

fn boilerplate() -> Vec<ReportBlock> {
    vec![paragraph("Template boilerplate.")]
}

fn template_section(id: &str, heading: &str) -> ReportSection {
    ReportSection {
        id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        blocks: boilerplate(),
        skipped: false,
        template_blocks: Some(boilerplate()),
        authorship: None,
    }
}

/// A client with one readable record and a three-section template applied as
/// revision 1, which is the shape every drafting run in this file builds on.
async fn seed_templated_report(s3: &aws_sdk_s3::Client, client_id: Uuid) -> Uuid {
    put_client(s3, client_id).await;
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::client_record_file(client_id, "intake.txt"),
        b"Referred by pediatrician for attention concerns.".to_vec(),
        Some("text/plain"),
    )
    .await
    .expect("put record");
    let workspace = store::start_report_workspace(s3, BUCKET, client_id)
        .await
        .expect("start writing session");
    store::save_report_draft_for_report(
        s3,
        BUCKET,
        client_id,
        workspace.report_id,
        0,
        ReportContent {
            title: "Evaluation Template".to_string(),
            sections: vec![
                template_section(REFERRAL_ID, "Reason for Referral"),
                template_section(BACKGROUND_ID, "Background"),
                template_section(SUMMARY_ID, "Summary and Clinical Interpretation"),
            ],
        },
    )
    .await
    .expect("seed template-shaped draft");
    workspace.report_id
}

async fn script(server: &MockServer, responses: Vec<ScriptedBedrockResponse>) {
    server
        .state
        .write()
        .await
        .bedrock_tool_responses
        .extend(responses);
}

fn ok(payload: serde_json::Value) -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(payload)
}

/// A Bedrock call that is answered with a refusal rather than a stream, which
/// is how a run is made to die at a chosen section boundary.
fn dead_call() -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::error(
        500,
        serde_json::json!({
            "__type": "InternalServerException",
            "message": "raw service detail must not reach UI"
        }),
    )
}

fn title_call(id: &str, title: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": id, "name": "set_full_draft_title", "input": {
        "title": title
    }}})
}

fn write_call(
    id: &str,
    section_id: &str,
    position: u32,
    heading: &str,
    text: &str,
) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": id, "name": "write_full_draft_section", "input": {
        "section_id": section_id,
        "position": position,
        "heading": heading,
        "blocks": [{"kind": "paragraph", "text": text}]
    }}})
}

fn skip_call(id: &str, section_id: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": id, "name": "skip_full_draft_section", "input": {
        "section_id": section_id
    }}})
}

fn finish_call(id: &str, summary: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": id, "name": "finish_full_draft", "input": {
        "summary": summary
    }}})
}

fn tool_round(calls: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    ok(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": calls}},
        "stopReason": "tool_use",
        "usage": {"inputTokens": 5, "outputTokens": 20}
    }))
}

fn closing_text(text: &str) -> ScriptedBedrockResponse {
    ok(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": [{"text": text}]}},
        "stopReason": "end_turn",
        "usage": {"inputTokens": 1, "outputTokens": 5}
    }))
}

async fn only_run(s3: &aws_sdk_s3::Client, client_id: Uuid, report_id: Uuid) -> DraftRun {
    let mut runs = store::list_draft_runs(s3, BUCKET, client_id, report_id)
        .await
        .expect("list drafting runs");
    assert_eq!(runs.len(), 1, "expected exactly one drafting run");
    runs.remove(0)
}

fn drafted(run: &DraftRun) -> Vec<(&str, &Vec<ReportBlock>)> {
    let mut sections = run
        .sections
        .iter()
        .filter(|section| section.state == RunSectionState::Drafted)
        .collect::<Vec<_>>();
    sections.sort_by_key(|section| section.position);
    sections
        .iter()
        .map(|section| (section.heading.as_str(), &section.blocks))
        .collect()
}

fn body(content: &ReportContent) -> Vec<(String, Vec<ReportBlock>, bool)> {
    content
        .sections
        .iter()
        .map(|section| {
            (
                section.heading.clone(),
                section.blocks.clone(),
                section.skipped,
            )
        })
        .collect()
}

#[tokio::test]
async fn a_run_that_dies_mid_draft_keeps_every_landed_section() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-1",
                    REFERRAL_ID,
                    0,
                    "Reason for Referral",
                    "Referred for attention concerns.",
                ),
            ]),
            tool_round(vec![write_call(
                "section-2",
                BACKGROUND_ID,
                1,
                "Background",
                "Documented developmental history.",
            )]),
            dead_call(),
        ],
    )
    .await;

    let error = pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation."),
    )
    .await
    .expect_err("the third call is refused");
    assert_eq!(
        error.failure_code(),
        Some(store::ReportFailureCode::Bedrock)
    );

    // Two sections landed before the call that killed the turn, and both are
    // still readable from the run object.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Failed);
    assert_eq!(run.base_revision, 1);
    assert_eq!(run.title.as_deref(), Some("Psychoeducational Evaluation"));
    assert_eq!(
        drafted(&run),
        vec![
            (
                "Reason for Referral",
                &vec![paragraph("Referred for attention concerns.")]
            ),
            (
                "Background",
                &vec![paragraph("Documented developmental history.")]
            ),
        ]
    );
    assert!(run.finalized_revision.is_none());
    assert!(
        run.sections
            .iter()
            .all(|section| section.state != RunSectionState::Drafting)
    );

    // Nothing of the failed turn reached the report, and the workspace is free
    // again: the drafted work is salvage, not a lock.
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.draft.revision, 1);
    assert_eq!(workspace.active_run_id, None);
    assert!(workspace.session.turns.is_empty());
    assert_eq!(workspace.draft.content.sections[0].blocks, boilerplate());
}

#[tokio::test]
async fn resuming_an_interrupted_run_finishes_the_document_it_started() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-1",
                    REFERRAL_ID,
                    0,
                    "Reason for Referral",
                    "Referred for attention concerns.",
                ),
            ]),
            tool_round(vec![write_call(
                "section-2",
                BACKGROUND_ID,
                1,
                "Background",
                "Documented developmental history.",
            )]),
            dead_call(),
        ],
    )
    .await;
    pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation."),
    )
    .await
    .expect_err("the interrupted run fails");
    let run_id = only_run(&s3, client_id, report_id).await.run_id;

    // Picking the run back up: the fresh conversation only has to decide the
    // section that never landed.
    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-2", "Psychoeducational Evaluation"),
                write_call(
                    "section-3",
                    SUMMARY_ID,
                    2,
                    "Summary and Clinical Interpretation",
                    "Findings are consistent with the referral question.",
                ),
            ]),
            tool_round(vec![finish_call("finish-1", "Completed the evaluation.")]),
            closing_text("The evaluation is complete."),
        ],
    )
    .await;
    let outcome = pipeline::resume_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL_ID,
        pipeline::FullReportRequest::new("Finish the summary."),
    )
    .await
    .expect("resume the interrupted run");

    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.active_run_id, None);
    assert_eq!(
        outcome.workspace.draft.content.title,
        "Psychoeducational Evaluation"
    );

    // Every section the run authored is stamped with the run that wrote it,
    // including the two that landed before the interruption.
    for section in &outcome.workspace.draft.content.sections {
        let authorship = section.authorship.as_ref().expect("authorship stamp");
        assert_eq!(authorship.kind, AuthorshipKind::ModelGenerated);
        assert_eq!(authorship.run_id, Some(run_id));
        assert_eq!(authorship.model_id.as_deref(), Some(MODEL_ID));
        assert_eq!(authorship.revision, 2);
    }

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(run.finalized_revision, Some(2));
    assert_eq!(run.instructions.len(), 2);

    // The same three tool calls, run start to finish without an interruption,
    // produce the same document.
    let control_id = Uuid::new_v4();
    let control_report = seed_templated_report(&s3, control_id).await;
    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-1",
                    REFERRAL_ID,
                    0,
                    "Reason for Referral",
                    "Referred for attention concerns.",
                ),
            ]),
            tool_round(vec![write_call(
                "section-2",
                BACKGROUND_ID,
                1,
                "Background",
                "Documented developmental history.",
            )]),
            tool_round(vec![write_call(
                "section-3",
                SUMMARY_ID,
                2,
                "Summary and Clinical Interpretation",
                "Findings are consistent with the referral question.",
            )]),
            tool_round(vec![finish_call("finish-1", "Completed the evaluation.")]),
            closing_text("The evaluation is complete."),
        ],
    )
    .await;
    let control = pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        control_id,
        control_report,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation."),
    )
    .await
    .expect("uninterrupted control run");
    assert_eq!(
        body(&outcome.workspace.draft.content),
        body(&control.workspace.draft.content)
    );
}

#[tokio::test]
async fn a_skip_through_the_run_keeps_the_template_copy() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-1",
                    REFERRAL_ID,
                    0,
                    "Reason for Referral",
                    "Referred for attention concerns.",
                ),
                write_call(
                    "section-2",
                    BACKGROUND_ID,
                    1,
                    "Background",
                    "Documented developmental history.",
                ),
                skip_call("skip-1", SUMMARY_ID),
            ]),
            tool_round(vec![finish_call(
                "finish-1",
                "Deferred the summary per guidance.",
            )]),
            closing_text("Two sections written, one deferred."),
        ],
    )
    .await;

    let outcome = pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Skip the summary."),
    )
    .await
    .expect("generate with one deferred section");

    let deferred = &outcome.workspace.draft.content.sections[2];
    assert_eq!(deferred.id.to_string(), SUMMARY_ID);
    assert!(deferred.skipped);
    assert!(deferred.blocks.is_empty());
    assert_eq!(deferred.template_blocks.as_ref(), Some(&boilerplate()));
    // The run did not author it, so it is not credited to the run.
    assert!(deferred.authorship.is_none());

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    let summary = run
        .sections
        .iter()
        .find(|section| section.section_id.to_string() == SUMMARY_ID)
        .expect("summary run section");
    assert_eq!(summary.state, RunSectionState::Skipped);
    assert!(summary.blocks.is_empty());
}

#[tokio::test]
async fn a_live_run_refuses_every_competing_report_mutation() {
    let (_server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    // Stand in for a run already in flight in another window.
    let mut workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("load workspace");
    workspace.active_run_id = Some(Uuid::new_v4());
    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::report_session_workspace(client_id, report_id),
        serde_json::to_vec(&workspace).expect("workspace JSON"),
        Some("application/json"),
    )
    .await
    .expect("re-put workspace with an active run");

    let hand_edit = store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        ReportContent {
            title: "Hand edited".to_string(),
            sections: Vec::new(),
        },
    )
    .await
    .expect_err("a hand edit is refused mid-run");
    assert_eq!(hand_edit.to_string(), store::ACTIVE_RUN_MESSAGE);

    let template = store::apply_report_template_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        store::ReportTemplateApplication {
            content: ReportContent {
                title: "Another template".to_string(),
                sections: Vec::new(),
            },
            source_sha256: "0".repeat(64),
            writer_template_id: Uuid::new_v4(),
            writer_template_name: "Another template".to_string(),
            warnings: Vec::new(),
        },
    )
    .await
    .expect_err("a template apply is refused mid-run");
    assert_eq!(template.to_string(), store::ACTIVE_RUN_MESSAGE);

    let second_draft = pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Start a second whole-report draft."),
    )
    .await
    .expect_err("a second whole-report draft is refused mid-run");
    assert_eq!(second_draft.to_string(), store::ACTIVE_RUN_MESSAGE);

    let targeted = pipeline::send_report_message_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::ReportMessageRequest::new("Tighten the referral section."),
    )
    .await
    .expect_err("a targeted edit is refused mid-run");
    assert_eq!(targeted.to_string(), store::ACTIVE_RUN_MESSAGE);
}

#[tokio::test]
async fn a_run_whose_completion_write_was_lost_is_healed_from_the_workspace() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-1",
                    REFERRAL_ID,
                    0,
                    "Reason for Referral",
                    "Referred for attention concerns.",
                ),
                skip_call("skip-1", BACKGROUND_ID),
                skip_call("skip-2", SUMMARY_ID),
            ]),
            tool_round(vec![finish_call("finish-1", "Wrote the referral section.")]),
            closing_text("Done."),
        ],
    )
    .await;
    pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the referral only."),
    )
    .await
    .expect("generate the report");

    // Rewind the run object to the instant before its completion write: the
    // revision was cut and the workspace saved, then the process died.
    let mut run = only_run(&s3, client_id, report_id).await;
    let run_id = run.run_id;
    assert_eq!(run.finalized_revision, Some(2));
    run.status = DraftRunStatus::Drafting;
    run.finalized_revision = None;
    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::report_draft_run(client_id, report_id, run_id),
        serde_json::to_vec(&run).expect("run JSON"),
        Some("application/json"),
    )
    .await
    .expect("re-put the unstamped run");

    let error = pipeline::resume_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL_ID,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect_err("a completed run cannot be picked back up");
    assert!(
        error
            .to_string()
            .contains("Only an interrupted whole-report draft"),
        "unexpected refusal: {error}"
    );

    let healed = only_run(&s3, client_id, report_id).await;
    assert_eq!(healed.status, DraftRunStatus::Completed);
    assert_eq!(healed.finalized_revision, Some(2));
}

#[tokio::test]
async fn a_run_that_fails_before_any_section_leaves_the_writer_usable() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let workspace = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("start writing session");
    let report_id = workspace.report_id;
    script(&server, vec![dead_call()]).await;

    pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        0,
        MODEL_ID,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect_err("the first call is refused");

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Failed);
    assert!(drafted(&run).is_empty());

    let released = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(released.active_run_id, None);
    assert_eq!(released.draft.revision, 0);

    // The failed run must not have cost the writer anything: the next targeted
    // turn behaves exactly as it did before runs existed.
    script(&server, vec![closing_text("Nothing to change yet.")]).await;
    let outcome = pipeline::send_report_message_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        0,
        MODEL_ID,
        pipeline::ReportMessageRequest::new("Review the current report."),
    )
    .await
    .expect("a targeted turn still works after a failed run");
    assert_eq!(outcome.assistant_text, "Nothing to change yet.");
    assert_eq!(outcome.workspace.active_run_id, None);
}
