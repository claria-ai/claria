//! A whole `plan` pass driven through the harness against the mock AWS
//! service — the same path the real subcommand takes, minus the config file
//! and the model discovery call.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_core::models::{
    report::{ReportBlock, ReportContent, ReportSection},
    report_run::{DraftRunStatus, SectionIntent},
};
use claria_eval::{
    EvalContext,
    pipeline::{self, WorkspaceChoice},
    progress::ProgressRecorder,
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_pipeline::ReportTurnProgress;
use uuid::Uuid;

const BUCKET: &str = "claria-eval-smoke";
const PLANNER_MODEL_ID: &str = "us.anthropic.claude-sonnet-test";
const WRITER_MODEL_ID: &str = "us.anthropic.claude-opus-test";
const REFERRAL_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const SUMMARY_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const INTAKE_FILE: &str = "intake.txt";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph {
        text: text.to_string(),
    }
}

fn template_section(id: &str, heading: &str) -> ReportSection {
    ReportSection {
        id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        blocks: vec![paragraph("Template boilerplate.")],
        skipped: false,
        template_blocks: Some(vec![paragraph("Template boilerplate.")]),
        template_directives: Vec::new(),
        authorship: None,
    }
}

/// A client with one readable record and a two-section report at revision 1.
async fn seed(s3: &aws_sdk_s3::Client, client_id: Uuid) -> Uuid {
    let now = jiff::Timestamp::now();
    let client = claria_core::models::client::Client {
        id: client_id,
        name: "Smoke client".to_string(),
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
        &claria_core::s3_keys::client_record_file(client_id, INTAKE_FILE),
        b"Referred by pediatrician for attention concerns.".to_vec(),
        Some("text/plain"),
    )
    .await
    .expect("put record");

    let workspace = claria_report_store::start_report_workspace(s3, BUCKET, client_id)
        .await
        .expect("start Writing session");
    claria_report_store::save_report_draft_for_report(
        s3,
        BUCKET,
        client_id,
        workspace.report_id,
        0,
        ReportContent {
            title: "Evaluation Template".to_string(),
            sections: vec![
                template_section(REFERRAL_ID, "Reason for Referral"),
                template_section(SUMMARY_ID, "Summary and Clinical Interpretation"),
            ],
        },
    )
    .await
    .expect("seed a template-shaped draft");
    workspace.report_id
}

fn plan_row(section_id: &str, action: &str) -> serde_json::Value {
    serde_json::json!({
        "section_id": section_id,
        "action": action,
        "scope": "What this section should cover.",
        "evidence": [{"filename": INTAKE_FILE, "relevance": "Names the referral question."}]
    })
}

fn section_plan(rows: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": [
            {"toolUse": {
                "toolUseId": "plan-1",
                "name": "submit_section_plan",
                "input": {"rows": rows}
            }}
        ]}},
        "stopReason": "tool_use",
        "usage": {"inputTokens": 900, "outputTokens": 120}
    }))
}

#[tokio::test]
async fn a_plan_pass_reports_its_events_plan_and_cost() {
    let server = MockServer::spawn().await;
    let sdk_config = sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk_config);
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");

    let client_id = Uuid::new_v4();
    seed(&s3, client_id).await;
    server
        .state
        .write()
        .await
        .bedrock_tool_responses
        .push(section_plan(vec![
            plan_row(REFERRAL_ID, "draft"),
            plan_row(SUMMARY_ID, "draft"),
        ]));

    let context = EvalContext {
        sdk_config,
        s3,
        bucket: BUCKET.to_string(),
    };

    // The listing is the no-Bedrock subcommand: it must find the seeded
    // client and count its one record.
    let clients = pipeline::list_clients(&context)
        .await
        .expect("list clients");
    let listing = clients
        .iter()
        .find(|listing| listing.client_id == client_id)
        .expect("the seeded client is listed");
    assert_eq!(listing.record_count, 1);
    assert_eq!(listing.workspace_count, 1);

    let workspace = pipeline::prepare_workspace(&context, client_id, WorkspaceChoice::Existing)
        .await
        .expect("the newest Writing session with sections");
    assert_eq!(workspace.draft.content.sections.len(), 2);

    let recorder = ProgressRecorder::new(false);
    let report = pipeline::plan(
        &context,
        &workspace,
        PLANNER_MODEL_ID,
        WRITER_MODEL_ID,
        "Draft the whole report from the readable records.",
        &recorder,
    )
    .await
    .expect("the planning pass");

    assert_eq!(report.run.status, DraftRunStatus::AwaitingApproval);
    let plan = report.run.plan.as_ref().expect("a plan at the gate");
    assert_eq!(plan.entries.len(), 2);
    assert!(
        plan.entries
            .iter()
            .all(|entry| entry.intent == SectionIntent::Draft)
    );
    assert_eq!(plan.entries[0].evidence[0].filename, INTAKE_FILE);

    // Instrumentation: every event is timestamped and the model call is
    // visible, which is the whole reason the harness exists.
    let events = recorder.events();
    assert!(
        events.iter().any(|timed| matches!(
            timed.event,
            ReportTurnProgress::ModelCallStarted { call_number: 1 }
        )),
        "the pass announced no model call: {events:#?}"
    );
    assert!(
        events.iter().any(|timed| matches!(
            timed.event,
            ReportTurnProgress::RecordContextPrepared {
                included_files: 1,
                ..
            }
        )),
        "the pass announced no record context: {events:#?}"
    );
    assert_eq!(recorder.model_calls(), report.converse_calls);

    // The scripted usage prices against the planner's tier, and the
    // harness's number is derived rather than copied off the TurnUsage.
    let usage = report.usage.as_ref().expect("captured usage");
    assert_eq!(usage.input_tokens, 900);
    assert_eq!(usage.output_tokens, 120);
    assert!(report.cost.cost_usd > 0.0);
    assert!(!report.cost.unpriced_calls);
    assert_eq!(report.cost.total_input_tokens(), 900);
    assert_eq!(report.cost.output_tokens, 120);
}

/// A client with no templated report is a usage error with a way out, not a
/// crash deep in the pipeline.
#[tokio::test]
async fn a_client_with_no_sectioned_report_says_what_to_do() {
    let server = MockServer::spawn().await;
    let sdk_config = sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk_config);
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");

    let client_id = Uuid::new_v4();
    let now = jiff::Timestamp::now();
    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::client(client_id),
        serde_json::to_vec(&claria_core::models::client::Client {
            id: client_id,
            name: "Empty client".to_string(),
            created_at: now,
            updated_at: now,
        })
        .expect("client JSON"),
        Some("application/json"),
    )
    .await
    .expect("put client");

    let context = EvalContext {
        sdk_config,
        s3,
        bucket: BUCKET.to_string(),
    };
    let error = pipeline::prepare_workspace(&context, client_id, WorkspaceChoice::Existing)
        .await
        .expect_err("no sectioned report to plan against");
    assert!(format!("{error}").contains("--template"));
}
