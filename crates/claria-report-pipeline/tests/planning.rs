//! The planning pass: what the host accepts from a planner, what it refuses,
//! what it warns about, and what a drafting run does with the plan it lands.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
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

/// A span that really is in `intake.txt`, wrapped across a line so a resolved
/// quote also proves whitespace normalization.
const INTAKE_TEXT: &str =
    "Referred by pediatrician for attention concerns.\nParent reports homework avoidance.";
const REAL_QUOTE: &str = "Referred by pediatrician for attention concerns.";
/// Same claim, reworded. Nothing in the record says this, so it must not
/// resolve.
const PARAPHRASED_QUOTE: &str = "The pediatrician referred him over attention problems.";

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

/// A client with one readable record and a three-section template applied as
/// revision 1 — the shape every plan in this file is built against.
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

fn plan_row(section_id: &str, action: &str, scope: &str, quote: Option<&str>) -> serde_json::Value {
    let evidence: Vec<serde_json::Value> = quote
        .into_iter()
        .map(|quote| {
            serde_json::json!({
                "filename": "intake.txt",
                "quote": quote,
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
    quote: Option<&str>,
) -> serde_json::Value {
    let mut row = serde_json::json!({
        "section_id": section_id,
        "decision": decision,
        "reason": reason
    });
    if let Some(scope) = scope {
        row["scope"] = serde_json::Value::String(scope.to_string());
    }
    if let Some(quote) = quote {
        row["evidence"] = serde_json::json!([{"filename": "intake.txt", "quote": quote}]);
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
                Some(REAL_QUOTE),
            ),
            plan_row(
                BACKGROUND_ID,
                "draft",
                "Summarize developmental and school history.",
                Some("Parent reports homework avoidance."),
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

#[tokio::test]
async fn the_planner_request_caches_the_corpus_above_the_question() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    script(
        &server,
        vec![section_plan(vec![
            plan_row(REFERRAL_ID, "draft", "Referral question.", Some(REAL_QUOTE)),
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
async fn an_unresolvable_quote_lands_as_a_warning_rather_than_a_failure() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![section_plan(vec![
            // A paraphrase of something the record does say.
            plan_row(
                REFERRAL_ID,
                "draft",
                "State the referral question.",
                Some(PARAPHRASED_QUOTE),
            ),
            // A real quote, reflowed — whitespace is the one difference the
            // resolver forgives.
            plan_row(
                BACKGROUND_ID,
                "draft",
                "Summarize the history.",
                Some("Parent reports    homework\navoidance."),
            ),
            plan_row(SUMMARY_ID, "skip", "Nothing to interpret.", None),
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
            .contains(&"unresolved_quote:intake.txt".to_string()),
        "warnings: {:?}",
        plan.plan_warnings
    );
    assert!(
        plan.plan_warnings
            .contains(&format!("no_resolved_evidence:{REFERRAL_ID}")),
        "warnings: {:?}",
        plan.plan_warnings
    );
    // The unresolved quote is dropped rather than passed to the writer as if
    // the record said it.
    assert!(entry(plan, REFERRAL_ID).evidence.is_empty());
    // The reflowed one resolves and survives.
    assert_eq!(entry(plan, BACKGROUND_ID).evidence.len(), 1);
    assert_eq!(outcome.run.status, DraftRunStatus::AwaitingApproval);
}

#[tokio::test]
async fn an_evidence_file_outside_the_corpus_is_warned_about() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    let mut invented = plan_row(REFERRAL_ID, "draft", "Referral question.", Some(REAL_QUOTE));
    invented["evidence"][0]["filename"] = serde_json::json!("chart-review.pdf");
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
    .expect("the plan lands");
    let plan = outcome.run.plan.as_ref().expect("plan");
    assert!(
        plan.plan_warnings
            .contains(&"unknown_evidence_file:chart-review.pdf".to_string()),
        "warnings: {:?}",
        plan.plan_warnings
    );
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
                plan_row(REFERRAL_ID, "draft", "Referral question.", Some(REAL_QUOTE)),
                plan_row(BACKGROUND_ID, "draft", "History.", Some(REAL_QUOTE)),
            ]),
            section_plan(vec![
                plan_row(REFERRAL_ID, "draft", "Referral question.", Some(REAL_QUOTE)),
                plan_row(BACKGROUND_ID, "draft", "History.", Some(REAL_QUOTE)),
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
            plan_row(REFERRAL_ID, "draft", "Referral question.", Some(REAL_QUOTE)),
            plan_row(BACKGROUND_ID, "draft", "History.", Some(REAL_QUOTE)),
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

#[tokio::test]
async fn a_planned_run_drafts_the_plan_and_pre_skips_the_rest() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;

    script(
        &server,
        vec![
            section_plan(vec![
                plan_row(
                    REFERRAL_ID,
                    "draft",
                    "State the referral question.",
                    Some(REAL_QUOTE),
                ),
                plan_row(BACKGROUND_ID, "draft", "Summarize the history.", None),
                plan_row(SUMMARY_ID, "skip", "No testing data is present.", None),
            ]),
            // The writer is never asked about the skipped section.
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
            tool_round(vec![finish_call("finish-1", "Drafted two sections.")]),
            closing_text("The evaluation is drafted."),
        ],
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

    // The drafting conversation was told what the plan said, and the guidance
    // typed at the plan step reached the writer even though Start carried
    // none of its own.
    let drafting = requests(&server).await.remove(1);
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
    assert!(kickoff.contains("Cover the referral question first."));
}

#[tokio::test]
async fn a_resume_with_no_new_instructions_skips_the_planner() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id).await;

    script(
        &server,
        vec![
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-3",
                    SUMMARY_ID,
                    1,
                    "Summary and Clinical Interpretation",
                    "Findings match the referral question.",
                ),
            ]),
            tool_round(vec![finish_call("finish-1", "Picked the run back up.")]),
            closing_text("Done."),
        ],
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

    // No planning call went out: the first request is the drafting round.
    let first = requests(&server).await.remove(0);
    assert!(
        first["toolConfig"]["toolChoice"].is_null(),
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
async fn a_resume_with_new_instructions_re_plans_through_the_model() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_interrupted_run(&s3, client_id, report_id).await;

    script(
        &server,
        vec![
            resume_plan(vec![
                resume_row(
                    REFERRAL_ID,
                    "rewrite",
                    "The new instructions change what this section must say.",
                    Some("Name the referring pediatrician's concern explicitly."),
                    Some(REAL_QUOTE),
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
                    Some(REAL_QUOTE),
                ),
            ]),
            tool_round(vec![
                title_call("title-1", "Psychoeducational Evaluation"),
                write_call(
                    "section-1",
                    REFERRAL_ID,
                    0,
                    "Reason for Referral",
                    "The pediatrician raised attention concerns.",
                ),
            ]),
            tool_round(vec![write_call(
                "section-3",
                SUMMARY_ID,
                1,
                "Summary and Clinical Interpretation",
                "Findings match the referral question.",
            )]),
            tool_round(vec![finish_call("finish-1", "Re-planned and finished.")]),
            closing_text("Done."),
        ],
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
                Some(REAL_QUOTE),
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
            plan_row(REFERRAL_ID, "draft", "Referral question.", Some(REAL_QUOTE)),
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
