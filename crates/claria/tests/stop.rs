//! Stopping the writer: a drafting run parks where the reader stopped it with
//! every landed section intact and resumable, a targeted turn stops without
//! touching the session, and a stop that arrives after the draft is finished
//! changes nothing at all.

use std::sync::{Arc, Mutex};

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria as pipeline;
use claria_bedrock::converse::StopSignal;
use claria_core::models::{
    report::{ReportBlock, ReportContent, ReportSection},
    report_run::{DraftRun, DraftRunStatus, RunSectionState},
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_store as store;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-writer-stop-test";
const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";

const REFERRAL_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const BACKGROUND_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SUMMARY_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

/// How often the watcher tasks below re-read the mock's state. Short enough
/// that a test never waits on it perceptibly; the conditions they wait for
/// are all latched, so a missed poll only costs another millisecond.
const POLL: std::time::Duration = std::time::Duration::from_millis(1);

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
        template_directives: Vec::new(),
        authorship: None,
    }
}

/// A client with one readable record and a three-section template applied as
/// revision 1 — the shape every run in this file builds on.
async fn seed_templated_report(s3: &aws_sdk_s3::Client, client_id: Uuid) -> Uuid {
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

fn finish_call(id: &str, summary: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": id, "name": "finish_full_draft", "input": {
        "summary": summary
    }}})
}

fn tool_round(calls: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": calls}},
        "stopReason": "tool_use",
        "usage": {"inputTokens": 5, "outputTokens": 20}
    }))
}

fn closing_text(text: &str) -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(serde_json::json!({
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

fn sections_in(run: &DraftRun, state: RunSectionState) -> Vec<&str> {
    let mut sections = run
        .sections
        .iter()
        .filter(|section| section.state == state)
        .collect::<Vec<_>>();
    sections.sort_by_key(|section| section.position);
    sections
        .iter()
        .map(|section| section.heading.as_str())
        .collect()
}

/// The durable receipt for one writer attempt.
async fn stored_attempt(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    attempt_id: Uuid,
) -> store::ReportAttemptMetadata {
    let key = claria_core::s3_keys::report_attempt(client_id, attempt_id);
    let (stored, _): (store::ReportAttemptMetadata, String) =
        claria_storage::state::load_state(s3, BUCKET, &key)
            .await
            .expect("attempt receipt");
    stored
}

/// Fire `stop` once the mock has taken delivery of its `nth` streamed
/// request. Paired with a stall injected at that same request, this lands the
/// Stop while the writer is parked waiting for a frame that will never come —
/// the case a check between frames could not catch.
fn stop_when_request_arrives(server: &MockServer, nth: usize, stop: StopSignal) {
    let state = server.state.clone();
    tokio::spawn(async move {
        loop {
            if state.read().await.bedrock_stream_request_count >= nth {
                stop.stop();
                return;
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

// ---------------------------------------------------------------------------
// Whole-report runs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_run_stopped_mid_stream_keeps_its_sections_and_stays_resumable() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    // Two rounds land two sections; the third call never finishes streaming,
    // which is where the reader presses Stop.
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
        ],
    )
    .await;
    server.state.write().await.bedrock_stream_stalls_after = Some(2);

    let stop = StopSignal::new();
    stop_when_request_arrives(&server, 3, stop.clone());

    let error = pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation.").with_stop(&stop),
    )
    .await
    .expect_err("a stopped run has no outcome");

    assert!(error.is_stopped(), "unexpected failure: {error}");
    assert_eq!(error.to_string(), "The draft run was stopped.");

    // Everything that landed before the Stop is still durable, and the run is
    // parked in the one state a resume accepts.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(error.stopped_run_id(), Some(run.run_id));
    assert_eq!(run.status, DraftRunStatus::Stopped);
    assert_eq!(run.title.as_deref(), Some("Psychoeducational Evaluation"));
    assert_eq!(
        sections_in(&run, RunSectionState::Drafted),
        vec!["Reason for Referral", "Background"]
    );
    assert_eq!(
        sections_in(&run, RunSectionState::Pending),
        vec!["Summary and Clinical Interpretation"]
    );
    assert!(run.finalized_revision.is_none());

    // Unlike a failure, a stop keeps the workspace pointing at its run: that
    // pointer is what the writing surface hydrates from, and what refuses
    // competing edits until the reader resumes, finalizes, or discards it.
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, Some(run.run_id));
    assert_eq!(workspace.draft.revision, 1);
    assert!(workspace.session.turns.is_empty());

    let attempt = error.attempt().expect("a stopped turn still has a receipt");
    assert_eq!(
        stored_attempt(&s3, client_id, attempt.attempt_id)
            .await
            .status,
        store::ReportAttemptStatus::Stopped
    );

    // And the run picks straight back up, deciding only the section the Stop
    // interrupted.
    server.state.write().await.bedrock_stream_stalls_after = None;
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
        run.run_id,
        MODEL_ID,
        pipeline::FullReportRequest::new("Finish the summary."),
    )
    .await
    .expect("a stopped run is resumable");

    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.active_run_id, None);
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph("Referred for attention concerns.")],
        "the section that landed before the Stop was re-written"
    );
    assert_eq!(
        only_run(&s3, client_id, report_id).await.status,
        DraftRunStatus::Completed
    );
}

#[tokio::test]
async fn a_stop_between_rounds_never_opens_another_call() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    // Scripted well past the Stop: if a third call went out it would be
    // answered, and the run would finish as though nothing had happened.
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

    // The reader presses Stop the moment the second section lands — which is
    // to say while the run is still executing that round, after the section
    // was made durable and before the next call could be issued.
    let stop = StopSignal::new();
    let stopper = stop.clone();
    let progress = |event: pipeline::ReportTurnProgress| {
        if let pipeline::ReportTurnProgress::SectionCompleted { section_id, .. } = &event
            && section_id == BACKGROUND_ID
        {
            stopper.stop();
        }
    };

    let error = pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation.")
            .with_stop(&stop)
            .with_progress(&progress),
    )
    .await
    .expect_err("a stopped run has no outcome");
    assert!(error.is_stopped(), "unexpected failure: {error}");

    assert_eq!(
        server.state.read().await.bedrock_tool_requests.len(),
        2,
        "a Stop during the second round still opened a third conversation"
    );

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Stopped);
    assert_eq!(
        sections_in(&run, RunSectionState::Drafted),
        vec!["Reason for Referral", "Background"]
    );
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, Some(run.run_id));
    assert_eq!(workspace.draft.revision, 1);
}

/// Once `finish_full_draft` has succeeded the run is past its cut, and a Stop
/// pressed in the seconds it takes the writer to say so must not throw the
/// finished draft away.
#[tokio::test]
async fn a_stop_after_the_finish_call_leaves_the_revision_standing() {
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

    let stop = StopSignal::new();
    let stopper = stop.clone();
    let progress = |event: pipeline::ReportTurnProgress| {
        if let pipeline::ReportTurnProgress::ToolFinished { name, .. } = &event
            && name == "finish_full_draft"
        {
            stopper.stop();
        }
    };

    let outcome = pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation.")
            .with_stop(&stop)
            .with_progress(&progress),
    )
    .await
    .expect("a Stop after the finish call is a no-op");

    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.active_run_id, None);
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(run.finalized_revision, Some(2));
}

// ---------------------------------------------------------------------------
// Targeted turns
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_stopped_targeted_turn_leaves_the_session_untouched() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    server.state.write().await.bedrock_stream_stalls = 1;

    let stop = StopSignal::new();
    stop_when_request_arrives(&server, 1, stop.clone());

    let error = pipeline::send_report_message_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::ReportMessageRequest::new("Tighten the referral section.").with_stop(&stop),
    )
    .await
    .expect_err("a stopped turn has no outcome");

    assert!(error.is_stopped(), "unexpected failure: {error}");
    assert_eq!(error.to_string(), "The writer turn was stopped.");
    // A targeted turn owns no run, and a clean abort leaves none behind.
    assert_eq!(error.stopped_run_id(), None);
    assert!(
        store::list_draft_runs(&s3, BUCKET, client_id, report_id)
            .await
            .expect("list drafting runs")
            .is_empty()
    );

    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.draft.revision, 1);
    assert_eq!(workspace.draft.content.sections[0].blocks, boilerplate());
    assert!(workspace.session.turns.is_empty());
    assert!(workspace.session.pending_proposal.is_none());
    assert_eq!(workspace.active_run_id, None);

    let attempt = error.attempt().expect("a stopped turn still has a receipt");
    assert_eq!(
        stored_attempt(&s3, client_id, attempt.attempt_id)
            .await
            .status,
        store::ReportAttemptStatus::Stopped
    );
}

/// A signal nobody fires costs a turn nothing: the default request shape is
/// the one that ran before stopping existed.
#[tokio::test]
async fn a_turn_with_no_stop_behind_it_runs_to_completion() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    script(&server, vec![closing_text("Nothing to change yet.")]).await;

    let outcome = pipeline::send_report_message_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::ReportMessageRequest::new("Review the current report."),
    )
    .await
    .expect("an unstoppable turn still finishes");
    assert_eq!(outcome.assistant_text, "Nothing to change yet.");
}

/// Two runs stopped from the same registry must not disturb each other: the
/// signal is per-turn, so firing one leaves the other's turn running.
#[tokio::test]
async fn one_stop_signal_ends_only_its_own_turn() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    script(&server, vec![closing_text("Nothing to change yet.")]).await;

    let stopped_already = StopSignal::new();
    stopped_already.stop();
    let other = StopSignal::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = {
        let seen = seen.clone();
        move |event: pipeline::ReportTurnProgress| {
            seen.lock().expect("progress lock").push(event);
        }
    };

    // The turn watching the untouched signal runs normally...
    pipeline::send_report_message_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::ReportMessageRequest::new("Review the current report.")
            .with_stop(&other)
            .with_progress(&recorder),
    )
    .await
    .expect("another turn's Stop is not this turn's");
    assert!(!seen.lock().expect("progress lock").is_empty());

    // ...while a turn whose own signal is already fired never calls Bedrock.
    let before = server.state.read().await.bedrock_tool_requests.len();
    let error = pipeline::send_report_message_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::ReportMessageRequest::new("Tighten the referral section.")
            .with_stop(&stopped_already),
    )
    .await
    .expect_err("a turn stopped before it started has no outcome");
    assert!(error.is_stopped(), "unexpected failure: {error}");
    assert_eq!(
        server.state.read().await.bedrock_tool_requests.len(),
        before,
        "a turn stopped before its first call still opened one"
    );
}
