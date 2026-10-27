//! Stop may cancel inference, but must not tear an admitted S3 write in half.

#[path = "support/s3_gate.rs"]
mod s3_gate;

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria::{FullReportRequest, ReportTurnProgress, generate_full_report_for_report};
use claria_bedrock::converse::StopSignal;
use claria_core::models::{
    report::{ReportBlock, ReportContent, ReportSection},
    report_run::{DraftRunStatus, RunSectionState},
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_store as store;
use serde_json::json;
use uuid::Uuid;

const BUCKET: &str = "claria-writer-durability-test";
const MODEL: &str = "us.anthropic.claude-sonnet-test";
const SECTION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const TEXT: &str = "Synthetic evidence — résumé ✓. Exact punctuation and Unicode survive.";

fn config(endpoint: &str) -> aws_config::SdkConfig {
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        )))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn seed(s3: &aws_sdk_s3::Client, client_id: Uuid) -> Uuid {
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("bucket");
    let now = jiff::Timestamp::now();
    let client = claria_core::models::client::Client {
        id: client_id,
        name: "Synthetic durability fixture".to_string(),
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
    .expect("client");
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::client_record_file(client_id, "synthetic.txt"),
        b"Synthetic source for a deterministic writing fixture.".to_vec(),
        Some("text/plain"),
    )
    .await
    .expect("record");
    let workspace = store::start_report_workspace(s3, BUCKET, client_id)
        .await
        .expect("session");
    store::save_report_draft_for_report(
        s3,
        BUCKET,
        client_id,
        workspace.report_id,
        0,
        ReportContent {
            title: "Synthetic template".to_string(),
            sections: vec![ReportSection {
                id: SECTION.parse().expect("section ID"),
                heading: "Synthetic section".to_string(),
                blocks: Vec::new(),
                skipped: false,
                template_blocks: Some(Vec::new()),
                template_directives: Vec::new(),
                authorship: None,
            }],
        },
    )
    .await
    .expect("template");
    workspace.report_id
}

#[derive(Clone, Copy)]
enum HeldWrite {
    Receipt,
    Section,
}

async fn stop_during_write(held: HeldWrite) {
    let server = MockServer::spawn().await;
    let sdk = config(&server.endpoint);
    let direct_s3 = claria_storage::client::from_config(&sdk);
    let client_id = Uuid::new_v4();
    let report_id = seed(&direct_s3, client_id).await;
    server
        .state
        .write()
        .await
        .bedrock_tool_responses
        .push(ScriptedBedrockResponse::success(json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "title", "name": "set_full_draft_title", "input": {
                    "title": "Synthetic accepted title"
                }}},
                {"toolUse": {"toolUseId": "section", "name": "write_full_draft_section", "input": {
                    "section_id": SECTION, "position": 0, "heading": "Synthetic section",
                    "blocks": [{"kind": "paragraph", "text": TEXT}]
                }}}
            ]}},
            "stopReason": "tool_use",
            "usage": {"inputTokens": 5, "outputTokens": 20}
        })));
    let mut gate = s3_gate::S3Gate::spawn(&server.endpoint, move |path, body| match held {
        HeldWrite::Receipt => path.contains("/usage/"),
        HeldWrite::Section => {
            path.contains("/runs/")
                && body
                    .windows(b"\"state\": \"drafted\"".len())
                    .any(|part| part == b"\"state\": \"drafted\"")
        }
    })
    .await;
    let s3 = claria_storage::client::from_config(&config(&gate.endpoint));
    let stop = StopSignal::new();
    let completed = AtomicUsize::new(0);
    let progress = |event| {
        if matches!(event, ReportTurnProgress::SectionCompleted { .. }) {
            completed.fetch_add(1, Ordering::SeqCst);
        }
    };
    let request = FullReportRequest::new("Write the synthetic section.")
        .with_stop(&stop)
        .with_progress(&progress);
    let mut turn = Box::pin(generate_full_report_for_report(
        &sdk, &s3, BUCKET, client_id, report_id, 1, MODEL, request,
    ));
    let held_path = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            path = gate.held() => path,
            outcome = &mut turn => panic!("turn ended before its held PUT: {outcome:?}"),
        }
    })
    .await
    .expect("selected write arrives promptly");
    stop.stop();
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut turn)
            .await
            .is_err(),
        "Stop dropped an admitted storage write"
    );
    assert_eq!(
        completed.load(Ordering::SeqCst),
        0,
        "progress preceded durability"
    );
    let mut before = store::list_draft_runs(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("independent reload while PUT is blocked");
    let before = before.pop().expect("active run");
    assert!(
        before
            .sections
            .iter()
            .all(|section| section.state != RunSectionState::Drafted)
    );
    if matches!(held, HeldWrite::Receipt) {
        let key = held_path
            .strip_prefix(&format!("/{BUCKET}/"))
            .expect("receipt path");
        assert!(
            direct_s3
                .head_object()
                .bucket(BUCKET)
                .key(key)
                .send()
                .await
                .is_err(),
            "receipt was already durable while its PUT was held"
        );
    }
    gate.release();
    let error = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("accepted writes settle promptly")
        .expect_err("Stop parks the unfinished run");
    assert!(error.is_stopped(), "{error}");
    let attempt = error.attempt().expect("stopped attempt receipt");
    assert_eq!(attempt.converse_calls, 1);
    assert_eq!(attempt.usage.input_tokens, 5);
    assert_eq!(attempt.usage.output_tokens, 20);
    assert!(attempt.usage_complete, "completed response has known usage");
    assert_eq!(completed.load(Ordering::SeqCst), 1);
    assert_eq!(
        server.state.read().await.bedrock_tool_requests.len(),
        1,
        "Stop opened another model call after the admitted batch"
    );
    let mut runs = store::list_draft_runs(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("reload durable run");
    assert_eq!(runs.len(), 1);
    let run = runs.remove(0);
    assert_eq!(run.status, DraftRunStatus::Stopped);
    assert_eq!(run.title.as_deref(), Some("Synthetic accepted title"));
    assert_eq!(run.sections[0].state, RunSectionState::Drafted);
    assert_eq!(
        run.sections[0].blocks,
        vec![ReportBlock::Paragraph {
            text: TEXT.to_string()
        }]
    );
    let workspace = store::load_report_workspace_by_id(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, Some(run.run_id));
    assert_eq!(workspace.draft.revision, 1);
    let key = claria_core::s3_keys::report_call_usage(client_id, attempt.attempt_id, 1);
    let (receipt, _): (store::ReportCallUsageRecord, String) =
        claria_storage::state::load_state(&direct_s3, BUCKET, &key)
            .await
            .expect("durable usage");
    assert!(receipt.usage_complete);
    assert_eq!(
        receipt.usage.expect("known token receipt").output_tokens,
        20
    );
}

#[tokio::test]
async fn stop_during_receipt_put_drains_the_admitted_batch() {
    stop_during_write(HeldWrite::Receipt).await;
}

#[tokio::test]
async fn stop_during_section_put_waits_for_durability_before_progress() {
    stop_during_write(HeldWrite::Section).await;
}
