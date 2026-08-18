use std::io::{Cursor, Write};

use claria_core::models::report::{
    MAX_SECTION_TEMPLATE_DIRECTIVES, MAX_TEMPLATE_DIRECTIVE_CHARACTERS, ReportBlock,
    ReportTemplateWarningCode,
};
use claria_docx::{DocxError, MAX_TEMPLATE_DOCX_BYTES, import_template};
use docx_rs::{
    Docx, IndentLevel, NumberingId, Paragraph, Run, Shading, Table, TableCell, TableRow,
};
use zip::{ZipWriter, write::SimpleFileOptions};

fn pack(document: Docx) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    document.build().pack(&mut output).expect("pack test DOCX");
    output.into_inner()
}

fn warning_count(imported: &claria_docx::ImportedTemplate, code: ReportTemplateWarningCode) -> u32 {
    imported
        .warnings
        .iter()
        .find(|warning| warning.code == code)
        .map_or(0, |warning| warning.count)
}

#[test]
fn headings_lists_and_simple_tables_become_structured_report_content() {
    let table = Table::new(vec![
        TableRow::new(vec![
            TableCell::new()
                .shading(Shading::new().fill("EEEEEE"))
                .add_paragraph(Paragraph::new().add_run(Run::new().bold().add_text("Measure"))),
            TableCell::new()
                .shading(Shading::new().fill("EEEEEE"))
                .add_paragraph(Paragraph::new().add_run(Run::new().bold().add_text("Score"))),
        ]),
        TableRow::new(vec![
            TableCell::new().add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("Attention {{review}}")),
            ),
            TableCell::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("87"))),
        ]),
    ])
    .set_grid(vec![7_000, 3_000]);
    let bytes = pack(
        Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Title")
                    .add_run(Run::new().add_text("Evaluation Template")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("Findings")),
            )
            .add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text("Narrative text"))
                    .add_run(Run::new().vanish().add_text("HIDDEN PRIOR CLIENT")),
            )
            .add_paragraph(
                Paragraph::new()
                    .numbering(NumberingId::new(1), IndentLevel::new(0))
                    .add_run(Run::new().add_text("First recommendation")),
            )
            .add_table(table),
    );

    let imported = import_template(&bytes).expect("import template");
    assert_eq!(imported.content.title, "Evaluation Template");
    assert_eq!(imported.content.sections.len(), 1);
    assert_eq!(imported.content.sections[0].heading, "Findings");
    assert!(matches!(
        &imported.content.sections[0].blocks[0],
        ReportBlock::Paragraph { text } if text == "Narrative text"
    ));
    assert!(matches!(
        &imported.content.sections[0].blocks[1],
        ReportBlock::BulletList { items } if items == &["First recommendation"]
    ));
    assert!(matches!(
        &imported.content.sections[0].blocks[2],
        ReportBlock::Table {
            rows,
            has_header: true,
            column_widths: Some(widths),
        } if rows[1][0] == "Attention {{review}}" && widths == &[7_000, 3_000]
    ));
    assert!(
        !serde_json::to_string(&imported.content)
            .expect("content JSON")
            .contains("HIDDEN PRIOR CLIENT")
    );
    assert_eq!(
        warning_count(
            &imported,
            ReportTemplateWarningCode::UnsupportedElementsOmitted,
        ),
        1
    );
    assert_eq!(imported.stats.tables, 1);
    assert_eq!(imported.stats.table_cells, 4);
    assert_eq!(imported.stats.placeholder_count, 1);
    assert_eq!(imported.source_sha256.len(), 64);
    assert_eq!(
        warning_count(
            &imported,
            ReportTemplateWarningCode::NumberedListsImportedAsBullets,
        ),
        1
    );
}

#[test]
fn missing_title_and_merged_table_are_reported_without_carrying_the_table() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("Narrative")),
            )
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Keep this")))
            .add_table(Table::new(vec![TableRow::new(vec![
                TableCell::new()
                    .grid_span(2)
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Merged"))),
            ])])),
    );

    let imported = import_template(&bytes).expect("partial import");
    assert_eq!(imported.content.title, "Imported report template");
    assert_eq!(imported.content.sections[0].blocks.len(), 1);
    assert_eq!(
        warning_count(&imported, ReportTemplateWarningCode::MissingTitle),
        1
    );
    assert_eq!(
        warning_count(&imported, ReportTemplateWarningCode::MergedTablesOmitted,),
        1
    );
}

#[test]
fn nested_heading_levels_are_flattened_with_an_explicit_warning() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Title")
                    .add_run(Run::new().add_text("Template")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("Parent")),
            )
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Parent text")))
            .add_paragraph(
                Paragraph::new()
                    .style("Heading2")
                    .add_run(Run::new().add_text("Nested")),
            )
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Nested text"))),
    );

    let imported = import_template(&bytes).expect("flatten headings");
    assert_eq!(imported.content.sections.len(), 2);
    assert_eq!(imported.content.sections[1].heading, "Nested");
    assert_eq!(
        warning_count(&imported, ReportTemplateWarningCode::HeadingLevelsFlattened,),
        1
    );
}

#[test]
fn oversized_and_macro_enabled_packages_are_rejected_before_parsing() {
    let oversized = vec![0_u8; usize::try_from(MAX_TEMPLATE_DOCX_BYTES + 1).unwrap()];
    assert!(matches!(
        import_template(&oversized),
        Err(DocxError::UnsafeTemplate(_))
    ));

    let mut output = Cursor::new(Vec::new());
    {
        let mut archive = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default();
        archive
            .start_file("[Content_Types].xml", options)
            .expect("content types");
        archive
            .write_all(br#"<Types>macroEnabled</Types>"#)
            .expect("write content types");
        archive
            .start_file("word/document.xml", options)
            .expect("document");
        archive
            .write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body /></w:document>"#)
            .expect("write document");
        archive.finish().expect("finish zip");
    }
    assert!(matches!(
        import_template(&output.into_inner()),
        Err(DocxError::UnsafeTemplate(_))
    ));
}

// ── Template authoring directives ───────────────────────────────────────────

fn heading(text: &str) -> Paragraph {
    Paragraph::new()
        .style("Heading1")
        .add_run(Run::new().add_text(text))
}

fn body(text: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(text))
}

/// The specimen behaviour: a clinical template's bracketed notes to whoever
/// fills it in are carried per section, in document order, out of paragraphs,
/// bullets, and table cells alike.
#[test]
fn bracketed_authoring_directives_are_extracted_per_section() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(heading("Reason for Referral"))
            .add_paragraph(body(
                "[a one sentence briefly stating why child was referred]",
            ))
            .add_paragraph(heading("Test Results"))
            .add_paragraph(body("Scores follow."))
            .add_paragraph(
                Paragraph::new()
                    .numbering(NumberingId::new(1), IndentLevel::new(0))
                    .add_run(Run::new().add_text("[Add a validity line]")),
            )
            .add_table(
                Table::new(vec![TableRow::new(vec![
                    TableCell::new().add_paragraph(body("Composite")),
                    TableCell::new().add_paragraph(body("[score]")),
                ])])
                .set_grid(vec![5_000, 5_000]),
            ),
    );

    let imported = import_template(&bytes).expect("import template");
    let sections = &imported.content.sections;
    assert_eq!(sections.len(), 2);
    assert_eq!(
        sections[0].template_directives,
        vec!["a one sentence briefly stating why child was referred".to_string()]
    );
    // Bullet items and table cells are read too, in the order their blocks
    // appear in the section.
    assert_eq!(
        sections[1].template_directives,
        vec!["Add a validity line".to_string(), "score".to_string()]
    );
}

/// A directive that quotes a slot inside itself is one directive, and a stray
/// bracket on either side is not a directive at all.
#[test]
fn nested_brackets_nest_and_an_unclosed_bracket_is_dropped() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(heading("Identifying Information"))
            .add_paragraph(body(
                "[child's age ; format = [number] years, [number] months]",
            ))
            .add_paragraph(body("[this one never closes"))
            .add_paragraph(body("Closing bracket with no opener] is ignored."))
            .add_paragraph(body("[]  [   ]")),
    );

    let imported = import_template(&bytes).expect("import template");
    assert_eq!(
        imported.content.sections[0].template_directives,
        vec!["child's age ; format = [number] years, [number] months".to_string()]
    );
}

/// A template that repeats the same slot in every paragraph is saying one
/// thing, and the count cap keeps the earliest directives rather than the
/// loudest.
#[test]
fn repeated_directives_collapse_and_the_count_cap_keeps_the_earliest() {
    let mut document = Docx::new().add_paragraph(heading("Behavioral Observations"));
    for _ in 0..4 {
        document = document.add_paragraph(body("[child's first name]"));
    }
    for index in 0..12 {
        document = document.add_paragraph(body(&format!("[directive number {index}]")));
    }
    let imported = import_template(&pack(document)).expect("import template");

    let directives = &imported.content.sections[0].template_directives;
    assert_eq!(directives.len(), MAX_SECTION_TEMPLATE_DIRECTIVES);
    assert_eq!(directives[0], "child's first name");
    assert_eq!(directives[1], "directive number 0");
    assert_eq!(directives[7], "directive number 6");
}

/// A directive longer than the ceiling is cut rather than dropped: the opening
/// of a long instruction still says what form the section takes.
#[test]
fn an_overlong_directive_is_truncated_to_the_ceiling() {
    let long = "x".repeat(MAX_TEMPLATE_DIRECTIVE_CHARACTERS + 40);
    let bytes = pack(
        Docx::new()
            .add_paragraph(heading("Summary"))
            .add_paragraph(body(&format!("[{long}]"))),
    );

    let imported = import_template(&bytes).expect("import template");
    let directive = &imported.content.sections[0].template_directives[0];
    assert_eq!(directive.chars().count(), MAX_TEMPLATE_DIRECTIVE_CHARACTERS);
    assert!(directive.ends_with('\u{2026}'));
}

/// A template with no brackets carries no directives, so the field costs an
/// ordinary document nothing.
#[test]
fn a_template_without_brackets_carries_no_directives() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(heading("Recommendations"))
            .add_paragraph(body("Continue weekly sessions.")),
    );

    let imported = import_template(&bytes).expect("import template");
    assert!(imported.content.sections[0].template_directives.is_empty());
}
