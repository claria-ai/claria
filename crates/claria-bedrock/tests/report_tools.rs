use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_bedrock::{
    chat::{CacheStrategy, ChatMessage, ChatRole},
    error::BedrockError,
    report::{
        PROPOSE_REPORT_CHANGES_TOOL, ReportStopReason, ReportToolRequest, converse_report,
        converse_report_with_tool_limit, decode_tool_request,
    },
};
use claria_core::models::report::{
    ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole, ReportToolResultStatus,
};
use claria_mock_aws::{
    state::{FoundationModel, ModelLifecycle, ScriptedBedrockResponse},
    testing::MockServer,
};

const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";

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
        content: vec![ReportProtocolBlock::Text {
            text: text.to_string(),
        }],
        created_at: "2026-08-01T12:00:00Z".parse().unwrap(),
    }
}

async fn script(server: &MockServer, responses: Vec<serde_json::Value>) {
    let mut state = server.state.write().await;
    state
        .bedrock_tool_responses
        .extend(responses.into_iter().map(ScriptedBedrockResponse::success));
}

#[tokio::test]
async fn request_carries_exactly_three_tools_without_choice_or_strict() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Ready."}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 12, "outputTokens": 3}
        })],
    )
    .await;

    let output = converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &[user_message("Help me draft a report")],
    )
    .await
    .expect("converse");
    assert_eq!(output.stop_reason, ReportStopReason::EndTurn);
    assert!(output.tool_calls.is_empty());
    assert_eq!(output.usage.expect("usage").input_tokens, 12);

    let state = server.state.read().await;
    assert_eq!(state.bedrock_tool_model_ids, vec![MODEL_ID]);
    assert_eq!(
        state.bedrock_count_token_model_ids,
        vec!["anthropic.claude-sonnet-test"]
    );
    assert_eq!(state.bedrock_tool_requests.len(), 1);
    assert_eq!(state.bedrock_count_token_requests.len(), 1);
    let request = &state.bedrock_tool_requests[0];
    let config = &request["toolConfig"];
    assert!(config.get("toolChoice").is_none());
    let tools = config["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 3);
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["toolSpec"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "list_record_files",
            "read_record_file",
            "propose_report_changes"
        ]
    );
    for tool in tools {
        assert!(tool["toolSpec"].get("strict").is_none());
        assert_eq!(tool["toolSpec"]["inputSchema"]["json"]["type"], "object");
    }
    let proposal = &tools[2]["toolSpec"]["inputSchema"]["json"];
    assert_eq!(
        proposal["properties"]["operations"]["maxItems"],
        claria_bedrock::report::MAX_PROPOSAL_OPERATIONS
    );
    // Every billed call reserves its enforced output budget on the wire.
    assert_eq!(
        request["inferenceConfig"]["maxTokens"],
        claria_bedrock::report::REPORT_OUTPUT_TOKEN_RESERVE
    );
}

#[tokio::test]
async fn nested_tool_input_round_trips_from_smithy_document() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [
                {"reasoningContent": {"reasoningText": {"text": "private reasoning", "signature": "signature-1"}}},
                {"text": ""},
                {
                "toolUse": {
                    "toolUseId": "proposal-1",
                    "name": "propose_report_changes",
                    "input": {
                        "summary": "Draft a summary",
                        "operations": [{
                            "kind": "add_section",
                            "position": 0,
                            "heading": "Summary",
                            "blocks": [
                                {"kind": "paragraph", "text": "Unicode: café — 東京"},
                                {"kind": "bullet_list", "items": ["One", "Two"]},
                                {"kind": "table", "rows": [["Measure", "Score"], ["Attention", "87"]], "has_header": true, "column_widths": [7000, 3000]}
                            ]
                        }]
                    }
                }
            }]}},
            "stopReason": "tool_use",
            "usage": {"inputTokens": 20, "outputTokens": 10}
        })],
    )
    .await;

    let output = converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &[user_message("Draft")],
    )
    .await
    .expect("converse");
    assert_eq!(output.stop_reason, ReportStopReason::ToolUse);
    assert_eq!(output.tool_calls.len(), 1);
    assert_eq!(output.message.content.len(), 2);
    assert!(matches!(
        &output.message.content[0],
        ReportProtocolBlock::ReasoningText { text, signature }
            if text == "private reasoning" && signature.as_deref() == Some("signature-1")
    ));
    let call = &output.tool_calls[0];
    assert_eq!(call.name, PROPOSE_REPORT_CHANGES_TOOL);
    assert_eq!(
        call.input["operations"][0]["blocks"][0]["text"],
        "Unicode: café — 東京"
    );
    let request = decode_tool_request(call).expect("typed request");
    let ReportToolRequest::ProposeReportChanges(request) = request else {
        panic!("proposal request");
    };
    let claria_bedrock::report::ReportProposalOperationRequest::AddSection { blocks, .. } =
        &request.operations[0]
    else {
        panic!("add section");
    };
    assert!(matches!(
        &blocks[2],
        claria_bedrock::report::ReportBlockRequest::Table {
            has_header: true,
            column_widths: Some(widths),
            ..
        } if widths == &[7_000, 3_000]
    ));
}

#[tokio::test]
async fn configured_tool_limit_rejects_oversized_model_responses() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "list-1", "name": "list_record_files", "input": {}}},
                {"toolUse": {"toolUseId": "list-2", "name": "list_record_files", "input": {}}}
            ]}},
            "stopReason": "tool_use"
        })],
    )
    .await;

    let error = converse_report_with_tool_limit(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &[user_message("Draft")],
        1,
        &mut claria_bedrock::report::ReportInputBudget::new(MODEL_ID),
    )
    .await
    .expect_err("configured tool limit");
    assert!(
        matches!(error, BedrockError::ResponseParse(ref message) if message.contains("configured maximum is 1"))
    );
}

#[tokio::test]
async fn tool_results_are_sent_as_correlated_json_blocks() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "I used the record."}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 30, "outputTokens": 5}
        })],
    )
    .await;

    let now = "2026-08-01T12:00:00Z".parse().unwrap();
    let messages = vec![
        user_message("Read the intake"),
        ReportProtocolMessage {
            role: ReportProtocolRole::Assistant,
            content: vec![
                ReportProtocolBlock::ReasoningText {
                    text: "private reasoning".to_string(),
                    signature: Some("signature-1".to_string()),
                },
                ReportProtocolBlock::ToolUse {
                    tool_use_id: "read-1".to_string(),
                    name: "read_record_file".to_string(),
                    input: serde_json::json!({"filename": "intake.txt"}),
                },
            ],
            created_at: now,
        },
        ReportProtocolMessage {
            role: ReportProtocolRole::User,
            content: vec![ReportProtocolBlock::ToolResult {
                tool_use_id: "read-1".to_string(),
                status: ReportToolResultStatus::Success,
                content: serde_json::json!({
                    "filename": "intake.txt",
                    "text": "nested",
                    "range": {"offset": 0, "next_offset": null}
                }),
            }],
            created_at: now,
        },
    ];
    converse_report(&sdk_config(&server.endpoint), MODEL_ID, "System", &messages)
        .await
        .expect("converse");

    let state = server.state.read().await;
    let request = &state.bedrock_tool_requests[0];
    assert_eq!(
        request["messages"][1]["content"][0]["reasoningContent"]["reasoningText"]["signature"],
        "signature-1"
    );
    let sent = &request["messages"][2]["content"][0]["toolResult"];
    assert_eq!(sent["toolUseId"], "read-1");
    assert_eq!(sent["status"], "success");
    assert_eq!(sent["content"][0]["json"]["range"]["offset"], 0);
    assert!(sent["content"][0]["json"]["range"]["next_offset"].is_null());
}

#[tokio::test]
async fn duplicate_and_empty_tool_ids_are_rejected() {
    for response in [
        serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "same", "name": "list_record_files", "input": {}}},
                {"toolUse": {"toolUseId": "same", "name": "list_record_files", "input": {}}}
            ]}},
            "stopReason": "tool_use"
        }),
        serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "", "name": "list_record_files", "input": {}}}
            ]}},
            "stopReason": "tool_use"
        }),
    ] {
        let server = MockServer::spawn().await;
        script(&server, vec![response]).await;
        let error = converse_report(
            &sdk_config(&server.endpoint),
            MODEL_ID,
            "System",
            &[user_message("Draft")],
        )
        .await
        .unwrap_err();
        assert!(matches!(error, BedrockError::ResponseParse(_)));
    }
}

/// A stop reason that disagrees with tool presence is surfaced as data — the
/// report loop owns the one corrective round — instead of aborting here.
#[tokio::test]
async fn stop_tool_mismatch_is_surfaced_not_rejected() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "id", "name": "list_record_files", "input": {}}}
            ]}},
            "stopReason": "end_turn"
        })],
    )
    .await;
    let output = converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &[user_message("Draft")],
    )
    .await
    .expect("mismatch passes through");
    assert_eq!(output.stop_reason, ReportStopReason::EndTurn);
    assert_eq!(output.tool_calls.len(), 1);
    assert!(output.stop_tool_mismatch());
}

#[tokio::test]
async fn malformed_and_unknown_requests_decode_to_schema_errors() {
    for (name, input) in [
        ("unknown_tool", serde_json::json!({})),
        (
            "read_record_file",
            serde_json::json!({"filename": "x.txt", "invented": true}),
        ),
        (
            "propose_report_changes",
            serde_json::json!({"summary": "x", "operations": []}),
        ),
    ] {
        let call = claria_bedrock::report::ReportToolCall {
            tool_use_id: "id".to_string(),
            name: name.to_string(),
            input,
        };
        let result = decode_tool_request(&call);
        if name == "propose_report_changes" {
            // Length limits are enforced by the report domain/controller after
            // deserialization; this shape itself is typed correctly.
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(BedrockError::SchemaViolation(_))));
        }
    }
}

#[tokio::test]
async fn validation_errors_preserve_structured_service_metadata() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_tool_responses
        .push(ScriptedBedrockResponse::error(
            400,
            serde_json::json!({
                "__type": "ValidationException",
                "message": "This model does not support tool use"
            }),
        ));

    let error = converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &[user_message("Draft")],
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            BedrockError::Service {
                status: Some(400),
                ref code,
                ..
            } if code == "ValidationException"
        ),
        "{error}"
    );
}

#[tokio::test]
async fn omitted_usage_is_preserved_as_missing() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Done"}]}},
            "stopReason": "end_turn",
            "usage": null
        })],
    )
    .await;
    let output = converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &[user_message("Draft")],
    )
    .await
    .expect("converse");
    assert!(output.usage.is_none());
}

#[tokio::test]
async fn unsupported_selected_tokenizer_falls_back_to_active_haiku() {
    let server = MockServer::spawn().await;
    let selected_model = "us.anthropic.claude-opus-4-7";
    {
        let mut state = server.state.write().await;
        state
            .bedrock_count_tokens_unsupported_models
            .insert("anthropic.claude-opus-4-7".to_string());
        state.foundation_models.push(FoundationModel {
            model_id: "anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
            model_name: "Claude Haiku 4.5".to_string(),
            provider_name: "Anthropic".to_string(),
            model_lifecycle: ModelLifecycle {
                status: "ACTIVE".to_string(),
            },
        });
    }
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Done"}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 10, "outputTokens": 2}
        })],
    )
    .await;

    converse_report(
        &sdk_config(&server.endpoint),
        selected_model,
        "System",
        &[user_message("Draft")],
    )
    .await
    .expect("fallback count and converse");

    let state = server.state.read().await;
    assert_eq!(
        state.bedrock_count_token_model_ids,
        vec![
            "anthropic.claude-opus-4-7",
            "anthropic.claude-haiku-4-5-20251001-v1:0"
        ]
    );
    assert_eq!(state.bedrock_tool_model_ids, vec![selected_model]);
}

#[tokio::test]
async fn exact_count_tokens_budget_is_enforced_before_converse() {
    let server = MockServer::spawn().await;
    server.state.write().await.bedrock_count_tokens_override = Some(199_000);
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "must not run"}]}},
            "stopReason": "end_turn"
        })],
    )
    .await;

    let error = converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &[user_message("Draft")],
    )
    .await
    .expect_err("context budget");
    assert!(matches!(error, BedrockError::ContextBudgetExceeded { .. }));
    let state = server.state.read().await;
    assert_eq!(state.bedrock_count_token_requests.len(), 1);
    assert!(state.bedrock_tool_requests.is_empty());
    assert_eq!(state.bedrock_tool_responses.len(), 1);
}

#[tokio::test]
async fn cache_points_follow_the_capability_table() {
    // Caching-capable model: cache points at the system boundary and the
    // conversation tail; the CountTokens request stays cache-free.
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Ready."}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 12, "outputTokens": 3}
        })],
    )
    .await;
    converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &[user_message("Draft")],
    )
    .await
    .expect("converse");
    {
        let state = server.state.read().await;
        let request = &state.bedrock_tool_requests[0];
        // Writer cache points stay on the default 5-minute TTL — the full
        // block shape is asserted so a `ttl` field can never sneak in.
        assert_eq!(
            request["system"][1]["cachePoint"],
            serde_json::json!({"type": "default"})
        );
        let tail = request["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_array()
            .unwrap();
        assert_eq!(
            tail.last().unwrap()["cachePoint"],
            serde_json::json!({"type": "default"}),
            "conversation tail must end with a ttl-less cache point"
        );
        assert!(
            !state.bedrock_count_token_requests[0]
                .to_string()
                .contains("cachePoint")
        );
    }

    // A Claude family on the caching denylist gets no cache points.
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Ready."}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 12, "outputTokens": 3}
        })],
    )
    .await;
    converse_report(
        &sdk_config(&server.endpoint),
        "us.anthropic.claude-3-sonnet-20240229-v1:0",
        "System prompt",
        &[user_message("Draft")],
    )
    .await
    .expect("converse");
    let state = server.state.read().await;
    assert!(
        !state.bedrock_tool_requests[0]
            .to_string()
            .contains("cachePoint")
    );
}

#[test]
fn tool_capability_and_context_limits_are_model_specific() {
    assert!(claria_bedrock::report::model_supports_report_tools(
        MODEL_ID
    ));
    assert!(!claria_bedrock::report::model_supports_report_tools(
        "amazon.titan-text"
    ));
    assert_eq!(
        claria_bedrock::report::report_model_context_tokens("us.anthropic.claude:48k"),
        48_000
    );
    assert_eq!(
        claria_bedrock::report::report_model_context_tokens(MODEL_ID),
        200_000
    );
}

#[tokio::test]
async fn ordinary_chat_still_sends_no_tool_configuration() {
    let server = MockServer::spawn().await;
    let messages = vec![ChatMessage {
        role: ChatRole::User,
        content: "Hello".to_string(),
    }];
    let outcome = claria_bedrock::chat::chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &messages,
        CacheStrategy::disabled(),
        |_| {},
    )
    .await
    .expect("ordinary chat");
    assert!(!outcome.text.is_empty());

    let state = server.state.read().await;
    assert!(state.bedrock_tool_requests.is_empty());
    assert!(state.bedrock_tool_model_ids.is_empty());
}
