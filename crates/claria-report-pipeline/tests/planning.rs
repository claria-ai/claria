//! The planning pass: what the host accepts from a planner, what it refuses,
//! what it warns about, and what a drafting run does with the plan it lands.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_bedrock::converse::StopSignal;
use claria_core::models::{
    report::{ReportBlock, ReportContent, ReportSection},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, PlanEntry, PlanEntryEdit,
        RunInstruction, RunPlan, RunSection, RunSectionState, SectionIntent,
    },
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_pipeline as pipeline;
use claria_report_store as store;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-planning-test";
const PLANNER_MODEL_ID: &str = "us.anthropic.claude-sonnet-test";
const WRITER_MODEL_ID: &str = "us.anthropic.claude-opus-test";

const REFERRAL_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const BACKGROUND_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SUMMARY_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

/// The one record the seeded client has. Filenames, not spans, are what a
/// plan cites, so this only has to be readable text the corpus can list.
const INTAKE_TEXT: &str =
    "Referred by pediatrician for attention concerns.\nParent reports homework avoidance.";
/// The name that record is listed under in the corpus.
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
        authorship: None,
    }
}

/// A client with one readable record and `sections` applied as revision 1.
async fn seed_report(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    sections: Vec<ReportSection>,
) -> Uuid {
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
    store::save_report_draft_for_report(
        s3,
        BUCKET,
        client_id,
        workspace.report_id,
        0,
        ReportContent {
            title: "Evaluation Template".to_string(),
            sections,
        },
    )
    .await
    .expect("seed template-shaped draft");
    workspace.report_id
}

/// A three-section template — the shape most plans in this file are built
/// against, and one that fits a single planning batch.
async fn seed_templated_report(s3: &aws_sdk_s3::Client, client_id: Uuid) -> Uuid {
    seed_report(
        s3,
        client_id,
        vec![
            template_section(REFERRAL_ID, "Reason for Referral"),
            template_section(BACKGROUND_ID, "Background"),
            template_section(SUMMARY_ID, "Summary and Clinical Interpretation"),
        ],
    )
    .await
}

/// Sections in the ten-section template the batching tests plan, which
/// `PLAN_BATCH_SECTIONS` splits into a full batch of eight and a remainder of
/// two — the interesting shape, because the second call has to be shorter
/// than the first and still be a complete answer.
const WIDE_SECTIONS: usize = 10;

/// The `n`th section ID of that template. Written out rather than random so a
/// failing assertion names a section the reader can find in the scripts.
fn wide_id(index: usize) -> String {
    format!("{index:08x}-0000-4000-8000-000000000000")
}

fn wide_ids() -> Vec<String> {
    (0..WIDE_SECTIONS).map(wide_id).collect()
}

/// A ten-section template, so one plan spans two batches.
async fn seed_wide_report(s3: &aws_sdk_s3::Client, client_id: Uuid) -> Uuid {
    let sections = wide_ids()
        .iter()
        .enumerate()
        .map(|(index, id)| template_section(id, &format!("Section {index}")))
        .collect();
    seed_report(s3, client_id, sections).await
}

/// The scripted answer for one batch: a drafted row for every ID in it.
fn wide_batch_plan(ids: &[String]) -> ScriptedBedrockResponse {
    section_plan(
        ids.iter()
            .map(|id| {
                plan_row(
                    id,
                    "draft",
                    "State what this section covers.",
                    Some(INTAKE_FILE),
                )
            })
            .collect(),
    )
}

/// The two batches the ten-section template is planned in.
fn wide_batches() -> (Vec<String>, Vec<String>) {
    let ids = wide_ids();
    let (first, second) = ids.split_at(pipeline::PLAN_BATCH_SECTIONS);
    (first.to_vec(), second.to_vec())
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

/// One forced-tool answer, in the wire shape Bedrock returns.
fn forced_tool_call(
    tool_use_id: &str,
    name: &str,
    input: serde_json::Value,
) -> ScriptedBedrockResponse {
    ok(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": [
            {"toolUse": {"toolUseId": tool_use_id, "name": name, "input": input}}
        ]}},
        "stopReason": "tool_use",
        "usage": {"inputTokens": 900, "outputTokens": 120}
    }))
}

fn plan_row(
    section_id: &str,
    action: &str,
    scope: &str,
    evidence_file: Option<&str>,
) -> serde_json::Value {
    let evidence: Vec<serde_json::Value> = evidence_file
        .into_iter()
        .map(|filename| {
            serde_json::json!({
                "filename": filename,
                "relevance": "Names the referral question."
            })
        })
        .collect();
    serde_json::json!({
        "section_id": section_id,
        "action": action,
        "scope": scope,
        "evidence": evidence
    })
}

fn section_plan(rows: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    forced_tool_call(
        "plan-1",
        "submit_section_plan",
        serde_json::json!({"rows": rows}),
    )
}

fn resume_row(
    section_id: &str,
    decision: &str,
    reason: &str,
    scope: Option<&str>,
    evidence_file: Option<&str>,
) -> serde_json::Value {
    let mut row = serde_json::json!({
        "section_id": section_id,
        "decision": decision,
        "reason": reason
    });
    if let Some(scope) = scope {
        row["scope"] = serde_json::Value::String(scope.to_string());
    }
    if let Some(filename) = evidence_file {
        row["evidence"] = serde_json::json!([{"filename": filename}]);
    }
    row
}

fn resume_plan(rows: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    forced_tool_call(
        "resume-1",
        "submit_resume_plan",
        serde_json::json!({"rows": rows}),
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

fn tool_round(calls: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    ok(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": calls}},
        "stopReason": "tool_use",
        "usage": {"inputTokens": 5, "outputTokens": 20}
    }))
}

/// The kick-off phrase only the branch assigned `section_id` carries. Never a
/// bare UUID: the plan block lists every section's ID in every branch's
/// request, so a bare UUID matches all of them at once.
fn section_marker(section_id: &str) -> String {
    format!("Assigned section: {section_id}")
}

/// The title branch's own phrase.
const TITLE_MARKER: &str = "Assigned task: the report title.";

/// Answer one drafting branch with the write it was assigned.
async fn script_branch(server: &MockServer, section_id: &str, heading: &str, text: &str) {
    server
        .state
        .write()
        .await
        .bedrock_tool_responses_by_marker
        .entry(section_marker(section_id))
        .or_default()
        .push(tool_round(vec![write_call(
            "branch-write",
            section_id,
            0,
            heading,
            text,
        )]));
}

/// Answer the title branch.
async fn script_title(server: &MockServer, title: &str) {
    server
        .state
        .write()
        .await
        .bedrock_tool_responses_by_marker
        .entry(TITLE_MARKER.to_string())
        .or_default()
        .push(tool_round(vec![title_call("branch-title", title)]));
}

fn models() -> pipeline::PlanModels<'static> {
    pipeline::PlanModels {
        planner_model_id: PLANNER_MODEL_ID,
        writer_model_id: WRITER_MODEL_ID,
    }
}

/// The captured Bedrock requests, in wire order.
async fn requests(server: &MockServer) -> Vec<serde_json::Value> {
    server.state.read().await.bedrock_tool_requests.clone()
}

fn entry<'a>(plan: &'a RunPlan, section_id: &str) -> &'a PlanEntry {
    plan.entries
        .iter()
        .find(|entry| entry.section_id.to_string() == section_id)
        .expect("plan entry")
}

async fn only_run(s3: &aws_sdk_s3::Client, client_id: Uuid, report_id: Uuid) -> DraftRun {
    let mut runs = store::list_draft_runs(s3, BUCKET, client_id, report_id)
        .await
        .expect("list drafting runs");
    assert_eq!(runs.len(), 1, "expected exactly one drafting run");
    runs.remove(0)
}

#[tokio::test]
async fn a_plan_lands_at_the_gate_with_scope_and_evidence() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "State the referral question and who raised it.",
                Some(INTAKE_FILE),
            ),
            plan_row(
                BACKGROUND_ID,
                "draft",
                "Summarize developmental and school history.",
                Some(INTAKE_FILE),
            ),
            plan_row(
                SUMMARY_ID,
                "skip",
                "No testing data is present, so nothing can be interpreted yet.",
                None,
            ),
        ])],
    )
    .await;

    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("Lead with the referral question."),
    )
    .await
    .expect("the plan lands");

    assert_eq!(outcome.run.status, DraftRunStatus::AwaitingApproval);
    assert_eq!(outcome.converse_calls, 1);
    let plan = outcome.run.plan.as_ref().expect("plan");
    assert_eq!(plan.model_id, PLANNER_MODEL_ID);
    assert!(!plan.user_edited);
    assert_eq!(plan.approved_at, None, "the gate stamps the approval");
    assert!(
        plan.plan_warnings.is_empty(),
        "unexpected warnings: {:?}",
        plan.plan_warnings
    );

    // One row per template section, in template order.
    assert_eq!(
        plan.entries
            .iter()
            .map(|entry| entry.section_id.to_string())
            .collect::<Vec<_>>(),
        vec![REFERRAL_ID, BACKGROUND_ID, SUMMARY_ID]
    );
    let referral = entry(plan, REFERRAL_ID);
    assert_eq!(referral.intent, SectionIntent::Draft);
    assert!(referral.required);
    assert_eq!(
        referral.scope,
        "State the referral question and who raised it."
    );
    assert_eq!(referral.evidence.len(), 1);
    assert_eq!(referral.evidence[0].filename, "intake.txt");
    assert_eq!(
        referral.evidence[0].note.as_deref(),
        Some("Names the referral question.")
    );
    assert_eq!(entry(plan, SUMMARY_ID).intent, SectionIntent::Skip);

    // The gate window: the report is held by the run while the plan waits.
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, Some(outcome.run.run_id));
    assert_eq!(workspace.draft.revision, 1);
}

/// The planning call is one request that can run for minutes, so the rows it
/// has written are counted off the answer while it is still open.
#[tokio::test]
async fn planning_counts_its_rows_off_while_the_answer_streams() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "State the referral question and who raised it.",
                Some(INTAKE_FILE),
            ),
            plan_row(
                BACKGROUND_ID,
                "draft",
                "Summarize developmental and school history.",
                Some(INTAKE_FILE),
            ),
            plan_row(SUMMARY_ID, "skip", "Nothing to interpret yet.", None),
        ])],
    )
    .await;

    let progress = std::sync::Mutex::new(Vec::new());
    let emit_progress = |event| progress.lock().expect("progress lock").push(event);
    pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("Lead with the referral question.")
            .with_progress(&emit_progress),
    )
    .await
    .expect("the plan lands");

    let events = progress.into_inner().expect("progress values");
    let counted: Vec<u32> = events
        .iter()
        .filter_map(|event| match event {
            pipeline::ReportTurnProgress::PlanRowPlanned { planned, total } => {
                assert_eq!(*total, 3, "the denominator is the document's sections");
                Some(*planned)
            }
            _ => None,
        })
        .collect();
    // Every row is counted, once, in order, and the count arrives before the
    // call it is describing has returned.
    assert_eq!(counted, vec![1, 2, 3]);
    let first_count = events
        .iter()
        .position(|event| matches!(event, pipeline::ReportTurnProgress::PlanRowPlanned { .. }))
        .expect("a row count");
    let call_started = events
        .iter()
        .position(|event| {
            matches!(
                event,
                pipeline::ReportTurnProgress::ModelCallStarted { call_number: 1 }
            )
        })
        .expect("the planning call");
    assert!(call_started < first_count);
}

#[tokio::test]
async fn the_planner_request_caches_the_corpus_above_the_question() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    script(
        &server,
        vec![section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "Referral question.",
                Some(INTAKE_FILE),
            ),
            plan_row(BACKGROUND_ID, "skip", "Deferred.", None),
            plan_row(SUMMARY_ID, "skip", "Deferred.", None),
        ])],
    )
    .await;
    pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect("the plan lands");

    let request = requests(&server).await.remove(0);

    // The expensive, role-independent part rides in the system tier: policy,
    // corpus, template, then the cache point that makes the next analysis
    // call on this model read all three instead of re-paying for them.
    let system = request["system"].as_array().expect("system blocks");
    assert_eq!(system.len(), 4, "unexpected system layout: {system:?}");
    assert!(
        system[0]["text"]
            .as_str()
            .expect("policy")
            .contains("You are planning a clinical report")
    );
    assert!(
        system[1]["text"]
            .as_str()
            .expect("corpus")
            .starts_with("<untrusted_record_context>")
    );
    assert!(
        system[2]["text"]
            .as_str()
            .expect("template")
            .starts_with("<untrusted_template_context>")
    );
    assert_eq!(
        system[3]["cachePoint"],
        serde_json::json!({"type": "default", "ttl": "1h"}),
        "the cache point is not after the last system block"
    );

    // Structure comes out of a forced tool, never out of prose.
    assert_eq!(
        request["toolConfig"]["toolChoice"]["tool"]["name"],
        "submit_section_plan"
    );
    // The shared analysis tool set: the tools tier stays identical across
    // roles so only the forced choice differs.
    let tools: Vec<&str> = request["toolConfig"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["toolSpec"]["name"].as_str())
        .collect();
    assert_eq!(
        tools,
        vec![
            "submit_section_plan",
            "submit_resume_plan",
            "submit_review_rows"
        ]
    );

    // Only the question is below the cache point.
    let messages = request["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 1);
    let question = messages[0]["content"][0]["text"]
        .as_str()
        .expect("planning instruction");
    assert!(question.contains("exactly one row per section"));
    assert!(question.contains("No additional user guidance was supplied."));
    assert!(
        !messages[0]["content"]
            .as_array()
            .expect("content")
            .iter()
            .any(|block| block.get("cachePoint").is_some()),
        "the messages tier must not carry a cache point"
    );
}

#[tokio::test]
async fn an_evidence_file_outside_the_corpus_is_warned_about() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    let invented = plan_row(
        REFERRAL_ID,
        "draft",
        "Referral question.",
        Some("chart-review.pdf"),
    );
    script(
        &server,
        vec![section_plan(vec![
            invented,
            plan_row(BACKGROUND_ID, "skip", "Deferred.", None),
            plan_row(SUMMARY_ID, "skip", "Deferred.", None),
        ])],
    )
    .await;

    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect("a plan with weak evidence still lands");
    let plan = outcome.run.plan.as_ref().expect("plan");
    assert!(
        plan.plan_warnings
            .contains(&"unknown_evidence_file:chart-review.pdf".to_string()),
        "warnings: {:?}",
        plan.plan_warnings
    );
    // A draft row left with nothing backing it is flagged for the clinician.
    assert!(
        plan.plan_warnings
            .contains(&format!("no_resolved_evidence:{REFERRAL_ID}")),
        "warnings: {:?}",
        plan.plan_warnings
    );
    // The invented file is dropped rather than handed to the writer as if the
    // client had such a record.
    assert!(entry(plan, REFERRAL_ID).evidence.is_empty());
    // Soft, not fatal: the plan still reaches the gate.
    assert_eq!(outcome.run.status, DraftRunStatus::AwaitingApproval);
}

#[tokio::test]
async fn a_plan_that_misses_a_section_is_repaired_in_one_round() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            // Two rows for three sections.
            section_plan(vec![
                plan_row(
                    REFERRAL_ID,
                    "draft",
                    "Referral question.",
                    Some(INTAKE_FILE),
                ),
                plan_row(BACKGROUND_ID, "draft", "History.", Some(INTAKE_FILE)),
            ]),
            section_plan(vec![
                plan_row(
                    REFERRAL_ID,
                    "draft",
                    "Referral question.",
                    Some(INTAKE_FILE),
                ),
                plan_row(BACKGROUND_ID, "draft", "History.", Some(INTAKE_FILE)),
                plan_row(SUMMARY_ID, "skip", "Nothing to interpret yet.", None),
            ]),
        ],
    )
    .await;

    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect("the repair round succeeds");
    assert_eq!(outcome.converse_calls, 2);
    assert_eq!(outcome.run.plan.expect("plan").entries.len(), 3);

    // The repair round names the missing section rather than saying the plan
    // was wrong, and the whole exchange stays in one conversation.
    let repair = requests(&server).await.remove(1);
    let messages = repair["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 3, "the repair is a tool_result round");
    let result = &messages[2]["content"][0]["toolResult"];
    assert_eq!(result["status"], "error");
    let message = result["content"][0]["json"]["error"]["message"]
        .as_str()
        .expect("diagnostic");
    assert!(message.contains(SUMMARY_ID), "diagnostic: {message}");
    assert!(
        message.contains("have no row"),
        "the diagnostic is not specific: {message}"
    );
}

#[tokio::test]
async fn a_plan_that_fails_twice_fails_the_pass_and_releases_the_report() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    // An invented section ID, twice over.
    let invented = || {
        section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "Referral question.",
                Some(INTAKE_FILE),
            ),
            plan_row(BACKGROUND_ID, "draft", "History.", Some(INTAKE_FILE)),
            plan_row(
                "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
                "draft",
                "A section nobody asked for.",
                None,
            ),
        ])
    };
    script(&server, vec![invented(), invented()]).await;

    let error = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect_err("an invented section is not a plan");
    assert!(
        error
            .to_string()
            .contains("could not produce a usable plan"),
        "unexpected failure: {error}"
    );

    // The report is usable again, and the run is salvage rather than a lock.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Failed);
    assert!(run.plan.is_none());
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, None);
    assert_eq!(workspace.draft.revision, 1);
}

/// A severed planner stream is a transport failure, not a bad plan: the
/// identical request goes out again and the pass completes. Before this the
/// planner was the one Bedrock call in the pipeline with no retry at all, so
/// a dropped connection failed a run that had already paid to read the
/// corpus.
#[tokio::test]
async fn planning_retries_a_dropped_stream_and_completes() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    server.state.write().await.bedrock_stream_drops = 1;

    let plan = || {
        section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "State the referral question.",
                Some(INTAKE_FILE),
            ),
            plan_row(BACKGROUND_ID, "draft", "Summarize history.", None),
            plan_row(SUMMARY_ID, "skip", "No testing data is present yet.", None),
        ])
    };
    // The severed attempt consumes its scripted payload; the retry gets the
    // same plan back.
    script(&server, vec![plan(), plan()]).await;

    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("Lead with the referral question."),
    )
    .await
    .expect("the retried plan lands");

    assert_eq!(outcome.run.status, DraftRunStatus::AwaitingApproval);
    // A retry is transport recovery, not a planner round.
    assert_eq!(outcome.converse_calls, 1);
    assert_eq!(outcome.run.plan.as_ref().expect("plan").entries.len(), 3);

    let state = server.state.read().await;
    assert_eq!(state.bedrock_stream_drops, 0, "the drop was consumed");
    assert_eq!(
        state.bedrock_stream_request_count, 2,
        "the severed attempt plus the retry"
    );
    // The retry re-sent the same conversation the severed attempt carried.
    assert_eq!(
        state.bedrock_tool_requests[0]["messages"],
        state.bedrock_tool_requests[1]["messages"]
    );
    // Count once: the verified count belongs to the request, so a re-sent
    // request does not pay for a second CountTokens.
    assert_eq!(
        state.bedrock_count_token_requests.len(),
        1,
        "the retry re-counted the request it had already counted"
    );
}

/// Attempts the AWS SDK spends on one request before the error it has been
/// swallowing reaches the pipeline. A severed body fails while the operation
/// is still resolving, so the SDK's standard dispatch policy — not the
/// pipeline's wrapper — sees it first.
const SDK_DISPATCH_ATTEMPTS: u32 = 3;

/// A retry is invisible from the outside — the identical request goes out
/// under the same call number — so the pass says it is happening. The reader
/// gets the attempt about to be made, and the re-sent call announces itself
/// again, which is what lets the surface retire the line.
#[tokio::test]
async fn a_retried_planner_call_reports_the_attempt_it_is_about_to_make() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    // Enough drops to spend the AWS SDK's own dispatch retries, so the
    // failure reaches the pipeline's wrapper instead of being absorbed
    // underneath it. Each severed request consumes a scripted payload.
    server.state.write().await.bedrock_stream_drops = SDK_DISPATCH_ATTEMPTS;

    let plan = || {
        section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "State the referral question.",
                Some(INTAKE_FILE),
            ),
            plan_row(BACKGROUND_ID, "draft", "Summarize history.", None),
            plan_row(SUMMARY_ID, "skip", "No testing data is present yet.", None),
        ])
    };
    script(
        &server,
        (0..=SDK_DISPATCH_ATTEMPTS).map(|_| plan()).collect(),
    )
    .await;

    let progress = std::sync::Mutex::new(Vec::new());
    let emit_progress = |event| progress.lock().expect("progress lock").push(event);
    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("Lead with the referral question.")
            .with_progress(&emit_progress),
    )
    .await
    .expect("the retried plan lands");

    assert_eq!(outcome.run.status, DraftRunStatus::AwaitingApproval);
    let events = progress.into_inner().expect("progress values");
    let retry = events
        .iter()
        .position(|event| {
            matches!(
                event,
                pipeline::ReportTurnProgress::ModelCallRetrying {
                    call_number: 1,
                    attempt: 2,
                    ..
                }
            )
        })
        .expect("the severed attempt is reported as a retry");
    assert!(
        matches!(
            events[retry],
            pipeline::ReportTurnProgress::ModelCallRetrying {
                max_attempts,
                delay_ms,
                ..
            } if max_attempts == claria_bedrock::retry::MAX_ATTEMPTS && delay_ms > 0
        ),
        "the retry does not say what it is counting towards or how long it waits"
    );
    // The re-sent request announces itself, so the reader's retry line has
    // something to retire it.
    assert!(
        events[retry + 1..].iter().any(|event| matches!(
            event,
            pipeline::ReportTurnProgress::ModelCallStarted { call_number: 1 }
        )),
        "the re-sent call never said it started"
    );
    // One retry, not one per repair round: the plan validated first time.
    assert_eq!(outcome.converse_calls, 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                pipeline::ReportTurnProgress::ModelCallRetrying { .. }
            ))
            .count(),
        1
    );
}

/// When every attempt is severed the pass fails on the transport, and says
/// so: a planner that answered nothing is not a planner that wrote a bad
/// plan, and the two failures have different fixes.
#[tokio::test]
async fn planning_exhausts_retries_with_an_actionable_error() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    // Over-provisioned on purpose: the AWS SDK's own dispatch-retry layer
    // can add server requests beyond the retry wrapper's attempts, and each
    // one consumes a drop and a scripted payload.
    server.state.write().await.bedrock_stream_drops = 12;

    let plan = section_plan(vec![plan_row(
        REFERRAL_ID,
        "draft",
        "Never delivered.",
        None,
    )]);
    script(&server, vec![plan; 12]).await;

    let error = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect_err("a planner that never answers cannot produce a plan");

    let message = error.to_string();
    assert!(
        message.contains("got no response"),
        "the failure does not name the transport as the cause: {message}"
    );
    assert!(
        message.contains("Try again later"),
        "the failure does not say what to do next: {message}"
    );
    assert!(
        !message.contains("usable plan"),
        "a severed connection was reported as a bad plan: {message}"
    );

    // Every attempt really went out, and the report is usable again.
    let state = server.state.read().await;
    assert!(
        state.bedrock_stream_request_count >= 4,
        "the planner gave up before exhausting its attempts: {}",
        state.bedrock_stream_request_count
    );
    drop(state);
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Failed);
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, None);
}

// ── Batched planning ────────────────────────────────────────────────────────

/// A document wider than one batch is planned in several calls, each a
/// complete answer about its own sections. The expensive half of the request
/// is identical across them, so the second call reads a cached prefix the
/// first paid to write — and pays for no second `CountTokens`.
#[tokio::test]
async fn a_wide_plan_is_batched_over_calls_that_share_one_cached_request() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_wide_report(&s3, client_id).await;
    let (first_batch, second_batch) = wide_batches();
    script(
        &server,
        vec![
            wide_batch_plan(&first_batch),
            wide_batch_plan(&second_batch),
        ],
    )
    .await;

    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("Lead with the referral question."),
    )
    .await
    .expect("the batched plan lands");

    assert_eq!(outcome.run.status, DraftRunStatus::AwaitingApproval);
    assert_eq!(outcome.converse_calls, 2, "one call per batch");
    let plan = outcome.run.plan.as_ref().expect("plan");
    // Every section, once, in template order — the batches concatenate.
    assert_eq!(
        plan.entries
            .iter()
            .map(|entry| entry.section_id.to_string())
            .collect::<Vec<_>>(),
        wide_ids()
    );

    let requests = requests(&server).await;
    assert_eq!(requests.len(), 2);
    // The whole expensive tier is byte-identical, which is what makes the
    // cache point after it worth having.
    assert_eq!(
        requests[0]["system"], requests[1]["system"],
        "the batches do not share a cached system prefix"
    );
    assert_eq!(requests[0]["toolConfig"], requests[1]["toolConfig"]);
    // A batch carries its own question and nothing else: prior batches'
    // transcripts are not re-sent, because the messages tier is not cached
    // and the earlier rows are already decided.
    assert_eq!(
        requests[1]["messages"].as_array().expect("messages").len(),
        1
    );
    let second_question = requests[1]["messages"][0]["content"][0]["text"]
        .as_str()
        .expect("the second batch's question");
    for id in &second_batch {
        assert!(
            second_question.contains(id.as_str()),
            "the second batch was not told to plan {id}"
        );
    }
    for id in &first_batch {
        assert!(
            !second_question.contains(id.as_str()),
            "the second batch was told to plan {id} again"
        );
    }
    assert!(second_question.contains("batch 2 of 2"));

    // One count for the pass: every batch sends the same request shape, so a
    // per-batch count would be paying repeatedly for one number.
    assert_eq!(
        server.state.read().await.bedrock_count_token_requests.len(),
        1,
        "a batch re-counted a request shape the pass had already counted"
    );
}

/// Each batch owns its repair round. A batch that comes back short is
/// corrected in its own conversation, and the batches that already validated
/// are not re-asked.
#[tokio::test]
async fn a_short_batch_is_repaired_without_re_planning_the_batches_that_landed() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_wide_report(&s3, client_id).await;
    let (first_batch, second_batch) = wide_batches();
    script(
        &server,
        vec![
            wide_batch_plan(&first_batch),
            // One row for the two the second batch was given.
            wide_batch_plan(&second_batch[..1]),
            wide_batch_plan(&second_batch),
        ],
    )
    .await;

    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect("the repaired batch completes the plan");
    assert_eq!(outcome.converse_calls, 3, "two batches plus one repair");
    assert_eq!(outcome.run.plan.expect("plan").entries.len(), WIDE_SECTIONS);

    let repair = requests(&server).await.remove(2);
    let messages = repair["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 3, "the repair is a tool_result round");
    // The repair conversation is the failing batch's, and only its.
    let question = messages[0]["content"][0]["text"]
        .as_str()
        .expect("the repaired batch's question");
    assert!(question.contains(second_batch[1].as_str()));
    assert!(!question.contains(first_batch[0].as_str()));
    let diagnostic =
        messages[2]["content"][0]["toolResult"]["content"][0]["json"]["error"]["message"]
            .as_str()
            .expect("diagnostic");
    assert!(
        diagnostic.contains(second_batch[1].as_str()),
        "the diagnostic does not name the missing row: {diagnostic}"
    );
    assert!(
        !diagnostic.contains(first_batch[0].as_str()),
        "the diagnostic blamed the batch for a section it was never given: {diagnostic}"
    );
}

/// A severed batch is transport recovery, not a planning failure: the
/// identical request goes out again, the batches around it are untouched, and
/// the pass still counts its tokens once.
#[tokio::test]
async fn a_severed_batch_is_re_sent_and_the_others_are_not() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_wide_report(&s3, client_id).await;
    // Enough drops to spend the AWS SDK's own dispatch retries, so the
    // failure reaches the pipeline's wrapper instead of being absorbed
    // underneath it. Each severed request consumes a scripted payload.
    server.state.write().await.bedrock_stream_drops = SDK_DISPATCH_ATTEMPTS;
    let (first_batch, second_batch) = wide_batches();
    let mut scripted: Vec<ScriptedBedrockResponse> = (0..=SDK_DISPATCH_ATTEMPTS)
        .map(|_| wide_batch_plan(&first_batch))
        .collect();
    scripted.push(wide_batch_plan(&second_batch));
    script(&server, scripted).await;

    let progress = std::sync::Mutex::new(Vec::new());
    let emit_progress = |event| progress.lock().expect("progress lock").push(event);
    let outcome = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("").with_progress(&emit_progress),
    )
    .await
    .expect("the retried batch lands");

    assert_eq!(outcome.converse_calls, 2, "a retry is not a planning round");
    assert_eq!(outcome.run.plan.expect("plan").entries.len(), WIDE_SECTIONS);

    let requests = requests(&server).await;
    let attempts = SDK_DISPATCH_ATTEMPTS as usize + 1;
    assert_eq!(requests.len(), attempts + 1);
    // Every severed attempt re-sent the first batch's question verbatim.
    for attempt in 1..attempts {
        assert_eq!(
            requests[0]["messages"], requests[attempt]["messages"],
            "the re-sent batch changed its question"
        );
    }
    // The batch that never failed went out exactly once.
    let last = requests[attempts]["messages"][0]["content"][0]["text"]
        .as_str()
        .expect("the second batch's question");
    assert!(last.contains(second_batch[0].as_str()));
    assert!(!last.contains(first_batch[0].as_str()));

    let state = server.state.read().await;
    assert_eq!(state.bedrock_stream_drops, 0, "the drops were consumed");
    assert_eq!(
        state.bedrock_count_token_requests.len(),
        1,
        "the retried batch re-counted a request the pass had already counted"
    );
    drop(state);

    let events = progress.into_inner().expect("progress values");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                pipeline::ReportTurnProgress::ModelCallRetrying { call_number: 1, .. }
            ))
            .count(),
        1,
        "the severed batch is reported as one retry of its own call"
    );
}

/// The pass reports a checkpoint per batch and a row count that never walks
/// backwards, even though each batch's stream starts counting its own rows
/// from one.
#[tokio::test]
async fn batched_planning_reports_checkpoints_and_never_counts_backwards() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_wide_report(&s3, client_id).await;
    let (first_batch, second_batch) = wide_batches();
    script(
        &server,
        vec![
            wide_batch_plan(&first_batch),
            wide_batch_plan(&second_batch),
        ],
    )
    .await;

    let progress = std::sync::Mutex::new(Vec::new());
    let emit_progress = |event| progress.lock().expect("progress lock").push(event);
    pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("").with_progress(&emit_progress),
    )
    .await
    .expect("the batched plan lands");

    let events = progress.into_inner().expect("progress values");
    let batches: Vec<(u32, u32, u32)> = events
        .iter()
        .filter_map(|event| match event {
            pipeline::ReportTurnProgress::PlanBatchPlanned { first, last, total } => {
                Some((*first, *last, *total))
            }
            _ => None,
        })
        .collect();
    // One-based and inclusive, against the whole document.
    assert_eq!(batches, vec![(1, 8, 10), (9, 10, 10)]);

    let counted: Vec<u32> = events
        .iter()
        .filter_map(|event| match event {
            pipeline::ReportTurnProgress::PlanRowPlanned { planned, total } => {
                assert_eq!(*total, 10, "the denominator is the whole document");
                Some(*planned)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        counted,
        (1..=10).collect::<Vec<u32>>(),
        "the row count restarted at the batch boundary"
    );
}

/// Between batches is a stop point. Nothing is saved: a plan is persisted
/// only once every batch has validated, so a pass stopped halfway leaves the
/// report exactly as it found it.
#[tokio::test]
async fn a_stop_between_batches_saves_nothing_and_opens_no_second_call() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_wide_report(&s3, client_id).await;
    let (first_batch, second_batch) = wide_batches();
    // The second batch is scripted and deliberately never asked for.
    script(
        &server,
        vec![
            wide_batch_plan(&first_batch),
            wide_batch_plan(&second_batch),
        ],
    )
    .await;

    let stop = StopSignal::new();
    let press_stop = |event: pipeline::ReportTurnProgress| {
        if matches!(event, pipeline::ReportTurnProgress::PlanBatchPlanned { .. }) {
            stop.stop();
        }
    };
    let error = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("")
            .with_progress(&press_stop)
            .with_stop(&stop),
    )
    .await
    .expect_err("a stopped pass has no plan to return");
    assert!(
        error
            .to_string()
            .contains("stopped before it finished. Nothing was saved."),
        "unexpected failure: {error}"
    );

    assert_eq!(
        server.state.read().await.bedrock_stream_request_count,
        1,
        "the stop did not prevent the next batch's call"
    );
    // Half a plan is not a plan: the run fails and the report is released.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Failed);
    assert!(run.plan.is_none());
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, None);
    assert_eq!(workspace.draft.revision, 1);
}

/// The planner is shown the document's shape, not its prose. Template bodies
/// are the writer's material — the planner is ordered to treat them as facts
/// about somebody else — and they were the largest avoidable block in an
/// analysis request.
#[tokio::test]
async fn the_planner_is_never_shown_the_template_prose() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    script(
        &server,
        vec![section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "Referral question.",
                Some(INTAKE_FILE),
            ),
            plan_row(BACKGROUND_ID, "skip", "Deferred.", None),
            plan_row(SUMMARY_ID, "skip", "Deferred.", None),
        ])],
    )
    .await;
    pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect("the plan lands");

    let request = requests(&server).await.remove(0);
    let template = request["system"][2]["text"]
        .as_str()
        .expect("template block");
    assert!(template.starts_with("<untrusted_template_context>"));
    assert!(
        !template.contains("template_body"),
        "the planner was sent the template prose: {template}"
    );
    assert!(
        !template.contains("Template boilerplate."),
        "the planner was sent the template prose: {template}"
    );
    // What it does need is still there: the IDs it copies and the headings
    // that say what each section is.
    assert!(template.contains(REFERRAL_ID));
    assert!(template.contains("Reason for Referral"));
}

#[tokio::test]
async fn a_planned_run_drafts_the_plan_and_pre_skips_the_rest() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "State the referral question.",
                Some(INTAKE_FILE),
            ),
            plan_row(BACKGROUND_ID, "draft", "Summarize the history.", None),
            plan_row(SUMMARY_ID, "skip", "No testing data is present.", None),
        ])],
    )
    .await;
    // One branch per drafted row, plus the title. The writer is never asked
    // about the skipped section at all.
    script_branch(
        &server,
        REFERRAL_ID,
        "Reason for Referral",
        "Referred for attention concerns.",
    )
    .await;
    script_branch(
        &server,
        BACKGROUND_ID,
        "Background",
        "Documented developmental history.",
    )
    .await;
    script_title(&server, "Psychoeducational Evaluation").await;

    let planned = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new("Cover the referral question first."),
    )
    .await
    .expect("plan");
    let run_id = planned.run.run_id;
    // The row with no evidence gets flagged for the clinician, who fixes it
    // at the gate — which is the whole point of the gate.
    assert!(
        planned
            .run
            .plan
            .as_ref()
            .expect("plan")
            .plan_warnings
            .contains(&format!("no_resolved_evidence:{BACKGROUND_ID}"))
    );

    let edited = pipeline::update_draft_plan(
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        &[PlanEntryEdit {
            section_id: BACKGROUND_ID.parse().expect("section UUID"),
            scope: Some("Summarize school history from the intake note.".to_string()),
            evidence: Some(vec![claria_core::models::report_run::EvidenceRef {
                filename: "intake.txt".to_string(),
                note: None,
            }]),
            instruction: Some("  ".to_string()),
            ..Default::default()
        }],
    )
    .await
    .expect("the gate edit applies");
    let plan = edited.plan.as_ref().expect("plan");
    assert!(plan.user_edited);
    assert_eq!(
        entry(plan, BACKGROUND_ID).scope,
        "Summarize school history from the intake note."
    );
    assert_eq!(entry(plan, BACKGROUND_ID).evidence.len(), 1);
    assert_eq!(
        entry(plan, BACKGROUND_ID).instruction,
        None,
        "an all-whitespace instruction clears rather than stores a blank"
    );

    let outcome = pipeline::start_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        WRITER_MODEL_ID,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect("the approved plan drafts");

    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.active_run_id, None);
    let sections = &outcome.workspace.draft.content.sections;
    assert_eq!(sections.len(), 3);
    assert_eq!(
        sections[0].blocks,
        vec![paragraph("Referred for attention concerns.")]
    );
    assert_eq!(
        sections[1].blocks,
        vec![paragraph("Documented developmental history.")]
    );
    // The plan's skip needed no model call and no template body reached the
    // export.
    assert!(sections[2].skipped);
    assert!(sections[2].blocks.is_empty());
    assert_eq!(sections[2].template_blocks.as_ref(), Some(&boilerplate()));

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(run.finalized_revision, Some(2));
    assert_eq!(run.writer_model_id, WRITER_MODEL_ID);
    assert!(run.plan.as_ref().expect("plan").approved_at.is_some());

    // The plan pass, one branch per drafted row, and the title — nothing was
    // spent on the skip, and nothing was spent finishing.
    let captured = requests(&server).await;
    assert_eq!(captured.len(), 4);

    // Every branch was told what the plan said, and the guidance typed at the
    // plan step reached the writer even though Start carried none of its own.
    let drafting = &captured[1];
    let opening = drafting["messages"][0]["content"]
        .as_array()
        .expect("opening message");
    let plan_block = opening
        .iter()
        .filter_map(|block| block["text"].as_str())
        .find(|text| text.starts_with("<plan_context>"))
        .expect("plan context");
    assert!(plan_block.contains("State the referral question."));
    assert!(plan_block.contains("\"intent\": \"skip\""));
    let kickoff = opening
        .iter()
        .filter_map(|block| block["text"].as_str())
        .next_back()
        .expect("kick-off text");
    assert!(kickoff.starts_with(&section_marker(REFERRAL_ID)));
    assert!(kickoff.contains("Cover the referral question first."));
}

#[tokio::test]
async fn a_resume_with_no_new_instructions_skips_the_planner() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id).await;

    script_branch(
        &server,
        SUMMARY_ID,
        "Summary and Clinical Interpretation",
        "Findings match the referral question.",
    )
    .await;

    let outcome = pipeline::resume_planned_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        PLANNER_MODEL_ID,
        WRITER_MODEL_ID,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect("resume without new instructions");

    // No planning call went out, and the one section still pending is the
    // whole fan-out: the run already has a title, and the keep and the skip
    // are the host's to carry out.
    let captured = requests(&server).await;
    assert_eq!(captured.len(), 1);
    assert!(
        captured[0]["toolConfig"]["toolChoice"].is_null(),
        "a resume with no new instructions called the planner"
    );

    let run = only_run(&s3, client_id, report_id).await;
    let plan = run.plan.as_ref().expect("plan");
    assert_eq!(plan.model_id, pipeline::DETERMINISTIC_PLAN_MODEL_ID);
    assert_eq!(entry(plan, REFERRAL_ID).intent, SectionIntent::Keep);
    assert_eq!(entry(plan, BACKGROUND_ID).intent, SectionIntent::Skip);
    assert_eq!(entry(plan, SUMMARY_ID).intent, SectionIntent::Draft);

    // The section that already landed was kept, not rewritten.
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph("Referred for attention concerns.")]
    );
    assert_eq!(run.status, DraftRunStatus::Completed);
}

#[tokio::test]
async fn a_resume_gate_directive_survives_the_deterministic_re_plan() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id).await;

    // The clinician asks for a section that already landed to be written
    // again. A resume with no new instructions re-derives its plan from the
    // section states, so the directive has to reach them.
    pipeline::update_draft_plan(
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        &[PlanEntryEdit {
            section_id: REFERRAL_ID.parse().expect("section UUID"),
            intent: Some(SectionIntent::Rewrite),
            ..Default::default()
        }],
    )
    .await
    .expect("the resume-gate edit applies");

    script_branch(
        &server,
        REFERRAL_ID,
        "Reason for Referral",
        "Rewritten referral paragraph.",
    )
    .await;
    script_branch(
        &server,
        SUMMARY_ID,
        "Summary and Clinical Interpretation",
        "Findings match the referral question.",
    )
    .await;

    let outcome = pipeline::resume_planned_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        PLANNER_MODEL_ID,
        WRITER_MODEL_ID,
        pipeline::FullReportRequest::new(""),
    )
    .await
    .expect("resume after a gate edit");

    let run = only_run(&s3, client_id, report_id).await;
    let plan = run.plan.as_ref().expect("plan");
    assert_eq!(entry(plan, REFERRAL_ID).intent, SectionIntent::Draft);
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph("Rewritten referral paragraph.")]
    );
}

#[tokio::test]
async fn a_resume_with_new_instructions_re_plans_through_the_model() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id).await;

    script(
        &server,
        vec![resume_plan(vec![
            resume_row(
                REFERRAL_ID,
                "rewrite",
                "The new instructions change what this section must say.",
                Some("Name the referring pediatrician's concern explicitly."),
                Some(INTAKE_FILE),
            ),
            resume_row(
                BACKGROUND_ID,
                "skip",
                "Still nothing to write from.",
                None,
                None,
            ),
            resume_row(
                SUMMARY_ID,
                "draft",
                "Never written.",
                Some("Interpret the referral question."),
                Some(INTAKE_FILE),
            ),
        ])],
    )
    .await;
    script_branch(
        &server,
        REFERRAL_ID,
        "Reason for Referral",
        "The pediatrician raised attention concerns.",
    )
    .await;
    script_branch(
        &server,
        SUMMARY_ID,
        "Summary and Clinical Interpretation",
        "Findings match the referral question.",
    )
    .await;

    let outcome = pipeline::resume_planned_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        PLANNER_MODEL_ID,
        WRITER_MODEL_ID,
        pipeline::FullReportRequest::new("Say who raised the concern, in the referral section."),
    )
    .await
    .expect("resume with new instructions");

    // The re-plan is the first call, forced onto the resume tool, and it is
    // shown the run's durable state.
    let planning = requests(&server).await.remove(0);
    assert_eq!(
        planning["toolConfig"]["toolChoice"]["tool"]["name"],
        "submit_resume_plan"
    );
    let question = planning["messages"][0]["content"][0]["text"]
        .as_str()
        .expect("resume question");
    assert!(question.contains("RESUMES") || question.contains("picked back up"));
    assert!(question.contains("Section state:"));
    assert!(question.contains("\"drafted\""));
    assert!(question.contains("Say who raised the concern, in the referral section."));

    // A rewrite really replaced the section that had already landed.
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph("The pediatrician raised attention concerns.")]
    );
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(
        entry(run.plan.as_ref().expect("plan"), REFERRAL_ID).intent,
        SectionIntent::Rewrite
    );
}

#[tokio::test]
async fn a_resume_plan_cannot_keep_a_section_that_was_never_drafted() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id).await;

    // "keep" over the section that never landed, twice.
    let bad = || {
        resume_plan(vec![
            resume_row(REFERRAL_ID, "keep", "Already written.", None, None),
            resume_row(BACKGROUND_ID, "skip", "Deferred.", None, None),
            resume_row(SUMMARY_ID, "keep", "Nothing to do.", None, None),
        ])
    };
    script(&server, vec![bad(), bad()]).await;

    let error = pipeline::resume_planned_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        PLANNER_MODEL_ID,
        WRITER_MODEL_ID,
        pipeline::FullReportRequest::new("Tighten the referral section."),
    )
    .await
    .expect_err("keeping an undrafted section is not a plan");
    assert!(
        error
            .to_string()
            .contains("could not produce a usable plan"),
        "unexpected failure: {error}"
    );

    // The repair round told the model exactly which row was wrong and why.
    let repair = requests(&server).await.remove(1);
    let message = repair["messages"][2]["content"][0]["toolResult"]["content"][0]["json"]["error"]
        ["message"]
        .as_str()
        .expect("diagnostic");
    assert!(message.contains(SUMMARY_ID), "diagnostic: {message}");
    assert!(message.contains("keep"), "diagnostic: {message}");
}

#[tokio::test]
async fn a_resume_plan_demands_a_scope_for_a_rewrite() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id).await;

    let bad = || {
        resume_plan(vec![
            resume_row(
                REFERRAL_ID,
                "rewrite",
                "Needs work.",
                None,
                Some(INTAKE_FILE),
            ),
            resume_row(BACKGROUND_ID, "skip", "Deferred.", None, None),
            resume_row(SUMMARY_ID, "draft", "Write it.", Some("Interpret."), None),
        ])
    };
    script(&server, vec![bad(), bad()]).await;

    let error = pipeline::resume_planned_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        PLANNER_MODEL_ID,
        WRITER_MODEL_ID,
        pipeline::FullReportRequest::new("Rewrite the referral section."),
    )
    .await
    .expect_err("a scope-less rewrite is not a plan");
    assert!(
        error
            .to_string()
            .contains("could not produce a usable plan")
    );

    let repair = requests(&server).await.remove(1);
    let message = repair["messages"][2]["content"][0]["toolResult"]["content"][0]["json"]["error"]
        ["message"]
        .as_str()
        .expect("diagnostic");
    assert!(message.contains("without a scope"), "diagnostic: {message}");
}

#[tokio::test]
async fn a_gate_edit_refuses_a_record_the_client_does_not_have() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    script(
        &server,
        vec![section_plan(vec![
            plan_row(
                REFERRAL_ID,
                "draft",
                "Referral question.",
                Some(INTAKE_FILE),
            ),
            plan_row(BACKGROUND_ID, "skip", "Deferred.", None),
            plan_row(SUMMARY_ID, "skip", "Deferred.", None),
        ])],
    )
    .await;
    let planned = pipeline::generate_draft_plan(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        models(),
        pipeline::DraftPlanRequest::new(""),
    )
    .await
    .expect("plan");

    let error = pipeline::update_draft_plan(
        &s3,
        BUCKET,
        client_id,
        report_id,
        planned.run.run_id,
        &[PlanEntryEdit {
            section_id: REFERRAL_ID.parse().expect("section UUID"),
            evidence: Some(vec![claria_core::models::report_run::EvidenceRef {
                filename: "not-a-record.pdf".to_string(),
                note: None,
            }]),
            ..Default::default()
        }],
    )
    .await
    .expect_err("an invented record is refused");
    assert!(
        error
            .to_string()
            .contains("no record named not-a-record.pdf"),
        "unexpected failure: {error}"
    );
}

/// A run that drafted the referral section, deliberately skipped the
/// background, and never reached the summary before it died.
async fn seed_interrupted_run(s3: &aws_sdk_s3::Client, client_id: Uuid, report_id: Uuid) -> Uuid {
    let now = jiff::Timestamp::now();
    let run_id = Uuid::new_v4();
    let mut drafted = run_section(
        REFERRAL_ID,
        "Reason for Referral",
        0,
        RunSectionState::Drafted,
    );
    drafted.blocks = vec![paragraph("Referred for attention concerns.")];
    let run = DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id,
        report_id,
        client_id,
        base_revision: 1,
        status: DraftRunStatus::Stopped,
        plan: Some(RunPlan {
            model_id: PLANNER_MODEL_ID.to_string(),
            entries: vec![
                plan_entry(REFERRAL_ID, "Reason for Referral", SectionIntent::Draft),
                plan_entry(BACKGROUND_ID, "Background", SectionIntent::Skip),
                plan_entry(
                    SUMMARY_ID,
                    "Summary and Clinical Interpretation",
                    SectionIntent::Draft,
                ),
            ],
            user_edited: true,
            synthetic: false,
            approved_at: Some(now),
            plan_warnings: Vec::new(),
            created_at: now,
        }),
        title: Some("Psychoeducational Evaluation".to_string()),
        sections: vec![
            drafted,
            run_section(BACKGROUND_ID, "Background", 1, RunSectionState::Skipped),
            run_section(
                SUMMARY_ID,
                "Summary and Clinical Interpretation",
                2,
                RunSectionState::Pending,
            ),
        ],
        instructions: vec![RunInstruction {
            text: "Write the whole evaluation.".to_string(),
            added_at: now,
        }],
        writer_model_id: WRITER_MODEL_ID.to_string(),
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
    }
}
