//! A failed tool write must not be replayed by failure cleanup.

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

const BUCKET: &str = "claria-writer-crash-test";
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
        .retry_config(aws_config::retry::RetryConfig::standard().with_max_attempts(1))
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

#[tokio::test]
async fn a_refused_section_write_is_not_persisted_by_failure_cleanup() {
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
                {"toolUse": {"toolUseId": "section", "name": "write_full_draft_section", "input": {
                    "section_id": SECTION, "position": 0, "heading": "Synthetic section",
                    "blocks": [{"kind": "paragraph", "text": TEXT}]
                }}}
            ]}},
            "stopReason": "tool_use",
            "usage": {"inputTokens": 5, "outputTokens": 20}
        })));
    let mut gate = s3_gate::S3Gate::spawn(&server.endpoint, |path, body| {
        path.contains("/runs/")
            && body
                .windows(b"\"state\": \"drafted\"".len())
                .any(|part| part == b"\"state\": \"drafted\"")
    })
    .await;
    let s3 = claria_storage::client::from_config(&config(&gate.endpoint));
    let completed = AtomicUsize::new(0);
    let progress = |event| {
        if matches!(event, ReportTurnProgress::SectionCompleted { .. }) {
            completed.fetch_add(1, Ordering::SeqCst);
        }
    };
    let mut turn = Box::pin(generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL,
        FullReportRequest::new("Write the synthetic section.").with_progress(&progress),
    ));
    let path = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            path = gate.held() => path,
            outcome = &mut turn => panic!("turn ended before section write: {outcome:?}"),
        }
    })
    .await
    .expect("selected section write");
    let key = path
        .split('?')
        .next()
        .expect("target path")
        .strip_prefix(&format!("/{BUCKET}/"))
        .expect("run key")
        .to_string();
    server
        .state
        .write()
        .await
        .s3_conditional_conflicts_remaining
        .insert(key.clone(), 4);
    gate.release();
    let error = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("failed turn settles")
        .expect_err("section save is refused");
    assert_eq!(
        error.failure_code(),
        Some(store::ReportFailureCode::RecordStorage)
    );
    assert_eq!(completed.load(Ordering::SeqCst), 0);
    assert_eq!(
        server.state.read().await.s3_conditional_conflicts_remaining[&key],
        0
    );
    let mut runs = store::list_draft_runs(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("reload runs");
    let run = runs.pop().expect("one failed run");
    assert_eq!(run.status, DraftRunStatus::Failed);
    assert_eq!(
        run.sections[0].state,
        RunSectionState::Pending,
        "failure cleanup persisted the section whose conditional PUT was refused"
    );
    assert!(run.sections[0].blocks.is_empty());
    let workspace = store::load_report_workspace_by_id(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, None);
    assert_eq!(workspace.draft.revision, 1);
    assert_eq!(server.state.read().await.bedrock_tool_requests.len(), 1);
}

#[tokio::test]
async fn a_lost_section_put_reply_preserves_the_commit_and_resume_does_not_rewrite_it() {
    let server = MockServer::spawn().await;
    let sdk = config(&server.endpoint);
    let direct_s3 = claria_storage::client::from_config(&sdk);
    let client_id = Uuid::new_v4();
    let report_id = seed(&direct_s3, client_id).await;
    server.state.write().await.bedrock_tool_responses.push(
        ScriptedBedrockResponse::success(json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "title", "name": "set_full_draft_title", "input": {"title": "Synthetic complete title"}}},
                {"toolUse": {"toolUseId": "section", "name": "write_full_draft_section", "input": {
                    "section_id": SECTION, "position": 0, "heading": "Synthetic section",
                    "blocks": [{"kind": "paragraph", "text": TEXT}]
                }}}
            ]}}, "stopReason": "tool_use", "usage": {"inputTokens": 5, "outputTokens": 20}
        }))
    );
    let mut gate = s3_gate::S3Gate::after_write(
        &server.endpoint,
        |path, body| {
            path.contains("/runs/")
                && body
                    .windows(b"\"state\": \"drafted\"".len())
                    .any(|part| part == b"\"state\": \"drafted\"")
        },
        true,
    )
    .await;
    let s3 = claria_storage::client::from_config(&config(&gate.endpoint));
    let completed = AtomicUsize::new(0);
    let progress = |event| {
        if matches!(event, ReportTurnProgress::SectionCompleted { .. }) {
            completed.fetch_add(1, Ordering::SeqCst);
        }
    };
    let mut turn = Box::pin(generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL,
        FullReportRequest::new("Write the synthetic section.").with_progress(&progress),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = gate.held() => {},
            outcome = &mut turn => panic!("turn ended before committed reply: {outcome:?}"),
        }
    })
    .await
    .expect("section commits before its lost response");
    let mut runs = store::list_draft_runs(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("read committed section independently");
    let committed = runs.pop().expect("one run");
    assert_eq!(committed.sections[0].state, RunSectionState::Drafted);
    assert_eq!(completed.load(Ordering::SeqCst), 0);
    gate.release();
    let error = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("lost reply settles")
        .expect_err("caller did not observe success");
    assert_eq!(
        error.failure_code(),
        Some(store::ReportFailureCode::RecordStorage)
    );
    let durable = store::load_draft_run(&direct_s3, BUCKET, client_id, report_id, committed.run_id)
        .await
        .expect("reconciled durable run")
        .run;
    assert_eq!(durable.status, DraftRunStatus::Failed);
    assert_eq!(durable.sections, committed.sections);
    assert_eq!(completed.load(Ordering::SeqCst), 0);
    let workspace = store::load_report_workspace_by_id(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("released workspace");
    assert_eq!(workspace.active_run_id, None);
    server.state.write().await.bedrock_tool_responses.extend([
        ScriptedBedrockResponse::success(json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "finish", "name": "finish_full_draft", "input": {"summary": "Finished saved content."}}}
            ]}}, "stopReason": "tool_use", "usage": {"inputTokens": 5, "outputTokens": 2}
        })),
        ScriptedBedrockResponse::success(json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Synthetic complete response."}]}},
            "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 2}
        })),
    ]);
    let outcome = claria::resume_draft_run(
        &sdk,
        &direct_s3,
        BUCKET,
        client_id,
        report_id,
        committed.run_id,
        MODEL,
        FullReportRequest::new("Finish the already saved content."),
    )
    .await
    .expect("resume needs no repeated section write");
    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.draft.content.sections.len(), 1);
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![ReportBlock::Paragraph {
            text: TEXT.to_string()
        }]
    );
    assert_eq!(server.state.read().await.bedrock_tool_requests.len(), 3);
}

#[derive(Clone, Copy, PartialEq)]
enum ResumeRace {
    Resumed,
    RolledBack,
    Abandoned,
}

async fn failure_resume_race(race: ResumeRace) {
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
        .push(ScriptedBedrockResponse::error(
            400,
            json!({"__type": "ValidationException", "message": "Synthetic failure"}),
        ));
    let mut gate = s3_gate::S3Gate::after_write(
        &server.endpoint,
        |path, body| {
            path.contains("/runs/")
                && body
                    .windows(b"\"status\": \"failed\"".len())
                    .any(|part| part == b"\"status\": \"failed\"")
        },
        false,
    )
    .await;
    let s3 = claria_storage::client::from_config(&config(&gate.endpoint));
    let mut old_turn = Box::pin(generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL,
        FullReportRequest::new("Fail before any tool effects."),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = gate.held() => {},
            outcome = &mut old_turn => panic!("turn ended before Failed became visible: {outcome:?}"),
        }
    }).await.expect("failure status lands");
    let mut runs = store::list_draft_runs(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("read failed run");
    let before = runs.pop().expect("one run");
    let run_id = before.run_id;
    let mut claim_gate =
        s3_gate::S3Gate::spawn(&server.endpoint, |path, _| path.contains("/sessions/")).await;
    let claim_s3 = claria_storage::client::from_config(&config(&claim_gate.endpoint));
    server.state.write().await.bedrock_stream_stalls = 1;
    let stop = StopSignal::new();
    let started = tokio::sync::Notify::new();
    let progress = |event| {
        if matches!(event, ReportTurnProgress::ModelCallStarted { .. }) {
            started.notify_one();
        }
    };
    let mut resumed = Box::pin(claria::resume_draft_run(
        &sdk,
        &claim_s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL,
        FullReportRequest::new("New owner resumes.")
            .with_stop(&stop)
            .with_progress(&progress),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = claim_gate.held() => {},
            outcome = &mut resumed => panic!("resume ended before workspace claim: {outcome:?}"),
        }
    })
    .await
    .expect("resume claimed run before workspace write");
    if race != ResumeRace::Resumed {
        gate.release();
        let original = tokio::time::timeout(Duration::from_secs(5), old_turn)
            .await
            .expect("old cleanup settles first")
            .expect_err("original failure");
        assert_eq!(
            original.failure_code(),
            Some(store::ReportFailureCode::Bedrock)
        );
        let superseding_run = if race == ResumeRace::Abandoned {
            claria::abandon_draft_run(&direct_s3, BUCKET, client_id, report_id, run_id)
                .await
                .expect("another window abandons the pending claim");
            Some(
                store::load_draft_run(&direct_s3, BUCKET, client_id, report_id, run_id)
                    .await
                    .expect("newer run body")
                    .run,
            )
        } else {
            None
        };
        claim_gate.release();
        let conflict = tokio::time::timeout(Duration::from_secs(5), resumed)
            .await
            .expect("failed claim settles")
            .expect_err("workspace claim lost CAS");
        assert!(matches!(conflict, claria::ReportPipelineError::Conflict));
        let stored = store::load_draft_run(&direct_s3, BUCKET, client_id, report_id, run_id)
            .await
            .expect("compensated run")
            .run;
        if let Some(superseding_run) = superseding_run {
            assert_eq!(
                stored, superseding_run,
                "rollback overwrote a newer run owner"
            );
        } else {
            assert_eq!(stored.status, DraftRunStatus::Failed);
            assert_eq!(
                stored.instructions, before.instructions,
                "unclaimed resume guidance was persisted"
            );
            assert_eq!(stored.sections, before.sections);
        }
        let workspace =
            store::load_report_workspace_by_id(&direct_s3, BUCKET, client_id, report_id)
                .await
                .expect("workspace released by original owner");
        assert_eq!(workspace.active_run_id, None);
        assert_eq!(
            server.state.read().await.bedrock_tool_requests.len(),
            1,
            "failed claim opened inference"
        );
        return;
    }
    claim_gate.release();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            () = started.notified() => {},
            outcome = &mut resumed => panic!("resumed owner ended before inference: {outcome:?}"),
        }
    })
    .await
    .expect("new owner claims run and workspace");
    gate.release();
    let error = tokio::time::timeout(Duration::from_secs(5), old_turn)
        .await
        .expect("old cleanup settles")
        .expect_err("original model failure remains");
    assert_eq!(
        error.failure_code(),
        Some(store::ReportFailureCode::Bedrock)
    );
    let workspace = store::load_report_workspace_by_id(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("reload resumed workspace");
    assert_eq!(
        workspace.active_run_id,
        Some(run_id),
        "old cleanup cleared the new owner's claim"
    );
    let run = store::load_draft_run(&direct_s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("reload resumed run")
        .run;
    assert_eq!(run.status, DraftRunStatus::Drafting);
    stop.stop();
    let stopped = tokio::time::timeout(Duration::from_secs(5), resumed)
        .await
        .expect("resumed fixture stops")
        .expect_err("stop parks resumed owner");
    assert!(stopped.is_stopped());
}

#[tokio::test]
async fn old_failure_cleanup_cannot_release_a_resumed_owner_of_the_same_run() {
    failure_resume_race(ResumeRace::Resumed).await;
}

#[tokio::test]
async fn a_resume_that_loses_the_workspace_claim_restores_its_prior_run() {
    failure_resume_race(ResumeRace::RolledBack).await;
}

#[tokio::test]
async fn resume_compensation_does_not_overwrite_a_newer_run_owner() {
    failure_resume_race(ResumeRace::Abandoned).await;
}

#[tokio::test]
async fn a_lost_workspace_claim_reply_is_reconciled_before_inference() {
    let server = MockServer::spawn().await;
    let sdk = config(&server.endpoint);
    let direct_s3 = claria_storage::client::from_config(&sdk);
    let client_id = Uuid::new_v4();
    let report_id = seed(&direct_s3, client_id).await;
    let initial_stop = StopSignal::new();
    initial_stop.stop();
    let stopped = generate_full_report_for_report(
        &sdk,
        &direct_s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL,
        FullReportRequest::new("Create a parked run.").with_stop(&initial_stop),
    )
    .await
    .expect_err("park without inference");
    assert!(stopped.is_stopped());
    let mut runs = store::list_draft_runs(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("parked run");
    let run_id = runs.pop().expect("one run").run_id;
    let mut gate = s3_gate::S3Gate::after_write(
        &server.endpoint,
        |path, _| path.contains("/sessions/"),
        true,
    )
    .await;
    let s3 = claria_storage::client::from_config(&config(&gate.endpoint));
    server.state.write().await.bedrock_stream_stalls = 1;
    let stop = StopSignal::new();
    let started = tokio::sync::Notify::new();
    let progress = |event| {
        if matches!(event, ReportTurnProgress::ModelCallStarted { .. }) {
            started.notify_one();
        }
    };
    let mut resumed = Box::pin(claria::resume_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL,
        FullReportRequest::new("Resume exactly once.")
            .with_stop(&stop)
            .with_progress(&progress),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = gate.held() => {},
            outcome = &mut resumed => panic!("resume ended before workspace commit: {outcome:?}"),
        }
    })
    .await
    .expect("workspace claim lands before response loss");
    assert!(server.state.read().await.bedrock_tool_requests.is_empty());
    gate.release();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            () = started.notified() => {},
            outcome = &mut resumed => panic!("lost reply prevented reconciliation: {outcome:?}"),
        }
    })
    .await
    .expect("reconciled claim admits inference");
    tokio::time::timeout(Duration::from_secs(5), async {
        let arrived = async {
            while server.state.read().await.bedrock_tool_requests.is_empty() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::select! {
            () = arrived => {},
            outcome = &mut resumed => panic!("reconciled inference ended unexpectedly: {outcome:?}"),
        }
    }).await.expect("exactly one model request reaches the local service");
    let workspace = store::load_report_workspace_by_id(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("durable claimed workspace");
    assert_eq!(workspace.active_run_id, Some(run_id));
    stop.stop();
    let stopped = tokio::time::timeout(Duration::from_secs(5), resumed)
        .await
        .expect("reconciled owner stops")
        .expect_err("stopped fixture");
    assert!(stopped.is_stopped());
    let run = store::load_draft_run(&direct_s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("parked reconciled owner")
        .run;
    assert_eq!(run.status, DraftRunStatus::Stopped);
    assert_eq!(
        run.instructions
            .iter()
            .filter(|instruction| instruction.text == "Resume exactly once.")
            .count(),
        1
    );
    assert_eq!(server.state.read().await.bedrock_tool_requests.len(), 1);
}

#[tokio::test]
async fn failure_cleanup_cannot_release_a_newer_run_without_a_workspace_change() {
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
        .push(ScriptedBedrockResponse::error(
            400,
            json!({"__type": "ValidationException", "message": "Synthetic failure"}),
        ));
    let mut gate = s3_gate::S3Gate::spawn(&server.endpoint, |path, body| {
        path.contains("/runs/")
            && body
                .windows(b"\"status\": \"failed\"".len())
                .any(|part| part == b"\"status\": \"failed\"")
    })
    .await;
    let s3 = claria_storage::client::from_config(&config(&gate.endpoint));
    let mut turn = Box::pin(generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL,
        FullReportRequest::new("Fail before any tool effects."),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = gate.held() => {},
            outcome = &mut turn => panic!("turn ended before Failed write: {outcome:?}"),
        }
    })
    .await
    .expect("old cleanup reaches Failed write");
    let workspace = store::load_for_report(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("owned workspace");
    let run_id = workspace.workspace.active_run_id.expect("active run");
    let mut newer = store::load_draft_run(&direct_s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("durable run before newer mutation");
    newer
        .run
        .instructions
        .push(claria_core::models::report_run::RunInstruction {
            text: "A newer run-only edit must retain its claim.".to_string(),
            added_at: jiff::Timestamp::now(),
        });
    store::save_draft_run(&direct_s3, BUCKET, &mut newer)
        .await
        .expect("newer run CAS");
    gate.release();
    let error = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("old cleanup loses run CAS")
        .expect_err("original failure survives");
    assert_eq!(
        error.failure_code(),
        Some(store::ReportFailureCode::Bedrock)
    );
    let current = store::load_for_report(&direct_s3, BUCKET, client_id, report_id)
        .await
        .expect("same workspace after run-only edit");
    assert_eq!(
        current.etag, workspace.etag,
        "old cleanup released another owner's workspace"
    );
    assert_eq!(current.workspace.active_run_id, Some(run_id));
    let current_run = store::load_draft_run(&direct_s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("newer run survives");
    assert_eq!(current_run.run, newer.run);
    assert_eq!(server.state.read().await.bedrock_tool_requests.len(), 1);
}
