use claria_core::models::{
    report::{
        AuthorshipKind, MAX_PARAGRAPH_CHARACTERS, MAX_REPORT_TEXT_CHARACTERS, MAX_REPORT_TURNS,
        REPORT_WORKSPACE_SCHEMA_VERSION, ReportAuthoringTurn, ReportBlock, ReportContent,
        ReportOperation, ReportProposal, ReportProtocolBlock, ReportProtocolMessage,
        ReportProtocolRole, ReportSection, ReportTemplateImport, ReportTemplateWarning,
        ReportTemplateWarningCode, ReportWorkspace, SectionAuthorship, decode_report_workspace,
        prompt_content_view, report_template_placeholder_count, validate_report_content,
    },
    turn_usage::TurnUsage,
};
use jiff::Timestamp;
use uuid::Uuid;

fn timestamp(value: &str) -> Timestamp {
    value.parse().expect("timestamp")
}

fn workspace() -> ReportWorkspace {
    ReportWorkspace::new(Uuid::new_v4(), timestamp("2026-08-01T12:00:00Z"))
}

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph {
        text: text.to_string(),
    }
}

fn section(id: Uuid, heading: &str, text: &str) -> ReportSection {
    ReportSection {
        skipped: false,
        template_blocks: None,
        authorship: None,
        id,
        heading: heading.to_string(),
        blocks: vec![paragraph(text)],
    }
}

fn proposal(workspace: &ReportWorkspace, operations: Vec<ReportOperation>) -> ReportProposal {
    ReportProposal {
        id: Uuid::new_v4(),
        report_id: workspace.report_id,
        base_revision: workspace.draft.revision,
        model_id: "us.anthropic.claude-sonnet-test".to_string(),
        summary: "Add the reviewed assessment".to_string(),
        proposed_content: workspace
            .draft
            .preview(&operations)
            .expect("valid operations"),
        operations,
        tool_use_id: "tool-1".to_string(),
        created_at: timestamp("2026-08-01T12:01:00Z"),
    }
}

#[test]
fn new_workspace_has_versioned_empty_accepted_draft() {
    let workspace = workspace();
    assert_eq!(workspace.schema_version, REPORT_WORKSPACE_SCHEMA_VERSION);
    assert_eq!(workspace.session_name, "Writer Session (1)");
    assert_eq!(workspace.draft.revision, 0);
    assert_eq!(workspace.draft.content.title, "Untitled report");
    assert!(workspace.draft.content.sections.is_empty());
    assert!(workspace.session.pending_proposal.is_none());
    assert_eq!(workspace.session.last_agent_revision, None);
    assert_eq!(workspace.session.last_export, None);
    workspace.validate().expect("valid workspace");
}

#[test]
fn legacy_workspace_defaults_writing_queue_and_export_status() {
    let workspace = workspace();
    let mut value = serde_json::to_value(&workspace).expect("serialize workspace");
    value["schema_version"] = serde_json::json!(1);
    value
        .as_object_mut()
        .expect("workspace object")
        .remove("template_import");
    value
        .as_object_mut()
        .expect("workspace object")
        .remove("session_name");
    let session = value["session"].as_object_mut().expect("session object");
    session.remove("last_agent_revision");
    session.remove("last_export");
    let bytes = serde_json::to_vec(&value).expect("legacy bytes");

    let decoded = decode_report_workspace(&bytes).expect("decode legacy workspace");
    assert_eq!(decoded.schema_version, REPORT_WORKSPACE_SCHEMA_VERSION);
    assert_eq!(decoded.session.last_agent_revision, None);
    assert_eq!(decoded.session.last_export, None);
    assert_eq!(decoded.template_import, None);
    assert_eq!(decoded.session_name, "Writer Session (1)");
}

#[test]
fn version_three_template_metadata_defaults_managed_identity() {
    let mut workspace = workspace();
    workspace.draft = workspace
        .draft
        .replace_content(
            0,
            ReportContent {
                title: "Imported".to_string(),
                sections: vec![],
            },
            timestamp("2026-08-01T12:01:00Z"),
        )
        .expect("import revision");
    workspace.updated_at = timestamp("2026-08-01T12:01:00Z");
    workspace.template_import = Some(ReportTemplateImport {
        source_sha256: "a".repeat(64),
        writer_template_id: None,
        writer_template_name: None,
        imported_revision: 1,
        imported_at: timestamp("2026-08-01T12:01:00Z"),
        warnings: vec![],
        reviewed_revision: Some(1),
    });
    let mut value = serde_json::to_value(workspace).expect("workspace JSON");
    value["schema_version"] = serde_json::json!(3);
    let template = value["template_import"]
        .as_object_mut()
        .expect("template metadata");
    template.remove("writer_template_id");
    template.remove("writer_template_name");

    let decoded = decode_report_workspace(&serde_json::to_vec(&value).unwrap())
        .expect("decode version three workspace");
    let template = decoded.template_import.expect("template metadata");
    assert_eq!(decoded.schema_version, REPORT_WORKSPACE_SCHEMA_VERSION);
    assert_eq!(template.writer_template_id, None);
    assert_eq!(template.writer_template_name, None);
}

#[test]
fn version_four_turns_default_preloaded_record_provenance() {
    let mut workspace = workspace();
    workspace
        .push_turn(complete_turn(0, 0))
        .expect("complete turn");
    let mut value = serde_json::to_value(workspace).expect("workspace JSON");
    value["schema_version"] = serde_json::json!(4);
    value["session"]["turns"][0]
        .as_object_mut()
        .expect("turn")
        .remove("record_context_files");

    let decoded = decode_report_workspace(&serde_json::to_vec(&value).unwrap())
        .expect("decode version four workspace");
    assert_eq!(decoded.schema_version, REPORT_WORKSPACE_SCHEMA_VERSION);
    assert!(decoded.session.turns[0].record_context_files.is_empty());
}

#[test]
fn version_five_workspaces_migrate_and_sections_default_to_written() {
    let mut workspace = workspace();
    workspace.draft.content.sections = vec![section(Uuid::new_v4(), "History", "Documented.")];
    let mut value = serde_json::to_value(workspace).expect("workspace JSON");
    value["schema_version"] = serde_json::json!(5);
    value["draft"]["content"]["sections"][0]
        .as_object_mut()
        .expect("section")
        .remove("skipped");

    let decoded = decode_report_workspace(&serde_json::to_vec(&value).unwrap())
        .expect("decode version five workspace");
    assert_eq!(decoded.schema_version, REPORT_WORKSPACE_SCHEMA_VERSION);
    assert!(!decoded.draft.content.sections[0].skipped);
}

#[test]
fn skipped_sections_must_be_empty_and_pass_with_bare_headings() {
    let deferred = ReportSection {
        id: Uuid::new_v4(),
        heading: "Summary and Clinical Interpretation".to_string(),
        blocks: Vec::new(),
        skipped: true,
        template_blocks: None,
        authorship: None,
    };
    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![deferred.clone()],
    };
    validate_report_content(&content).expect("bare deferred section is valid");

    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![ReportSection {
            blocks: vec![paragraph("Contradiction")],
            ..deferred
        }],
    };
    let error = validate_report_content(&content).unwrap_err();
    assert!(error.to_string().contains("skipped section"));
}

#[test]
fn replacing_a_deferred_section_un_defers_it() {
    let mut workspace = workspace();
    let deferred_id = Uuid::new_v4();
    workspace.draft.content.sections = vec![ReportSection {
        id: deferred_id,
        heading: "Observations".to_string(),
        blocks: Vec::new(),
        skipped: true,
        template_blocks: None,
        authorship: None,
    }];

    let replaced = workspace
        .draft
        .preview(&[ReportOperation::ReplaceSection {
            section_id: deferred_id,
            heading: "Observations".to_string(),
            blocks: vec![paragraph("Attended two sessions.")],
        }])
        .expect("replace deferred section");
    assert!(!replaced.sections[0].skipped);
    assert_eq!(
        replaced.sections[0].blocks,
        vec![paragraph("Attended two sessions.")]
    );
}

#[test]
fn version_six_workspaces_migrate_and_sections_default_to_no_template_copy() {
    let mut workspace = workspace();
    workspace.draft.content.sections = vec![section(Uuid::new_v4(), "History", "Documented.")];
    let mut value = serde_json::to_value(workspace).expect("workspace JSON");
    value["schema_version"] = serde_json::json!(6);
    value
        .as_object_mut()
        .expect("workspace object")
        .remove("active_run_id");
    let section = value["draft"]["content"]["sections"][0]
        .as_object_mut()
        .expect("section");
    section.remove("template_blocks");
    section.remove("authorship");

    let decoded = decode_report_workspace(&serde_json::to_vec(&value).unwrap())
        .expect("decode version six workspace");
    assert_eq!(decoded.schema_version, REPORT_WORKSPACE_SCHEMA_VERSION);
    assert_eq!(decoded.active_run_id, None);
    assert_eq!(decoded.draft.content.sections[0].template_blocks, None);
    assert_eq!(decoded.draft.content.sections[0].authorship, None);
}

#[test]
fn skipped_sections_keep_a_template_copy_but_no_written_body() {
    let deferred = ReportSection {
        id: Uuid::new_v4(),
        heading: "Summary and Clinical Interpretation".to_string(),
        blocks: Vec::new(),
        skipped: true,
        template_blocks: Some(vec![paragraph("Summarize the findings here.")]),
        authorship: None,
    };
    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![deferred.clone()],
    };
    validate_report_content(&content).expect("a deferred section may keep its template copy");

    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![ReportSection {
            blocks: vec![paragraph("Contradiction")],
            ..deferred.clone()
        }],
    };
    let error = validate_report_content(&content).unwrap_err();
    assert!(error.to_string().contains("skipped section"));

    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![ReportSection {
            template_blocks: Some(vec![paragraph("   ")]),
            ..deferred
        }],
    };
    let error = validate_report_content(&content).unwrap_err();
    assert!(error.to_string().contains("paragraph"));
}

#[test]
fn template_copies_are_excluded_from_the_document_text_ceiling() {
    let body = "a".repeat(MAX_PARAGRAPH_CHARACTERS);
    let sections: Vec<ReportSection> = (0..20)
        .map(|index| ReportSection {
            id: Uuid::new_v4(),
            heading: format!("Section {index}"),
            blocks: vec![paragraph(&body)],
            skipped: false,
            template_blocks: Some(vec![paragraph(&body)]),
            authorship: None,
        })
        .collect();
    let written_characters: usize = sections
        .iter()
        .flat_map(|section| &section.blocks)
        .chain(
            sections
                .iter()
                .filter_map(|section| section.template_blocks.as_ref())
                .flatten(),
        )
        .map(|block| match block {
            ReportBlock::Paragraph { text } => text.chars().count(),
            _ => 0,
        })
        .sum();
    assert!(written_characters > MAX_REPORT_TEXT_CHARACTERS);

    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: sections.clone(),
    };
    validate_report_content(&content).expect("template copies do not spend the document budget");

    // The same text written into the authored bodies does exceed it.
    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: sections
            .into_iter()
            .map(|section| ReportSection {
                blocks: vec![paragraph(&body), paragraph(&body)],
                template_blocks: None,
                ..section
            })
            .collect(),
    };
    let error = validate_report_content(&content).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("exceeds {MAX_REPORT_TEXT_CHARACTERS} characters"))
    );
}

#[test]
fn section_authorship_may_not_outrun_the_accepted_revision() {
    let mut workspace = workspace();
    let stamp = SectionAuthorship {
        kind: AuthorshipKind::Template,
        revision: 0,
        model_id: None,
        run_id: None,
        updated_at: timestamp("2026-08-01T12:00:00Z"),
    };
    workspace.draft.content.sections = vec![ReportSection {
        authorship: Some(stamp.clone()),
        ..section(Uuid::new_v4(), "History", "Documented.")
    }];
    workspace.validate().expect("a current stamp is valid");

    workspace.draft.content.sections[0].authorship = Some(SectionAuthorship {
        revision: 1,
        ..stamp
    });
    let error = workspace.validate().unwrap_err();
    assert!(error.to_string().contains("authorship is newer"));
}

#[test]
fn prompt_content_view_hides_template_copies_and_authorship() {
    let content = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![ReportSection {
            template_blocks: Some(vec![paragraph("Template body.")]),
            authorship: Some(SectionAuthorship {
                kind: AuthorshipKind::ModelGenerated,
                revision: 0,
                model_id: Some("us.anthropic.claude-sonnet-test".to_string()),
                run_id: Some(Uuid::new_v4()),
                updated_at: timestamp("2026-08-01T12:00:00Z"),
            }),
            ..section(Uuid::new_v4(), "History", "Documented.")
        }],
    };

    let view = prompt_content_view(&content);
    let viewed = &view["sections"][0];
    assert_eq!(viewed["heading"], serde_json::json!("History"));
    assert_eq!(
        viewed["blocks"],
        serde_json::json!(content.sections[0].blocks)
    );
    assert_eq!(viewed["skipped"], serde_json::json!(false));
    assert!(viewed.get("template_blocks").is_none());
    assert!(viewed.get("authorship").is_none());

    // A section with neither field serializes exactly as it always did.
    let plain = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![section(Uuid::new_v4(), "History", "Documented.")],
    };
    assert_eq!(
        prompt_content_view(&plain),
        serde_json::to_value(&plain).expect("serialize content")
    );
}

#[test]
fn filling_a_deferred_section_keeps_its_template_copy() {
    let mut workspace = workspace();
    let deferred_id = Uuid::new_v4();
    let template_blocks = vec![paragraph("Describe observed behaviour here.")];
    workspace.draft.content.sections = vec![ReportSection {
        id: deferred_id,
        heading: "Observations".to_string(),
        blocks: Vec::new(),
        skipped: true,
        template_blocks: Some(template_blocks.clone()),
        authorship: None,
    }];

    let replaced = workspace
        .draft
        .preview(&[ReportOperation::ReplaceSection {
            section_id: deferred_id,
            heading: "Observations".to_string(),
            blocks: vec![paragraph("Attended two sessions.")],
        }])
        .expect("replace deferred section");
    assert!(!replaced.sections[0].skipped);
    assert_eq!(
        replaced.sections[0].template_blocks.as_ref(),
        Some(&template_blocks)
    );
}

#[test]
fn writer_session_name_is_trimmed_and_validated() {
    let mut workspace = workspace();
    workspace
        .rename_session("  Intake report  ", timestamp("2026-08-01T12:05:00Z"))
        .expect("rename");
    assert_eq!(workspace.session_name, "Intake report");
    assert!(
        workspace
            .rename_session("   ", timestamp("2026-08-01T12:06:00Z"))
            .is_err()
    );
}

#[test]
fn report_enums_round_trip_with_snake_case_tags() {
    let block = ReportBlock::BulletList {
        items: vec!["One".to_string(), "Two".to_string()],
    };
    let json = serde_json::to_value(&block).expect("serialize");
    assert_eq!(json["kind"], "bullet_list");
    assert_eq!(serde_json::from_value::<ReportBlock>(json).unwrap(), block);

    let operation = ReportOperation::SetTitle {
        title: "Assessment".to_string(),
    };
    let json = serde_json::to_value(&operation).expect("serialize");
    assert_eq!(json["kind"], "set_title");
    assert_eq!(
        serde_json::from_value::<ReportOperation>(json).unwrap(),
        operation
    );
}

#[test]
fn decode_rejects_future_schema_version() {
    let mut value = serde_json::to_value(workspace()).expect("serialize");
    value["schema_version"] = serde_json::json!(REPORT_WORKSPACE_SCHEMA_VERSION + 1);
    let error = decode_report_workspace(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert!(error.to_string().contains("newer than supported"));
}

#[test]
fn preview_applies_ordered_operations_atomically() {
    let workspace = workspace();
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let operations = vec![
        ReportOperation::SetTitle {
            title: "Clinical report".to_string(),
        },
        ReportOperation::AddSection {
            position: 0,
            section: section(first_id, "Background", "Initial history"),
        },
        ReportOperation::AddSection {
            position: 1,
            section: section(second_id, "Findings", "Initial findings"),
        },
        ReportOperation::ReplaceSection {
            section_id: first_id,
            heading: "History".to_string(),
            blocks: vec![paragraph("Reviewed history")],
        },
        ReportOperation::RemoveSection {
            section_id: second_id,
        },
    ];

    let content = workspace.draft.preview(&operations).expect("preview");
    assert_eq!(content.title, "Clinical report");
    assert_eq!(content.sections.len(), 1);
    assert_eq!(content.sections[0].id, first_id);
    assert_eq!(content.sections[0].heading, "History");
}

#[test]
fn invalid_operation_leaves_source_draft_unchanged() {
    let workspace = workspace();
    let before = workspace.draft.clone();
    let error = workspace
        .draft
        .preview(&[
            ReportOperation::SetTitle {
                title: "Changed".to_string(),
            },
            ReportOperation::RemoveSection {
                section_id: Uuid::new_v4(),
            },
        ])
        .unwrap_err();
    assert!(error.to_string().contains("does not exist"));
    assert_eq!(workspace.draft, before);
}

#[test]
fn preview_rejects_duplicate_ids_invalid_positions_and_no_op() {
    let mut workspace = workspace();
    let id = Uuid::new_v4();
    workspace
        .draft
        .content
        .sections
        .push(section(id, "A", "Text"));

    let duplicate = workspace.draft.preview(&[ReportOperation::AddSection {
        position: 1,
        section: section(id, "B", "Text"),
    }]);
    assert!(
        duplicate
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );

    let outside = workspace.draft.preview(&[ReportOperation::AddSection {
        position: 3,
        section: section(Uuid::new_v4(), "B", "Text"),
    }]);
    assert!(outside.unwrap_err().to_string().contains("outside"));

    let no_op = workspace.draft.preview(&[ReportOperation::SetTitle {
        title: "Untitled report".to_string(),
    }]);
    assert!(no_op.unwrap_err().to_string().contains("does not change"));
}

#[test]
fn acceptance_recomputes_candidate_and_increments_once() {
    let workspace = workspace();
    let operations = vec![ReportOperation::AddSection {
        position: 0,
        section: section(Uuid::new_v4(), "Summary", "Accepted text"),
    }];
    let proposal = proposal(&workspace, operations);
    let accepted = workspace
        .draft
        .accept(&proposal, timestamp("2026-08-01T12:02:00Z"))
        .expect("accept");

    assert_eq!(accepted.revision, 1);
    assert_eq!(accepted.last_applied_proposal_id, Some(proposal.id));
    assert_eq!(accepted.content, proposal.proposed_content);

    let stale = accepted.accept(&proposal, timestamp("2026-08-01T12:03:00Z"));
    assert!(stale.unwrap_err().to_string().contains("based on revision"));
}

#[test]
fn acceptance_never_trusts_tampered_candidate() {
    let workspace = workspace();
    let operations = vec![ReportOperation::SetTitle {
        title: "Reviewed title".to_string(),
    }];
    let mut proposal = proposal(&workspace, operations);
    proposal.proposed_content.title = "Unreviewed title".to_string();

    let error = workspace
        .draft
        .accept(&proposal, timestamp("2026-08-01T12:02:00Z"))
        .unwrap_err();
    assert!(error.to_string().contains("does not match"));
}

#[test]
fn manual_replace_checks_revision_and_increments_once() {
    let workspace = workspace();
    let content = ReportContent {
        title: "Manually edited".to_string(),
        sections: vec![],
    };
    let saved = workspace
        .draft
        .replace_content(0, content, timestamp("2026-08-01T12:02:00Z"))
        .expect("save");
    assert_eq!(saved.revision, 1);

    let error = saved
        .replace_content(0, saved.content.clone(), timestamp("2026-08-01T12:03:00Z"))
        .unwrap_err();
    assert!(error.to_string().contains("expected revision"));
}

#[test]
fn limits_and_empty_text_are_rejected() {
    let workspace = workspace();
    let too_long_title = "x".repeat(201);
    assert!(
        workspace
            .draft
            .preview(&[ReportOperation::SetTitle {
                title: too_long_title
            }])
            .unwrap_err()
            .to_string()
            .contains("200")
    );

    let invalid_bullets = ReportOperation::AddSection {
        position: 0,
        section: ReportSection {
            skipped: false,
            template_blocks: None,
            authorship: None,
            id: Uuid::new_v4(),
            heading: "Findings".to_string(),
            blocks: vec![ReportBlock::BulletList { items: vec![] }],
        },
    };
    assert!(
        workspace
            .draft
            .preview(&[invalid_bullets])
            .unwrap_err()
            .to_string()
            .contains("bullet list")
    );
}

#[test]
fn structured_tables_validate_shape_widths_and_empty_cells() {
    let valid = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![ReportSection {
            skipped: false,
            template_blocks: None,
            authorship: None,
            id: Uuid::new_v4(),
            heading: "Scores".to_string(),
            blocks: vec![ReportBlock::Table {
                rows: vec![
                    vec!["Measure".to_string(), "Score".to_string()],
                    vec!["Attention".to_string(), String::new()],
                ],
                has_header: true,
                column_widths: Some(vec![7_000, 3_000]),
            }],
        }],
    };
    validate_report_content(&valid).expect("valid table");

    for rows in [
        vec![
            vec!["A".to_string()],
            vec!["B".to_string(), "C".to_string()],
        ],
        vec![vec![String::new(), String::new()]],
    ] {
        let mut invalid = valid.clone();
        invalid.sections[0].blocks[0] = ReportBlock::Table {
            rows,
            has_header: false,
            column_widths: None,
        };
        assert!(validate_report_content(&invalid).is_err());
    }

    let mut invalid_widths = valid.clone();
    invalid_widths.sections[0].blocks[0] = ReportBlock::Table {
        rows: vec![vec!["A".to_string(), "B".to_string()]],
        has_header: true,
        column_widths: Some(vec![5_000, 4_999]),
    };
    assert!(
        validate_report_content(&invalid_widths)
            .unwrap_err()
            .to_string()
            .contains("sum to 10000")
    );

    let mut full_rows = vec![vec![String::new(); 20]; 200];
    full_rows[0][0] = "value".to_string();
    let too_many_cells = ReportContent {
        title: "Assessment".to_string(),
        sections: vec![ReportSection {
            skipped: false,
            template_blocks: None,
            authorship: None,
            id: Uuid::new_v4(),
            heading: "Large tables".to_string(),
            blocks: (0..6)
                .map(|_| ReportBlock::Table {
                    rows: full_rows.clone(),
                    has_header: false,
                    column_widths: None,
                })
                .collect(),
        }],
    };
    assert!(
        validate_report_content(&too_many_cells)
            .unwrap_err()
            .to_string()
            .contains("at most 20000 cells")
    );
}

#[test]
fn template_metadata_is_revision_bound_and_contains_no_local_identity() {
    let mut workspace = workspace();
    workspace.draft = workspace
        .draft
        .replace_content(
            0,
            ReportContent {
                title: "Imported".to_string(),
                sections: vec![],
            },
            timestamp("2026-08-01T12:01:00Z"),
        )
        .expect("import revision");
    workspace.updated_at = timestamp("2026-08-01T12:01:00Z");
    workspace.template_import = Some(ReportTemplateImport {
        source_sha256: "a".repeat(64),
        writer_template_id: None,
        writer_template_name: None,
        imported_revision: 1,
        imported_at: timestamp("2026-08-01T12:01:00Z"),
        warnings: vec![ReportTemplateWarning {
            code: ReportTemplateWarningCode::MissingTitle,
            count: 1,
        }],
        reviewed_revision: Some(1),
    });
    workspace.validate().expect("valid metadata");
    let json = serde_json::to_string(&workspace).expect("serialize");
    assert!(!json.contains("filename"));
    assert!(!json.contains("path"));

    workspace.draft.revision = 2;
    assert!(
        workspace
            .validate()
            .unwrap_err()
            .to_string()
            .contains("current accepted revision")
    );
}

#[test]
fn unresolved_template_markers_are_counted_across_tables() {
    let content = ReportContent {
        title: "Report for {{client}}".to_string(),
        sections: vec![ReportSection {
            skipped: false,
            template_blocks: None,
            authorship: None,
            id: Uuid::new_v4(),
            heading: "[DATE]".to_string(),
            blocks: vec![ReportBlock::Table {
                rows: vec![vec!["<<score>>".to_string(), "_____".to_string()]],
                has_header: false,
                column_widths: None,
            }],
        }],
    };
    assert_eq!(report_template_placeholder_count(&content), 4);
}

fn complete_turn(index: usize, text_size: usize) -> ReportAuthoringTurn {
    let now = timestamp("2026-08-01T12:00:00Z");
    ReportAuthoringTurn {
        id: Uuid::new_v4(),
        model_id: "test-model".to_string(),
        messages: vec![
            ReportProtocolMessage {
                role: ReportProtocolRole::User,
                content: vec![ReportProtocolBlock::Text {
                    text: "Read the record".to_string(),
                }],
                created_at: now,
            },
            ReportProtocolMessage {
                role: ReportProtocolRole::Assistant,
                content: vec![ReportProtocolBlock::ToolUse {
                    tool_use_id: format!("tool-{index}"),
                    name: "read_record_file".to_string(),
                    input: serde_json::json!({"filename": "record.txt"}),
                }],
                created_at: now,
            },
            ReportProtocolMessage {
                role: ReportProtocolRole::User,
                content: vec![ReportProtocolBlock::ToolResult {
                    tool_use_id: format!("tool-{index}"),
                    status: claria_core::models::report::ReportToolResultStatus::Success,
                    content: serde_json::json!({"metadata": "x".repeat(text_size)}),
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
        record_context_files: vec![],
        usage: TurnUsage {
            model_id: "test-model".to_string(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_input_tokens: 0,
            cache_write_input_tokens: 0,
            cache_ttl: None,
            cost_usd: 0.0,
            pricing_version: 0,
        },
        usage_complete: true,
        converse_calls: 1,
        tool_uses: 1,
        created_at: now,
        completed_at: now,
    }
}

#[test]
fn history_pruning_removes_whole_oldest_turns() {
    let mut workspace = workspace();
    for index in 0..(MAX_REPORT_TURNS + 3) {
        workspace
            .push_turn(complete_turn(index, 10))
            .expect("push turn");
    }
    assert_eq!(workspace.session.turns.len(), MAX_REPORT_TURNS);
    let oldest_json = serde_json::to_string(&workspace.session.turns[0]).unwrap();
    assert!(oldest_json.contains("tool-3"));
    assert!(oldest_json.contains("ToolUse") || oldest_json.contains("tool_use"));
    assert!(oldest_json.contains("tool_result"));
}

#[test]
fn configured_history_pruning_uses_the_lower_runtime_limit() {
    let mut workspace = workspace();
    for index in 0..5 {
        workspace
            .push_turn(complete_turn(index, 10))
            .expect("push turn");
    }
    workspace.prune_turns(3).expect("lower retention limit");
    assert_eq!(workspace.session.turns.len(), 3);
    let oldest_json = serde_json::to_string(&workspace.session.turns[0]).unwrap();
    assert!(oldest_json.contains("tool-2"));
}

#[test]
fn byte_pruning_keeps_newest_complete_turn() {
    let mut workspace = workspace();
    for index in 0..5 {
        workspace
            .push_turn(complete_turn(index, 140_000))
            .expect("push turn");
    }
    assert!(workspace.session.turns.len() < 5);
    let newest = workspace.session.turns.last().expect("newest turn");
    let json = serde_json::to_string(newest).unwrap();
    assert!(json.contains("tool-4"));
    assert_eq!(newest.messages.len(), 4);
}

#[test]
fn report_text_rejects_xml_illegal_controls() {
    let content = ReportContent {
        title: "Valid title".to_string(),
        sections: vec![section(
            Uuid::new_v4(),
            "Findings",
            "contains a vertical tab\u{000B}",
        )],
    };
    let error = validate_report_content(&content).expect_err("illegal XML character");
    assert!(
        error
            .to_string()
            .contains("cannot be represented in Word XML")
    );
}

#[test]
fn turn_protocol_requires_role_order_unique_ids_and_exact_results() {
    let mut wrong_role = complete_turn(1, 10);
    wrong_role.messages[0].role = ReportProtocolRole::Assistant;
    assert!(
        workspace()
            .push_turn(wrong_role)
            .expect_err("wrong role")
            .to_string()
            .contains("alternate")
    );

    let mut duplicate = complete_turn(2, 10);
    if let ReportProtocolBlock::ToolUse {
        tool_use_id,
        name,
        input,
    } = duplicate.messages[1].content[0].clone()
    {
        duplicate.messages[1]
            .content
            .push(ReportProtocolBlock::ToolUse {
                tool_use_id,
                name,
                input,
            });
    }
    assert!(
        workspace()
            .push_turn(duplicate)
            .expect_err("duplicate tool ID")
            .to_string()
            .contains("unique")
    );

    let mut mismatched = complete_turn(3, 10);
    if let ReportProtocolBlock::ToolResult { tool_use_id, .. } =
        &mut mismatched.messages[2].content[0]
    {
        *tool_use_id = "different-id".to_string();
    }
    assert!(
        workspace()
            .push_turn(mismatched)
            .expect_err("mismatched result")
            .to_string()
            .contains("correlate exactly")
    );

    let mut retained_reasoning = complete_turn(4, 10);
    retained_reasoning.messages[1].content.insert(
        0,
        ReportProtocolBlock::ReasoningText {
            text: "must remain transient".to_string(),
            signature: Some("signature".to_string()),
        },
    );
    assert!(
        workspace()
            .push_turn(retained_reasoning)
            .expect_err("persisted reasoning")
            .to_string()
            .contains("must not retain model reasoning")
    );
}
