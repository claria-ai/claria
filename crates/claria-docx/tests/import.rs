use std::io::{Cursor, Write};

use claria_core::models::report::{ReportBlock, ReportTemplateWarningCode};
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
