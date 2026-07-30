use claria_core::models::{
    report::{
        ReportAuthoringTurn, ReportBlock, ReportContent, ReportProtocolBlock,
        ReportProtocolMessage, ReportProtocolRole, ReportSection, ReportTemplateImport,
        ReportTemplateWarning, ReportTemplateWarningCode, ReportToolResultStatus, ReportWorkspace,
    },
    turn_usage::TurnUsage,
};
use claria_desktop::report_authoring::{
    ReportBlockView, ReportTimelineItemView, ReportToolActivityStatus, workspace_view,
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
    assert!(view.turns[0].context_reads.is_empty());
    let raw_read = view.turns[0]
        .timeline
        .iter()
        .find_map(|item| match item {
            ReportTimelineItemView::ToolActivity {
                name,
                invocation_json,
                result_json,
                ..
            } if name == "read_record_file" => Some((invocation_json, result_json.as_deref())),
            _ => None,
        })
        .expect("raw read invocation");
    assert!(raw_read.0.contains("record.txt"));
    assert!(raw_read.0.contains("toolUseId"));
    assert!(
        raw_read
            .1
            .expect("raw result")
            .contains("record_read_failed")
    );
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

#[test]
fn successful_record_reads_are_exposed_as_context_metadata() {
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
                        text: "Read the intake".to_string(),
                    }],
                    created_at: now,
                },
                ReportProtocolMessage {
                    role: ReportProtocolRole::Assistant,
                    content: vec![ReportProtocolBlock::ToolUse {
                        tool_use_id: "read".to_string(),
                        name: "read_record_file".to_string(),
                        input: serde_json::json!({"filename": "intake.txt"}),
                    }],
                    created_at: now,
                },
                ReportProtocolMessage {
                    role: ReportProtocolRole::User,
                    content: vec![ReportProtocolBlock::ToolResult {
                        tool_use_id: "read".to_string(),
                        status: ReportToolResultStatus::Success,
                        content: serde_json::json!({
                            "offset": 12,
                            "returned_characters": 34,
                            "total_characters": 90
                        }),
                    }],
                    created_at: now,
                },
                ReportProtocolMessage {
                    role: ReportProtocolRole::Assistant,
                    content: vec![ReportProtocolBlock::Text {
                        text: "Done".to_string(),
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
            tool_uses: 1,
            created_at: now,
            completed_at: now,
        })
        .expect("valid turn");

    let view = workspace_view(&workspace);
    let read = &view.turns[0].context_reads[0];
    assert_eq!(read.filename, "intake.txt");
    assert_eq!(read.offset, 12);
    assert_eq!(read.returned_characters, 34);
    assert_eq!(read.total_characters, Some(90));
}

#[test]
fn table_and_template_review_metadata_are_exposed_without_source_identity() {
    let created = "2026-08-01T12:00:00Z".parse().expect("timestamp");
    let imported_at = "2026-08-01T12:01:00Z".parse().expect("timestamp");
    let mut workspace = ReportWorkspace::new(Uuid::new_v4(), created);
    workspace.draft = workspace
        .draft
        .replace_content(
            0,
            ReportContent {
                title: "Imported".to_string(),
                sections: vec![ReportSection {
                    id: Uuid::new_v4(),
                    heading: "Scores".to_string(),
                    blocks: vec![ReportBlock::Table {
                        rows: vec![
                            vec!["Measure".to_string(), "Score".to_string()],
                            vec!["{{name}}".to_string(), "87".to_string()],
                        ],
                        has_header: true,
                        column_widths: Some(vec![7_000, 3_000]),
                    }],
                }],
            },
            imported_at,
        )
        .expect("import revision");
    workspace.updated_at = imported_at;
    workspace.template_import = Some(ReportTemplateImport {
        source_sha256: "b".repeat(64),
        imported_revision: 1,
        imported_at,
        warnings: vec![ReportTemplateWarning {
            code: ReportTemplateWarningCode::ImagesOmitted,
            count: 2,
        }],
        reviewed_revision: None,
    });

    let view = workspace_view(&workspace);
    assert!(matches!(
        &view.draft.content.sections[0].blocks[0],
        ReportBlockView::Table {
            has_header: true,
            column_widths: Some(widths),
            ..
        } if widths == &[7_000, 3_000]
    ));
    let template = view.template_import.expect("template view");
    assert!(template.review_required);
    assert_eq!(template.placeholder_count, 1);
    assert_eq!(template.warnings[0].code, "images_omitted");
    let json = serde_json::to_string(&template).expect("view JSON");
    assert!(!json.contains("sha256"));
    assert!(!json.contains("filename"));
    assert!(!json.contains("path"));
}
