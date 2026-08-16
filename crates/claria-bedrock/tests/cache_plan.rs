//! `CachePlan` — the caller-decided placement of `cachePoint` blocks on a
//! report-family request, and what that placement puts on the wire.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_bedrock::{
    converse::{CachePlan, StopSignal},
    error::BedrockError,
    report::{ReportInputBudget, converse_report_with_tool_limit},
};
use claria_core::{
    model_id::{CacheTtlChoice, ModelCapabilities},
    models::report::{ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole},
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};

const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";
/// A family the capability table denies caching to.
const NON_CACHING_MODEL_ID: &str = "us.anthropic.claude-3-sonnet-20240229-v1:0";
/// A caching family that predates the extended one-hour tier.
const FIVE_MINUTE_ONLY_MODEL_ID: &str = "us.anthropic.claude-sonnet-4-20250514-v1:0";

fn caps(model_id: &str) -> ModelCapabilities {
    ModelCapabilities::for_id(model_id)
}

#[test]
fn the_default_plan_marks_the_system_policy_and_the_tail_at_five_minutes() {
    let plan = CachePlan::report_default(caps(MODEL_ID));
    assert!(plan.after_system);
    assert!(plan.tail);
    assert!(plan.after_blocks.is_empty());
    assert_eq!(plan.ttl, CacheTtlChoice::FiveMinutes);
    assert_eq!(plan.point_count(), 2);
    assert_eq!(plan.effective_ttl(), Some(CacheTtlChoice::FiveMinutes));
}

/// A model that cannot cache, or a user who turned caching off, must get the
/// pre-caching request shape — not a plan that quietly marks nothing.
#[test]
fn either_gate_saying_no_yields_a_plan_that_emits_nothing() {
    let intended = CachePlan::new(CacheTtlChoice::OneHour, true, true, vec![(0, 0)])
        .expect("three points is under the ceiling");

    let no_model = intended.clone().gated(caps(NON_CACHING_MODEL_ID), true);
    let no_config = intended.gated(caps(MODEL_ID), false);
    for plan in [no_model, no_config, CachePlan::default()] {
        assert_eq!(plan, CachePlan::disabled());
        assert!(!plan.is_enabled());
        assert_eq!(plan.point_count(), 0);
        // Uncached turns must price as uncached rather than claim a tier.
        assert_eq!(plan.effective_ttl(), None);
    }
}

/// Asking for a tier the family rejects downgrades rather than sending a TTL
/// the service would refuse — placement survives, the tier does not.
#[test]
fn an_unsupported_one_hour_tier_is_downgraded_not_dropped() {
    let plan = CachePlan::new(CacheTtlChoice::OneHour, true, true, Vec::new())
        .expect("two points")
        .gated(caps(FIVE_MINUTE_ONLY_MODEL_ID), true);
    assert_eq!(plan.ttl, CacheTtlChoice::FiveMinutes);
    assert!(plan.after_system && plan.tail);

    let kept = CachePlan::new(CacheTtlChoice::OneHour, true, true, Vec::new())
        .expect("two points")
        .gated(caps(MODEL_ID), true);
    assert_eq!(kept.ttl, CacheTtlChoice::OneHour);
}

/// Bedrock rejects a request carrying more than four cache points outright,
/// so the plan refuses to be built rather than failing at the wire.
#[test]
fn a_plan_over_the_cache_point_ceiling_is_refused() {
    let ok = CachePlan::new(
        CacheTtlChoice::FiveMinutes,
        true,
        true,
        vec![(0, 0), (1, 0)],
    );
    assert!(ok.is_ok());

    let error = CachePlan::new(
        CacheTtlChoice::FiveMinutes,
        true,
        true,
        vec![(0, 0), (1, 0), (2, 0)],
    )
    .expect_err("five points is over the ceiling");
    assert!(matches!(error, BedrockError::SchemaViolation(message) if message.contains("5")));
}

/// A branch of the drafting fan-out is one request that never comes back for
/// a second, so it marks the shared prefix and nothing else.
#[test]
fn the_parallel_draft_plan_marks_the_checkpoints_and_no_tail() {
    let plan = CachePlan::parallel_draft(caps(MODEL_ID), vec![(0, 1), (0, 2)])
        .expect("two points is under the ceiling");
    assert!(!plan.after_system);
    assert!(!plan.tail, "a single-shot branch reads no tail point back");
    assert_eq!(plan.after_blocks, vec![(0, 1), (0, 2)]);
    assert_eq!(plan.ttl, CacheTtlChoice::OneHour);
    assert_eq!(plan.point_count(), 2);
    assert_eq!(plan.effective_ttl(), Some(CacheTtlChoice::OneHour));
}

#[test]
fn the_parallel_draft_plan_follows_the_family_gates() {
    let downgraded = CachePlan::parallel_draft(caps(FIVE_MINUTE_ONLY_MODEL_ID), vec![(0, 1), (0, 2)])
        .expect("two points");
    assert_eq!(downgraded.ttl, CacheTtlChoice::FiveMinutes);
    assert_eq!(
        downgraded.after_blocks,
        vec![(0, 1), (0, 2)],
        "the tier is downgraded, the placement is not"
    );

    let disabled = CachePlan::parallel_draft(caps(NON_CACHING_MODEL_ID), vec![(0, 1), (0, 2)])
        .expect("two points");
    assert_eq!(disabled, CachePlan::disabled());
}

/// The warm branch counts once and hands the number on; a seeded budget
/// reports the seed back so the next sibling can be seeded from the same
/// count rather than taking one of its own.
#[test]
fn a_seeded_report_budget_carries_the_warm_branch_count() {
    let unseeded = ReportInputBudget::new(MODEL_ID);
    assert_eq!(
        unseeded.verified(),
        None,
        "a fresh budget has taken no count yet"
    );

    let seeded = ReportInputBudget::seeded(MODEL_ID, 41_000, 160_000);
    assert_eq!(seeded.verified(), Some((41_000, 160_000)));
}

/// A seeded budget estimates forward from its seed instead of calling
/// `CountTokens`: a sibling request that grew by a short kickoff sends
/// nothing extra to the service.
#[tokio::test]
async fn a_seeded_budget_sends_no_count_tokens_call() {
    let server = MockServer::spawn().await;
    script(&server).await;

    converse_report_with_tool_limit(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &[user_message("Draft")],
        8,
        &mut ReportInputBudget::seeded(MODEL_ID, 1_000, 4_000),
        claria_bedrock::converse::ModelTuning::default(),
        CachePlan::parallel_draft(caps(MODEL_ID), vec![(0, 0)]).expect("one point"),
        &StopSignal::new(),
    )
    .await
    .expect("converse");

    assert!(
        server
            .state
            .read()
            .await
            .bedrock_count_token_requests
            .is_empty(),
        "a seeded budget took a count of its own"
    );
}

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

fn user_message(text: &str) -> ReportProtocolMessage {
    ReportProtocolMessage {
        role: ReportProtocolRole::User,
        content: vec![
            ReportProtocolBlock::Text {
                text: text.to_string(),
            },
            ReportProtocolBlock::Text {
                text: "second block".to_string(),
            },
        ],
        created_at: "2026-08-01T12:00:00Z".parse().unwrap(),
    }
}

async fn script(server: &MockServer) {
    server
        .state
        .write()
        .await
        .bedrock_tool_responses
        .push(ScriptedBedrockResponse::success(serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Ready."}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 12, "outputTokens": 3}
        })));
}

/// A mid-message coordinate lands the cache point after the block it names,
/// not at the end of the message — that boundary is the whole point of
/// letting a caller place one.
#[tokio::test]
async fn an_after_block_coordinate_places_the_point_mid_message() {
    let server = MockServer::spawn().await;
    script(&server).await;

    let plan = CachePlan::new(CacheTtlChoice::OneHour, false, false, vec![(0, 0)])
        .expect("one point")
        .gated(caps(MODEL_ID), true);
    converse_report_with_tool_limit(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &[user_message("Draft")],
        8,
        &mut ReportInputBudget::new(MODEL_ID),
        claria_bedrock::converse::ModelTuning::default(),
        plan,
        &StopSignal::new(),
    )
    .await
    .expect("converse");

    let state = server.state.read().await;
    let request = &state.bedrock_tool_requests[0];
    let content = request["messages"][0]["content"]
        .as_array()
        .expect("blocks");
    // Text, the requested cache point, then the block that followed it.
    assert_eq!(content.len(), 3);
    assert_eq!(
        content[1]["cachePoint"],
        serde_json::json!({"type": "default", "ttl": "1h"})
    );
    assert_eq!(content[2]["text"], "second block");
    // No system point and no tail point: the plan asked for neither.
    assert!(request["system"].as_array().expect("system").len() == 1);
    assert!(content.last().unwrap().get("cachePoint").is_none());
    // Cache points never reach CountTokens — they do not change the count.
    assert!(
        !state.bedrock_count_token_requests[0]
            .to_string()
            .contains("cachePoint")
    );
}

/// A coordinate that does not exist is a failed plan, not a silently skipped
/// cache point — a plan that misses is a plan that stops caching.
#[tokio::test]
async fn a_coordinate_outside_the_conversation_is_an_error() {
    let server = MockServer::spawn().await;
    script(&server).await;

    let plan = CachePlan::new(CacheTtlChoice::FiveMinutes, false, false, vec![(0, 9)])
        .expect("one point")
        .gated(caps(MODEL_ID), true);
    let error = converse_report_with_tool_limit(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &[user_message("Draft")],
        8,
        &mut ReportInputBudget::new(MODEL_ID),
        claria_bedrock::converse::ModelTuning::default(),
        plan,
        &StopSignal::new(),
    )
    .await
    .expect_err("out-of-range coordinate");
    assert!(matches!(error, BedrockError::SchemaViolation(message) if message.contains("block 9")));
}
