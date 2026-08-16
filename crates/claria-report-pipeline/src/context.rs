//! Prompt-context assembly and persisted-history sanitization.

use std::collections::HashMap;

use claria_bedrock::report;
use claria_core::models::report::{
    ReportBlock, ReportProtocolBlock, ReportProtocolMessage, ReportToolResultStatus,
    ReportWorkspace, prompt_content_view,
};
use sha2::{Digest, Sha256};

use crate::ReportBlockReference;

pub(crate) fn flatten_protocol_history(workspace: &ReportWorkspace) -> Vec<ReportProtocolMessage> {
    workspace
        .session
        .turns
        .iter()
        .flat_map(|turn| turn.messages.iter().cloned())
        .collect()
}

pub(crate) fn build_untrusted_context(
    workspace: &ReportWorkspace,
    references: &[ReportBlockReference],
) -> Result<String, String> {
    let resolutions: Vec<serde_json::Value> = workspace
        .session
        .resolutions
        .iter()
        .rev()
        .take(20)
        .map(|resolution| {
            serde_json::json!({
                "proposal_id": resolution.proposal_id.to_string(),
                "decision": resolution.decision,
                "resulting_revision": resolution.resulting_revision
            })
        })
        .collect();
    let focused_blocks = references
        .iter()
        .map(|reference| {
            let section = workspace
                .draft
                .content
                .sections
                .iter()
                .find(|section| section.id == reference.section_id)
                .ok_or_else(|| {
                    format!(
                        "Referenced report section {} no longer exists. Remove the reference and retry.",
                        reference.section_id
                    )
                })?;
            let block_index = usize::try_from(reference.block_index)
                .map_err(|_| "Referenced block index is too large.".to_string())?;
            let block = section.blocks.get(block_index).ok_or_else(|| {
                "A referenced report block moved or was removed. Remove the reference and retry."
                    .to_string()
            })?;
            if matches!(block, ReportBlock::BulletList { .. }) {
                return Err(
                    "Only report paragraphs and tables can be attached to a message.".to_string(),
                );
            }
            Ok(serde_json::json!({
                "section_id": section.id.to_string(),
                "section_heading": section.heading,
                "block_index": reference.block_index,
                "block": block
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let template_context = template_provenance(workspace);
    let value = serde_json::json!({
        "accepted_revision": workspace.draft.revision,
        // Template copies and authorship stamps are host bookkeeping: the
        // model must not read a section's template body as accepted content.
        "accepted_report": prompt_content_view(&workspace.draft.content),
        "template_import": template_context,
        "report_changed_since_last_assistant_turn": workspace
            .session
            .last_agent_revision
            .map_or(workspace.draft.revision > 0, |revision| revision < workspace.draft.revision),
        "user_focused_blocks": focused_blocks,
        "recent_user_proposal_resolutions": resolutions
    });
    // Pretty-printed JSON inside the delimiter tags the system prompt names:
    // the wrapper marks the trust boundary, while indentation gives the model
    // line-anchored structure cues for copying section UUIDs and locating
    // blocks. Compacting this payload measurably degraded targeted edits.
    serde_json::to_string_pretty(&value)
        .map(|json| {
            format!(
                "<untrusted_report_context>\n{}\n</untrusted_report_context>",
                escape_delimiter_characters(&json)
            )
        })
        .map_err(|_| "Claria could not serialize the accepted report context.".to_string())
}

/// Keep untrusted text from opening or closing the named host delimiters:
/// only a `<` that begins one of them — `<untrusted_...`, `</untrusted_...`,
/// `<plan_context>`, `</plan_context>` — is rewritten to its six-character
/// JSON unicode-escape form, so the serialized payload stays valid JSON and
/// decodes back to the original characters. Ordinary clinical text —
/// `T-score >70`, `<3rd percentile` — passes through verbatim; blanket
/// angle-bracket escaping measurably mangled exactly that kind of prose.
pub(crate) fn escape_delimiter_characters(value: &str) -> String {
    claria_bedrock::context::escape_delimiter_forgeries(
        value,
        &["untrusted_", "plan_context"],
        "\\u003c",
    )
}

/// DOCX-template provenance for the model: how the structure got here and
/// whether its carryover has been reviewed against the current revision.
/// Never the source bytes, path, or original filename.
pub(crate) fn template_provenance(workspace: &ReportWorkspace) -> Option<serde_json::Value> {
    workspace.template_import.as_ref().map(|template| {
        serde_json::json!({
            "imported_from_docx": true,
            "imported_revision": template.imported_revision,
            "warning_codes": template
                .warnings
                .iter()
                .map(|warning| warning.code)
                .collect::<Vec<_>>(),
            "carryover_reviewed_for_current_revision":
                template.reviewed_revision == Some(workspace.draft.revision)
        })
    })
}

/// Lowercase hex SHA-256 digest — a PHI-free stand-in for prompt text in
/// telemetry records.
pub(crate) fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn sanitize_turn_messages(
    messages: Vec<ReportProtocolMessage>,
) -> Vec<ReportProtocolMessage> {
    let names: HashMap<String, String> = messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|block| match block {
            ReportProtocolBlock::ToolUse {
                tool_use_id, name, ..
            } => Some((tool_use_id.clone(), name.clone())),
            _ => None,
        })
        .collect();

    messages
        .into_iter()
        .map(|message| ReportProtocolMessage {
            role: message.role,
            created_at: message.created_at,
            content: message
                .content
                .into_iter()
                .filter_map(|block| match block {
                    // Reasoning may contain PHI and is needed only for the
                    // immediate Bedrock tool round. Never persist or display it.
                    ReportProtocolBlock::ReasoningText { .. }
                    | ReportProtocolBlock::ReasoningRedacted { .. } => None,
                    ReportProtocolBlock::ToolUse {
                        tool_use_id,
                        name,
                        input,
                    } => Some(ReportProtocolBlock::ToolUse {
                        input: sanitize_tool_input(&name, &input),
                        tool_use_id,
                        name,
                    }),
                    ReportProtocolBlock::ToolResult {
                        tool_use_id,
                        status,
                        content,
                    } => Some(ReportProtocolBlock::ToolResult {
                        content: sanitize_tool_result(
                            names.get(&tool_use_id).map(String::as_str),
                            status,
                            &content,
                        ),
                        tool_use_id,
                        status,
                    }),
                    other => Some(other),
                })
                .collect(),
        })
        .collect()
}

fn sanitize_tool_input(name: &str, input: &serde_json::Value) -> serde_json::Value {
    match name {
        report::SET_FULL_DRAFT_TITLE_TOOL => {
            let sha256 = Sha256::digest(
                input
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .as_bytes(),
            )
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
            serde_json::json!({"title_sha256": sha256, "content_retained": false})
        }
        report::WRITE_FULL_DRAFT_SECTION_TOOL => {
            let sha256 = Sha256::digest(input.to_string().as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            serde_json::json!({
                "section_id": input.get("section_id"),
                "position": input.get("position"),
                "block_count": input
                    .get("blocks")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len),
                "content_sha256": sha256,
                "content_retained": false
            })
        }
        report::FINISH_FULL_DRAFT_TOOL => serde_json::json!({
            "summary_retained": false
        }),
        // The failure reason is durable on the run object, which is where the
        // UI reads it. The conversation copy is free model prose, so it is
        // dropped here like every other free-text tool input.
        report::MARK_SECTION_FAILED_TOOL => serde_json::json!({
            "section_id": input.get("section_id"),
            "reason_retained": false
        }),
        _ => input.clone(),
    }
}

fn sanitize_tool_result(
    name: Option<&str>,
    status: ReportToolResultStatus,
    content: &serde_json::Value,
) -> serde_json::Value {
    if status == ReportToolResultStatus::Error {
        return serde_json::json!({
            "error": {
                "code": content.pointer("/error/code").and_then(serde_json::Value::as_str).unwrap_or("tool_failed"),
                "message": content.pointer("/error/message").and_then(serde_json::Value::as_str).unwrap_or("The tool failed safely.")
            }
        });
    }
    match name {
        Some(report::LIST_RECORD_FILES_TOOL) => serde_json::json!({
            "file_count": content.get("files").and_then(serde_json::Value::as_array).map_or(0, Vec::len),
            "truncated": content.get("truncated").and_then(serde_json::Value::as_bool).unwrap_or(false)
        }),
        Some(report::READ_RECORD_FILE_TOOL) => serde_json::json!({
            "filename": content.get("filename"),
            "offset": content.get("offset"),
            "returned_characters": content.get("returned_characters"),
            "total_characters": content.get("total_characters"),
            "next_offset": content.get("next_offset"),
            "sha256": content.get("sha256"),
            "content_retained": false
        }),
        Some(report::PROPOSE_REPORT_CHANGES_TOOL)
        | Some(report::SET_FULL_DRAFT_TITLE_TOOL)
        | Some(report::WRITE_FULL_DRAFT_SECTION_TOOL)
        | Some(report::SKIP_FULL_DRAFT_SECTION_TOOL)
        | Some(report::MARK_SECTION_FAILED_TOOL)
        | Some(report::FINISH_FULL_DRAFT_TOOL) => content.clone(),
        _ => serde_json::json!({"status": "succeeded", "content_retained": false}),
    }
}
