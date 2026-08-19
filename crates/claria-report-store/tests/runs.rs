//! Durable drafting-run behaviour: create-once objects, ETag-conditional
//! saves, the per-session listing, and the interrupted-section demotion an
//! interrupted run resumes through.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_core::models::{
    report::ReportBlock,
    report_run::{DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, RunSection, RunSectionState},
};
use claria_mock_aws::testing::MockServer;
use claria_report_store::{self as store, REPORT_CONFLICT_MESSAGE};
use claria_storage::client;
use jiff::Timestamp;
use uuid::Uuid;

const BUCKET: &str = "claria-draft-run-test";

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

fn timestamp(value: &str) -> Timestamp {
    value.parse().expect("timestamp")
}

fn section(position: u32, heading: &str, state: RunSectionState) -> RunSection {
    RunSection {
        section_id: Uuid::new_v4(),
        heading: heading.to_string(),
        position,
        state,
        blocks: Vec::new(),
        citations: Vec::new(),
        attempts: 0,
        error: None,
        updated_at: timestamp("2026-08-01T12:00:00Z"),
    }
}

fn run(client_id: Uuid, report_id: Uuid, updated_at: &str) -> DraftRun {
    DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id: Uuid::new_v4(),
        report_id,
        client_id,
        base_revision: 2,
        status: DraftRunStatus::Drafting,
        plan: None,
        title: None,
        sections: vec![
            section(0, "Background", RunSectionState::Pending),
            section(1, "Test results", RunSectionState::Pending),
        ],
        instructions: Vec::new(),
        writer_model_id: "us.anthropic.claude-opus-test".to_string(),
        finalized_revision: None,
        partial: false,
        created_at: timestamp("2026-07-01T00:00:00Z"),
        updated_at: timestamp(updated_at),
    }
}

#[tokio::test]
async fn a_run_is_created_once_and_reloads_with_its_landed_sections() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = Uuid::new_v4();

    let created = store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, report_id, "2026-08-01T12:00:00Z"),
    )
    .await
    .expect("create run");
    assert_eq!(
        created.key,
        claria_core::s3_keys::report_draft_run(client_id, report_id, created.run.run_id)
    );
    assert!(!created.etag.is_empty());

    // A replayed create must never clobber a run that may already hold
    // sections the writer landed.
    let mut replay = created.run.clone();
    replay.sections.clear();
    let conflict = store::create_draft_run(&s3, BUCKET, replay)
        .await
        .expect_err("duplicate create");
    assert_eq!(conflict.to_string(), REPORT_CONFLICT_MESSAGE);

    let loaded = store::load_draft_run(&s3, BUCKET, client_id, report_id, created.run.run_id)
        .await
        .expect("load run");
    assert_eq!(loaded.run, created.run);
    assert_eq!(loaded.etag, created.etag);
}

#[tokio::test]
async fn saving_a_landed_section_refreshes_the_etag_and_stamps_the_run() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = Uuid::new_v4();
    let mut loaded = store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, report_id, "2026-08-01T12:00:00Z"),
    )
    .await
    .expect("create run");
    let original_etag = loaded.etag.clone();
    let original_updated_at = loaded.run.updated_at;

    loaded.run.sections[0].state = RunSectionState::Drafted;
    loaded.run.sections[0].blocks = vec![ReportBlock::Paragraph {
        text: "Referred for a neuropsychological evaluation.".to_string(),
    }];
    loaded.run.sections[0].attempts = 1;
    store::save_draft_run(&s3, BUCKET, &mut loaded)
        .await
        .expect("save landed section");
    assert_ne!(loaded.etag, original_etag);
    assert!(loaded.run.updated_at > original_updated_at);

    let reloaded = store::load_draft_run(&s3, BUCKET, client_id, report_id, loaded.run.run_id)
        .await
        .expect("reload run");
    assert_eq!(reloaded.run.sections[0].state, RunSectionState::Drafted);
    assert_eq!(reloaded.run.sections[0].blocks.len(), 1);
    assert_eq!(reloaded.run.updated_at, loaded.run.updated_at);
}

#[tokio::test]
async fn a_stale_etag_conflicts_instead_of_overwriting_a_concurrent_run() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = Uuid::new_v4();
    let created = store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, report_id, "2026-08-01T12:00:00Z"),
    )
    .await
    .expect("create run");
    let run_id = created.run.run_id;

    let mut first = store::load_draft_run(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("first reader");
    let mut second = store::load_draft_run(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("second reader");

    second.run.status = DraftRunStatus::Stopped;
    store::save_draft_run(&s3, BUCKET, &mut second)
        .await
        .expect("second reader saves first");

    first.run.status = DraftRunStatus::Failed;
    let conflict = store::save_draft_run(&s3, BUCKET, &mut first)
        .await
        .expect_err("stale save");
    assert_eq!(conflict.to_string(), REPORT_CONFLICT_MESSAGE);

    let reloaded = store::load_draft_run(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("reload run");
    assert_eq!(reloaded.run.status, DraftRunStatus::Stopped);
}

#[tokio::test]
async fn runs_list_newest_first_and_stay_scoped_to_their_writing_session() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = Uuid::new_v4();
    let other_report_id = Uuid::new_v4();

    let oldest = store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, report_id, "2026-08-01T09:00:00Z"),
    )
    .await
    .expect("oldest run");
    let newest = store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, report_id, "2026-08-03T09:00:00Z"),
    )
    .await
    .expect("newest run");
    let middle = store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, report_id, "2026-08-02T09:00:00Z"),
    )
    .await
    .expect("middle run");
    store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, other_report_id, "2026-08-04T09:00:00Z"),
    )
    .await
    .expect("run for another session");

    let listed = store::list_draft_runs(&s3, BUCKET, client_id, report_id)
        .await
        .expect("list runs");
    assert_eq!(
        listed.iter().map(|run| run.run_id).collect::<Vec<_>>(),
        vec![newest.run.run_id, middle.run.run_id, oldest.run.run_id]
    );
    assert!(
        store::list_draft_runs(&s3, BUCKET, client_id, Uuid::new_v4())
            .await
            .expect("list runs for an untouched session")
            .is_empty()
    );
}

#[tokio::test]
async fn an_interrupted_section_is_demoted_when_the_run_is_picked_back_up() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = Uuid::new_v4();
    let mut loaded = store::create_draft_run(
        &s3,
        BUCKET,
        run(client_id, report_id, "2026-08-01T12:00:00Z"),
    )
    .await
    .expect("create run");
    let run_id = loaded.run.run_id;

    // What a run that died mid-section leaves behind.
    loaded.run.sections[0].state = RunSectionState::Drafted;
    loaded.run.sections[0].blocks = vec![ReportBlock::Paragraph {
        text: "Landed before the interruption.".to_string(),
    }];
    loaded.run.sections[1].state = RunSectionState::Drafting;
    loaded.run.sections[1].attempts = 1;
    store::save_draft_run(&s3, BUCKET, &mut loaded)
        .await
        .expect("persist interrupted run");

    let mut resumed = store::load_draft_run(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("resume run");
    assert_eq!(resumed.run.sections[1].state, RunSectionState::Drafting);
    assert_eq!(
        resumed
            .run
            .demote_interrupted_sections(jiff::Timestamp::now()),
        1
    );
    resumed.run.status = DraftRunStatus::Stopped;
    store::save_draft_run(&s3, BUCKET, &mut resumed)
        .await
        .expect("persist demotion");

    let reloaded = store::load_draft_run(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("reload run");
    assert_eq!(reloaded.run.sections[0].state, RunSectionState::Drafted);
    assert_eq!(reloaded.run.sections[0].blocks.len(), 1);
    assert_eq!(reloaded.run.sections[1].state, RunSectionState::Pending);
    assert_eq!(reloaded.run.sections[1].attempts, 1);
}

#[tokio::test]
async fn loading_an_unknown_run_says_it_is_gone_rather_than_failing_obscurely() {
    let (_server, s3) = setup().await;
    let error = store::load_draft_run(&s3, BUCKET, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
        .await
        .expect_err("missing run");
    assert_eq!(
        error.to_string(),
        "That drafting run is no longer available."
    );

    let nil = store::load_draft_run(&s3, BUCKET, Uuid::nil(), Uuid::new_v4(), Uuid::new_v4())
        .await
        .expect_err("nil client");
    assert!(nil.to_string().contains("must not be nil"), "{nil}");
}
