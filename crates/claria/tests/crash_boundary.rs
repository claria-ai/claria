//! Abrupt command loss is not an orderly Stop. Reload preserves durable
//! sections but cannot infer that a stored owner has stopped on every device.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria as pipeline;
use claria_core::models::{
    report::{ReportBlock, ReportContent, ReportSection},
    report_run::{DraftRunStatus, RunSectionState},
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_store as store;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-writer-crash-boundary";
const MODEL: &str = "us.anthropic.claude-sonnet-test";
const FIRST: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const SECOND: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const LANDED: &str = "Synthetic finding saved before interruption.";

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph { text: text.into() }
}

fn section(id: &str, heading: &str) -> ReportSection {
    ReportSection {
        id: id.parse().expect("section UUID"),
        heading: heading.into(),
        blocks: vec![paragraph("Synthetic template text.")],
        skipped: false,
        template_blocks: None,
        template_directives: Vec::new(),
        authorship: None,
    }
}

fn write(id: &str, position: u32, heading: &str, text: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {
        "toolUseId": format!("write-{id}"), "name": "write_full_draft_section",
        "input": {"section_id": id, "position": position, "heading": heading,
            "blocks": [{"kind": "paragraph", "text": text}]}
    }})
}

fn tools(calls: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": calls}},
        "stopReason": "tool_use", "usage": {"inputTokens": 20, "outputTokens": 10}
    }))
}

fn closing() -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": [{"text": "Draft ready."}]}},
        "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5}
    }))
}

struct CrashedRun {
    server: MockServer,
    sdk: aws_config::SdkConfig,
    s3: aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    run: store::LoadedRun,
    workspace: store::LoadedWorkspace,
}

/// Run the real serial writer, then drop its whole command future after one
/// section commits and the next model request reaches a stalled mock stream.
/// No Stop settlement or failure cleanup gets a chance to stamp the run.
async fn crash_after_one_section() -> CrashedRun {
    let server = MockServer::spawn().await;
    let sdk = aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        )))
        .endpoint_url(&server.endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build();
    let s3 = client::from_config(&sdk);
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("bucket");
    let client_id = Uuid::new_v4();
    let now = jiff::Timestamp::now();
    let client_record = claria_core::models::client::Client {
        id: client_id,
        name: "Synthetic persistence fixture".into(),
        created_at: now,
        updated_at: now,
    };
    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::client(client_id),
        serde_json::to_vec(&client_record).expect("synthetic client JSON"),
        Some("application/json"),
    )
    .await
    .expect("synthetic client");
    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::client_record_file(client_id, "synthetic.txt"),
        b"Synthetic evidence for a persistence fixture.".to_vec(),
        Some("text/plain"),
    )
    .await
    .expect("synthetic evidence");
    let workspace = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("workspace");
    let report_id = workspace.report_id;
    store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        0,
        ReportContent {
            title: "Synthetic report".into(),
            sections: vec![section(FIRST, "First"), section(SECOND, "Second")],
        },
    )
    .await
    .expect("template");
    {
        let mut state = server.state.write().await;
        state.bedrock_tool_responses =
            vec![tools(vec![write(FIRST, 0, "First", LANDED)]), closing()];
        state.bedrock_stream_stalls_after = Some(1);
    }
    let task = {
        let sdk = sdk.clone();
        let s3 = s3.clone();
        tokio::spawn(async move {
            pipeline::generate_full_report_for_report(
                &sdk,
                &s3,
                BUCKET,
                client_id,
                report_id,
                1,
                MODEL,
                pipeline::FullReportRequest::new("Use only the synthetic evidence."),
            )
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if server.state.read().await.bedrock_stream_request_count >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("second model request is in flight");
    task.abort();
    assert!(task.await.expect_err("abrupt command loss").is_cancelled());
    let workspace = store::load_for_report(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    let run_id = workspace
        .workspace
        .active_run_id
        .expect("durable run owner");
    let run = store::load_draft_run(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("reload interrupted run");
    assert_eq!(run.run.status, DraftRunStatus::Drafting);
    assert_eq!(run.run.sections[0].state, RunSectionState::Drafted);
    assert_eq!(run.run.sections[0].blocks, vec![paragraph(LANDED)]);
    assert_eq!(run.run.sections[1].state, RunSectionState::Pending);
    assert_eq!(workspace.workspace.draft.revision, 1);
    CrashedRun {
        server,
        sdk,
        s3,
        client_id,
        report_id,
        run,
        workspace,
    }
}

#[tokio::test]
async fn abrupt_command_loss_preserves_sections_but_requires_explicit_abandon_before_a_new_run() {
    let mut fixture = crash_after_one_section().await;
    let run_id = fixture.run.run.run_id;
    let hydrated = pipeline::load_resumable_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
    )
    .await
    .expect("hydrate")
    .expect("active run");
    assert_eq!(hydrated, fixture.run.run);
    let refused = pipeline::resume_draft_run(
        &fixture.sdk,
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        run_id,
        MODEL,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect_err("reload cannot infer that another owner is dead");
    assert!(
        refused
            .to_string()
            .contains("Only an interrupted whole-report draft")
    );
    pipeline::finalize_partial_draft(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        run_id,
    )
    .await
    .expect_err("partial finalization cannot take over a Drafting run");
    assert_eq!(
        fixture
            .server
            .state
            .read()
            .await
            .bedrock_tool_requests
            .len(),
        2
    );
    let still_owned =
        store::load_for_report(&fixture.s3, BUCKET, fixture.client_id, fixture.report_id)
            .await
            .expect("unchanged owner");
    assert_eq!(still_owned.etag, fixture.workspace.etag);
    assert_eq!(still_owned.workspace, fixture.workspace.workspace);

    pipeline::abandon_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        run_id,
    )
    .await
    .expect("explicit abandon releases the report");
    let abandoned = store::load_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        run_id,
    )
    .await
    .expect("abandoned history");
    assert_eq!(abandoned.run.status, DraftRunStatus::Abandoned);
    assert_eq!(abandoned.run.sections[0].blocks, vec![paragraph(LANDED)]);
    // Even an old owner holding the latest pre-abandon tokens cannot put its
    // run or accepted workspace back after the explicit action.
    fixture.run.run.title = Some("Stale owner".into());
    store::save_draft_run(&fixture.s3, BUCKET, &mut fixture.run)
        .await
        .expect_err("abandon fences the old run token");
    store::save_loaded(&fixture.s3, BUCKET, &mut fixture.workspace)
        .await
        .expect_err("abandon fences the old workspace token");
    {
        let mut state = fixture.server.state.write().await;
        state.bedrock_stream_stalls_after = None;
        state.bedrock_tool_responses = vec![
            tools(vec![
                serde_json::json!({"toolUse": {"toolUseId": "title", "name": "set_full_draft_title",
                    "input": {"title": "Fresh synthetic report"}}}),
                write(FIRST, 0, "First", "Fresh first section."),
                write(SECOND, 1, "Second", "Fresh second section."),
                serde_json::json!({"toolUse": {"toolUseId": "finish", "name": "finish_full_draft",
                    "input": {"summary": "Finished a new synthetic run."}}}),
            ]),
            closing(),
        ];
    }
    let fresh = pipeline::generate_full_report_for_report(
        &fixture.sdk,
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        1,
        MODEL,
        pipeline::FullReportRequest::new("Start a new synthetic draft."),
    )
    .await
    .expect("a new run is allowed after explicit abandon");
    assert_eq!(fresh.workspace.draft.revision, 2);
    assert_eq!(fresh.workspace.active_run_id, None);
    assert_eq!(
        fresh.workspace.draft.content.sections[0].blocks,
        vec![paragraph("Fresh first section.")]
    );
    let fresh_run = fresh.workspace.draft.content.sections[0]
        .authorship
        .as_ref()
        .expect("new run provenance")
        .run_id
        .expect("run ID");
    assert_ne!(fresh_run, run_id);
    let old = store::load_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        run_id,
    )
    .await
    .expect("original history survives new generation");
    assert_eq!(old.run, abandoned.run);
}

/// Produce a genuine accepted revision through the writer, then restore only
/// the pre-completion run stamp to represent its lost final PUT. Authorship
/// and revision evidence come from the real finish path, not invented fields.
async fn completed_revision_with_lost_run_stamp() -> CrashedRun {
    let mut fixture = crash_after_one_section().await;
    fixture.run.run.status = DraftRunStatus::Stopped;
    store::save_draft_run(&fixture.s3, BUCKET, &mut fixture.run)
        .await
        .expect("seed known interrupted status for the completion fixture");
    {
        let mut state = fixture.server.state.write().await;
        state.bedrock_stream_stalls_after = None;
        state.bedrock_tool_responses = vec![
            tools(vec![
                serde_json::json!({"toolUse": {"toolUseId": "title", "name": "set_full_draft_title",
                    "input": {"title": "Completed synthetic report"}}}),
                write(SECOND, 1, "Second", "A completed second section."),
                serde_json::json!({"toolUse": {"toolUseId": "finish", "name": "finish_full_draft",
                    "input": {"summary": "Finished the synthetic report."}}}),
            ]),
            closing(),
        ];
    }
    pipeline::resume_draft_run(
        &fixture.sdk,
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
        MODEL,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect("real writer completion");
    fixture.workspace =
        store::load_for_report(&fixture.s3, BUCKET, fixture.client_id, fixture.report_id)
            .await
            .expect("accepted revision");
    assert_eq!(fixture.workspace.workspace.draft.revision, 2);
    fixture.run = store::load_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
    )
    .await
    .expect("completed run");
    assert_eq!(fixture.run.run.status, DraftRunStatus::Completed);
    assert_eq!(fixture.run.run.finalized_revision, Some(2));
    fixture.run.run.status = DraftRunStatus::Drafting;
    fixture.run.run.finalized_revision = None;
    store::save_draft_run(&fixture.s3, BUCKET, &mut fixture.run)
        .await
        .expect("restore the pre-completion stamp");
    fixture
}

async fn assert_resume_refuses_without_healing(fixture: &CrashedRun) {
    let calls = fixture
        .server
        .state
        .read()
        .await
        .bedrock_tool_requests
        .len();
    pipeline::resume_draft_run(
        &fixture.sdk,
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
        MODEL,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect_err("unproven completion must not authorize resume");
    let reloaded = store::load_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
    )
    .await
    .expect("unhealed run");
    assert_eq!(reloaded.etag, fixture.run.etag);
    assert_eq!(reloaded.run, fixture.run.run);
    assert_eq!(
        fixture
            .server
            .state
            .read()
            .await
            .bedrock_tool_requests
            .len(),
        calls
    );
}

#[tokio::test]
async fn a_lost_completion_stamp_heals_to_its_own_revision_after_a_later_title_edit() {
    let fixture = completed_revision_with_lost_run_stamp().await;
    let mut content = fixture.workspace.workspace.draft.content.clone();
    content.title = "A later manual title".into();
    let edited = store::save_report_draft_for_report(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        2,
        content,
    )
    .await
    .expect("later manual revision");
    assert_eq!(edited.draft.revision, 3);
    assert_eq!(
        edited.draft.content.sections[0]
            .authorship
            .as_ref()
            .expect("retained section stamp")
            .revision,
        2
    );
    let calls = fixture
        .server
        .state
        .read()
        .await
        .bedrock_tool_requests
        .len();
    pipeline::resume_draft_run(
        &fixture.sdk,
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
        MODEL,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect_err("a completed run is not restarted");
    let healed = store::load_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
    )
    .await
    .expect("healed run");
    assert_eq!(healed.run.status, DraftRunStatus::Completed);
    assert_eq!(
        healed.run.finalized_revision,
        Some(2),
        "the later manual revision is not this run's work"
    );
    assert_eq!(
        fixture
            .server
            .state
            .read()
            .await
            .bedrock_tool_requests
            .len(),
        calls
    );
}

#[tokio::test]
async fn an_unrelated_manual_revision_does_not_prove_an_interrupted_run_completed() {
    let mut fixture = crash_after_one_section().await;
    // A lost failure stamp after workspace release leaves Drafting while an
    // unrelated manual edit can advance the accepted document.
    fixture.workspace.workspace.active_run_id = None;
    store::save_loaded(&fixture.s3, BUCKET, &mut fixture.workspace)
        .await
        .expect("released workspace");
    let mut content = fixture.workspace.workspace.draft.content.clone();
    content.title = "Unrelated manual revision".into();
    store::save_report_draft_for_report(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        1,
        content,
    )
    .await
    .expect("manual revision without model authorship");
    assert_resume_refuses_without_healing(&fixture).await;
}

#[tokio::test]
async fn another_runs_authorship_does_not_prove_a_staged_run_completed() {
    let mut fixture = completed_revision_with_lost_run_stamp().await;
    let mut different = fixture.run.run.clone();
    different.run_id = Uuid::new_v4();
    fixture.run = store::create_draft_run(&fixture.s3, BUCKET, different)
        .await
        .expect("another unfinished run");
    assert_resume_refuses_without_healing(&fixture).await;
}

#[tokio::test]
async fn erased_authorship_does_not_authorize_completion_healing() {
    let fixture = completed_revision_with_lost_run_stamp().await;
    let mut content = fixture.workspace.workspace.draft.content.clone();
    for section in &mut content.sections {
        section.blocks = vec![paragraph("A later manual replacement.")];
    }
    let edited = store::save_report_draft_for_report(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        2,
        content,
    )
    .await
    .expect("replace every authored section");
    assert!(edited.draft.content.sections.iter().all(|section| {
        section
            .authorship
            .as_ref()
            .is_none_or(|stamp| stamp.run_id != Some(fixture.run.run.run_id))
    }));
    assert_resume_refuses_without_healing(&fixture).await;
}

#[tokio::test]
async fn a_stopped_run_cannot_resume_over_a_different_stored_owner() {
    let mut fixture = crash_after_one_section().await;
    fixture.run.run.status = DraftRunStatus::Stopped;
    store::save_draft_run(&fixture.s3, BUCKET, &mut fixture.run)
        .await
        .expect("known interrupted run");
    let mut next = fixture.run.run.clone();
    next.run_id = Uuid::new_v4();
    next.status = DraftRunStatus::Drafting;
    let next = store::create_draft_run(&fixture.s3, BUCKET, next)
        .await
        .expect("another stored owner");
    fixture.workspace.workspace.active_run_id = Some(next.run.run_id);
    store::save_loaded(&fixture.s3, BUCKET, &mut fixture.workspace)
        .await
        .expect("workspace belongs to the other run");
    let error = pipeline::resume_draft_run(
        &fixture.sdk,
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
        MODEL,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect_err("a different stored owner refuses resume");
    assert_eq!(error.to_string(), store::ACTIVE_RUN_MESSAGE);
    let run = store::load_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        fixture.run.run.run_id,
    )
    .await
    .expect("original run unchanged");
    assert_eq!(run.etag, fixture.run.etag);
    assert_eq!(run.run, fixture.run.run);
    let workspace =
        store::load_for_report(&fixture.s3, BUCKET, fixture.client_id, fixture.report_id)
            .await
            .expect("owner unchanged");
    assert_eq!(workspace.etag, fixture.workspace.etag);
    assert_eq!(workspace.workspace, fixture.workspace.workspace);
    let owner = store::load_draft_run(
        &fixture.s3,
        BUCKET,
        fixture.client_id,
        fixture.report_id,
        next.run.run_id,
    )
    .await
    .expect("competing run unchanged");
    assert_eq!(owner.etag, next.etag);
    assert_eq!(owner.run, next.run);
    assert_eq!(
        fixture
            .server
            .state
            .read()
            .await
            .bedrock_tool_requests
            .len(),
        2
    );
}
