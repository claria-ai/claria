//! The opening message of a whole-report drafting conversation.
//!
//! The layout exists to make prompt caching work across a run that may issue
//! a hundred Bedrock calls. Everything that cannot change for the life of the
//! session goes first, the plan goes next, and only the kick-off instruction —
//! the one block a resume rewrites — trails behind both checkpoints:
//!
//! ```text
//! <untrusted_record_context>    compact record corpus     ┐ session-stable
//! <untrusted_template_context>  base-revision structure   ┘ ● CachePoint 1
//! <plan_context>                the run's plan              ● CachePoint 2
//! kick-off instruction                                      (uncached)
//! ```
//!
//! The mutable draft is deliberately absent. A section the writer lands
//! exists only as an appended tool_use/tool_result pair in the tail, so
//! nothing above a checkpoint changes while the document is being written —
//! which is the whole reason the prefix survives to be read again.
//!
//! Every builder here is a pure function of durable state and must stay
//! byte-deterministic: a single reordered map key costs the run its cache.

use claria_core::models::{
    report::{ReportBlock, ReportContent, ReportWorkspace},
    report_run::{DraftRun, PlanEntry, RunPlan, RunSection, RunSectionState, SectionIntent},
};

use crate::context::{escape_delimiter_characters, template_provenance};

/// Zero-based index of each block of message 0, named once so the cache-point
/// coordinates and the layout cannot drift apart. Block 0 is the record
/// corpus; it needs no name because no cache point lands on it.
const TEMPLATE_CONTEXT_BLOCK: usize = 1;
const PLAN_CONTEXT_BLOCK: usize = 2;

/// Where the drafting conversation's fixed cache points go: after the
/// template block (session-stable) and after the plan block (plan-stable).
/// The third point is the moving tail, which the cache plan owns separately.
pub(crate) fn cache_checkpoints() -> Vec<(usize, usize)> {
    vec![(0, TEMPLATE_CONTEXT_BLOCK), (0, PLAN_CONTEXT_BLOCK)]
}

/// The base revision's structure, and every section's template body.
///
/// This replaces the accepted-report dump the whole-draft mode used to send.
/// The accepted report is the thing the run is about to replace: putting it
/// above a checkpoint would mean re-writing the cached prefix on every
/// re-run, and the writer never needs the draft it is overwriting. What it
/// does need is the structure — the section UUIDs it must copy and the
/// template prose it may rewrite from — and that is frozen for the run.
pub(crate) fn template_context(
    workspace: &ReportWorkspace,
    base: &ReportContent,
) -> Result<String, String> {
    let value = serde_json::json!({
        "title": base.title,
        "template_import": template_provenance(workspace),
        "sections": base
            .sections
            .iter()
            .map(|section| serde_json::json!({
                "section_id": section.id,
                "heading": section.heading,
                "skipped": section.skipped,
                "template_body": template_body(section)
            }))
            .collect::<Vec<_>>()
    });
    // Pretty-printed, unlike the record corpus: the model copies section UUIDs
    // out of this block, and line-anchored structure is what makes that
    // reliable. It is also small next to the corpus, so the indentation costs
    // little and is paid once per session.
    serde_json::to_string_pretty(&value)
        .map(|json| {
            format!(
                "<untrusted_template_context>\n{}\n</untrusted_template_context>",
                escape_delimiter_characters(&json)
            )
        })
        .map_err(|_| "Claria could not serialize the report template context.".to_string())
}

/// The template prose for one section: the stamped template copy when the
/// section has one, otherwise its base-revision body.
fn template_body(section: &claria_core::models::report::ReportSection) -> Vec<ReportBlock> {
    section
        .template_blocks
        .clone()
        .unwrap_or_else(|| section.blocks.clone())
}

/// The run's plan, as host data the writer works through in order.
pub(crate) fn plan_context(plan: Option<&RunPlan>) -> Result<String, String> {
    let entries: Vec<serde_json::Value> = plan
        .map(|plan| plan.entries.as_slice())
        .unwrap_or_default()
        .iter()
        .map(plan_entry_view)
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({"sections": entries}))
        .map(|json| {
            // Host data, so no untrusted_ wrapper — but the planner's own
            // words ride inside it, so a forged closing tag is still escaped.
            format!(
                "<plan_context>\n{}\n</plan_context>",
                escape_delimiter_characters(&json)
            )
        })
        .map_err(|_| "Claria could not serialize the drafting plan.".to_string())
}

fn plan_entry_view(entry: &PlanEntry) -> serde_json::Value {
    serde_json::json!({
        "section_id": entry.section_id,
        "heading": entry.heading,
        "intent": entry.intent,
        "required": entry.required,
        "scope": entry.scope,
        "evidence": entry.evidence,
        "instruction": entry.instruction
    })
}

/// Whether this turn opens a fresh run or picks an interrupted one back up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftTurnKind {
    Fresh,
    Resume,
}

/// The one block above the tail that a resume rewrites: what to do, the
/// user's guidance, and the sequencing contract. On a resume it also carries
/// the durable per-section state, the instructions added since, and a verbatim
/// template copy for every section the plan wants rewritten.
pub(crate) fn kickoff_instruction(
    kind: DraftTurnKind,
    guidance: &str,
    run: &DraftRun,
    base: &ReportContent,
) -> String {
    let mut text = String::from(
        "Whole-document request: fill the complete working report from the supplied \
         readable-record snapshot.\n",
    );
    if guidance.is_empty() {
        text.push_str("No additional user guidance was supplied.\n");
    } else {
        text.push_str(&format!("Additional user guidance:\n{guidance}\n"));
    }
    text.push_str(
        "\nWork through the plan in order. Write ONE section per response with \
         write_full_draft_section, then wait for its tool result before continuing.\n",
    );

    if kind == DraftTurnKind::Fresh {
        return text;
    }

    text.push_str(
        "\nThis drafting run RESUMES an interrupted session. Do not re-write sections marked \
         drafted. Pick up the sections that are still pending, honour any updated instructions \
         below, and finish the document.\n",
    );
    text.push_str(&format!(
        "\nSection state:\n{}\n",
        section_state_table(&run.sections)
    ));

    // instructions[0] is the guidance the run started from and is already
    // above; anything after it was typed when the run was picked back up.
    let updated: Vec<&str> = run
        .instructions
        .iter()
        .skip(1)
        .map(|instruction| instruction.text.as_str())
        .collect();
    if !updated.is_empty() {
        text.push_str("\nUpdated instructions for this resume:\n");
        for instruction in updated {
            text.push_str(&format!("- {instruction}\n"));
        }
    }

    for entry in run
        .plan
        .iter()
        .flat_map(|plan| plan.entries.iter())
        .filter(|entry| entry.intent == SectionIntent::Rewrite)
    {
        let Some(section) = base
            .sections
            .iter()
            .find(|section| section.id == entry.section_id)
        else {
            continue;
        };
        let copy = serde_json::json!({
            "section_id": entry.section_id,
            "heading": section.heading,
            "template_body": template_body(section)
        });
        let Ok(json) = serde_json::to_string_pretty(&copy) else {
            continue;
        };
        text.push_str(&format!(
            "\n<template_copy_for_rewrite>\n{}\n</template_copy_for_rewrite>\n",
            escape_delimiter_characters(&json)
        ));
    }
    text
}

fn section_state_table(sections: &[RunSection]) -> String {
    let mut ordered: Vec<&RunSection> = sections.iter().collect();
    ordered.sort_by_key(|section| section.position);
    let rows: Vec<serde_json::Value> = ordered
        .into_iter()
        .map(|section| {
            let mut row = serde_json::json!({
                "section_id": section.section_id,
                "heading": section.heading,
                "state": section.state
            });
            if section.state == RunSectionState::Failed
                && let Some(reason) = &section.error
                && let Some(object) = row.as_object_mut()
            {
                object.insert(
                    "failed_reason".to_string(),
                    serde_json::Value::String(reason.clone()),
                );
            }
            row
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string())
}
