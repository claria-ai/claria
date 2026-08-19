use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_bedrockruntime::types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStart, ContentBlockStartEvent,
    ConverseStreamOutput, MessageStopEvent, StopReason, ToolUseBlockDelta, ToolUseBlockStart,
};
use claria_bedrock::{
    chat::{ChatMessage, ChatRole},
    converse::{StopSignal, StreamBounds},
    error::BedrockError,
    report::{
        DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE, PROPOSE_REPORT_CHANGES_TOOL, ReportInputBudget,
        ReportStopReason, ReportStreamCollector, ReportToolRequest,
        converse_full_report_with_tool_limit, converse_report, converse_report_with_tool_limit,
        decode_tool_request,
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

/// The placement every writer turn uses today, so a test that is not about
/// caching still sends the shape production sends.
fn default_cache_plan() -> claria_bedrock::converse::CachePlan {
    claria_bedrock::converse::CachePlan::report_default(
        claria_core::model_id::ModelCapabilities::for_id(MODEL_ID),
    )
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
    assert_eq!(tools.len(), 4);
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["toolSpec"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "list_record_files",
            "read_record_file",
            "read_report_section",
            "propose_report_changes"
        ]
    );
    for tool in tools {
        assert!(tool["toolSpec"].get("strict").is_none());
        assert_eq!(tool["toolSpec"]["inputSchema"]["json"]["type"], "object");
    }
    // The section reader is a contract, not a caption: it names where a
    // copyable ID comes from, when it is mandatory, and whose budget it spends.
    let section_reader = &tools[2]["toolSpec"];
    assert_eq!(
        section_reader["inputSchema"]["json"]["required"],
        serde_json::json!(["section_id"])
    );
    assert_eq!(
        section_reader["inputSchema"]["json"]["properties"]["section_id"]["minLength"],
        36
    );
    let section_reader_description = section_reader["description"]
        .as_str()
        .expect("section reader description");
    assert!(section_reader_description.contains("document_outline"));
    assert!(section_reader_description.contains("target_sections"));
    assert!(section_reader_description.contains("before proposing a replace_section"));
    assert!(section_reader_description.contains("48000-character per-turn budget"));

    let proposal = &tools[3]["toolSpec"]["inputSchema"]["json"];
    assert_eq!(
        proposal["properties"]["operations"]["maxItems"],
        claria_bedrock::report::MAX_PROPOSAL_OPERATIONS
    );
    // Every billed call reserves its enforced output budget on the wire.
    assert_eq!(
        request["inferenceConfig"]["maxTokens"],
        claria_bedrock::report::DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE
    );

    // GOLDEN wire shape: literal values so a silent constant change fails
    // CI. Cutting these caps (25 → 10 operations) and the output ceiling
    // (32k → 8k) — and telling the model to keep proposals small — caused
    // the v0.20–v0.23 "robotic, choppy writing" regression. Changing them
    // is a deliberate quality decision requiring a golden update.
    assert_eq!(proposal["properties"]["operations"]["maxItems"], 25);
    assert_eq!(request["inferenceConfig"]["maxTokens"], 32_768);
    let description = tools[3]["toolSpec"]["description"]
        .as_str()
        .expect("proposal description");
    assert!(description.contains("completed tool calls still execute"));
    // Section IDs are copied from the outline the targeted context actually
    // carries — naming a payload the model was never given invites invention.
    assert!(description.contains("document_outline"));
    assert!(!description.contains("accepted_report"));
    assert!(!description.contains("keep one proposal small"));
    assert!(!description.to_ascii_lowercase().contains("output tokens:"));

    // Untuned requests stay byte-identical to the pre-tuning shape.
    assert!(request.get("additionalModelRequestFields").is_none());
    assert!(request["inferenceConfig"].get("temperature").is_none());
}

#[tokio::test]
async fn full_draft_request_exposes_only_atomic_candidate_tools() {
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

    converse_full_report_with_tool_limit(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "Full draft policy",
        &[user_message("Complete record snapshot")],
        12,
        &mut ReportInputBudget::new(MODEL_ID, DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE),
        claria_bedrock::converse::ModelTuning::default(),
        default_cache_plan(),
        StreamBounds::conversational(),
        &StopSignal::new(),
    )
    .await
    .expect("full-draft converse");

    let state = server.state.read().await;
    let tools = state.bedrock_tool_requests[0]["toolConfig"]["tools"]
        .as_array()
        .expect("tools");
    let names = tools
        .iter()
        .map(|tool| tool["toolSpec"]["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "set_full_draft_title",
            "write_full_draft_section",
            "skip_full_draft_section",
            "mark_section_failed",
            "finish_full_draft"
        ]
    );
    let skip = &tools[2]["toolSpec"]["inputSchema"]["json"];
    assert_eq!(skip["required"], serde_json::json!(["section_id"]));
    let skip_description = tools[2]["toolSpec"]["description"]
        .as_str()
        .expect("skip description");
    assert!(skip_description.contains("Skip only sections the plan marks skip"));
    assert!(skip_description.contains("never to shorten the job"));

    // Failing a section is a declaration about the records, not a shortcut,
    // and the description has to say so where the model reads it.
    let failed = &tools[3]["toolSpec"]["inputSchema"]["json"];
    assert_eq!(
        failed["required"],
        serde_json::json!(["section_id", "reason"])
    );
    assert_eq!(
        failed["properties"]["reason"]["maxLength"],
        claria_bedrock::report::MAX_SECTION_ERROR_CHARACTERS
    );
    let failed_description = tools[3]["toolSpec"]["description"]
        .as_str()
        .expect("mark_section_failed description");
    assert!(failed_description.contains("after a genuine attempt"));
    assert!(failed_description.contains("Never use this to shorten the job"));

    let section = &tools[1]["toolSpec"]["inputSchema"]["json"];
    assert_eq!(
        section["properties"]["blocks"]["maxItems"],
        claria_bedrock::report::MAX_SECTION_BLOCKS
    );
    // GOLDEN: literal value — lowering the per-section block ceiling forced
    // multi-turn splitting of long clinical sections (v0.20–v0.23).
    assert_eq!(section["properties"]["blocks"]["maxItems"], 200);
    assert!(section["properties"]["section_id"].get("anyOf").is_some());
    // Citations are optional, bounded, and must be verbatim spans long enough
    // for the completion gate to resolve later.
    let citations = &section["properties"]["citations"];
    assert!(
        !section["required"]
            .as_array()
            .expect("required")
            .contains(&serde_json::json!("citations"))
    );
    assert_eq!(
        citations["maxItems"],
        claria_bedrock::report::MAX_SECTION_CITATIONS
    );
    assert_eq!(
        citations["items"]["properties"]["quote"]["minLength"],
        claria_bedrock::report::MIN_CITATION_QUOTE_CHARACTERS
    );
    assert_eq!(
        citations["items"]["properties"]["quote"]["maxLength"],
        claria_bedrock::report::MAX_CITATION_QUOTE_CHARACTERS
    );
    let write_description = tools[1]["toolSpec"]["description"]
        .as_str()
        .expect("write description");
    assert!(write_description.contains("Work through plan_context"));
    assert!(write_description.contains("ONE section per"));
}

#[tokio::test]
async fn model_tuning_shapes_the_wire_request() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Tuned."}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 4, "outputTokens": 2}
        })],
    )
    .await;

    converse_report_with_tool_limit(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "Policy",
        &[user_message("Hello")],
        8,
        &mut ReportInputBudget::new(MODEL_ID, DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE),
        claria_bedrock::converse::ModelTuning {
            adaptive_thinking: true,
            effort: Some(claria_bedrock::converse::EffortLevel::High),
            temperature: Some(0.3),
        },
        default_cache_plan(),
        StreamBounds::conversational(),
        &StopSignal::new(),
    )
    .await
    .expect("tuned converse");

    let state = server.state.read().await;
    let request = &state.bedrock_tool_requests[0];
    assert_eq!(
        request["additionalModelRequestFields"],
        serde_json::json!({
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": "high"},
        })
    );
    // The SDK carries temperature as f32; compare with a tolerance.
    let temperature = request["inferenceConfig"]["temperature"]
        .as_f64()
        .expect("temperature");
    assert!((temperature - 0.3).abs() < 1e-6, "{temperature}");
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

/// The writer's output ceiling is far above the point where a non-streaming
/// Converse call risks an HTTP timeout mid-generation, so the transport is
/// part of the contract rather than an implementation detail.
#[tokio::test]
async fn writer_calls_the_streaming_endpoint() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "Drafted."}]}},
            "stopReason": "end_turn",
            "usage": {"inputTokens": 20, "outputTokens": 10}
        })],
    )
    .await;

    converse_report(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System",
        &[user_message("Draft")],
    )
    .await
    .expect("converse");

    let state = server.state.read().await;
    assert_eq!(
        state.bedrock_stream_request_count, 1,
        "the writer turn did not go through ConverseStream"
    );
    assert_eq!(state.bedrock_tool_requests.len(), 1);
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
        &mut claria_bedrock::report::ReportInputBudget::new(
            MODEL_ID,
            DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE,
        ),
        claria_bedrock::converse::ModelTuning::default(),
        default_cache_plan(),
        StreamBounds::conversational(),
        &StopSignal::new(),
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

/// `max_tokens` beside complete tool calls means the response was truncated
/// after the calls were emitted — the loop executes them and continues, so
/// this must not read as a stop/tool inconsistency.
#[tokio::test]
async fn max_tokens_beside_tool_calls_is_truncation_not_mismatch() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [
                {"toolUse": {"toolUseId": "id", "name": "list_record_files", "input": {}}}
            ]}},
            "stopReason": "max_tokens"
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
    .expect("truncated tool response passes through");
    assert_eq!(output.stop_reason, ReportStopReason::MaxTokens);
    assert_eq!(output.tool_calls.len(), 1);
    assert!(!output.stop_tool_mismatch());
}

/// A truncated response with no tool calls stays out of the corrective-round
/// path too — the loop's `max_tokens` salvage/failure arms own it.
#[tokio::test]
async fn max_tokens_without_tool_calls_is_not_a_mismatch() {
    let server = MockServer::spawn().await;
    script(
        &server,
        vec![serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "An unfinished"}]}},
            "stopReason": "max_tokens"
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
    .expect("truncated text response passes through");
    assert!(!output.stop_tool_mismatch());
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
        claria_bedrock::chat::ChatStreamOptions::default(),
        |_| {},
    )
    .await
    .expect("ordinary chat");
    assert!(!outcome.text.is_empty());

    let state = server.state.read().await;
    assert!(state.bedrock_tool_requests.is_empty());
    assert!(state.bedrock_tool_model_ids.is_empty());
}

// ── The report/analysis stream collector ────────────────────────────────────

fn tool_start_event(index: usize, tool_use_id: &str, name: &str) -> ConverseStreamOutput {
    ConverseStreamOutput::ContentBlockStart(
        ContentBlockStartEvent::builder()
            .content_block_index(index as i32)
            .start(ContentBlockStart::ToolUse(
                ToolUseBlockStart::builder()
                    .tool_use_id(tool_use_id)
                    .name(name)
                    .build()
                    .expect("tool use start"),
            ))
            .build()
            .expect("content block start"),
    )
}

fn tool_input_event(index: usize, input: &str) -> ConverseStreamOutput {
    ConverseStreamOutput::ContentBlockDelta(
        ContentBlockDeltaEvent::builder()
            .content_block_index(index as i32)
            .delta(ContentBlockDelta::ToolUse(
                ToolUseBlockDelta::builder()
                    .input(input)
                    .build()
                    .expect("tool use delta"),
            ))
            .build()
            .expect("content block delta"),
    )
}

fn text_event(index: usize, text: &str) -> ConverseStreamOutput {
    ConverseStreamOutput::ContentBlockDelta(
        ContentBlockDeltaEvent::builder()
            .content_block_index(index as i32)
            .delta(ContentBlockDelta::Text(text.to_string()))
            .build()
            .expect("content block delta"),
    )
}

/// Partial tool input is handed back fragment by fragment so a caller can
/// report progress on a structured answer that takes minutes, while the
/// joined document stays the only thing anyone parses.
#[test]
fn collector_hands_back_tool_input_as_it_grows() {
    let mut collector = ReportStreamCollector::default();
    assert!(collector.absorb(text_event(0, "Planning.")).is_none());
    assert!(
        collector
            .absorb(tool_start_event(1, "plan-1", "submit_section_plan"))
            .is_none()
    );

    let fragments = [
        "{\"rows\":[{\"section",
        "_id\":\"a\"},{\"section_id\"",
        ":\"b\"}]}",
    ];
    let mut seen = String::new();
    for fragment in fragments {
        assert_eq!(
            collector.absorb(tool_input_event(1, fragment)).as_deref(),
            Some(fragment)
        );
        seen.push_str(fragment);
    }
    assert!(
        collector
            .absorb(ConverseStreamOutput::MessageStop(
                MessageStopEvent::builder()
                    .stop_reason(StopReason::ToolUse)
                    .build()
                    .expect("message stop"),
            ))
            .is_none()
    );

    let (content, stop_reason, _usage) = collector.finish().expect("complete stream");
    assert_eq!(stop_reason, StopReason::ToolUse);
    let tool = content
        .iter()
        .find_map(|block| match block {
            aws_sdk_bedrockruntime::types::ContentBlock::ToolUse(tool) => Some(tool),
            _ => None,
        })
        .expect("the forced tool call");
    assert_eq!(tool.name(), "submit_section_plan");
    assert_eq!(tool.tool_use_id(), "plan-1");
    // The fragments the caller was shown are the answer, split: two rows are
    // countable in them, and they are only JSON once joined — which is why a
    // fragment is worth a progress line and nothing more.
    assert_eq!(seen.matches("\"section_id\"").count(), 2);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&seen).expect("joined tool input JSON"),
        serde_json::json!({"rows": [{"section_id": "a"}, {"section_id": "b"}]})
    );
    assert!(serde_json::from_str::<serde_json::Value>(fragments[0]).is_err());
}

#[tokio::test]
async fn a_raised_output_ceiling_reaches_the_wire_and_the_input_budget() {
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

    let raised = 64_000;
    converse_full_report_with_tool_limit(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "Full draft policy",
        &[user_message("Complete record snapshot")],
        12,
        &mut ReportInputBudget::new(MODEL_ID, raised),
        claria_bedrock::converse::ModelTuning::default(),
        default_cache_plan(),
        StreamBounds::conversational(),
        &StopSignal::new(),
    )
    .await
    .expect("full-draft converse");

    let state = server.state.read().await;
    assert_eq!(
        state.bedrock_tool_requests[0]["inferenceConfig"]["maxTokens"], raised,
        "the configured ceiling is what the model is actually allowed"
    );
    // The reserve is enforced on both sides: what the wire allows is what
    // the input allowance held back.
    assert_eq!(
        claria_bedrock::report::report_input_token_budget(MODEL_ID, raised),
        claria_bedrock::report::report_model_context_tokens(MODEL_ID) - raised
    );
}

#[test]
fn an_output_ceiling_can_never_eat_the_whole_context_window() {
    // Half the window is the cap. Past it there is nothing left to ask the
    // question with, and a budget of nothing fails every call on the input
    // side rather than doing what the setting was raised for.
    let window = claria_bedrock::report::report_model_context_tokens(MODEL_ID);
    let effective = claria_bedrock::report::effective_output_reserve(window, u32::MAX);

    assert!(effective <= window / 2);
    assert!(claria_bedrock::report::report_input_token_budget(MODEL_ID, u32::MAX) > 0);

    // A tiny window is bound by the same rule, not by the floor.
    assert_eq!(
        claria_bedrock::report::effective_output_reserve(10_000, 32_768),
        5_000
    );
    // A ceiling inside the range passes through untouched.
    assert_eq!(
        claria_bedrock::report::effective_output_reserve(200_000, 32_768),
        32_768
    );
}
