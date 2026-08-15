use std::{
    fs,
    io::{Cursor, Read},
    process::Command,
};

use claria_core::models::report::{ReportBlock, ReportContent, ReportDraft, ReportSection};
use claria_docx::{import_template, render_report, render_report_with_template};
use docx_rs::{
    Docx, Footer, Header, LineSpacing, LineSpacingType, Paragraph, Run, RunFonts, Shading, Table,
    TableCell, TableRow,
};
use quick_xml::{Reader, events::Event};
use zip::ZipArchive;

fn draft() -> ReportDraft {
    ReportDraft {
        revision: 7,
        content: ReportContent {
            title: "Evaluation & Plan <Final>".to_string(),
            sections: vec![
                ReportSection {
                    skipped: false,
                    id: "11111111-1111-4111-8111-111111111111".parse().unwrap(),
                    heading: "History — 東京".to_string(),
                    blocks: vec![ReportBlock::Paragraph {
                        text: "First paragraph\nSecond café paragraph".to_string(),
                    }],
                },
                ReportSection {
                    skipped: false,
                    id: "22222222-2222-4222-8222-222222222222".parse().unwrap(),
                    heading: "Recommendations".to_string(),
                    blocks: vec![ReportBlock::BulletList {
                        items: vec![
                            "Continue treatment".to_string(),
                            "Review in six weeks".to_string(),
                        ],
                    }],
                },
            ],
        },
        created_at: "2026-08-01T12:00:00Z".parse().unwrap(),
        updated_at: "2026-08-02T13:14:15Z".parse().unwrap(),
        last_applied_proposal_id: None,
    }
}

fn entry(bytes: &[u8], name: &str) -> String {
    String::from_utf8(entry_bytes(bytes, name)).expect("utf-8 XML")
}

fn entry_bytes(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
    let mut file = archive.by_name(name).expect("entry");
    let mut value = Vec::new();
    file.read_to_end(&mut value).expect("read entry");
    value
}

#[test]
fn repeated_render_is_byte_identical_valid_docx() {
    let draft = draft();
    let first = render_report(&draft).expect("render");
    let second = render_report(&draft).expect("render again");
    assert_eq!(first, second);
    assert!(first.starts_with(b"PK"));

    let mut archive = ZipArchive::new(Cursor::new(&first)).expect("valid zip");
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "docProps/core.xml",
        "word/document.xml",
        "word/styles.xml",
        "word/numbering.xml",
    ] {
        assert!(archive.by_name(required).is_ok(), "missing {required}");
    }
}

#[test]
fn every_xml_package_entry_is_strictly_well_formed() {
    let bytes = render_report(&draft()).expect("render");
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
    let mut parsed = 0usize;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        if !name.ends_with(".xml") && !name.ends_with(".rels") {
            continue;
        }
        let mut xml = Vec::new();
        file.read_to_end(&mut xml).expect("read XML entry");
        let mut reader = Reader::from_reader(xml.as_slice());
        reader.config_mut().check_end_names = true;
        let mut buffer = Vec::new();
        loop {
            match reader.read_event_into(&mut buffer) {
                Ok(Event::Eof) => break,
                Ok(_) => buffer.clear(),
                Err(error) => panic!("malformed OOXML entry {name}: {error}"),
            }
        }
        parsed += 1;
    }
    assert!(parsed >= 6, "expected core OOXML package entries");
}

#[test]
fn xml_illegal_control_characters_are_rejected_before_rendering() {
    let mut invalid = draft();
    invalid.content.sections[0].blocks[0] = ReportBlock::Paragraph {
        text: "invalid\u{000B}control".to_string(),
    };
    let error = render_report(&invalid).expect_err("illegal XML character");
    assert!(
        error
            .to_string()
            .contains("cannot be represented in Word XML")
    );
}

#[test]
fn document_contains_ordered_structured_content_and_page_setup() {
    let bytes = render_report(&draft()).expect("render");
    let document = entry(&bytes, "word/document.xml");

    let title = document
        .find("Evaluation &amp; Plan &lt;Final&gt;")
        .unwrap();
    let history = document.find("History — 東京").unwrap();
    let first = document.find("First paragraph").unwrap();
    let second = document.find("Second café paragraph").unwrap();
    let recommendations = document.find("Recommendations").unwrap();
    let bullet = document.find("Continue treatment").unwrap();
    assert!(title < history);
    assert!(history < first && first < second);
    assert!(second < recommendations && recommendations < bullet);

    assert!(document.contains(r#"<w:pgSz w:w="12240" w:h="15840" />"#));
    assert!(
        document.contains(r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440""#)
    );
    // Two source lines are separate paragraph elements, not one run with a
    // bullet character or raw newline.
    assert!(document.matches("<w:p ").count() >= 7);
}

#[test]
fn structured_tables_render_with_grid_header_style_and_cell_text() {
    let mut report = draft();
    report.content.sections[0].blocks.push(ReportBlock::Table {
        rows: vec![
            vec!["Measure".to_string(), "Score".to_string()],
            vec!["Attention".to_string(), "87\npercentile".to_string()],
        ],
        has_header: true,
        column_widths: Some(vec![7_000, 3_000]),
    });
    let bytes = render_report(&report).expect("render table");
    let document = entry(&bytes, "word/document.xml");

    assert!(document.contains("<w:tbl>"));
    assert!(document.contains(r#"<w:gridCol w:w="6552" w:type="dxa" />"#));
    assert!(document.contains(r#"<w:gridCol w:w="2808" w:type="dxa" />"#));
    assert!(document.contains(r#"<w:shd w:val="clear" w:color="auto" w:fill="E2E8F0" />"#));
    assert!(document.contains("Measure"));
    assert!(document.contains("Attention"));
    assert!(document.contains("percentile"));
    assert!(document.contains("<w:cantSplit />"));
}

#[test]
fn bullets_reference_real_ooxml_numbering() {
    let bytes = render_report(&draft()).expect("render");
    let document = entry(&bytes, "word/document.xml");
    let numbering = entry(&bytes, "word/numbering.xml");

    assert!(document.contains(r#"<w:numId w:val="42" />"#));
    assert!(document.contains("<w:numPr>"));
    assert!(numbering.contains(r#"<w:abstractNum w:abstractNumId="42""#));
    assert!(numbering.contains(r#"<w:numFmt w:val="bullet" />"#));
    assert!(numbering.contains(r#"<w:lvlText w:val="•" />"#));
    assert!(!document.contains("• Continue treatment"));
}

#[test]
fn fixed_styles_and_persisted_metadata_are_emitted() {
    let bytes = render_report(&draft()).expect("render");
    let styles = entry(&bytes, "word/styles.xml");
    let properties = entry(&bytes, "docProps/core.xml");

    assert!(styles.contains(r#"w:styleId="Normal""#));
    assert!(styles.contains(r#"w:styleId="Title""#));
    assert!(styles.contains(r#"w:styleId="Heading1""#));
    assert!(styles.contains("Times New Roman"));
    assert!(properties.contains("2026-08-01T12:00:00Z"));
    assert!(properties.contains("2026-08-02T13:14:15Z"));
}

#[test]
#[ignore = "requires LibreOffice to be installed"]
fn libreoffice_can_open_the_generated_document() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let document = directory.path().join("report.docx");
    fs::write(&document, render_report(&draft()).expect("render")).expect("write DOCX");
    let output = Command::new("libreoffice")
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(directory.path())
        .arg(&document)
        .output()
        .expect("start LibreOffice");
    assert!(
        output.status.success(),
        "LibreOffice rejected DOCX: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.path().join("report.pdf").is_file());
}

#[test]
fn template_render_retains_spacing_fonts_runs_and_blank_paragraphs() {
    let body_fonts = RunFonts::new()
        .ascii("Aptos")
        .hi_ansi("Aptos")
        .east_asia("Aptos");
    let template = {
        let document = Docx::new()
            .header(Header::new().add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("Practice letterhead")),
            ))
            .footer(Footer::new().add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("Confidential footer")),
            ))
            .add_paragraph(
                Paragraph::new()
                    .style("Title")
                    .add_run(Run::new().add_text("Evaluation Template")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .page_break_before(true)
                    .add_run(Run::new().add_text("Identifying Information")),
            )
            .add_paragraph(
                Paragraph::new()
                    .line_spacing(
                        LineSpacing::new()
                            .before(240)
                            .after(480)
                            .line(360)
                            .line_rule(LineSpacingType::Auto),
                    )
                    .add_run(
                        Run::new()
                            .fonts(body_fonts.clone())
                            .size(26)
                            .bold()
                            .add_text("Client: "),
                    )
                    .add_run(
                        Run::new()
                            .fonts(body_fonts)
                            .size(26)
                            .italic()
                            .add_text("{{name}}"),
                    ),
            )
            // Intentional vertical space is not represented in ReportContent,
            // but must remain in the source-backed Word export.
            .add_paragraph(Paragraph::new().line_spacing(LineSpacing::new().after(720)));
        let mut output = Cursor::new(Vec::new());
        document.build().pack(&mut output).expect("pack template");
        output.into_inner()
    };

    let imported = import_template(&template).expect("import template");
    let mut report = draft();
    report.content = imported.content;
    assert_eq!(
        render_report_with_template(&template, &report)
            .expect("unchanged template export")
            .0,
        template,
        "an unchanged accepted template should be byte-identical"
    );

    report.content.sections[0].blocks[0] = ReportBlock::Paragraph {
        text: "Client: Morgan Lee".to_string(),
    };
    let (rendered, _) = render_report_with_template(&template, &report).expect("formatted export");
    let source_document = entry(&template, "word/document.xml");
    let rendered_document = entry(&rendered, "word/document.xml");

    for marker in [
        r#"<w:spacing w:before="240" w:after="480" w:line="360" w:lineRule="auto" />"#,
        r#"<w:pageBreakBefore />"#,
        r#"w:ascii="Aptos""#,
        r#"<w:b />"#,
        r#"<w:i />"#,
        r#"w:after="720""#,
    ] {
        assert!(source_document.contains(marker), "source missing {marker}");
        assert!(
            rendered_document.contains(marker),
            "rendered output lost {marker}"
        );
    }
    assert!(rendered_document.contains("Client: "));
    assert!(rendered_document.contains("Morgan Lee"));
    assert!(!rendered_document.contains("{{name}}"));
    for package_part in ["word/styles.xml", "word/header1.xml", "word/footer1.xml"] {
        assert_eq!(
            entry_bytes(&rendered, package_part),
            entry_bytes(&template, package_part),
            "the original {package_part} must be copied unchanged"
        );
    }
}

#[test]
fn template_render_updates_cells_without_rebuilding_table_formatting() {
    let template = {
        let header_cell = |text| {
            TableCell::new()
                .shading(Shading::new().fill("1F4E78"))
                .add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .fonts(RunFonts::new().ascii("Aptos Narrow"))
                            .bold()
                            .color("FFFFFF")
                            .add_text(text),
                    ),
                )
        };
        let document = Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Title")
                    .add_run(Run::new().add_text("Table template")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("Scores")),
            )
            .add_table(
                Table::new(vec![
                    TableRow::new(vec![header_cell("Measure"), header_cell("Score")]),
                    TableRow::new(vec![
                        TableCell::new().add_paragraph(
                            Paragraph::new().add_run(Run::new().add_text("Attention")),
                        ),
                        TableCell::new().add_paragraph(
                            Paragraph::new().add_run(Run::new().italic().add_text("{{score}}")),
                        ),
                    ]),
                ])
                .set_grid(vec![7_000, 3_000]),
            );
        let mut output = Cursor::new(Vec::new());
        document.build().pack(&mut output).expect("pack template");
        output.into_inner()
    };
    let imported = import_template(&template).expect("import table template");
    let mut report = draft();
    report.content = imported.content;
    let ReportBlock::Table { rows, .. } = &mut report.content.sections[0].blocks[0] else {
        panic!("expected table")
    };
    rows[1][1] = "87th percentile".to_string();

    let (rendered, _) = render_report_with_template(&template, &report).expect("table export");
    let document = entry(&rendered, "word/document.xml");
    assert!(document.contains("87th percentile"));
    assert!(!document.contains("{{score}}"));
    for marker in [
        "1F4E78",
        "Aptos Narrow",
        "FFFFFF",
        "<w:i />",
        "7000",
        "3000",
    ] {
        assert!(document.contains(marker), "table formatting lost {marker}");
    }
}

#[test]
fn template_render_reuses_source_styles_for_new_sections() {
    let template = {
        let document = Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Title")
                    .add_run(Run::new().add_text("Template")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("Existing section")),
            )
            .add_paragraph(
                Paragraph::new()
                    .line_spacing(LineSpacing::new().after(360))
                    .add_run(
                        Run::new()
                            .fonts(RunFonts::new().ascii("Garamond").hi_ansi("Garamond"))
                            .add_text("Existing text"),
                    ),
            );
        let mut output = Cursor::new(Vec::new());
        document.build().pack(&mut output).expect("pack template");
        output.into_inner()
    };
    let imported = import_template(&template).expect("import template");
    let mut report = draft();
    report.content = imported.content;
    report.content.sections.push(ReportSection {
        skipped: false,
        id: "33333333-3333-4333-8333-333333333333".parse().unwrap(),
        heading: "Added section".to_string(),
        blocks: vec![ReportBlock::Paragraph {
            text: "Added narrative".to_string(),
        }],
    });

    let (rendered, _) = render_report_with_template(&template, &report).expect("structural export");
    let round_trip = import_template(&rendered).expect("re-import structural export");
    assert_eq!(round_trip.content.title, report.content.title);
    assert_eq!(round_trip.content.sections.len(), 2);
    assert_eq!(round_trip.content.sections[1].heading, "Added section");
    assert!(matches!(
        &round_trip.content.sections[1].blocks[0],
        ReportBlock::Paragraph { text } if text == "Added narrative"
    ));
    let document = entry(&rendered, "word/document.xml");
    assert!(document.matches("Garamond").count() >= 2);
    assert!(document.matches(r#"w:after="360""#).count() >= 2);
    assert_eq!(
        entry_bytes(&rendered, "word/styles.xml"),
        entry_bytes(&template, "word/styles.xml")
    );
}

#[test]
fn only_the_accepted_draft_is_rendered() {
    let mut accepted = draft();
    accepted.content.title = "Accepted title".to_string();
    let bytes = render_report(&accepted).expect("render");
    let document = entry(&bytes, "word/document.xml");
    assert!(document.contains("Accepted title"));
    assert!(!document.contains("Pending unaccepted title"));
}
