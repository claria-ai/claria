use claria_core::models::{
    report::{
        ReportAuthoringTurn, ReportProtocolBlock, ReportProtocolMessage, ReportProtocolRole,
        ReportToolResultStatus, ReportWorkspace,
    },
    turn_usage::TurnUsage,
};
use claria_desktop::report_authoring::{
    ReportTimelineItemView, ReportToolActivityStatus, workspace_view,
};
use uuid::Uuid;

#[test]
fn failed_tool_activity_uses_safe_error_copy_without_success_summary() {
    let now = "2026-08-01T12:00:00Z".parse().expect("timestamp");
    let mut workspace = ReportWorkspace::new(Uuid::new_v4(), now);
    workspace
        .push_turn(ReportAuthoringTurn {
            id: Uuid::new_v4(),
            model_id: "us.anthropic.claude-sonnet-test".to_string(),
            messages: vec![
                ReportProtocolMessage {
                    role: ReportProtocolRole::User,
                    content: vec![ReportProtocolBlock::Text {
                        text: "Draft".to_string(),
                    }],
                    created_at: now,
                },
                ReportProtocolMessage {
                    role: ReportProtocolRole::Assistant,
                    content: vec![
                        ReportProtocolBlock::ToolUse {
                            tool_use_id: "read".to_string(),
                            name: "read_record_file".to_string(),
                            input: serde_json::json!({"filename": "record.txt"}),
                        },
                        ReportProtocolBlock::ToolUse {
                            tool_use_id: "proposal".to_string(),
                            name: "propose_report_changes".to_string(),
                            input: serde_json::json!({"summary": "bad", "operations": []}),
                        },
                    ],
                    created_at: now,
                },
                ReportProtocolMessage {
                    role: ReportProtocolRole::User,
                    content: vec![
                        ReportProtocolBlock::ToolResult {
                            tool_use_id: "read".to_string(),
                            status: ReportToolResultStatus::Error,
                            content: serde_json::json!({"error": {"code": "record_read_failed", "message": "The record is temporarily unavailable."}}),
                        },
                        ReportProtocolBlock::ToolResult {
                            tool_use_id: "proposal".to_string(),
                            status: ReportToolResultStatus::Error,
                            content: serde_json::json!({"error": {"code": "invalid_proposal", "message": "The proposal was invalid."}}),
                        },
                    ],
                    created_at: now,
                },
                ReportProtocolMessage {
                    role: ReportProtocolRole::Assistant,
                    content: vec![ReportProtocolBlock::Text {
                        text: "I could not complete that request.".to_string(),
                    }],
                    created_at: now,
                },
            ],
            usage: TurnUsage {
                model_id: "us.anthropic.claude-sonnet-test".to_string(),
                input_tokens: 10,
                output_tokens: 2,
                cache_read_input_tokens: 0,
                cache_write_input_tokens: 0,
                cost_usd: 0.0,
                pricing_version: 0,
            },
            usage_complete: true,
            converse_calls: 2,
            tool_uses: 2,
            created_at: now,
            completed_at: now,
        })
        .expect("valid turn");

    let view = workspace_view(&workspace);
    let activities: Vec<_> = view.turns[0]
        .timeline
        .iter()
        .filter_map(|item| match item {
            ReportTimelineItemView::ToolActivity {
                summary, status, ..
            } => Some((summary.as_str(), status)),
            _ => None,
        })
        .collect();
    assert_eq!(activities.len(), 2);
    assert_eq!(activities[0].0, "The record is temporarily unavailable.");
    assert!(matches!(activities[0].1, ReportToolActivityStatus::Failed));
    assert_eq!(activities[1].0, "The proposal was invalid.");
    assert!(!activities[1].0.contains("Staged"));
}
