//! Parallel section drafting: what a gated run sends, what the host does with
//! the answers, and what survives one branch going wrong.
//!
//! Every scripted answer is keyed on the whole `Assigned section: {uuid}`
//! phrase, never a bare UUID. The plan block lists every section's ID in every
//! branch's request, and the mock matches a marker anywhere in the request's
//! text, so a bare UUID would hand one branch's answer to all of them.

use std::sync::{Arc, Mutex};

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria as pipeline;
use claria_bedrock::converse::StopSignal;
use claria_core::models::{
    report::{AuthorshipKind, ReportBlock, ReportContent, ReportSection},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, PlanEntry, RunInstruction, RunPlan,
        RunSection, RunSectionRecords, RunSectionState, SectionIntent,
    },
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_store as store;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-parallel-draft-test";
const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";

const REFERRAL_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const BACKGROUND_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SUMMARY_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

const REFERRAL_TEXT: &str = "Referred for attention concerns.";
/// The kind of instruction a clinical template writes into its own body, and
/// the one this run's referral section carries.
const REFERRAL_DIRECTIVE: &str =
    "a one sentence briefly stating why the child was referred and by whom";
const BACKGROUND_TEXT: &str = "Documented developmental history.";
const SUMMARY_TEXT: &str = "Findings are consistent with the referral question.";
const TITLE: &str = "Psychoeducational Evaluation";
/// The title the seeded template revision already carries, and therefore the
/// one a failed title branch falls back to.
const BASE_TITLE: &str = "Evaluation Template";
const GUIDANCE: &str = "Write the whole evaluation.";

/// How often the stop watcher re-reads the mock's state. The condition it
/// waits for is latched, so a missed poll costs another millisecond.
const POLL: std::time::Duration = std::time::Duration::from_millis(1);

// ── Harness ─────────────────────────────────────────────────────────────────

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

/// The same section, carrying the bracketed instruction its template author
/// wrote into it.
fn directed_template_section(id: &str, heading: &str, directive: &str) -> ReportSection {
    ReportSection {
        template_directives: vec![directive.to_string()],
        ..template_section(id, heading)
    }
}

/// A client with one readable record and a three-section template applied as
/// revision 1 — the shape every run in this file builds on.
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

/// The same report, with an authoring directive on the referral section only.
async fn seed_directed_report(s3: &aws_sdk_s3::Client, client_id: Uuid) -> Uuid {
    seed_report(
        s3,
        client_id,
        vec![
            directed_template_section(REFERRAL_ID, "Reason for Referral", REFERRAL_DIRECTIVE),
            template_section(BACKGROUND_ID, "Background"),
            template_section(SUMMARY_ID, "Summary and Clinical Interpretation"),
        ],
    )
    .await
}

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
            title: BASE_TITLE.to_string(),
            sections,
        },
    )
    .await
    .expect("seed template-shaped draft");
    workspace.report_id
}

fn plan_entry(id: &str, heading: &str, intent: SectionIntent) -> PlanEntry {
    PlanEntry {
        section_id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        intent,
        required: true,
        scope: "State what this section must assert.".to_string(),
        evidence: Vec::new(),
        instruction: None,
        curated_records: None,
    }
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
        records: None,
        updated_at: jiff::Timestamp::now(),
    }
}

/// A plan from a planning pass a clinician approved — which is the whole gate
/// on parallel drafting, so `synthetic` is false in every run here.
fn approved_plan(intents: [SectionIntent; 3]) -> RunPlan {
    let now = jiff::Timestamp::now();
    RunPlan {
        model_id: MODEL_ID.to_string(),
        entries: vec![
            plan_entry(REFERRAL_ID, "Reason for Referral", intents[0]),
            plan_entry(BACKGROUND_ID, "Background", intents[1]),
            plan_entry(
                SUMMARY_ID,
                "Summary and Clinical Interpretation",
                intents[2],
            ),
        ],
        user_edited: true,
        synthetic: false,
        approved_at: Some(now),
        plan_warnings: Vec::new(),
        created_at: now,
    }
}

fn all_draft() -> RunPlan {
    approved_plan([
        SectionIntent::Draft,
        SectionIntent::Draft,
        SectionIntent::Draft,
    ])
}

async fn put_run(s3: &aws_sdk_s3::Client, run: &DraftRun) {
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::report_draft_run(run.client_id, run.report_id, run.run_id),
        serde_json::to_vec(run).expect("run JSON"),
        Some("application/json"),
    )
    .await
    .expect("seed the drafting run");
}

/// A run waiting at the plan gate, which is what `start_draft_run` accepts.
async fn seed_gated_run(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    plan: RunPlan,
) -> Uuid {
    let now = jiff::Timestamp::now();
    let run_id = Uuid::new_v4();
    let run = DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id,
        report_id,
        client_id,
        base_revision: 1,
        status: DraftRunStatus::AwaitingApproval,
        plan: Some(plan),
        title: None,
        sections: vec![
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
        instructions: vec![RunInstruction {
            text: GUIDANCE.to_string(),
            added_at: now,
        }],
        writer_model_id: MODEL_ID.to_string(),
        finalized_revision: None,
        partial: false,
        record_snapshot: None,
        created_at: now,
        updated_at: now,
    };
    put_run(s3, &run).await;
    run_id
}

fn start(
    sdk: &aws_config::SdkConfig,
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
) -> impl std::future::Future<
    Output = Result<pipeline::FullReportGenerationOutcome, pipeline::ReportPipelineError>,
> {
    let sdk = sdk.clone();
    let s3 = s3.clone();
    async move {
        pipeline::start_draft_run(
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
    }
}

async fn only_run(s3: &aws_sdk_s3::Client, client_id: Uuid, report_id: Uuid) -> DraftRun {
    let mut runs = store::list_draft_runs(s3, BUCKET, client_id, report_id)
        .await
        .expect("list drafting runs");
    assert_eq!(runs.len(), 1, "expected exactly one drafting run");
    runs.remove(0)
}

fn section_of<'a>(run: &'a DraftRun, section_id: &str) -> &'a RunSection {
    run.sections
        .iter()
        .find(|section| section.section_id.to_string() == section_id)
        .expect("run section")
}

// ── Scripting ───────────────────────────────────────────────────────────────

fn ok(payload: serde_json::Value) -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(payload)
}

fn tool_round(calls: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    ok(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": calls}},
        "stopReason": "tool_use",
        "usage": {"inputTokens": 500, "outputTokens": 40, "cacheReadInputTokens": 400}
    }))
}

/// A response that answers in prose instead of calling the assigned tool.
fn prose(text: &str) -> ScriptedBedrockResponse {
    ok(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": [{"text": text}]}},
        "stopReason": "end_turn",
        "usage": {"inputTokens": 500, "outputTokens": 8}
    }))
}

/// A refusal the shared backoff will re-send.
fn throttled() -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::error(
        429,
        serde_json::json!({
            "__type": "ThrottlingException",
            "message": "slow down"
        }),
    )
}

/// A refusal nothing retries, which is how a branch is made to die outright.
fn dead_call() -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::error(
        500,
        serde_json::json!({
            "__type": "InternalServerException",
            "message": "raw service detail must not reach UI"
        }),
    )
}

/// Tool-use IDs have to be unique across a branch's whole conversation: a
/// repair round re-sends every earlier `tool_use` block, and two calls sharing
/// an ID is a protocol violation the wire layer refuses before it sends.
fn tool_use_id() -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    format!(
        "branch-call-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    )
}

fn write_call(section_id: &str, heading: &str, text: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": tool_use_id(), "name": "write_full_draft_section", "input": {
        "section_id": section_id,
        "position": 0,
        "heading": heading,
        "blocks": [{"kind": "paragraph", "text": text}]
    }}})
}

/// A write whose citation names a file the run's snapshot does not carry —
/// the one rejection whose diagnostic is fixed word for word.
fn write_call_citing_nothing(section_id: &str, heading: &str) -> serde_json::Value {
    let mut call = write_call(section_id, heading, "Text with an unsupported citation.");
    call["toolUse"]["input"]["citations"] = serde_json::json!([{
        "filename": "not-in-the-corpus.txt",
        "quote": "a quote long enough to be resolvable"
    }]);
    call
}

fn title_call(title: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": tool_use_id(), "name": "set_full_draft_title", "input": {
        "title": title
    }}})
}

fn skip_call(section_id: &str) -> serde_json::Value {
    serde_json::json!({"toolUse": {"toolUseId": tool_use_id(), "name": "skip_full_draft_section", "input": {
        "section_id": section_id
    }}})
}

/// The phrase only the branch assigned `section_id` carries.
fn section_marker(section_id: &str) -> String {
    format!("Assigned section: {section_id}")
}

const TITLE_MARKER: &str = "Assigned task: the report title.";

async fn script_marker(
    server: &MockServer,
    marker: String,
    responses: Vec<ScriptedBedrockResponse>,
) {
    server
        .state
        .write()
        .await
        .bedrock_tool_responses_by_marker
        .entry(marker)
        .or_default()
        .extend(responses);
}

async fn script_section(
    server: &MockServer,
    section_id: &str,
    responses: Vec<ScriptedBedrockResponse>,
) {
    script_marker(server, section_marker(section_id), responses).await;
}

async fn script_title(server: &MockServer, responses: Vec<ScriptedBedrockResponse>) {
    script_marker(server, TITLE_MARKER.to_string(), responses).await;
}

/// The straightforward run: every section writes once, the title lands once.
async fn script_clean_fan_out(server: &MockServer) {
    script_section(
        server,
        REFERRAL_ID,
        vec![tool_round(vec![write_call(
            REFERRAL_ID,
            "Reason for Referral",
            REFERRAL_TEXT,
        )])],
    )
    .await;
    script_section(
        server,
        BACKGROUND_ID,
        vec![tool_round(vec![write_call(
            BACKGROUND_ID,
            "Background",
            BACKGROUND_TEXT,
        )])],
    )
    .await;
    script_section(
        server,
        SUMMARY_ID,
        vec![tool_round(vec![write_call(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            SUMMARY_TEXT,
        )])],
    )
    .await;
    script_title(server, vec![tool_round(vec![title_call(TITLE)])]).await;
}

// ── Reading the wire ────────────────────────────────────────────────────────

async fn requests(server: &MockServer) -> Vec<serde_json::Value> {
    server.state.read().await.bedrock_tool_requests.clone()
}

/// The text blocks of one message, in order. Cache points sit between them, so
/// a fixed index into `content` is not a stable way to find one.
fn text_blocks(message: &serde_json::Value) -> Vec<String> {
    message["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| Some(block.get("text")?.as_str()?.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// One request's kick-off block: the last text block of its opening message,
/// and the only bytes that differ between branches.
fn kickoff(request: &serde_json::Value) -> String {
    text_blocks(&request["messages"][0])
        .pop()
        .expect("a kick-off block")
}

/// Which branch sent a request, as the marker it carries.
fn branch_of(request: &serde_json::Value) -> String {
    let kickoff = kickoff(request);
    for id in [REFERRAL_ID, BACKGROUND_ID, SUMMARY_ID] {
        if kickoff.starts_with(&section_marker(id)) {
            return id.to_string();
        }
    }
    assert!(
        kickoff.starts_with(TITLE_MARKER),
        "a request belonged to no branch: {kickoff}"
    );
    "title".to_string()
}

/// Every error `tool_result` one branch was handed, as `(code, message)`.
///
/// Read from that branch's LAST request, which carries its whole accumulated
/// conversation exactly once — the earlier requests are prefixes of it, so
/// folding them together would count each refusal again for every round that
/// followed it.
async fn branch_errors(server: &MockServer, branch: &str) -> Vec<(String, String)> {
    requests(server)
        .await
        .iter()
        .rev()
        .find(|request| branch_of(request) == branch)
        .expect("a request from this branch")["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|message| {
            message["content"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .filter_map(|block| {
            let error = block["toolResult"]["content"][0]["json"]["error"].clone();
            Some((
                error["code"].as_str()?.to_string(),
                error["message"].as_str()?.to_string(),
            ))
        })
        .collect()
}

// ── The happy path ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_gated_run_drafts_every_planned_section_in_parallel_and_cuts_one_revision() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;

    // Background answers in prose first, so it costs a corrective round and
    // lands after the section behind it in the plan. The document must still
    // come out in plan order: positions are handed out before any branch
    // starts, and completion order decides nothing.
    script_section(
        &server,
        BACKGROUND_ID,
        vec![
            prose("Let me think about this section."),
            tool_round(vec![write_call(
                BACKGROUND_ID,
                "Background",
                BACKGROUND_TEXT,
            )]),
        ],
    )
    .await;
    script_section(
        &server,
        REFERRAL_ID,
        vec![tool_round(vec![write_call(
            REFERRAL_ID,
            "Reason for Referral",
            REFERRAL_TEXT,
        )])],
    )
    .await;
    script_section(
        &server,
        SUMMARY_ID,
        vec![tool_round(vec![write_call(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            SUMMARY_TEXT,
        )])],
    )
    .await;
    script_title(&server, vec![tool_round(vec![title_call(TITLE)])]).await;

    let outcome = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.active_run_id, None);
    let content = &outcome.workspace.draft.content;
    assert_eq!(content.title, TITLE);
    assert_eq!(
        content
            .sections
            .iter()
            .map(|section| (section.heading.as_str(), section.blocks.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("Reason for Referral", vec![paragraph(REFERRAL_TEXT)]),
            ("Background", vec![paragraph(BACKGROUND_TEXT)]),
            (
                "Summary and Clinical Interpretation",
                vec![paragraph(SUMMARY_TEXT)]
            ),
        ]
    );

    // Every section the fan-out wrote is credited to the run that wrote it.
    for section in &content.sections {
        let authorship = section.authorship.as_ref().expect("authorship stamp");
        assert_eq!(authorship.kind, AuthorshipKind::ModelGenerated);
        assert_eq!(authorship.revision, 2);
        assert_eq!(authorship.run_id, Some(run_id));
        assert_eq!(authorship.model_id.as_deref(), Some(MODEL_ID));
    }

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(run.finalized_revision, Some(2));
    assert_eq!(run.title.as_deref(), Some(TITLE));
    // Positions are the plan's, not the order the branches came back in.
    assert_eq!(section_of(&run, REFERRAL_ID).position, 0);
    assert_eq!(section_of(&run, BACKGROUND_ID).position, 1);
    assert_eq!(section_of(&run, SUMMARY_ID).position, 2);

    // Nothing was spent finishing: the host assembles the document itself.
    assert_eq!(requests(&server).await.len(), 5);
    assert!(outcome.assistant_text.contains("Drafted 3 of 3"));
}

#[tokio::test]
async fn every_branch_sends_the_same_prefix_and_differs_only_in_its_kickoff() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    assert_eq!(captured.len(), 4);
    let first = &captured[0];
    let mut kickoffs: Vec<String> = Vec::new();
    for request in &captured {
        let branch = branch_of(request);
        assert_eq!(
            request.get("system"),
            first.get("system"),
            "{branch} sent a different system prompt"
        );
        let blocks = text_blocks(&request["messages"][0]);
        assert_eq!(blocks.len(), 4);
        assert_eq!(
            blocks[..3],
            text_blocks(&first["messages"][0])[..3],
            "{branch} sent a different corpus, template, or plan"
        );

        // Checkpoints after the template and the plan, and no tail point: a
        // single-shot branch never comes back to read one.
        let content = request["messages"][0]["content"]
            .as_array()
            .expect("opening message");
        assert_eq!(content.len(), 6, "{branch} sent an unexpected block layout");
        assert_eq!(
            content[2]["cachePoint"],
            serde_json::json!({"type": "default", "ttl": "1h"})
        );
        assert_eq!(
            content[4]["cachePoint"],
            serde_json::json!({"type": "default", "ttl": "1h"})
        );
        assert!(content[5].get("cachePoint").is_none());
        assert!(
            request["system"]
                .as_array()
                .expect("system")
                .iter()
                .all(|block| block.get("cachePoint").is_none()),
            "{branch} marked the system policy, which the plan does not ask for"
        );
        kickoffs.push(blocks[3].clone());
    }

    kickoffs.sort();
    kickoffs.dedup();
    assert_eq!(kickoffs.len(), 4, "two branches sent the same kick-off");
}

/// The template's instruction for a section reaches the branch that writes it,
/// as host guidance in the kick-off rather than as prose inside the untrusted
/// block the writer is ordered to ignore. Only the assigned branch gets it,
/// and only the branch whose section has one.
#[tokio::test]
async fn a_branch_is_handed_its_own_sections_authoring_directives() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_directed_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    for request in &captured {
        let branch = branch_of(request);
        let kickoff = kickoff(request);
        if branch == REFERRAL_ID {
            assert!(
                kickoff.contains("Template authoring directives for this section"),
                "the referral branch was not handed its directives: {kickoff}"
            );
            assert!(kickoff.contains(REFERRAL_DIRECTIVE));
            assert!(
                kickoff.contains("never as a source of client facts"),
                "the directives arrived without the rule that bounds them: {kickoff}"
            );
        } else {
            assert!(
                !kickoff.contains(REFERRAL_DIRECTIVE),
                "{branch} was handed another section's directives: {kickoff}"
            );
            assert!(
                !kickoff.contains("Template authoring directives"),
                "{branch} was handed a directive block for a section that has none: {kickoff}"
            );
        }
    }

    // The same sentence still rides in the shared template block, where it is
    // named as a field the host extracted rather than left as loose prose.
    let template = text_blocks(&captured[0]["messages"][0])[1].clone();
    assert!(template.contains("template_directives"));
    assert!(template.contains(REFERRAL_DIRECTIVE));
}

/// Two runs over the same durable state build the same kick-off, directives
/// included: the branch block is the only thing separating one request from
/// its siblings, so an unstable byte in it costs a resume its cached prefix.
#[tokio::test]
async fn the_directive_block_is_byte_identical_across_runs() {
    let mut kickoffs = Vec::new();
    for _ in 0..2 {
        let (server, sdk, s3) = setup().await;
        let client_id = Uuid::new_v4();
        let report_id = seed_directed_report(&s3, client_id).await;
        let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
        script_clean_fan_out(&server).await;
        start(&sdk, &s3, client_id, report_id, run_id)
            .await
            .expect("the fan-out drafts the report");
        let referral = requests(&server)
            .await
            .into_iter()
            .find(|request| branch_of(request) == REFERRAL_ID)
            .expect("the referral branch");
        kickoffs.push(kickoff(&referral));
    }
    assert_eq!(kickoffs[0], kickoffs[1]);
}

#[tokio::test]
async fn the_warming_branch_finishes_before_the_fan_out_starts() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;

    // The warm branch takes two calls. If the siblings had been fired at the
    // same time, one of their requests would sit between the two.
    script_section(
        &server,
        REFERRAL_ID,
        vec![
            prose("One moment."),
            tool_round(vec![write_call(
                REFERRAL_ID,
                "Reason for Referral",
                REFERRAL_TEXT,
            )]),
        ],
    )
    .await;
    script_section(
        &server,
        BACKGROUND_ID,
        vec![tool_round(vec![write_call(
            BACKGROUND_ID,
            "Background",
            BACKGROUND_TEXT,
        )])],
    )
    .await;
    script_section(
        &server,
        SUMMARY_ID,
        vec![tool_round(vec![write_call(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            SUMMARY_TEXT,
        )])],
    )
    .await;
    script_title(&server, vec![tool_round(vec![title_call(TITLE)])]).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    assert_eq!(captured.len(), 5);
    assert_eq!(branch_of(&captured[0]), REFERRAL_ID);
    assert_eq!(
        branch_of(&captured[1]),
        REFERRAL_ID,
        "a sibling started before the warming branch finished"
    );

    // The warm branch takes the run's one exact count; every branch after it
    // estimates forward from that number.
    assert_eq!(
        server.state.read().await.bedrock_count_token_requests.len(),
        1
    );
}

// ── Failure, one branch at a time ───────────────────────────────────────────

#[tokio::test]
async fn a_throttled_branch_is_retried_rather_than_failed() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;
    // One throttle ahead of the summary branch's answer, which the shared
    // backoff rides out.
    script_marker(
        &server,
        section_marker(SUMMARY_ID),
        vec![tool_round(vec![write_call(
            SUMMARY_ID,
            "Summary and Clinical Interpretation",
            SUMMARY_TEXT,
        )])],
    )
    .await;
    {
        let mut state = server.state.write().await;
        let queue = state
            .bedrock_tool_responses_by_marker
            .get_mut(&section_marker(SUMMARY_ID))
            .expect("the summary branch's queue");
        queue.insert(0, throttled());
    }

    let outcome = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("a retried branch still lands its section");

    assert_eq!(
        outcome.workspace.draft.content.sections[2].blocks,
        vec![paragraph(SUMMARY_TEXT)]
    );
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(section_of(&run, SUMMARY_ID).state, RunSectionState::Drafted);
}

#[tokio::test]
async fn a_branch_that_stays_throttled_fails_alone() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;
    // Enough throttles to outlast every attempt the backoff grants.
    script_marker(
        &server,
        section_marker(SUMMARY_ID),
        vec![throttled(), throttled(), throttled(), throttled()],
    )
    .await;
    {
        // The clean fan-out already queued the summary branch's answer; drop
        // it so the throttles are all it ever gets.
        let mut state = server.state.write().await;
        let queue = state
            .bedrock_tool_responses_by_marker
            .get_mut(&section_marker(SUMMARY_ID))
            .expect("the summary branch's queue");
        queue.remove(0);
    }

    let outcome = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the run completes around the branch that failed");

    // The siblings landed and the revision was cut; the failed section kept
    // the base revision's text rather than vanishing.
    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph(REFERRAL_TEXT)]
    );
    assert_eq!(
        outcome.workspace.draft.content.sections[2].blocks,
        boilerplate()
    );

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    let summary = section_of(&run, SUMMARY_ID);
    assert_eq!(summary.state, RunSectionState::Failed);
    let error = summary.error.as_deref().expect("a failure diagnostic");
    assert!(error.starts_with("branch_failed: "), "{error}");
    assert!(
        !error.contains("raw service detail"),
        "the service's own words reached the run: {error}"
    );
    // The failure is visible in what the reader is told, not swallowed.
    assert!(outcome.assistant_text.contains("1 failed"));
}

#[tokio::test]
async fn a_rejected_write_gets_repair_rounds_then_fails_the_section() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;
    {
        let mut state = server.state.write().await;
        let queue = state
            .bedrock_tool_responses_by_marker
            .get_mut(&section_marker(SUMMARY_ID))
            .expect("the summary branch's queue");
        queue.clear();
        for _ in 0..3 {
            queue.push(tool_round(vec![write_call_citing_nothing(
                SUMMARY_ID,
                "Summary and Clinical Interpretation",
            )]));
        }
    }

    let outcome = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the run completes around the branch that failed");

    // Three attempts, then the section is failed and the run moves on.
    let summary_calls = requests(&server)
        .await
        .iter()
        .filter(|request| branch_of(request) == SUMMARY_ID)
        .count();
    assert_eq!(summary_calls, 3);

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(
        section_of(&run, SUMMARY_ID).error.as_deref(),
        Some(
            "unknown_citation_file: A citation named a file that is not in this run's record snapshot. Copy filenames exactly from the record context; unreadable records cannot be cited."
        ),
        "the validator's diagnostic is what the run records, verbatim"
    );
    assert_eq!(outcome.workspace.draft.revision, 2);
}

#[tokio::test]
async fn a_write_naming_another_branchs_section_is_refused() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;
    {
        let mut state = server.state.write().await;
        let queue = state
            .bedrock_tool_responses_by_marker
            .get_mut(&section_marker(REFERRAL_ID))
            .expect("the referral branch's queue");
        // The referral branch reaches for its neighbour's section, and for the
        // two tools the host keeps to itself, before writing its own.
        queue.insert(
            0,
            tool_round(vec![write_call(
                BACKGROUND_ID,
                "Background",
                "Written by the wrong branch.",
            )]),
        );
        queue.insert(1, tool_round(vec![skip_call(REFERRAL_ID)]));
    }

    let outcome = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the branch recovers and writes its own section");

    let errors = branch_errors(&server, REFERRAL_ID).await;
    let codes: Vec<&str> = errors.iter().map(|(code, _)| code.as_str()).collect();
    assert_eq!(
        codes,
        vec!["wrong_section", "tool_not_available_in_parallel_mode"]
    );
    assert!(
        errors[0].1.contains(REFERRAL_ID),
        "the refusal did not name the section the branch owns: {}",
        errors[0].1
    );

    // Background is what its own branch wrote, not what the referral branch
    // tried to write for it.
    assert_eq!(
        outcome.workspace.draft.content.sections[1].blocks,
        vec![paragraph(BACKGROUND_TEXT)]
    );
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph(REFERRAL_TEXT)]
    );
}

// ── Stopping, resuming, and the ways a run ends ─────────────────────────────

#[tokio::test]
async fn a_stop_mid_fan_out_keeps_landed_sections_and_parks_the_run() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;
    // The warm branch answers; every branch after it hangs mid-stream, which
    // is where the reader presses Stop.
    server.state.write().await.bedrock_stream_stalls_after = Some(1);

    let stop = StopSignal::new();
    {
        let state = server.state.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            loop {
                if state.read().await.bedrock_stream_request_count >= 2 {
                    stop.stop();
                    return;
                }
                tokio::time::sleep(POLL).await;
            }
        });
    }

    let error = pipeline::start_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL_ID,
        pipeline::FullReportRequest::new("").with_stop(&stop),
    )
    .await
    .expect_err("a stopped run has no outcome");
    assert!(error.is_stopped(), "unexpected failure: {error}");
    assert_eq!(error.stopped_run_id(), Some(run_id));

    // The section that landed before the Stop is durable; the ones still in
    // flight are back to pending, and the run is parked where a resume can
    // pick it up.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Stopped);
    assert_eq!(
        section_of(&run, REFERRAL_ID).state,
        RunSectionState::Drafted
    );
    assert_eq!(
        section_of(&run, REFERRAL_ID).blocks,
        vec![paragraph(REFERRAL_TEXT)]
    );
    assert_eq!(
        section_of(&run, BACKGROUND_ID).state,
        RunSectionState::Pending
    );
    assert_eq!(section_of(&run, SUMMARY_ID).state, RunSectionState::Pending);
    assert!(run.finalized_revision.is_none());

    // A stop keeps the workspace pointing at its run: that pointer is what the
    // writing surface hydrates from.
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, Some(run_id));
    assert_eq!(workspace.draft.revision, 1);
}

/// The fan-out's own "past the cut, Stop is a no-op" window: every branch has
/// committed and nothing is left but assembling the plan's answer and cutting
/// the revision, neither of which calls a model. Parking here would throw away
/// a draft the run finished and make the reader click "keep what it wrote" to
/// get back exactly what they already had.
#[tokio::test]
async fn a_stop_after_every_branch_committed_still_cuts_the_revision() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;

    // Pressed on the last commit of the fan-out: every section is durable and
    // the title has landed, so nothing is in flight to cut short.
    let stop = StopSignal::new();
    let stopper = stop.clone();
    let landed = Arc::new(Mutex::new((0_u32, 0_u32, false)));
    let progress = move |event: pipeline::ReportTurnProgress| {
        let mut landed = landed.lock().expect("the landed counters");
        match event {
            pipeline::ReportTurnProgress::SectionCompleted { drafted, total, .. } => {
                landed.0 = drafted;
                landed.1 = total;
            }
            pipeline::ReportTurnProgress::TitleSet { .. } => landed.2 = true,
            _ => return,
        }
        let (drafted, total, title) = *landed;
        if title && total > 0 && drafted == total {
            stopper.stop();
        }
    };

    let outcome = pipeline::start_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL_ID,
        pipeline::FullReportRequest::new("")
            .with_stop(&stop)
            .with_progress(&progress),
    )
    .await
    .expect("a Stop past the cut is a no-op");

    assert!(stop.is_stopped(), "the Stop never fired");
    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(outcome.workspace.active_run_id, None);
    assert_eq!(outcome.workspace.draft.content.title, TITLE);

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(run.finalized_revision, Some(2));
    assert!(!run.partial);
    for id in [REFERRAL_ID, BACKGROUND_ID, SUMMARY_ID] {
        assert_eq!(section_of(&run, id).state, RunSectionState::Drafted);
    }
}

#[tokio::test]
async fn a_resume_after_a_partial_run_drafts_only_the_remaining_sections() {
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
    drafted.blocks = vec![paragraph(REFERRAL_TEXT)];
    let run_id = Uuid::new_v4();
    put_run(
        &s3,
        &DraftRun {
            schema_version: DRAFT_RUN_SCHEMA_VERSION,
            run_id,
            report_id,
            client_id,
            base_revision: 1,
            status: DraftRunStatus::Stopped,
            plan: Some(all_draft()),
            title: Some(TITLE.to_string()),
            sections: vec![
                drafted,
                run_section(BACKGROUND_ID, "Background", 1, RunSectionState::Pending),
                run_section(
                    SUMMARY_ID,
                    "Summary and Clinical Interpretation",
                    2,
                    RunSectionState::Pending,
                ),
            ],
            instructions: vec![RunInstruction {
                text: GUIDANCE.to_string(),
                added_at: now,
            }],
            writer_model_id: MODEL_ID.to_string(),
            finalized_revision: None,
            partial: false,
            record_snapshot: None,
            created_at: now,
            updated_at: now,
        },
    )
    .await;
    script_clean_fan_out(&server).await;

    let outcome = pipeline::resume_draft_run(
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
    .expect("a gated run resumes in parallel");

    // Only the two sections still pending were sent, and the title the first
    // attempt already set was not paid for again.
    let branches: Vec<String> = requests(&server).await.iter().map(branch_of).collect();
    assert_eq!(branches.len(), 2);
    assert!(branches.contains(&BACKGROUND_ID.to_string()));
    assert!(branches.contains(&SUMMARY_ID.to_string()));

    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph(REFERRAL_TEXT)],
        "the section that already landed was written again"
    );
    assert_eq!(outcome.workspace.draft.content.title, TITLE);
    assert_eq!(
        only_run(&s3, client_id, report_id).await.status,
        DraftRunStatus::Completed
    );
}

#[tokio::test]
async fn every_branch_failing_fails_the_run_and_releases_the_workspace() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    for section_id in [REFERRAL_ID, BACKGROUND_ID, SUMMARY_ID] {
        script_section(&server, section_id, vec![dead_call()]).await;
    }
    script_title(&server, vec![dead_call()]).await;

    let error = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect_err("a run with nothing written cuts no revision");
    assert!(!error.is_stopped(), "a failure was reported as a stop");
    // One failure with four symptoms: the first branch's typed cause is what
    // the reader is told, not an empty revision.
    assert!(
        error
            .to_string()
            .contains("Bedrock was temporarily unavailable"),
        "unexpected failure: {error}"
    );
    assert!(
        !error.to_string().contains("raw service detail"),
        "the service's own words reached the UI: {error}"
    );

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Failed);
    assert!(run.finalized_revision.is_none());

    // A failed run must never leave the session locked behind it.
    let workspace = store::load_report_workspace_by_id(&s3, BUCKET, client_id, report_id)
        .await
        .expect("reload workspace");
    assert_eq!(workspace.active_run_id, None);
    assert_eq!(workspace.draft.revision, 1);
    assert_eq!(workspace.draft.content.title, BASE_TITLE);
}

// ── Decisions the host makes itself ─────────────────────────────────────────

#[tokio::test]
async fn keep_and_skip_plan_rows_are_decided_without_a_model_call() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        approved_plan([
            SectionIntent::Draft,
            SectionIntent::Keep,
            SectionIntent::Skip,
        ]),
    )
    .await;
    script_clean_fan_out(&server).await;

    let outcome = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the one row that asked for it");

    // One draftable row plus the title. Nothing was spent on the keep or the
    // skip: the plan already decided both.
    let branches: Vec<String> = requests(&server).await.iter().map(branch_of).collect();
    assert_eq!(branches, vec![REFERRAL_ID.to_string(), "title".to_string()]);

    let sections = &outcome.workspace.draft.content.sections;
    assert_eq!(sections[0].blocks, vec![paragraph(REFERRAL_TEXT)]);
    // A kept section is the base revision, verbatim and not marked skipped.
    assert_eq!(sections[1].blocks, boilerplate());
    assert!(!sections[1].skipped);
    // A skipped section keeps its heading and its template copy, and exports
    // leave it out.
    assert!(sections[2].skipped);
    assert!(sections[2].blocks.is_empty());
    assert_eq!(sections[2].template_blocks.as_ref(), Some(&boilerplate()));

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(section_of(&run, BACKGROUND_ID).state, RunSectionState::Kept);
    assert_eq!(section_of(&run, SUMMARY_ID).state, RunSectionState::Skipped);
    assert_eq!(run.status, DraftRunStatus::Completed);
}

#[tokio::test]
async fn a_failed_title_branch_falls_back_to_the_base_title() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;
    {
        let mut state = server.state.write().await;
        let queue = state
            .bedrock_tool_responses_by_marker
            .get_mut(TITLE_MARKER)
            .expect("the title branch's queue");
        queue.clear();
        queue.push(dead_call());
    }

    let outcome = start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("a nameless document is still a document");

    assert_eq!(
        outcome.workspace.draft.content.title, BASE_TITLE,
        "the base revision's title should have stood"
    );
    assert_eq!(outcome.workspace.draft.revision, 2);
    assert_eq!(
        outcome.workspace.draft.content.sections[0].blocks,
        vec![paragraph(REFERRAL_TEXT)]
    );

    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert!(run.title.is_none());
    // No section was failed on the title branch's account.
    assert!(
        run.sections
            .iter()
            .all(|section| section.state == RunSectionState::Drafted)
    );
}

// ── Progress ────────────────────────────────────────────────────────────────

/// The drafted/total counters a reader watches. Branches finish out of order,
/// so the one thing the channel must never do is walk them backwards.
#[tokio::test]
async fn section_progress_counters_never_walk_backwards() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;

    let events: Arc<Mutex<Vec<pipeline::ReportTurnProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = events.clone();
        move |event: pipeline::ReportTurnProgress| {
            events.lock().expect("progress lock").push(event);
        }
    };
    pipeline::start_draft_run(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        run_id,
        MODEL_ID,
        pipeline::FullReportRequest::new("").with_progress(&sink),
    )
    .await
    .expect("the fan-out drafts the report");

    let events = events.lock().expect("progress lock").clone();
    assert!(
        events.iter().any(|event| matches!(
            event,
            pipeline::ReportTurnProgress::PlanReady { section_count: 3 }
        )),
        "the plan's row count was never announced"
    );
    let mut highest = 0;
    for event in &events {
        if let pipeline::ReportTurnProgress::SectionCompleted { drafted, total, .. } = event {
            assert_eq!(*total, 3);
            assert!(*drafted > highest, "the drafted counter repeated or fell");
            highest = *drafted;
        }
    }
    assert_eq!(highest, 3);
    assert!(events.iter().any(
        |event| matches!(event, pipeline::ReportTurnProgress::TitleSet { title } if title == TITLE)
    ));
}

// ── User-curated per-section record context ─────────────────────────────────

const TEACHER_FILE: &str = "teacher-form.txt";
const INTAKE_FILE: &str = "intake.txt";

/// A second readable record, so that restricting a section to one of them is
/// a restriction to something.
async fn seed_second_record(s3: &aws_sdk_s3::Client, client_id: Uuid) {
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::client_record_file(client_id, TEACHER_FILE),
        b"Teacher rating scales completed in April.".to_vec(),
        Some("text/plain"),
    )
    .await
    .expect("put the second record");
}

/// The same approved plan, with one section restricted to `records`.
fn plan_curating(section_id: &str, records: &[&str]) -> RunPlan {
    let mut plan = all_draft();
    let entry = plan
        .entries
        .iter_mut()
        .find(|entry| entry.section_id.to_string() == section_id)
        .expect("a plan entry for the curated section");
    entry.curated_records = Some(records.iter().map(|name| (*name).to_string()).collect());
    plan
}

/// One request's record block: block 0 of its opening message, and the only
/// block a restriction changes.
fn record_block(request: &serde_json::Value) -> String {
    text_blocks(&request["messages"][0])[0].clone()
}

/// A curated branch sends a snapshot of only the files its plan row names,
/// while its siblings send the shared corpus untouched. The template and plan
/// blocks are the document's, not the section's, so they do not move.
#[tokio::test]
async fn a_curated_branch_sends_its_own_restricted_record_block() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        plan_curating(SUMMARY_ID, &[TEACHER_FILE]),
    )
    .await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    let shared = record_block(&captured[0]);
    assert!(shared.contains(INTAKE_FILE) && shared.contains(TEACHER_FILE));

    for request in &captured {
        let branch = branch_of(request);
        let blocks = text_blocks(&request["messages"][0]);
        assert_eq!(
            blocks[1..3],
            text_blocks(&captured[0]["messages"][0])[1..3],
            "{branch} sent a different template or plan block"
        );
        if branch == SUMMARY_ID {
            assert!(
                blocks[0].contains(TEACHER_FILE) && !blocks[0].contains(INTAKE_FILE),
                "the curated branch was shown a record it was restricted from: {}",
                blocks[0]
            );
        } else {
            assert_eq!(
                blocks[0], shared,
                "{branch} did not send the shared record snapshot"
            );
        }
    }
}

/// The restriction is stated to the curated branch twice — in the plan row it
/// copies and in the rule under it — and to nobody else.
#[tokio::test]
async fn only_the_curated_branch_is_told_it_was_restricted() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        plan_curating(SUMMARY_ID, &[TEACHER_FILE]),
    )
    .await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    for request in &captured {
        let branch = branch_of(request);
        let kickoff = kickoff(request);
        if branch == SUMMARY_ID {
            assert!(
                kickoff.contains(
                    "restricted by the user to the files in your plan row's curated_records"
                ),
                "the curated branch was not told about its restriction: {kickoff}"
            );
            assert!(
                kickoff.contains("curated_records") && kickoff.contains(TEACHER_FILE),
                "the curated branch's plan row did not name the files: {kickoff}"
            );
        } else {
            assert!(
                !kickoff.contains("curated_records"),
                "{branch} was told about another section's restriction: {kickoff}"
            );
        }
    }

    // The whole plan rides in every branch's shared block, so the restriction
    // is visible to the writers of the sections around it too.
    let plan_block = text_blocks(&captured[0]["messages"][0])[2].clone();
    assert!(plan_block.contains("curated_records"));
}

/// A curated branch is not seeded from the warming branch's count: it sends
/// different bytes, so estimating forward from that number would under-count
/// every call it bills. It pays for its own exact count instead.
#[tokio::test]
async fn a_curated_branch_takes_its_own_token_count_instead_of_the_shared_seed() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        plan_curating(SUMMARY_ID, &[TEACHER_FILE]),
    )
    .await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    // Two, not one: the warming branch's count for the shared prefix, and the
    // curated branch's for its own. Background and the title still estimate.
    assert_eq!(
        server.state.read().await.bedrock_count_token_requests.len(),
        2
    );
}

/// The warming branch is the first section that reads the SHARED prefix. A
/// curated section cannot warm anything — the prefix it writes is not the one
/// its siblings read — so it never goes first however the plan is ordered.
#[tokio::test]
async fn a_curated_first_row_does_not_become_the_warming_branch() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        plan_curating(REFERRAL_ID, &[TEACHER_FILE]),
    )
    .await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    assert_eq!(
        branch_of(&captured[0]),
        BACKGROUND_ID,
        "the curated first plan row warmed a prefix its siblings do not send"
    );
    assert_eq!(record_block(&captured[0]), record_block(&captured[1]));
}

/// A curated branch may cite only what it was shown. Checking it against the
/// whole corpus would accept a citation to a file deliberately withheld from
/// it, which is the one thing the restriction exists to make impossible.
#[tokio::test]
async fn a_curated_branch_cannot_cite_a_record_it_was_restricted_from() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        plan_curating(SUMMARY_ID, &[TEACHER_FILE]),
    )
    .await;

    let mut citing_withheld = write_call(
        SUMMARY_ID,
        "Summary and Clinical Interpretation",
        SUMMARY_TEXT,
    );
    citing_withheld["toolUse"]["input"]["citations"] = serde_json::json!([{
        "filename": INTAKE_FILE,
        "quote": "Referred by pediatrician for attention concerns."
    }]);
    // The curated branch cites a withheld record first, is refused, and lands
    // its section on the repair round. Scripted before the clean fan-out so
    // the refusal is the branch's first answer rather than its second.
    script_section(
        &server,
        SUMMARY_ID,
        vec![
            tool_round(vec![citing_withheld]),
            tool_round(vec![write_call(
                SUMMARY_ID,
                "Summary and Clinical Interpretation",
                SUMMARY_TEXT,
            )]),
        ],
    )
    .await;
    script_section(
        &server,
        REFERRAL_ID,
        vec![tool_round(vec![write_call(
            REFERRAL_ID,
            "Reason for Referral",
            REFERRAL_TEXT,
        )])],
    )
    .await;
    script_section(
        &server,
        BACKGROUND_ID,
        vec![tool_round(vec![write_call(
            BACKGROUND_ID,
            "Background",
            BACKGROUND_TEXT,
        )])],
    )
    .await;
    script_title(&server, vec![tool_round(vec![title_call(TITLE)])]).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let errors = branch_errors(&server, SUMMARY_ID).await;
    assert!(
        errors
            .iter()
            .any(|(code, _)| code == "unknown_citation_file"),
        "a curated branch cited a withheld record without being refused: {errors:?}"
    );
    // The record it cited is one the run really does hold — the section that
    // was shown it drafted fine — so what refused the citation was the
    // restriction and not a missing file.
    let run = only_run(&s3, client_id, report_id).await;
    assert_eq!(
        section_of(&run, REFERRAL_ID).state,
        RunSectionState::Drafted
    );
    assert_eq!(section_of(&run, SUMMARY_ID).state, RunSectionState::Drafted);
}

/// Two runs over the same durable state build the same restricted snapshot.
/// The filtered block is a cached prefix like any other — the curated branch's
/// own repair round reads it — so an unstable byte in it costs that branch the
/// prefix it just paid full write rates for.
#[tokio::test]
async fn a_curated_snapshot_is_byte_identical_across_runs() {
    let mut snapshots = Vec::new();
    for _ in 0..2 {
        let (server, sdk, s3) = setup().await;
        let client_id = Uuid::new_v4();
        let report_id = seed_templated_report(&s3, client_id).await;
        seed_second_record(&s3, client_id).await;
        let run_id = seed_gated_run(
            &s3,
            client_id,
            report_id,
            plan_curating(SUMMARY_ID, &[TEACHER_FILE, INTAKE_FILE]),
        )
        .await;
        script_clean_fan_out(&server).await;
        start(&sdk, &s3, client_id, report_id, run_id)
            .await
            .expect("the fan-out drafts the report");
        let curated = requests(&server)
            .await
            .into_iter()
            .find(|request| branch_of(request) == SUMMARY_ID)
            .expect("the curated branch");
        snapshots.push(record_block(&curated));
    }
    assert_eq!(snapshots[0], snapshots[1]);
}

/// A restriction naming every readable record is still a restriction — but it
/// has to reproduce the corpus byte for byte, or the ordering the filtered
/// builder imposes has drifted from the shared one. Named here in the opposite
/// order to the byte order the corpus lists them in, so a builder that
/// inherited its caller's order would be caught.
#[tokio::test]
async fn a_restriction_naming_every_record_reproduces_the_shared_corpus() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        plan_curating(SUMMARY_ID, &[TEACHER_FILE, INTAKE_FILE]),
    )
    .await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    let curated = captured
        .iter()
        .find(|request| branch_of(request) == SUMMARY_ID)
        .expect("the curated branch");
    assert_eq!(record_block(curated), record_block(&captured[0]));
}

/// The cache-regression guard. Nothing about curation may appear anywhere in
/// the three shared blocks of a run where no row is curated: those bytes are
/// the cached prefix of every drafting run in flight, and a key added to them
/// costs every one of those runs its prefix at the one-hour write rate.
#[tokio::test]
async fn an_uncurated_run_carries_no_trace_of_curation_in_its_shared_blocks() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let captured = requests(&server).await;
    let shared = text_blocks(&captured[0]["messages"][0]);
    for request in &captured {
        let branch = branch_of(request);
        let blocks = text_blocks(&request["messages"][0]);
        assert_eq!(
            blocks[..3],
            shared[..3],
            "{branch} sent a different corpus, template, or plan"
        );
        for block in &blocks {
            assert!(
                !block.contains("curated"),
                "{branch} carried curation bytes on a run where nothing was curated: {block}"
            );
        }
    }
    // And the run still costs exactly one count: the warming branch's.
    assert_eq!(
        server.state.read().await.bedrock_count_token_requests.len(),
        1
    );
}

/// A restriction naming a record the client no longer has is reported to the
/// branch rather than silently dropped. A section quietly written from fewer
/// records than its plan row names is what the restriction exists to prevent.
#[tokio::test]
async fn a_curated_record_the_client_has_lost_is_reported_as_unavailable() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    // Approved while the record existed; deleted before the run started.
    let plan = plan_curating(SUMMARY_ID, &[TEACHER_FILE, "deleted.txt"]);
    let run_id = seed_gated_run(&s3, client_id, report_id, plan).await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the fan-out drafts the report");

    let curated = requests(&server)
        .await
        .into_iter()
        .find(|request| branch_of(request) == SUMMARY_ID)
        .expect("the curated branch");
    let block = record_block(&curated);
    assert!(block.contains("not_in_record_inventory"), "{block}");
    assert!(block.contains("\"snapshot_complete\":false"), "{block}");
}

// ── What the run records about its own sources ──────────────────────────────

/// The run is the durable answer to "what was this written from". Before this
/// the answer only existed in the request bytes, which nobody keeps.
#[tokio::test]
async fn a_finished_run_records_the_corpus_and_what_each_section_read() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    seed_second_record(&s3, client_id).await;
    let run_id = seed_gated_run(
        &s3,
        client_id,
        report_id,
        plan_curating(SUMMARY_ID, &[TEACHER_FILE]),
    )
    .await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the gated run completes");

    let run = only_run(&s3, client_id, report_id).await;
    let snapshot = run
        .record_snapshot
        .as_ref()
        .expect("the run records the corpus it was built from");
    let filenames: Vec<&str> = snapshot
        .files
        .iter()
        .map(|file| file.filename.as_str())
        .collect();
    assert_eq!(
        filenames,
        vec![INTAKE_FILE, TEACHER_FILE],
        "the snapshot lists the shared corpus in the order the block did"
    );
    assert!(snapshot.files.iter().all(|file| !file.sha256.is_empty()));
    assert!(snapshot.total_characters > 0);
    assert!(snapshot.unavailable.is_empty());

    // An uncurated section read the shared corpus, so it points at the
    // snapshot rather than restating it.
    assert_eq!(
        section_of(&run, REFERRAL_ID).records,
        Some(RunSectionRecords::SharedCorpus)
    );
    // The curated one records what actually reached its call.
    assert_eq!(
        section_of(&run, SUMMARY_ID).records,
        Some(RunSectionRecords::Curated {
            filenames: vec![TEACHER_FILE.to_string()]
        })
    );
}

/// The two run lookups answer different questions, and the completed run is
/// exactly the case where they diverge: nothing to resume, everything to show.
#[tokio::test]
async fn a_completed_run_leaves_history_even_though_nothing_is_resumable() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_templated_report(&s3, client_id).await;
    let run_id = seed_gated_run(&s3, client_id, report_id, all_draft()).await;
    script_clean_fan_out(&server).await;

    start(&sdk, &s3, client_id, report_id, run_id)
        .await
        .expect("the gated run completes");

    assert_eq!(
        pipeline::load_resumable_draft_run(&s3, BUCKET, client_id, report_id)
            .await
            .expect("resumable lookup"),
        None,
        "a completed run is not something to pick back up"
    );

    let history = pipeline::load_draft_run_history(&s3, BUCKET, client_id, report_id)
        .await
        .expect("run history");
    assert_eq!(history.active_run_id, None, "the cut released the session");
    assert_eq!(history.runs.len(), 1);
    let run = &history.runs[0];
    assert_eq!(run.run_id, run_id);
    assert_eq!(run.status, DraftRunStatus::Completed);
    assert_eq!(run.finalized_revision, Some(2));
    // The plan, the guidance and the per-section outcomes are all still there,
    // which is what the Draft run tab renders after a reload.
    assert_eq!(run.plan.as_ref().expect("the plan").entries.len(), 3);
    assert_eq!(run.instructions[0].text, GUIDANCE);
    assert!(
        run.sections
            .iter()
            .all(|section| section.state == RunSectionState::Drafted),
        "every planned section landed"
    );
    assert!(run.record_snapshot.is_some());
}
