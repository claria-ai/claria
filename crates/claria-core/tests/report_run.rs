//! Decoding and validation of the durable drafting-run record.

use claria_core::models::{
    report::{MAX_REPORT_SECTIONS, ReportBlock},
    report_run::{
        DRAFT_RUN_SCHEMA_VERSION, DraftRun, DraftRunStatus, EvidenceRef,
        MAX_CITATION_QUOTE_CHARACTERS, MAX_CURATED_RECORDS, MAX_INSTRUCTION_CHARACTERS,
        MAX_PLAN_EVIDENCE, MAX_PLAN_SCOPE_CHARACTERS, MAX_RUN_INSTRUCTIONS, MAX_SECTION_CITATIONS,
        PlanEntry, RecordCitation, RunInstruction, RunPlan, RunRecordFile, RunRecordSnapshot,
        RunSection, RunSectionRecords, RunSectionState, RunUnavailableRecord, SectionIntent,
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
        records: None,
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
        curated_records: None,
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
            synthetic: false,
            approved_at: Some(timestamp("2026-08-01T12:05:00Z")),
            plan_warnings: Vec::new(),
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
        record_snapshot: None,
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

// ── User-curated per-section record context ─────────────────────────────────

/// The field is new, so every run object written before it existed omits it
/// entirely. Those runs are resumable, so they have to decode — and decode to
/// the unrestricted default rather than to anything the resume could act on.
#[test]
fn a_plan_written_before_curation_existed_decodes_unrestricted() {
    let run = run();
    let mut value: serde_json::Value =
        serde_json::from_slice(&encode(&run)).expect("run serializes");
    for entry in value["plan"]["entries"]
        .as_array_mut()
        .expect("plan entries")
    {
        let entry = entry.as_object_mut().expect("plan entry object");
        assert!(
            entry.remove("curated_records").is_some(),
            "a curated_records key should have been written to remove"
        );
    }
    let bytes = serde_json::to_vec(&value).expect("re-serialize");

    let decoded = decode_draft_run(&bytes).expect("an older plan still decodes");
    for entry in &decoded.plan.expect("plan").entries {
        assert_eq!(entry.curated_records, None);
    }
}

#[test]
fn a_curated_restriction_round_trips_through_decoding() {
    let mut run = run();
    let plan = run.plan.as_mut().expect("plan");
    plan.entries[0].curated_records = Some(vec![
        "intake.pdf".to_string(),
        "teacher-form.pdf".to_string(),
    ]);

    let decoded = decode_draft_run(&encode(&run)).expect("decode a curated run");
    let entries = decoded.plan.expect("plan").entries;
    assert_eq!(
        entries[0].curated_records.as_deref(),
        Some(["intake.pdf".to_string(), "teacher-form.pdf".to_string()].as_slice())
    );
    // Restricting one section says nothing about the others.
    assert_eq!(entries[1].curated_records, None);
}

/// The four shapes a restriction cannot take. An empty list is the sharpest
/// of them: it would mean a section drafted from no records at all, which is
/// never what anybody asked for, so it is refused rather than read as "no
/// restriction".
#[test]
fn a_curated_restriction_refuses_the_shapes_that_cannot_mean_anything() {
    let curated = |records: Vec<String>| {
        let mut run = run();
        run.plan.as_mut().expect("plan").entries[0].curated_records = Some(records);
        decode_error(&run)
    };

    assert!(
        curated(Vec::new()).contains("must name at least one record"),
        "{}",
        curated(Vec::new())
    );

    let over_cap: Vec<String> = (0..=MAX_CURATED_RECORDS)
        .map(|index| format!("record-{index}.pdf"))
        .collect();
    assert!(
        curated(over_cap.clone()).contains(&format!("at most {MAX_CURATED_RECORDS} records")),
        "{}",
        curated(over_cap)
    );

    let blank = vec!["intake.pdf".to_string(), "   ".to_string()];
    assert!(
        curated(blank.clone()).contains("curated record filename must not be empty"),
        "{}",
        curated(blank)
    );

    let repeated = vec!["intake.pdf".to_string(), "intake.pdf".to_string()];
    assert!(
        curated(repeated.clone()).contains("named twice"),
        "{}",
        curated(repeated)
    );
}

/// The ceiling is exactly what it says, not one either side of it.
#[test]
fn a_curated_restriction_may_name_the_whole_ceiling() {
    let mut run = run();
    run.plan.as_mut().expect("plan").entries[0].curated_records = Some(
        (0..MAX_CURATED_RECORDS)
            .map(|index| format!("record-{index}.pdf"))
            .collect(),
    );
    decode_draft_run(&encode(&run)).expect("a restriction at the ceiling is valid");
}

// ── Where a section's words came from ───────────────────────────────────────

fn snapshot() -> RunRecordSnapshot {
    RunRecordSnapshot {
        files: vec![
            RunRecordFile {
                filename: "intake.pdf".to_string(),
                sha256: "a".repeat(64),
                characters: 4_200,
            },
            RunRecordFile {
                filename: "teacher-form.pdf".to_string(),
                sha256: "b".repeat(64),
                characters: 1_100,
            },
        ],
        unavailable: vec![RunUnavailableRecord {
            filename: "scan.pdf".to_string(),
            reason: "extraction_failed".to_string(),
        }],
        total_characters: 5_300,
        captured_at: created(),
    }
}

/// The whole point of the additive shape: a run written by an earlier build
/// still decodes at the same schema version, and says "not recorded" rather
/// than "no records".
#[test]
fn a_run_written_before_provenance_existed_still_decodes() {
    let run = run();
    let mut value: serde_json::Value =
        serde_json::from_slice(&encode(&run)).expect("run serializes");
    let object = value.as_object_mut().expect("run object");
    assert!(
        object.remove("record_snapshot").is_some(),
        "a record_snapshot key should have been written to remove"
    );
    for section in value["sections"].as_array_mut().expect("sections") {
        let section = section.as_object_mut().expect("section object");
        assert!(section.remove("records").is_some());
    }
    let bytes = serde_json::to_vec(&value).expect("re-serialize");

    let decoded = decode_draft_run(&bytes).expect("an older run still decodes");
    assert_eq!(decoded.record_snapshot, None);
    assert!(
        decoded
            .sections
            .iter()
            .all(|section| section.records.is_none())
    );
}

#[test]
fn the_corpus_and_the_per_section_sources_round_trip() {
    let mut run = run();
    run.record_snapshot = Some(snapshot());
    run.sections[0].records = Some(RunSectionRecords::SharedCorpus);
    run.sections[1].records = Some(RunSectionRecords::Curated {
        filenames: vec!["teacher-form.pdf".to_string()],
    });

    let decoded = decode_draft_run(&encode(&run)).expect("decode a run with provenance");
    let stored = decoded.record_snapshot.expect("the snapshot");
    assert_eq!(stored.files.len(), 2);
    assert_eq!(stored.unavailable[0].reason, "extraction_failed");
    assert_eq!(stored.total_characters, 5_300);
    assert_eq!(
        decoded.sections[0].records,
        Some(RunSectionRecords::SharedCorpus)
    );
    assert_eq!(
        decoded.sections[1].records,
        Some(RunSectionRecords::Curated {
            filenames: vec!["teacher-form.pdf".to_string()]
        })
    );
}

/// A snapshot that names a file twice would make the run disagree with itself
/// about what the writer read, so it is refused rather than deduplicated.
#[test]
fn a_snapshot_cannot_name_one_record_twice() {
    let mut run = run();
    let mut duplicated = snapshot();
    duplicated.unavailable[0].filename = "intake.pdf".to_string();
    run.record_snapshot = Some(duplicated);

    let error = decode_draft_run(&encode(&run)).expect_err("a duplicate is refused");
    assert!(
        error.to_string().contains("appears twice"),
        "unexpected error: {error}"
    );
}

/// An empty curated list is a real outcome — every curated file had gone
/// missing — so it is recorded rather than normalized into "the shared
/// corpus", which would be a different and untrue claim.
#[test]
fn a_section_may_record_that_its_restriction_reached_nothing() {
    let mut run = run();
    run.sections[0].records = Some(RunSectionRecords::Curated {
        filenames: Vec::new(),
    });
    let decoded = decode_draft_run(&encode(&run)).expect("an empty restriction is recordable");
    assert_eq!(
        decoded.sections[0].records,
        Some(RunSectionRecords::Curated {
            filenames: Vec::new()
        })
    );
}
