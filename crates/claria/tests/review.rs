//! The review fan-out: what one sweep sends, what the host accepts from a
//! reviewing model, and what survives one branch going wrong.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria as pipeline;
use claria_core::models::{
    findings::{FindingStatus, ReviewPass},
    report::{ReportBlock, ReportContent, ReportSection},
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};
use claria_report_store as store;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-review-test";
const REVIEWER_MODEL_ID: &str = "us.anthropic.claude-sonnet-test";
/// Same family, but the capability table reads the `:48k` suffix as a small
/// context window — the one lever a test has on the preflight.
const SMALL_WINDOW_MODEL_ID: &str = "us.anthropic.claude-sonnet-test:48k";

const REFERRAL_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const BACKGROUND_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SUMMARY_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

const ALL_PROPERTIES: [&str; 7] = [
    "tense_drift",
    "terminology",
    "transitions",
    "redundancy",
    "internal_contradiction",
    "unsupported_claim",
    "cross_section_conflict",
];
const STYLE_PROPERTIES: [&str; 4] = ["tense_drift", "terminology", "transitions", "redundancy"];

const INTAKE_TEXT: &str =
    "Referred by pediatrician for attention concerns.\nParent reports homework avoidance.";
const RECORD_QUOTE: &str = "Referred by pediatrician for attention concerns.";

/// Wrapped on purpose: a quote that resolves across this line break is what
/// proves the anchor is matched with whitespace normalized.
const REFERRAL_TEXT: &str = "The client was referred by his pediatrician in March.\nThe referral question concerns \
     attention.";
const REFERRAL_QUOTE: &str = "referred by his pediatrician in March. The referral question";
const BACKGROUND_TEXT: &str = "The client attends fourth grade. He is doing well overall.";
const SUMMARY_TEXT: &str = "Testing is pending and no interpretation is offered yet.";

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

fn section(id: &str, heading: &str, text: &str) -> ReportSection {
    ReportSection {
        id: id.parse().expect("section UUID"),
        heading: heading.to_string(),
        blocks: vec![paragraph(text)],
        skipped: false,
        template_blocks: None,
        template_directives: Vec::new(),
        authorship: None,
    }
}

fn drafted_content() -> ReportContent {
    ReportContent {
        title: "Psychoeducational Evaluation".to_string(),
        sections: vec![
            section(REFERRAL_ID, "Reason for Referral", REFERRAL_TEXT),
            section(BACKGROUND_ID, "Background", BACKGROUND_TEXT),
            section(SUMMARY_ID, "Summary", SUMMARY_TEXT),
        ],
    }
}

/// A client with one readable record and a three-section drafted report at
/// revision 1 — the shape every sweep in this file reads.
async fn seed_drafted_report(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    content: ReportContent,
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
    store::save_report_draft_for_report(s3, BUCKET, client_id, workspace.report_id, 0, content)
        .await
        .expect("seed the drafted report");
    workspace.report_id
}

fn forced_tool_call(
    tool_use_id: &str,
    name: &str,
    input: serde_json::Value,
) -> ScriptedBedrockResponse {
    ScriptedBedrockResponse::success(serde_json::json!({
        "output": {"message": {"role": "assistant", "content": [
            {"toolUse": {"toolUseId": tool_use_id, "name": name, "input": input}}
        ]}},
        "stopReason": "tool_use",
        "usage": {
            "inputTokens": 4_000,
            "outputTokens": 200,
            "cacheReadInputTokens": 3_000,
            "cacheWriteInputTokens": 0
        }
    }))
}

fn review_rows(property: &str, rows: Vec<serde_json::Value>) -> ScriptedBedrockResponse {
    forced_tool_call(
        &format!("review-{property}"),
        "submit_review_rows",
        serde_json::json!({"property": property, "rows": rows}),
    )
}

fn no_issues(section_id: &str) -> serde_json::Value {
    serde_json::json!({
        "section_id": section_id,
        "status": "no_issues",
        "findings": []
    })
}

fn findings_row(section_id: &str, findings: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "section_id": section_id,
        "status": "findings",
        "findings": findings
    })
}

fn style_finding(quote: &str, find: &str, replace: &str) -> serde_json::Value {
    serde_json::json!({
        "span": {"quote": quote},
        "description": "The tense shifts inside this sentence.",
        "replacement": {"find": find, "replace": replace}
    })
}

fn consistency_finding(quote: &str) -> serde_json::Value {
    serde_json::json!({
        "span": {"quote": quote},
        "description": "Nothing in the records supports this claim.",
        "record_citation": {"filename": "intake.txt", "quote": RECORD_QUOTE}
    })
}

/// A clean sweep: every property returns a no-issues row for every section.
fn clean_rows() -> Vec<serde_json::Value> {
    vec![
        no_issues(REFERRAL_ID),
        no_issues(BACKGROUND_ID),
        no_issues(SUMMARY_ID),
    ]
}

/// Queue one answer per property, keyed on the property name so a branch gets
/// its own answer no matter what order the six parallel requests arrive in.
async fn script_by_property(
    server: &MockServer,
    answers: HashMap<&str, Vec<ScriptedBedrockResponse>>,
) {
    let mut state = server.state.write().await;
    for (property, responses) in answers {
        state
            .bedrock_tool_responses_by_marker
            .entry(property.to_string())
            .or_default()
            .extend(responses);
    }
}

/// The clean sweep every test starts from, with `overrides` replacing the
/// answers for the properties it names.
async fn script_sweep(
    server: &MockServer,
    overrides: HashMap<&'static str, Vec<ScriptedBedrockResponse>>,
) {
    let mut answers: HashMap<&str, Vec<ScriptedBedrockResponse>> = ALL_PROPERTIES
        .into_iter()
        .map(|property| (property, vec![review_rows(property, clean_rows())]))
        .collect();
    for (property, responses) in overrides {
        answers.insert(property, responses);
    }
    script_by_property(server, answers).await;
}

async fn requests(server: &MockServer) -> Vec<serde_json::Value> {
    server.state.read().await.bedrock_tool_requests.clone()
}

/// The text blocks of one message, in order. Cache points sit between them,
/// so a fixed index into `content` is not a stable way to find the
/// instruction.
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

/// Every review request the mock saw, paired with the property its
/// instruction asked about.
async fn review_requests(server: &MockServer) -> Vec<(String, serde_json::Value)> {
    requests(server)
        .await
        .into_iter()
        .filter_map(|request| {
            let opening = text_blocks(&request["messages"][0]).join("\n");
            let property = ALL_PROPERTIES.into_iter().find(|property| {
                opening.contains(&format!("property for this pass is {property}"))
            })?;
            Some((property.to_string(), request))
        })
        .collect()
}

fn sweep(
    sdk: &aws_config::SdkConfig,
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    model_id: &'static str,
) -> impl std::future::Future<Output = Result<pipeline::ReviewSweepOutcome, pipeline::ReportPipelineError>>
{
    let sdk = sdk.clone();
    let s3 = s3.clone();
    async move {
        pipeline::run_review_sweeps(
            &sdk,
            &s3,
            BUCKET,
            client_id,
            report_id,
            1,
            model_id,
            pipeline::ReviewSweepRequest::new(),
        )
        .await
    }
}

// ── The happy path ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_sweep_reviews_every_property_and_persists_anchored_findings() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    let mut overrides = HashMap::new();
    overrides.insert(
        "tense_drift",
        vec![review_rows(
            "tense_drift",
            vec![
                findings_row(
                    REFERRAL_ID,
                    vec![style_finding(
                        REFERRAL_QUOTE,
                        "concerns attention",
                        "concerned attention",
                    )],
                ),
                no_issues(BACKGROUND_ID),
                no_issues(SUMMARY_ID),
            ],
        )],
    );
    overrides.insert(
        "unsupported_claim",
        vec![review_rows(
            "unsupported_claim",
            vec![
                no_issues(REFERRAL_ID),
                findings_row(
                    BACKGROUND_ID,
                    vec![consistency_finding("attends fourth grade")],
                ),
                no_issues(SUMMARY_ID),
            ],
        )],
    );
    script_sweep(&server, overrides).await;

    let outcome = sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the sweep completes");

    assert_eq!(outcome.properties.len(), 7);
    assert!(
        outcome
            .properties
            .iter()
            .all(|status| status.error.is_none()),
        "a branch failed: {:?}",
        outcome.failed_properties()
    );
    assert_eq!(outcome.converse_calls, 7, "one call per property");

    // One coverage row per property, including the six that found nothing.
    let covered: HashSet<&str> = outcome
        .findings
        .coverage
        .iter()
        .map(|coverage| coverage.property.as_str())
        .collect();
    assert_eq!(covered, HashSet::from(ALL_PROPERTIES));
    for coverage in &outcome.findings.coverage {
        assert_eq!(coverage.revision, 1);
        assert_eq!(coverage.sections_reviewed, 3);
        assert_eq!(coverage.model_id, REVIEWER_MODEL_ID);
        let expected_pass = if STYLE_PROPERTIES.contains(&coverage.property.as_str()) {
            ReviewPass::Style
        } else {
            ReviewPass::Consistency
        };
        assert_eq!(coverage.pass, expected_pass, "{}", coverage.property);
    }

    assert_eq!(outcome.findings.findings.len(), 2);
    let style = outcome
        .findings
        .findings
        .iter()
        .find(|finding| finding.property == "tense_drift")
        .expect("the style finding");
    assert_eq!(style.pass, ReviewPass::Style);
    assert_eq!(style.status, FindingStatus::Open);
    // Stamped host-side from the request, not read back out of the answer.
    assert_eq!(style.model_id, REVIEWER_MODEL_ID);
    assert_eq!(style.anchor.revision, 1);
    assert_eq!(style.anchor.section_id.to_string(), REFERRAL_ID);
    let span = style.span.expect("the span resolved");
    assert_eq!(span.block_index, 0);
    // The quote spans the line break, so the range covers both halves.
    let referral_chars: Vec<char> = REFERRAL_TEXT.chars().collect();
    let anchored: String = referral_chars[span.start_char as usize..span.end_char as usize]
        .iter()
        .collect();
    assert!(
        anchored.starts_with("referred by his pediatrician"),
        "the anchor landed at {anchored:?}"
    );
    assert!(anchored.ends_with("The referral question"));
    let proposal = style.proposal.as_ref().expect("the style proposal");
    assert_eq!(proposal.block_index, 0);
    assert_eq!(proposal.original_text, "concerns attention");

    let consistency = outcome
        .findings
        .findings
        .iter()
        .find(|finding| finding.property == "unsupported_claim")
        .expect("the consistency finding");
    assert_eq!(consistency.pass, ReviewPass::Consistency);
    assert!(
        consistency.proposal.is_none(),
        "a consistency finding has no write path"
    );
    let citation = consistency
        .record_citation
        .as_ref()
        .expect("the record citation resolved");
    assert_eq!(citation.filename, "intake.txt");

    // The sweep is durable, not just returned.
    let stored = store::list_report_findings(&s3, BUCKET, client_id, report_id)
        .await
        .expect("stored findings");
    assert_eq!(stored.findings.len(), 2);
    assert_eq!(stored.coverage.len(), 7);
}

#[tokio::test]
async fn every_branch_sends_the_same_prefix_and_differs_only_in_its_instruction() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;
    script_sweep(&server, HashMap::new()).await;

    sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the sweep completes");

    let captured = review_requests(&server).await;
    assert_eq!(captured.len(), 7);
    let (_, first) = &captured[0];
    for (property, request) in &captured {
        assert_eq!(
            request.get("system"),
            first.get("system"),
            "{property} sent different system blocks"
        );
        assert_eq!(
            text_blocks(&request["messages"][0]).first(),
            text_blocks(&first["messages"][0]).first(),
            "{property} sent a different drafted-section block"
        );
        assert_eq!(
            request.get("toolConfig"),
            first.get("toolConfig"),
            "{property} sent a different tool configuration"
        );
        assert_eq!(
            request.pointer("/toolConfig/toolChoice/tool/name"),
            Some(&serde_json::json!("submit_review_rows")),
            "{property} forced a different tool"
        );
    }
    // The instruction is the one thing that moves, and every branch's names
    // its own property.
    let instructions: HashSet<String> = captured
        .iter()
        .filter_map(|(_, request)| text_blocks(&request["messages"][0]).into_iter().nth(1))
        .collect();
    assert_eq!(instructions.len(), 7, "two branches sent the same question");

    // One checkpoint after the system blocks, one after the drafted sections,
    // both at the extended tier.
    let system = first.get("system").and_then(|value| value.as_array());
    let last_system = system.and_then(|blocks| blocks.last());
    assert_eq!(
        last_system.and_then(|block| block.pointer("/cachePoint/ttl")),
        Some(&serde_json::json!("1h")),
        "no one-hour checkpoint after the system blocks"
    );
    // The checkpoint sits between the drafted-section block and the
    // instruction, which is what lets the branches share the draft and differ
    // below it.
    assert_eq!(
        first.pointer("/messages/0/content/1/cachePoint/ttl"),
        Some(&serde_json::json!("1h")),
        "no one-hour checkpoint after the drafted sections: {:?}",
        first.pointer("/messages/0/content")
    );
}

#[tokio::test]
async fn the_warming_branch_finishes_before_the_fan_out_starts() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;
    script_sweep(&server, HashMap::new()).await;

    let events = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let recorder = {
        let events = Arc::clone(&events);
        move |event: pipeline::ReportTurnProgress| match event {
            pipeline::ReportTurnProgress::ReviewPassStarted { property, .. } => {
                events.lock().expect("progress log").push((property, true));
            }
            pipeline::ReportTurnProgress::ReviewPassCompleted { property, .. } => {
                events.lock().expect("progress log").push((property, false));
            }
            _ => {}
        }
    };
    pipeline::run_review_sweeps(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        1,
        REVIEWER_MODEL_ID,
        pipeline::ReviewSweepRequest::new().with_progress(&recorder),
    )
    .await
    .expect("the sweep completes");

    let events = events.lock().expect("progress log").clone();
    assert_eq!(events.len(), 14, "one start and one finish per property");
    assert_eq!(
        &events[..2],
        &[
            ("tense_drift".to_string(), true),
            ("tense_drift".to_string(), false)
        ],
        "the warming branch has to finish before anything else starts: {events:?}"
    );

    // CountTokens runs once, on the warming branch; the siblings estimate
    // forward from its answer.
    let counted = server.state.read().await.bedrock_count_token_requests.len();
    assert_eq!(counted, 1, "the fan-out counted tokens {counted} times");
}

// ── Host validation ─────────────────────────────────────────────────────────

#[tokio::test]
async fn a_missing_section_row_costs_the_branch_one_correction() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    let mut overrides = HashMap::new();
    overrides.insert(
        "terminology",
        vec![
            // The clean section is left out entirely — the exact silence the
            // uniform-coverage rule exists to refuse.
            review_rows(
                "terminology",
                vec![no_issues(REFERRAL_ID), no_issues(BACKGROUND_ID)],
            ),
            review_rows("terminology", clean_rows()),
        ],
    );
    script_sweep(&server, overrides).await;

    let outcome = sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the sweep completes");

    assert!(
        outcome
            .properties
            .iter()
            .all(|status| status.error.is_none()),
        "the correction should have saved the branch: {:?}",
        outcome.failed_properties()
    );
    assert_eq!(
        outcome.converse_calls, 8,
        "one branch paid for a correction"
    );
    assert_eq!(outcome.findings.coverage.len(), 7);

    // The correction carried the diagnostic verbatim, naming the section that
    // had no row.
    let repair = review_requests(&server)
        .await
        .into_iter()
        .find(|(property, request)| {
            property == "terminology" && request["messages"].as_array().is_some_and(|m| m.len() > 1)
        })
        .expect("the repair round");
    let tool_result = repair
        .1
        .pointer("/messages/2/content/0/toolResult/content/0/json")
        .expect("the error tool_result")
        .to_string();
    assert!(tool_result.contains("invalid_review"), "{tool_result}");
    assert!(
        tool_result.contains(SUMMARY_ID),
        "the diagnostic did not name the missing section: {tool_result}"
    );
}

#[tokio::test]
async fn a_consistency_property_that_proposes_text_is_refused_then_repaired() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    let mut overrides = HashMap::new();
    overrides.insert(
        "internal_contradiction",
        vec![
            review_rows(
                "internal_contradiction",
                vec![
                    no_issues(REFERRAL_ID),
                    findings_row(
                        BACKGROUND_ID,
                        vec![serde_json::json!({
                            "span": {"quote": "attends fourth grade"},
                            "description": "This disagrees with the summary.",
                            "replacement": {"find": "fourth grade", "replace": "fifth grade"}
                        })],
                    ),
                    no_issues(SUMMARY_ID),
                ],
            ),
            review_rows(
                "internal_contradiction",
                vec![
                    no_issues(REFERRAL_ID),
                    findings_row(
                        BACKGROUND_ID,
                        vec![serde_json::json!({
                            "span": {"quote": "attends fourth grade"},
                            "description": "This disagrees with the summary.",
                            "conflicting_span": {
                                "section_id": SUMMARY_ID,
                                "quote": "Testing is pending"
                            }
                        })],
                    ),
                    no_issues(SUMMARY_ID),
                ],
            ),
        ],
    );
    script_sweep(&server, overrides).await;

    let outcome = sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the sweep completes");

    assert!(outcome.failed_properties().is_empty());
    let finding = outcome
        .findings
        .findings
        .iter()
        .find(|finding| finding.property == "internal_contradiction")
        .expect("the repaired finding");
    assert!(finding.proposal.is_none());
    let conflicting = finding.conflicting.as_ref().expect("the conflicting ref");
    assert_eq!(
        conflicting.section_id.map(|id| id.to_string()).as_deref(),
        Some(SUMMARY_ID)
    );
    assert!(
        conflicting.span.is_some(),
        "the conflicting quote should resolve in its own section"
    );
}

#[tokio::test]
async fn a_duplicated_quote_anchors_the_first_occurrence_and_says_so() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let mut content = drafted_content();
    content.sections[1].blocks = vec![paragraph("He is doing well. He is doing well.")];
    let report_id = seed_drafted_report(&s3, client_id, content).await;

    let mut overrides = HashMap::new();
    overrides.insert(
        "redundancy",
        vec![review_rows(
            "redundancy",
            vec![
                no_issues(REFERRAL_ID),
                findings_row(
                    BACKGROUND_ID,
                    vec![serde_json::json!({
                        "span": {"quote": "He is doing well."},
                        "description": "This sentence is stated twice.",
                        "replacement": {
                            "find": "He is doing well. He is doing well.",
                            "replace": "He is doing well."
                        }
                    })],
                ),
                no_issues(SUMMARY_ID),
            ],
        )],
    );
    script_sweep(&server, overrides).await;

    let outcome = sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the sweep completes");

    let finding = outcome
        .findings
        .findings
        .iter()
        .find(|finding| finding.property == "redundancy")
        .expect("the finding");
    assert!(
        finding.description.contains("duplicate_anchor"),
        "the ambiguity was not surfaced: {}",
        finding.description
    );
    let span = finding.span.expect("the span resolved");
    assert_eq!(span.start_char, 0, "the first occurrence is anchored");
    // The replacement itself is unique, so it still applies.
    assert!(finding.proposal.is_some());
}

// ── Failure isolation ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_branch_that_cannot_be_corrected_fails_alone() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    let mut overrides = HashMap::new();
    // Two answers, both filed under a property this branch was not asked
    // about: the one thing a correction cannot fix by trying again.
    overrides.insert(
        "transitions",
        vec![
            review_rows("terminology", clean_rows()),
            review_rows("terminology", clean_rows()),
        ],
    );
    script_sweep(&server, overrides).await;

    let outcome = sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the other six branches still land");

    assert_eq!(outcome.failed_properties(), vec!["transitions"]);
    let failure = outcome
        .properties
        .iter()
        .find(|status| status.property == "transitions")
        .and_then(|status| status.error.clone())
        .expect("the failure message");
    assert!(
        failure.contains("The other passes were unaffected"),
        "{failure}"
    );

    // No coverage row for the property nobody reviewed, and six for the rest.
    let covered: HashSet<&str> = outcome
        .findings
        .coverage
        .iter()
        .map(|coverage| coverage.property.as_str())
        .collect();
    assert_eq!(covered.len(), 6);
    assert!(!covered.contains("transitions"));
}

#[tokio::test]
async fn a_branch_throttled_past_its_backoff_fails_alone() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    // More throttles than the SDK's attempts and the fan-out's backoff can
    // absorb between them, so this branch runs out of patience while the
    // other six are answered normally.
    let mut overrides = HashMap::new();
    overrides.insert(
        "redundancy",
        (0..24)
            .map(|_| {
                ScriptedBedrockResponse::error(
                    429,
                    serde_json::json!({
                        "__type": "ThrottlingException",
                        "message": "Too many requests, please wait before trying again."
                    }),
                )
            })
            .collect(),
    );
    script_sweep(&server, overrides).await;

    let outcome = sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the other six branches still land");

    assert_eq!(outcome.failed_properties(), vec!["redundancy"]);
    let covered: HashSet<&str> = outcome
        .findings
        .coverage
        .iter()
        .map(|coverage| coverage.property.as_str())
        .collect();
    assert_eq!(covered.len(), 6);
    assert!(!covered.contains("redundancy"));
    // The raw service text never reaches the clinician.
    let failure = outcome
        .properties
        .iter()
        .find(|status| status.property == "redundancy")
        .and_then(|status| status.error.clone())
        .expect("the failure message");
    assert!(
        !failure.contains("Too many requests, please wait"),
        "raw service detail reached the UI: {failure}"
    );
}

#[tokio::test]
async fn a_throttled_branch_is_retried_rather_than_failed() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    // The SDK burns its own attempts on a throttle before the error reaches
    // the pipeline's retry, so the branch has to be throttled through those
    // first for this to exercise the backoff the fan-out adds.
    let throttled = || {
        ScriptedBedrockResponse::error(
            429,
            serde_json::json!({
                "__type": "ThrottlingException",
                "message": "Too many requests, please wait before trying again."
            }),
        )
    };
    // Five: more than the SDK's own attempt budget, so the last of them has
    // to reach the fan-out's backoff for this branch to recover at all.
    let mut overrides = HashMap::new();
    overrides.insert(
        "terminology",
        vec![
            throttled(),
            throttled(),
            throttled(),
            throttled(),
            throttled(),
            review_rows("terminology", clean_rows()),
        ],
    );
    script_sweep(&server, overrides).await;

    let outcome = sweep(&sdk, &s3, client_id, report_id, REVIEWER_MODEL_ID)
        .await
        .expect("the sweep completes");

    assert!(
        outcome.failed_properties().is_empty(),
        "a throttle should be waited out, not reported: {:?}",
        outcome.failed_properties()
    );
    assert_eq!(outcome.findings.coverage.len(), 7);
    assert_eq!(
        outcome.converse_calls, 7,
        "a retried call is the same call, not a correction round"
    );
}

// ── Guards ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_report_too_large_to_review_is_refused_before_any_call() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let mut content = drafted_content();
    let filler = paragraph(&"The client presented as engaged. ".repeat(450));
    for section in &mut content.sections {
        section.blocks = vec![filler.clone(); 3];
    }
    let report_id = seed_drafted_report(&s3, client_id, content).await;
    script_sweep(&server, HashMap::new()).await;

    let error = sweep(&sdk, &s3, client_id, report_id, SMALL_WINDOW_MODEL_ID)
        .await
        .expect_err("the sweep is refused");
    let message = error.to_string();
    assert!(
        message.contains("too large to review against the full record corpus"),
        "{message}"
    );
    assert!(
        requests(&server).await.is_empty(),
        "the preflight let a request out"
    );
}

#[tokio::test]
async fn a_stale_revision_is_refused() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;
    script_sweep(&server, HashMap::new()).await;

    let error = pipeline::run_review_sweeps(
        &sdk,
        &s3,
        BUCKET,
        client_id,
        report_id,
        0,
        REVIEWER_MODEL_ID,
        pipeline::ReviewSweepRequest::new(),
    )
    .await
    .expect_err("a sweep of a revision that has moved on is refused");
    assert!(
        error.to_string().contains("changed"),
        "{}",
        error.to_string()
    );
    assert!(requests(&server).await.is_empty());
}

// ── Choosing the passes ─────────────────────────────────────────────────────

fn sweep_with_passes(
    sdk: &aws_config::SdkConfig,
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    report_id: Uuid,
    passes: Vec<pipeline::ReviewPassSelection>,
) -> impl std::future::Future<Output = Result<pipeline::ReviewSweepOutcome, pipeline::ReportPipelineError>>
{
    let sdk = sdk.clone();
    let s3 = s3.clone();
    async move {
        pipeline::run_review_sweeps(
            &sdk,
            &s3,
            BUCKET,
            client_id,
            report_id,
            1,
            REVIEWER_MODEL_ID,
            pipeline::ReviewSweepRequest::new().with_passes(&passes),
        )
        .await
    }
}

fn pass(property: claria_bedrock::analysis::ReviewProperty) -> pipeline::ReviewPassSelection {
    pipeline::ReviewPassSelection {
        property,
        body: None,
    }
}

#[tokio::test]
async fn a_removed_pass_is_never_sent_and_earns_no_coverage_row() {
    use claria_bedrock::analysis::ReviewProperty;
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;
    script_sweep(&server, HashMap::new()).await;

    let outcome = sweep_with_passes(
        &sdk,
        &s3,
        client_id,
        report_id,
        vec![
            pass(ReviewProperty::TenseDrift),
            pass(ReviewProperty::UnsupportedClaim),
        ],
    )
    .await
    .expect("the sweep completes");

    assert_eq!(outcome.converse_calls, 2, "one call per requested pass");
    let requested: HashSet<&str> = outcome
        .properties
        .iter()
        .map(|status| status.property)
        .collect();
    assert_eq!(
        requested,
        HashSet::from(["tense_drift", "unsupported_claim"])
    );

    // Coverage is the receipt that a property was read over this revision, so
    // the five nobody asked for must not have one.
    let covered: HashSet<&str> = outcome
        .findings
        .coverage
        .iter()
        .map(|coverage| coverage.property.as_str())
        .collect();
    assert_eq!(covered, HashSet::from(["tense_drift", "unsupported_claim"]));

    let skipped: HashSet<&str> = outcome.skipped_properties().into_iter().collect();
    assert_eq!(
        skipped,
        HashSet::from([
            "terminology",
            "transitions",
            "redundancy",
            "internal_contradiction",
            "cross_section_conflict",
        ]),
        "the audit event has to name what nobody read"
    );

    let sent: HashSet<String> = review_requests(&server)
        .await
        .into_iter()
        .map(|(property, _)| property)
        .collect();
    assert_eq!(
        sent,
        HashSet::from(["tense_drift".to_string(), "unsupported_claim".to_string()]),
        "a removed pass must not reach Bedrock at all"
    );
}

#[tokio::test]
async fn an_edited_checklist_is_sent_inside_the_hosts_own_frame() {
    use claria_bedrock::analysis::ReviewProperty;
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;
    script_sweep(&server, HashMap::new()).await;

    let outcome = sweep_with_passes(
        &sdk,
        &s3,
        client_id,
        report_id,
        vec![pipeline::ReviewPassSelection {
            property: ReviewProperty::Terminology,
            body: Some("Check that every instrument keeps its acronym.".to_string()),
        }],
    )
    .await
    .expect("the sweep completes");

    assert_eq!(outcome.edited_properties, vec!["terminology"]);

    let (_, request) = review_requests(&server)
        .await
        .into_iter()
        .next()
        .expect("the one branch was sent");
    let instruction = text_blocks(&request["messages"][0]).join("\n");
    assert!(
        instruction.contains("Check that every instrument keeps its acronym."),
        "the clinician's checklist has to reach the model"
    );
    // The frame is composed around whatever was typed, so the validator's own
    // preconditions survive an edit that deleted them.
    assert!(instruction.contains("exactly one row per section"));
    assert!(instruction.contains("character for character"));
    assert!(instruction.contains("Call submit_review_rows exactly once"));
    assert!(
        !instruction.contains("Every acronym, for whether it is expanded"),
        "the shipped checklist must be replaced, not appended to"
    );
}

#[tokio::test]
async fn an_unedited_pass_sends_the_shipped_checklist_byte_for_byte() {
    use claria_bedrock::analysis::ReviewProperty;
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;
    script_sweep(&server, HashMap::new()).await;

    // Explicitly re-supplying the default is the same request as not supplying
    // one: the preflight round-trips its own text, and a reader who edits and
    // then undoes must not pay for a different prompt.
    let outcome = sweep_with_passes(
        &sdk,
        &s3,
        client_id,
        report_id,
        vec![pipeline::ReviewPassSelection {
            property: ReviewProperty::Redundancy,
            body: Some(
                pipeline::default_review_instructions(ReviewProperty::Redundancy).to_string(),
            ),
        }],
    )
    .await
    .expect("the sweep completes");

    assert!(
        outcome.edited_properties.is_empty(),
        "restoring the default is not an edit"
    );
    let (_, request) = review_requests(&server)
        .await
        .into_iter()
        .next()
        .expect("the one branch was sent");
    let instruction = text_blocks(&request["messages"][0]).join("\n");
    assert!(instruction.contains("Sentences that restate, in different words"));
}

#[tokio::test]
async fn a_sweep_with_no_passes_is_refused_before_any_call() {
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    let error = sweep_with_passes(&sdk, &s3, client_id, report_id, Vec::new())
        .await
        .expect_err("an empty sweep is refused");
    assert!(
        error.to_string().contains("at least one pass"),
        "unexpected error: {error}"
    );
    assert!(
        requests(&server).await.is_empty(),
        "nothing should have been sent"
    );
}

#[tokio::test]
async fn one_property_cannot_be_reviewed_twice_in_one_sweep() {
    use claria_bedrock::analysis::ReviewProperty;
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    let error = sweep_with_passes(
        &sdk,
        &s3,
        client_id,
        report_id,
        vec![
            pass(ReviewProperty::Transitions),
            pass(ReviewProperty::Transitions),
        ],
    )
    .await
    .expect_err("a duplicate property is refused");
    assert!(
        error.to_string().contains("listed twice"),
        "unexpected error: {error}"
    );
    assert!(requests(&server).await.is_empty());
}

#[tokio::test]
async fn a_blank_checklist_is_refused_rather_than_sent_empty() {
    use claria_bedrock::analysis::ReviewProperty;
    let (server, sdk, s3) = setup().await;
    let client_id = Uuid::new_v4();
    let report_id = seed_drafted_report(&s3, client_id, drafted_content()).await;

    let error = sweep_with_passes(
        &sdk,
        &s3,
        client_id,
        report_id,
        vec![pipeline::ReviewPassSelection {
            property: ReviewProperty::Transitions,
            body: Some("   \n  ".to_string()),
        }],
    )
    .await
    .expect_err("a blank checklist is refused");
    assert!(
        error.to_string().contains("no instructions"),
        "unexpected error: {error}"
    );
    assert!(requests(&server).await.is_empty());
}
