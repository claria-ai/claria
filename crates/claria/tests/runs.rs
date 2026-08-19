//! Durable whole-report drafting: sections survive a run that dies mid-flight,
//! a resumed run finishes the same document without redoing landed work, and
//! the workspace is never left locked behind a run that ended.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria as pipeline;
use claria_core::models::{
    report::{AuthorshipKind, ReportBlock, ReportContent, ReportSection},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, PlanEntry, RunInstruction, RunPlan,
        RunSection, RunSectionState, SectionIntent,
    },
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
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
        template_directives: Vec::new(),
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

/// A section write that also attributes its claims to record quotes.
fn write_call_citing(
    id: &str,
    section_id: &str,
    position: u32,
    heading: &str,
    text: &str,
    citations: &[(&str, &str)],
) -> serde_json::Value {
    let mut call = write_call(id, section_id, position, heading, text);
    call["toolUse"]["input"]["citations"] = serde_json::Value::Array(
        citations
            .iter()
            .map(|(filename, quote)| serde_json::json!({"filename": filename, "quote": quote}))
            .collect(),
    );
    call
}

fn skip_call(id: &str, section_id: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": id, "name": "skip_full_draft_section", "input": {
        "section_id": section_id
    }}})
}

fn fail_call(id: &str, section_id: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": id, "name": "mark_section_failed", "input": {
        "section_id": section_id,
        "reason": reason
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

fn run_section(id: &str, heading: &str, position: u32, state: RunSectionState) -> RunSection {
    RunSection {
        section_id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        position,
        state,
        blocks: Vec::new(),
        citations: Vec::new(),
        attempts: 0,
        error: None,
        updated_at: jiff::Timestamp::now(),
    }
}

fn plan_entry(id: &str, heading: &str, intent: SectionIntent) -> PlanEntry {
    PlanEntry {
        section_id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        intent,
        required: true,
        scope: String::new(),
        evidence: Vec::new(),
        instruction: None,
        curated_records: None,
    }
}

/// Put a hand-built run object in S3 so a test can start from run state the
/// synthetic planner never produces — a user-approved plan, a rewrite intent,
/// or a section that already failed.
async fn seed_run(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    plan: RunPlan,
    sections: Vec<RunSection>,
) -> Uuid {
    let now = jiff::Timestamp::now();
    let run_id = Uuid::new_v4();
    let run = DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id,
        report_id,
        client_id,
        base_revision: 1,
        status: DraftRunStatus::Stopped,
        plan: Some(plan),
        title: Some("Psychoeducational Evaluation".to_string()),
        sections,
        instructions: vec![RunInstruction {
            text: "Write the whole evaluation.".to_string(),
            added_at: now,
        }],
        writer_model_id: MODEL_ID.to_string(),
        finalized_revision: None,
        partial: false,
        created_at: now,
        updated_at: now,
    };
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::report_draft_run(client_id, report_id, run_id),
        serde_json::to_vec(&run).expect("run JSON"),
        Some("application/json"),
    )
    .await
    .expect("seed the drafting run");
    run_id
}

/// Every tool result the scripted conversation returned, in the order the
/// model saw them. Read from the last captured request, which carries the
/// whole accumulated conversation exactly once — the earlier requests are
/// prefixes of it, so folding them all together would count each result again
/// for every round that followed it.
async fn tool_results(server: &MockServer) -> Vec<serde_json::Value> {
    server
        .state
        .read()
        .await
        .bedrock_tool_requests
        .last()
        .and_then(|request| request["messages"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .flat_map(|message| {
            message["content"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .filter_map(|block| block.get("toolResult").cloned())
        .collect()
}

/// The error code of the first tool result carrying one, searched from the
/// end so the assertion names the round that produced it.
fn error_codes(results: &[serde_json::Value]) -> Vec<String> {
    results
        .iter()
        .filter_map(|result| {
            result["content"][0]["json"]["error"]["code"]
                .as_str()
                .map(ToString::to_string)
        })
        .collect()
}

/// The opening message of the first Bedrock call — the checkpoint layout.
async fn opening_message(server: &MockServer) -> Vec<serde_json::Value> {
    server.state.read().await.bedrock_tool_requests[0]["messages"][0]["content"]
        .as_array()
        .expect("opening message content")
        .clone()
}

#[tokio::test]
async fn citations_are_checked_against_the_run_snapshot_and_stored() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call_citing(
                    "section-1",
                    REFERRAL_ID,
                    0,
                    "Reason for Referral",
                    "Referred for attention concerns.",
                    &[("intake.txt", "Referred by pediatrician")],
                ),
            ]),
            // A file the snapshot never carried, and a quote too short to
            // resolve against a record later.
            tool_round(vec![write_call_citing(
                "section-2",
                BACKGROUND_ID,
                1,
                "Background",
                "Documented developmental history.",
                &[("chart-review.pdf", "Referred by pediatrician")],
            )]),
            tool_round(vec![write_call_citing(
                "section-2b",
                BACKGROUND_ID,
                1,
                "Background",
                "Documented developmental history.",
                &[("intake.txt", "too short")],
            )]),
            tool_round(vec![
                write_call(
                    "section-2c",
                    BACKGROUND_ID,
                    1,
                    "Background",
                    "Documented developmental history.",
                ),
                skip_call("skip-1", SUMMARY_ID),
            ]),
            tool_round(vec![finish_call("finish-1", "Completed the evaluation.")]),
            closing_text("The evaluation is complete."),
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
    .expect("generate with citations");

    let codes = error_codes(&tool_results(&server).await);
    assert!(
        codes.contains(&"unknown_citation_file".to_string()),
        "a citation naming a file outside the snapshot was accepted: {codes:?}"
    );
    assert!(
        codes.contains(&"invalid_citation_quote".to_string()),
        "an unresolvably short quote was accepted: {codes:?}"
    );

    let run = only_run(&s3, client_id, report_id).await;
    let referral = run
        .sections
        .iter()
        .find(|section| section.section_id.to_string() == REFERRAL_ID)
        .expect("referral run section");
    assert_eq!(referral.citations.len(), 1);
    assert_eq!(referral.citations[0].filename, "intake.txt");
    assert_eq!(referral.citations[0].quote, "Referred by pediatrician");
    // The rejected writes never landed a citation on the section they named.
    let background = run
        .sections
        .iter()
        .find(|section| section.section_id.to_string() == BACKGROUND_ID)
        .expect("background run section");
    assert_eq!(background.state, RunSectionState::Drafted);
    assert!(background.citations.is_empty());
}

#[tokio::test]
async fn a_section_marked_failed_is_durable_and_still_lets_the_run_finish() {
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
                fail_call(
                    "fail-1",
                    SUMMARY_ID,
                    "No testing data is present in the records, so no interpretation can be supported.",
                ),
            ]),
            tool_round(vec![finish_call(
                "finish-1",
                "Drafted the referral; the summary could not be supported.",
            )]),
            closing_text("One section could not be drafted."),
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
        pipeline::FullReportRequest::new("Write the whole evaluation."),
    )
    .await
    .expect("a failed section does not block finishing");

    // The run completed with the failure visible rather than stalling.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(run.finalized_revision, Some(2));
    let summary = run
        .sections
        .iter()
        .find(|section| section.section_id.to_string() == SUMMARY_ID)
        .expect("summary run section");
    assert_eq!(summary.state, RunSectionState::Failed);
    assert_eq!(
        summary.error.as_deref(),
        Some("No testing data is present in the records, so no interpretation can be supported.")
    );
    assert!(summary.blocks.is_empty());

    // Assembly leaves a failed section's base-revision content exactly as it
    // was: the writer could not replace that text, so nothing pretends it did.
    let assembled = &outcome.workspace.draft.content.sections[2];
    assert_eq!(assembled.id.to_string(), SUMMARY_ID);
    assert!(!assembled.skipped);
    assert_eq!(assembled.blocks, boilerplate());
    // Whatever stamp the section already carried survives; what must not
    // happen is the run taking credit for text it could not write.
    assert!(assembled.authorship.as_ref().is_none_or(|authorship| {
        authorship.kind != AuthorshipKind::ModelGenerated && authorship.run_id.is_none()
    }));

    let results = tool_results(&server).await;
    assert!(
        results.iter().any(|result| {
            result["content"][0]["json"]["status"] == "section_marked_failed"
                && result["content"][0]["json"]["section_id"] == SUMMARY_ID
        }),
        "the failure was not acknowledged to the model"
    );
    assert!(
        results
            .iter()
            .any(|result| result["content"][0]["json"]["failed_section_count"] == 1),
        "the finish result did not report the failure"
    );
}

#[tokio::test]
async fn a_third_rejected_write_fails_the_section_not_the_turn() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    // The same impossible position, three times over. Position 40 is far past
    // the end of a candidate that has no sections yet.
    let doomed = |id: &str| write_call(id, REFERRAL_ID, 40, "Reason for Referral", "Attempt.");
    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                doomed("attempt-1"),
            ]),
            tool_round(vec![doomed("attempt-2")]),
            tool_round(vec![doomed("attempt-3")]),
            tool_round(vec![
                write_call(
                    "section-2",
                    BACKGROUND_ID,
                    0,
                    "Background",
                    "Documented developmental history.",
                ),
                skip_call("skip-1", SUMMARY_ID),
            ]),
            tool_round(vec![finish_call(
                "finish-1",
                "Finished around one section.",
            )]),
            closing_text("One section could not be written."),
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
        pipeline::FullReportRequest::new("Write the whole evaluation."),
    )
    .await
    .expect("the bounded section does not fail the turn");

    let codes = error_codes(&tool_results(&server).await);
    assert_eq!(
        codes,
        vec![
            "invalid_section_position",
            "invalid_section_position",
            "section_attempts_exhausted"
        ],
        "the third strike did not bound the section"
    );

    let run = only_run(&s3, client_id, report_id).await;
    let referral = run
        .sections
        .iter()
        .find(|section| section.section_id.to_string() == REFERRAL_ID)
        .expect("referral run section");
    assert_eq!(referral.state, RunSectionState::Failed);
    // The stored diagnostic is the structural code and message the model saw
    // — never a fragment of the draft it was trying to write.
    let stored = referral.error.as_deref().expect("diagnostic");
    assert!(stored.starts_with("invalid_section_position:"), "{stored}");
    assert!(!stored.contains("Attempt."));

    // The run still finished, and the bounded section kept its base content.
    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        boilerplate()
    );
}

#[tokio::test]
async fn a_skip_is_challenged_only_when_a_human_approved_the_plan() {
    let (server, sdk, s3) = setup().await;

    // A synthetic plan is nobody's decision, so skipping through it is not a
    // conflict — today's only path, and it must stay quiet.
    let synthetic_client = Uuid::new_v4();
    let synthetic_report = seed_templated_report(&s3, synthetic_client).await;
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
            tool_round(vec![finish_call("finish-1", "Wrote the referral.")]),
            closing_text("Done."),
        ],
    )
    .await;
    pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        synthetic_client,
        synthetic_report,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the referral only."),
    )
    .await
    .expect("a synthetic plan raises no conflict");
    assert!(
        !error_codes(&tool_results(&server).await).contains(&"plan_conflict".to_string()),
        "a synthetic plan challenged a skip"
    );
    server.state.write().await.bedrock_tool_requests.clear();

    // A plan a human edited is a different matter: contradicting it costs one
    // corrective round, and then the writer's judgement stands. The plan is
    // synthetic-but-edited on purpose — a clinician can edit the plan of an
    // un-gated run at the resume gate, and that run keeps the serial
    // conversation this rule lives in. A plan from an actual planning pass
    // drafts in parallel, where the host carries out its skips itself and the
    // writer never gets the chance to argue with one.
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let now = jiff::Timestamp::now();
    let run_id = seed_run(
        &s3,
        client_id,
        report_id,
        RunPlan {
            model_id: MODEL_ID.to_string(),
            entries: vec![
                plan_entry(REFERRAL_ID, "Reason for Referral", SectionIntent::Draft),
                plan_entry(BACKGROUND_ID, "Background", SectionIntent::Draft),
                plan_entry(
                    SUMMARY_ID,
                    "Summary and Clinical Interpretation",
                    SectionIntent::Skip,
                ),
            ],
            user_edited: true,
            synthetic: true,
            approved_at: Some(now),
            plan_warnings: Vec::new(),
            created_at: now,
        },
        vec![
            run_section(
                REFERRAL_ID,
                "Reason for Referral",
                0,
                RunSectionState::Pending,
            ),
            run_section(BACKGROUND_ID, "Background", 1, RunSectionState::Pending),
            run_section(
                SUMMARY_ID,
                "Summary and Clinical Interpretation",
                2,
                RunSectionState::Pending,
            ),
        ],
    )
    .await;

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
                // Contradicts an approved `draft` row.
                skip_call("skip-1", BACKGROUND_ID),
                // Matches its approved `skip` row, so it passes untouched.
                skip_call("skip-2", SUMMARY_ID),
            ]),
            tool_round(vec![skip_call("skip-3", BACKGROUND_ID)]),
            tool_round(vec![finish_call("finish-1", "Deferred the background.")]),
            closing_text("Done."),
        ],
    )
    .await;
    pipeline::resume_draft_run(
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
    .expect("the second skip is accepted");

    let results = tool_results(&server).await;
    assert_eq!(
        error_codes(&results),
        vec!["plan_conflict"],
        "the approved plan was not defended exactly once"
    );
    let conflict = results
        .iter()
        .find(|result| result["content"][0]["json"]["error"]["code"] == "plan_conflict")
        .expect("plan_conflict result");
    // The row the model is arguing with goes back verbatim.
    let row = &conflict["content"][0]["json"]["error"]["plan_entry"];
    assert_eq!(row["section_id"], BACKGROUND_ID);
    assert_eq!(row["heading"], "Background");
    assert_eq!(row["intent"], "draft");

    let run = only_run(&s3, client_id, report_id).await;
    let background = run
        .sections
        .iter()
        .find(|section| section.section_id.to_string() == BACKGROUND_ID)
        .expect("background run section");
    assert_eq!(background.state, RunSectionState::Skipped);
    assert_eq!(
        background.error.as_deref(),
        Some("skip diverged from the approved plan")
    );
    // The skip that agreed with its plan row is recorded without a note.
    let summary = run
        .sections
        .iter()
        .find(|section| section.section_id.to_string() == SUMMARY_ID)
        .expect("summary run section");
    assert_eq!(summary.state, RunSectionState::Skipped);
    assert!(summary.error.is_none());
}

/// The serial resume's kick-off block. A run whose plan came from a planning
/// pass resumes in parallel and gets a per-section kick-off instead, so this
/// pins itself to the serial path with a synthetic plan the clinician edited.
#[tokio::test]
async fn a_resume_kicks_off_with_durable_state_and_the_rewrite_source() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    let now = jiff::Timestamp::now();
    let mut drafted = run_section(
        REFERRAL_ID,
        "Reason for Referral",
        0,
        RunSectionState::Drafted,
    );
    drafted.blocks = vec![paragraph("Referred for attention concerns.")];
    let mut failed = run_section(
        SUMMARY_ID,
        "Summary and Clinical Interpretation",
        2,
        RunSectionState::Failed,
    );
    failed.error = Some("no testing data present".to_string());
    let run_id = seed_run(
        &s3,
        client_id,
        report_id,
        RunPlan {
            model_id: MODEL_ID.to_string(),
            entries: vec![
                plan_entry(REFERRAL_ID, "Reason for Referral", SectionIntent::Keep),
                plan_entry(BACKGROUND_ID, "Background", SectionIntent::Rewrite),
                plan_entry(
                    SUMMARY_ID,
                    "Summary and Clinical Interpretation",
                    SectionIntent::Draft,
                ),
            ],
            user_edited: true,
            synthetic: true,
            approved_at: Some(now),
            plan_warnings: Vec::new(),
            created_at: now,
        },
        vec![
            drafted,
            run_section(BACKGROUND_ID, "Background", 1, RunSectionState::Pending),
            failed,
        ],
    )
    .await;

    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-2",
                    BACKGROUND_ID,
                    1,
                    "Background",
                    "Rewritten developmental history.",
                ),
                write_call(
                    "section-3",
                    SUMMARY_ID,
                    2,
                    "Summary and Clinical Interpretation",
                    "Findings are consistent with the referral question.",
                ),
            ]),
            tool_round(vec![finish_call("finish-1", "Picked the run back up.")]),
            closing_text("The evaluation is complete."),
        ],
    )
    .await;

    pipeline::resume_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL_ID,
        pipeline::FullReportRequest::new("Rewrite the background from the template."),
    )
    .await
    .expect("resume the seeded run");

    let opening = opening_message(&server).await;
    let kickoff = opening[5]["text"].as_str().expect("kick-off text");
    assert!(kickoff.contains("This drafting run RESUMES an interrupted session."));
    assert!(kickoff.contains("Do not re-write sections marked drafted."));

    // The durable per-section state, so the writer picks up rather than
    // re-deriving what landed.
    assert!(kickoff.contains("Section state:"));
    assert!(kickoff.contains(REFERRAL_ID) && kickoff.contains("\"drafted\""));
    assert!(kickoff.contains(BACKGROUND_ID) && kickoff.contains("\"pending\""));
    assert!(kickoff.contains(SUMMARY_ID) && kickoff.contains("\"failed\""));
    assert!(kickoff.contains("\"failed_reason\": \"no testing data present\""));

    // The instruction typed at the resume, listed apart from the original.
    assert!(kickoff.contains("Updated instructions for this resume:"));
    assert!(kickoff.contains("- Rewrite the background from the template."));

    // A rewrite has to be given the text it is rewriting: the section's
    // template body travels with the kick-off, verbatim.
    assert!(kickoff.contains("<template_copy_for_rewrite>"));
    assert!(kickoff.contains("Template boilerplate."));
    // Only the rewrite row gets one — a keep and a draft do not.
    assert_eq!(kickoff.matches("<template_copy_for_rewrite>").count(), 1);
    let rewrite_copy = kickoff
        .split("<template_copy_for_rewrite>")
        .nth(1)
        .expect("rewrite copy");
    assert!(rewrite_copy.contains(BACKGROUND_ID));
}

#[tokio::test]
async fn the_opening_message_is_byte_identical_across_identical_runs() {
    let (server, sdk, s3) = setup().await;

    let first = one_run_opening(&server, &sdk, &s3).await;
    let second = one_run_opening(&server, &sdk, &s3).await;

    // Two runs over the same records and the same template produce the same
    // bytes above the checkpoints. Anything else — an unsorted map, a
    // timestamp, a listing order — costs every call after the first its cache.
    for (index, label) in [(0, "record corpus"), (1, "template context"), (2, "plan")] {
        assert_eq!(
            first[index], second[index],
            "the {label} block is not byte-stable"
        );
    }

    let corpus = first[0]["text"].as_str().expect("record corpus");
    let alpha = corpus.find("alpha-history.txt").expect("alpha listed");
    let intake = corpus.find("intake.txt").expect("intake listed");
    let zeta = corpus.find("zeta-notes.txt").expect("zeta listed");
    assert!(
        alpha < intake && intake < zeta,
        "the corpus is not in filename order"
    );
}

#[tokio::test]
async fn the_cache_tier_follows_what_the_writer_model_accepts() {
    // The extended one-hour tier where the family takes it...
    let extended = cache_points_for("us.anthropic.claude-opus-4-6-20260301-v1:0").await;
    assert!(!extended.is_empty());
    for point in &extended {
        assert_eq!(point, &serde_json::json!({"type": "default", "ttl": "1h"}));
    }

    // ...and the five-minute default where it does not. A mixed-TTL request is
    // rejected outright, so the downgrade has to reach every point.
    let default_tier = cache_points_for("us.anthropic.claude-sonnet-4-20250514-v1:0").await;
    assert_eq!(default_tier.len(), extended.len());
    for point in &default_tier {
        assert_eq!(point, &serde_json::json!({"type": "default"}));
    }
}

/// Seed a fresh client with the same records and template as every other call
/// of this helper, run one whole-report draft, and return its opening message.
/// The captured requests are cleared afterwards so two calls can be compared.
async fn one_run_opening(
    server: &MockServer,
    sdk: &aws_config::SdkConfig,
    s3: &aws_sdk_s3::Client,
) -> Vec<serde_json::Value> {
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(s3, client_id).await;
    // Deliberately created out of alphabetical order; the corpus is sorted by
    // filename regardless of the order the walk produced.
    for (filename, text) in [
        ("zeta-notes.txt", "Zeta observations from the classroom."),
        ("alpha-history.txt", "Alpha developmental history."),
    ] {
        claria_storage::objects::put_object(
            s3,
            BUCKET,
            &claria_core::s3_keys::client_record_file(client_id, filename),
            text.as_bytes().to_vec(),
            Some("text/plain"),
        )
        .await
        .expect("put record");
    }
    script(
        server,
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
            tool_round(vec![finish_call("finish-1", "Wrote the referral.")]),
            closing_text("Done."),
        ],
    )
    .await;
    pipeline::generate_full_report_for_report(
        sdk,
        s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation."),
    )
    .await
    .expect("generate");
    let opening = opening_message(server).await;
    server.state.write().await.bedrock_tool_requests.clear();
    opening
}

/// Drive one whole-report run on `model_id` and return every `cachePoint`
/// block its opening Bedrock request carried.
async fn cache_points_for(model_id: &str) -> Vec<serde_json::Value> {
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
            tool_round(vec![finish_call("finish-1", "Wrote the referral.")]),
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
        model_id,
        pipeline::FullReportRequest::new("Write the whole evaluation."),
    )
    .await
    .expect("generate");
    opening_message(&server)
        .await
        .into_iter()
        .filter_map(|block| block.get("cachePoint").cloned())
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
    // The run did not author it, so it is not credited to the run — whatever
    // stamp the section carried before the run travels with it untouched.
    assert!(deferred.authorship.as_ref().is_none_or(|authorship| {
        authorship.kind != AuthorshipKind::ModelGenerated && authorship.run_id.is_none()
    }));

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

// ---------------------------------------------------------------------------
// Progress: what the run tells the writing surface while it works
// ---------------------------------------------------------------------------

/// A drafted run section carrying staged content, which `run_section` (always
/// empty, never drafted) cannot express.
fn drafted_run_section(id: &str, heading: &str, position: u32, text: &str) -> RunSection {
    RunSection {
        blocks: vec![paragraph(text)],
        ..run_section(id, heading, position, RunSectionState::Drafted)
    }
}

/// Point the workspace at a hand-seeded run, standing in for the pointer a
/// live run writes before its first model call.
async fn point_workspace_at_run(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
) {
    let mut workspace = store::load_report_workspace_by_id(s3, BUCKET, client_id, report_id)
        .await
        .expect("load workspace");
    workspace.active_run_id = Some(run_id);
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::report_session_workspace(client_id, report_id),
        serde_json::to_vec(&workspace).expect("workspace JSON"),
        Some("application/json"),
    )
    .await
    .expect("re-put workspace pointing at the run");
}

/// The run's own progress, with the per-call and per-tool events dropped —
/// those predate runs and are asserted in `workflow.rs`.
fn section_progress(events: &[pipeline::ReportTurnProgress]) -> Vec<&pipeline::ReportTurnProgress> {
    events
        .iter()
        .filter(|event| {
            !matches!(
                event,
                pipeline::ReportTurnProgress::RecordContextPrepared { .. }
                    | pipeline::ReportTurnProgress::ModelCallStarted { .. }
                    | pipeline::ReportTurnProgress::ToolStarted { .. }
                    | pipeline::ReportTurnProgress::ToolFinished { .. }
            )
        })
        .collect()
}

#[tokio::test]
async fn every_landed_section_is_reported_with_its_content_and_counters() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            tool_round(vec![title_call("title-1", "Psychoeducational Evaluation")]),
            tool_round(vec![write_call(
                "section-1",
                REFERRAL_ID,
                0,
                "Reason for Referral",
                "Referred for attention concerns.",
            )]),
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

    let progress = std::sync::Mutex::new(Vec::new());
    let emit_progress = |event| progress.lock().expect("progress lock").push(event);
    pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write the whole evaluation.")
            .with_progress(&emit_progress),
    )
    .await
    .expect("generate the whole evaluation");

    let events = progress.into_inner().expect("progress values");
    // The denominator is announced before any model call, so a progress bar
    // never has to guess how many sections are coming.
    let first_model_call = events
        .iter()
        .position(|event| {
            matches!(
                event,
                pipeline::ReportTurnProgress::ModelCallStarted { call_number: 1 }
            )
        })
        .expect("a first model call");
    let plan_ready = events
        .iter()
        .position(|event| {
            matches!(
                event,
                pipeline::ReportTurnProgress::PlanReady { section_count: 3 }
            )
        })
        .expect("the plan's section count");
    assert!(
        plan_ready < first_model_call,
        "the plan was announced after the first model call"
    );

    let sections = section_progress(&events);
    assert_eq!(sections.len(), 8, "unexpected section-progress sequence");
    assert!(matches!(
        sections[0],
        pipeline::ReportTurnProgress::PlanReady { section_count: 3 }
    ));
    assert!(matches!(
        sections[1],
        pipeline::ReportTurnProgress::TitleSet { title }
            if title == "Psychoeducational Evaluation"
    ));
    for (index, section_id) in [REFERRAL_ID, BACKGROUND_ID, SUMMARY_ID].iter().enumerate() {
        let started = sections[2 + index * 2];
        let completed = sections[3 + index * 2];
        let slot = u32::try_from(index).expect("section index");
        assert!(
            matches!(
                started,
                pipeline::ReportTurnProgress::SectionStarted { section_id: id, index, total: 3 }
                    if id == section_id && *index == slot
            ),
            "unexpected start event at slot {slot}: {started:?}"
        );
        assert!(
            matches!(
                completed,
                pipeline::ReportTurnProgress::SectionCompleted {
                    section_id: id,
                    drafted,
                    total: 3,
                    ..
                } if id == section_id && *drafted == slot + 1
            ),
            "unexpected completion event at slot {slot}: {completed:?}"
        );
    }

    // The completion event carries the section whole, so the preview renders
    // it without asking for the workspace again.
    let pipeline::ReportTurnProgress::SectionCompleted { section, .. } = sections[3] else {
        panic!("expected the first section's completion event");
    };
    assert_eq!(section.id.to_string(), REFERRAL_ID);
    assert_eq!(section.heading, "Reason for Referral");
    assert_eq!(
        section.blocks,
        vec![paragraph("Referred for attention concerns.")]
    );
    assert!(!section.skipped);
}

#[tokio::test]
async fn a_skip_and_a_failure_report_themselves_with_the_same_counters() {
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
                fail_call("fail-1", SUMMARY_ID, "No testing data is present."),
            ]),
            tool_round(vec![finish_call("finish-1", "Drafted the referral only.")]),
            closing_text("One section was deferred and one could not be written."),
        ],
    )
    .await;

    let progress = std::sync::Mutex::new(Vec::new());
    let emit_progress = |event| progress.lock().expect("progress lock").push(event);
    pipeline::generate_full_report_for_report(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        MODEL_ID,
        pipeline::FullReportRequest::new("Write what the records support.")
            .with_progress(&emit_progress),
    )
    .await
    .expect("a skip and a failure still finish the run");

    let events = progress.into_inner().expect("progress values");
    let sections = section_progress(&events);
    assert!(
        sections.iter().any(|event| matches!(
            event,
            pipeline::ReportTurnProgress::SectionSkipped { section_id, drafted: 2, total: 3 }
                if section_id == BACKGROUND_ID
        )),
        "the skip was not reported with its counters: {sections:?}"
    );
    assert!(
        sections.iter().any(|event| matches!(
            event,
            pipeline::ReportTurnProgress::SectionFailed {
                section_id,
                message,
                drafted: 3,
                total: 3,
            } if section_id == SUMMARY_ID && message == "No testing data is present."
        )),
        "the failure was not reported with its stored reason: {sections:?}"
    );
}

// ---------------------------------------------------------------------------
// Hydration, and the two ways an interrupted run ends without a model
// ---------------------------------------------------------------------------

fn three_section_plan() -> RunPlan {
    let now = jiff::Timestamp::now();
    RunPlan {
        model_id: MODEL_ID.to_string(),
        entries: vec![
            plan_entry(REFERRAL_ID, "Reason for Referral", SectionIntent::Draft),
            plan_entry(BACKGROUND_ID, "Background", SectionIntent::Draft),
            plan_entry(
                SUMMARY_ID,
                "Summary and Clinical Interpretation",
                SectionIntent::Draft,
            ),
        ],
        plan_warnings: Vec::new(),
        user_edited: false,
        synthetic: false,
        approved_at: Some(now),
        created_at: now,
    }
}

/// One stopped run with a referral in `referral_state`, a section it never
/// reached, and a section it failed — the shape a partial finalize handles.
async fn seed_interrupted_run(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    referral_state: RunSectionState,
) -> Uuid {
    let referral = if referral_state == RunSectionState::Drafted {
        drafted_run_section(
            REFERRAL_ID,
            "Reason for Referral",
            0,
            "Referred for attention concerns.",
        )
    } else {
        run_section(REFERRAL_ID, "Reason for Referral", 0, referral_state)
    };
    seed_run(
        s3,
        client_id,
        report_id,
        three_section_plan(),
        vec![
            referral,
            run_section(BACKGROUND_ID, "Background", 1, RunSectionState::Pending),
            RunSection {
                error: Some("section_attempts_exhausted".to_string()),
                ..run_section(
                    SUMMARY_ID,
                    "Summary and Clinical Interpretation",
                    2,
                    RunSectionState::Failed,
                )
            },
        ],
    )
    .await
}

#[tokio::test]
async fn hydration_offers_only_a_run_the_report_still_fits() {
    let (_server, _sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    assert!(
        pipeline::load_resumable_draft_run(&s3, BUCKET, client_id, report_id)
            .await
            .expect("hydration read")
            .is_none(),
        "a report with no runs offered one"
    );

    let run_id = seed_interrupted_run(&s3, client_id, report_id, RunSectionState::Drafted).await;
    let hydrated = pipeline::load_resumable_draft_run(&s3, BUCKET, client_id, report_id)
        .await
        .expect("hydration read")
        .expect("the stopped run is resumable");
    assert_eq!(hydrated.run_id, run_id);

    // A hand edit moves the report past the revision the run built on, and a
    // run whose sections no longer fit must not be offered.
    store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        ReportContent {
            title: "Evaluation Template".to_string(),
            sections: vec![template_section(REFERRAL_ID, "Reason for Referral")],
        },
    )
    .await
    .expect("hand edit");
    assert!(
        pipeline::load_resumable_draft_run(&s3, BUCKET, client_id, report_id)
            .await
            .expect("hydration read")
            .is_none(),
        "a run built on an older revision was still offered"
    );
}

#[tokio::test]
async fn finalizing_a_partial_draft_keeps_what_landed_and_defers_the_rest() {
    let (_server, _sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id, RunSectionState::Drafted).await;
    point_workspace_at_run(&s3, client_id, report_id, run_id).await;

    let outcome = pipeline::finalize_partial_draft(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("finalize what the interrupted run wrote");

    assert_eq!(outcome.revision, 2);
    assert_eq!(outcome.drafted_sections, 1);
    assert_eq!(outcome.skipped_sections, 2);
    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.active_run_id, None);
    assert_eq!(
        outcome.workspace.draft.content.title,
        "Psychoeducational Evaluation"
    );
    assert_eq!(
        body(&outcome.workspace.draft.content),
        vec![
            (
                "Reason for Referral".to_string(),
                vec![paragraph("Referred for attention concerns.")],
                false,
            ),
            ("Background".to_string(), Vec::new(), true),
            (
                "Summary and Clinical Interpretation".to_string(),
                Vec::new(),
                true,
            ),
        ]
    );
    // The deferred sections keep the template copy the greyed-out preview
    // renders, exactly as a deliberate skip does.
    for section in &outcome.workspace.draft.content.sections[1..] {
        assert_eq!(section.template_blocks.as_ref(), Some(&boilerplate()));
    }
    // Only the section the run actually authored is credited to it.
    assert_eq!(
        outcome.workspace.draft.content.sections[0]
            .authorship
            .as_ref()
            .map(|authorship| authorship.kind),
        Some(AuthorshipKind::ModelGenerated)
    );
    // Whatever stamp a deferred section carried before the run travels with
    // it untouched; it just must not be credited to the run.
    assert!(
        outcome.workspace.draft.content.sections[1]
            .authorship
            .as_ref()
            .is_none_or(|authorship| {
                authorship.kind != AuthorshipKind::ModelGenerated && authorship.run_id.is_none()
            })
    );

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert!(run.partial);
    assert_eq!(run.finalized_revision, Some(2));
}

#[tokio::test]
async fn a_partial_finalize_needs_a_finished_section_and_a_matching_revision() {
    let (_server, _sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    // Nothing landed, so there is no revision worth cutting.
    let empty_run = seed_interrupted_run(&s3, client_id, report_id, RunSectionState::Pending).await;
    let error = pipeline::finalize_partial_draft(&s3, BUCKET, client_id, report_id, empty_run)
        .await
        .expect_err("a run with no drafted sections cannot be finalized");
    assert!(
        error.to_string().contains("no finished sections"),
        "unexpected refusal: {error}"
    );

    // The report moved on underneath the run, so its sections no longer fit.
    store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        ReportContent {
            title: "Evaluation Template".to_string(),
            sections: vec![template_section(REFERRAL_ID, "Reason for Referral")],
        },
    )
    .await
    .expect("hand edit");
    let error = pipeline::finalize_partial_draft(&s3, BUCKET, client_id, report_id, empty_run)
        .await
        .expect_err("a run built on an older revision cannot be finalized");
    assert!(
        error.to_string().contains("has changed since"),
        "unexpected refusal: {error}"
    );

    let reloaded = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(
        reloaded.draft.revision, 2,
        "a refused finalize cut a revision"
    );
}

#[tokio::test]
async fn abandoning_a_run_releases_the_session_and_leaves_the_report_alone() {
    let (_server, _sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id, RunSectionState::Drafted).await;
    point_workspace_at_run(&s3, client_id, report_id, run_id).await;

    let workspace = pipeline::abandon_draft_run(&s3, BUCKET, client_id, report_id, run_id)
        .await
        .expect("abandon the run");

    assert_eq!(workspace.active_run_id, None);
    assert_eq!(workspace.draft.revision, 1);
    assert_eq!(
        body(&workspace.draft.content),
        body(&ReportContent {
            title: "Evaluation Template".to_string(),
            sections: vec![
                template_section(REFERRAL_ID, "Reason for Referral"),
                template_section(BACKGROUND_ID, "Background"),
                template_section(SUMMARY_ID, "Summary and Clinical Interpretation"),
            ],
        })
    );

    // The drafted section stays in the run object as history.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Abandoned);
    assert_eq!(
        drafted(&run),
        vec![(
            "Reason for Referral",
            &vec![paragraph("Referred for attention concerns.")]
        )]
    );

    assert!(
        pipeline::load_resumable_draft_run(&s3, BUCKET, client_id, report_id)
            .await
            .expect("hydration read")
            .is_none(),
        "an abandoned run was still offered for hydration"
    );
}
