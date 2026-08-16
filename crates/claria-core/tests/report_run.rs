//! Decoding and validation of the durable drafting-run record.

use claria_core::models::{
    report::{MAX_REPORT_SECTIONS, ReportBlock},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, EvidenceRef,
        MAX_CITATION_QUOTE_CHARACTERS, MAX_INSTRUCTION_CHARACTERS, MAX_PLAN_EVIDENCE,
        MAX_PLAN_SCOPE_CHARACTERS, MAX_RUN_INSTRUCTIONS, MAX_SECTION_CITATIONS, PlanEntry,
        RecordCitation, RunInstruction, RunPlan, RunSection, RunSectionState, SectionIntent,
        decode_draft_run,
    },
};
use jiff::Timestamp;
use uuid::Uuid;

fn timestamp(value: &str) -> Timestamp {
    value.parse().expect("timestamp")
}

fn created() -> Timestamp {
    timestamp("2026-08-01T12:00:00Z")
}

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph {
        text: text.to_string(),
    }
}

fn section(position: u32, heading: &str, state: RunSectionState) -> RunSection {
    RunSection {
        section_id: Uuid::new_v4(),
        heading: heading.to_string(),
        position,
        state,
        blocks: Vec::new(),
        citations: Vec::new(),
        attempts: 0,
        error: None,
        updated_at: created(),
    }
}

fn plan_entry(section: &RunSection, intent: SectionIntent) -> PlanEntry {
    PlanEntry {
        section_id: section.section_id,
        heading: section.heading.clone(),
        intent,
        required: true,
        scope: format!("Cover {}", section.heading),
        evidence: vec![EvidenceRef {
            filename: "intake.pdf".to_string(),
            note: Some("Presenting concerns".to_string()),
        }],
        instruction: None,
    }
}

/// A two-section run: one already drafted and cited, one still pending.
fn run() -> DraftRun {
    let mut drafted = section(0, "Background", RunSectionState::Drafted);
    drafted.blocks = vec![paragraph("Referred for a neuropsychological evaluation.")];
    drafted.citations = vec![RecordCitation {
        filename: "intake.pdf".to_string(),
        quote: "referred by her primary care physician".to_string(),
    }];
    drafted.attempts = 1;
    let pending = section(1, "Test results", RunSectionState::Pending);

    DraftRun {
        schema_version: DRAFT_RUN_SCHEMA_VERSION,
        run_id: Uuid::new_v4(),
        report_id: Uuid::new_v4(),
        client_id: Uuid::new_v4(),
        base_revision: 3,
        status: DraftRunStatus::Drafting,
        plan: Some(RunPlan {
            model_id: "us.anthropic.claude-sonnet-test".to_string(),
            entries: vec![
                plan_entry(&drafted, SectionIntent::Draft),
                plan_entry(&pending, SectionIntent::Draft),
            ],
            user_edited: false,
            approved_at: Some(timestamp("2026-08-01T12:05:00Z")),
            created_at: created(),
        }),
        title: Some("Neuropsychological evaluation".to_string()),
        sections: vec![drafted, pending],
        instructions: vec![RunInstruction {
            text: "Keep the tone clinical.".to_string(),
            added_at: created(),
        }],
        writer_model_id: "us.anthropic.claude-opus-test".to_string(),
        finalized_revision: None,
        partial: false,
        created_at: created(),
        updated_at: timestamp("2026-08-01T12:10:00Z"),
    }
}

fn encode(run: &DraftRun) -> Vec<u8> {
    serde_json::to_vec(run).expect("serialize run")
}

fn decode_error(run: &DraftRun) -> String {
    decode_draft_run(&encode(run))
        .expect_err("invalid run")
        .to_string()
}

#[test]
fn a_valid_run_round_trips_through_decoding() {
    let run = run();
    run.validate().expect("valid run");
    let decoded = decode_draft_run(&encode(&run)).expect("decode run");
    assert_eq!(decoded, run);
}

#[test]
fn a_newer_schema_version_tells_the_user_to_update() {
    let mut value = serde_json::to_value(run()).expect("serialize run");
    value["schema_version"] = serde_json::json!(DRAFT_RUN_SCHEMA_VERSION + 1);
    let error = decode_draft_run(&serde_json::to_vec(&value).expect("bytes"))
        .expect_err("future run refused")
        .to_string();
    assert!(error.contains("newer than supported version"), "{error}");
    assert!(error.contains("update Claria"), "{error}");
}

#[test]
fn a_run_without_a_schema_version_is_refused() {
    let mut value = serde_json::to_value(run()).expect("serialize run");
    value
        .as_object_mut()
        .expect("run object")
        .remove("schema_version");
    let error = decode_draft_run(&serde_json::to_vec(&value).expect("bytes"))
        .expect_err("unversioned run refused")
        .to_string();
    assert!(error.contains("missing its schema version"), "{error}");
}

#[test]
fn section_ids_and_positions_must_be_unique() {
    let mut duplicate_id = run();
    duplicate_id.sections[1].section_id = duplicate_id.sections[0].section_id;
    duplicate_id.plan = None;
    assert!(
        decode_error(&duplicate_id).contains("duplicate section ID"),
        "{}",
        decode_error(&duplicate_id)
    );

    let mut duplicate_position = run();
    duplicate_position.sections[1].position = duplicate_position.sections[0].position;
    assert!(
        decode_error(&duplicate_position).contains("duplicate section position"),
        "{}",
        decode_error(&duplicate_position)
    );
}

#[test]
fn the_plan_must_have_exactly_one_entry_per_section() {
    let mut missing = run();
    missing.plan.as_mut().expect("plan").entries.truncate(1);
    assert!(
        decode_error(&missing).contains("every section of the run needs one plan entry"),
        "{}",
        decode_error(&missing)
    );

    let mut duplicated = run();
    let plan = duplicated.plan.as_mut().expect("plan");
    plan.entries[1].section_id = plan.entries[0].section_id;
    assert!(
        decode_error(&duplicated).contains("duplicate plan entry for section"),
        "{}",
        decode_error(&duplicated)
    );

    let mut invented = run();
    invented.plan.as_mut().expect("plan").entries[1].section_id = Uuid::new_v4();
    assert!(
        decode_error(&invented).contains("does not match a section of the run"),
        "{}",
        decode_error(&invented)
    );
}

#[test]
fn only_a_drafted_section_may_carry_staged_content() {
    let mut run = run();
    run.sections[1].blocks = vec![paragraph("Half-written content.")];
    assert!(
        decode_error(&run).contains("only a drafted section may carry staged content"),
        "{}",
        decode_error(&run)
    );
}

#[test]
fn staged_blocks_go_through_the_accepted_report_block_validator() {
    let mut empty_paragraph = run();
    empty_paragraph.sections[0].blocks = vec![paragraph("   ")];
    assert!(
        decode_error(&empty_paragraph).contains("paragraph must not be empty"),
        "{}",
        decode_error(&empty_paragraph)
    );

    let mut empty_bullets = run();
    empty_bullets.sections[0].blocks = vec![ReportBlock::BulletList { items: Vec::new() }];
    assert!(
        decode_error(&empty_bullets).contains("bullet list"),
        "{}",
        decode_error(&empty_bullets)
    );
}

#[test]
fn instructions_are_bounded_in_count_and_length() {
    let mut too_many = run();
    too_many.instructions = (0..=MAX_RUN_INSTRUCTIONS)
        .map(|index| RunInstruction {
            text: format!("Instruction {index}"),
            added_at: created(),
        })
        .collect();
    assert!(
        decode_error(&too_many).contains(&format!("at most {MAX_RUN_INSTRUCTIONS} instructions")),
        "{}",
        decode_error(&too_many)
    );

    let mut too_long = run();
    too_long.instructions[0].text = "x".repeat(MAX_INSTRUCTION_CHARACTERS + 1);
    assert!(
        decode_error(&too_long)
            .contains(&format!("exceeds {MAX_INSTRUCTION_CHARACTERS} characters")),
        "{}",
        decode_error(&too_long)
    );
}

#[test]
fn citations_are_bounded_in_count_and_quote_length() {
    let mut too_many = run();
    too_many.sections[0].citations = (0..=MAX_SECTION_CITATIONS)
        .map(|index| RecordCitation {
            filename: "intake.pdf".to_string(),
            quote: format!("quote {index}"),
        })
        .collect();
    assert!(
        decode_error(&too_many).contains(&format!("at most {MAX_SECTION_CITATIONS} record quotes")),
        "{}",
        decode_error(&too_many)
    );

    let mut too_long = run();
    too_long.sections[0].citations[0].quote = "q".repeat(MAX_CITATION_QUOTE_CHARACTERS + 1);
    assert!(
        decode_error(&too_long).contains(&format!(
            "citation quote exceeds {MAX_CITATION_QUOTE_CHARACTERS} characters"
        )),
        "{}",
        decode_error(&too_long)
    );
}

#[test]
fn plan_scope_and_evidence_are_bounded() {
    let mut long_scope = run();
    long_scope.plan.as_mut().expect("plan").entries[0].scope =
        "s".repeat(MAX_PLAN_SCOPE_CHARACTERS + 1);
    assert!(
        decode_error(&long_scope).contains(&format!(
            "plan scope exceeds {MAX_PLAN_SCOPE_CHARACTERS} characters"
        )),
        "{}",
        decode_error(&long_scope)
    );

    let mut too_much_evidence = run();
    too_much_evidence.plan.as_mut().expect("plan").entries[0].evidence = (0..=MAX_PLAN_EVIDENCE)
        .map(|index| EvidenceRef {
            filename: format!("record-{index}.pdf"),
            note: None,
        })
        .collect();
    assert!(
        decode_error(&too_much_evidence).contains(&format!("at most {MAX_PLAN_EVIDENCE} records")),
        "{}",
        decode_error(&too_much_evidence)
    );
}

#[test]
fn headings_and_section_counts_reuse_the_accepted_report_ceilings() {
    let mut long_heading = run();
    long_heading.sections[0].heading = "h".repeat(201);
    long_heading.plan = None;
    assert!(
        decode_error(&long_heading).contains("section heading exceeds 200 characters"),
        "{}",
        decode_error(&long_heading)
    );

    let mut too_many_sections = run();
    too_many_sections.plan = None;
    too_many_sections.sections = (0..=MAX_REPORT_SECTIONS)
        .map(|position| {
            section(
                u32::try_from(position).expect("position"),
                "Section",
                RunSectionState::Pending,
            )
        })
        .collect();
    assert!(
        decode_error(&too_many_sections)
            .contains(&format!("at most {MAX_REPORT_SECTIONS} sections")),
        "{}",
        decode_error(&too_many_sections)
    );
}

#[test]
fn a_finalized_run_must_be_completed_on_a_newer_revision() {
    let mut still_drafting = run();
    still_drafting.finalized_revision = Some(4);
    assert!(
        decode_error(&still_drafting).contains("must be completed"),
        "{}",
        decode_error(&still_drafting)
    );

    let mut stale_cut = run();
    stale_cut.status = DraftRunStatus::Completed;
    stale_cut.finalized_revision = Some(stale_cut.base_revision);
    assert!(
        decode_error(&stale_cut).contains("must be newer than the revision the run built on"),
        "{}",
        decode_error(&stale_cut)
    );

    let mut completed = run();
    completed.status = DraftRunStatus::Completed;
    completed.finalized_revision = Some(completed.base_revision + 1);
    completed.sections[1].state = RunSectionState::Skipped;
    completed.validate().expect("completed run is valid");
}

#[test]
fn a_partial_run_must_record_the_revision_it_landed_in() {
    let mut run = run();
    run.partial = true;
    assert!(
        decode_error(&run).contains("must record the revision it was finalized into"),
        "{}",
        decode_error(&run)
    );
}

#[test]
fn interrupted_sections_demote_to_pending() {
    let mut run = run();
    run.sections[1].state = RunSectionState::Drafting;
    // A `Drafting` section is legal on the wire — it is what an interrupted
    // run looks like — and the loader is what resolves it.
    let mut decoded = decode_draft_run(&encode(&run)).expect("decode interrupted run");

    let demoted_at = timestamp("2026-08-02T09:00:00Z");
    assert_eq!(decoded.demote_interrupted_sections(demoted_at), 1);
    assert_eq!(decoded.sections[1].state, RunSectionState::Pending);
    assert_eq!(decoded.sections[1].updated_at, demoted_at);
    // Drafted content is untouched, and demotion is idempotent.
    assert_eq!(decoded.sections[0].state, RunSectionState::Drafted);
    assert_eq!(decoded.sections[0].updated_at, created());
    assert_eq!(decoded.demote_interrupted_sections(demoted_at), 0);
}
