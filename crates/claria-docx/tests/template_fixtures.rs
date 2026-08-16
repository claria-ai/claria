//! Template-export tests against Word-authored fixture packages.
//!
//! The fixtures in `fixtures/docx-templates/` carry the constructs desktop
//! Word emits and docx-rs cannot build — custom-named heading styles with
//! `w:outlineLvl`, direct run formatting, rsid/proofErr noise, blank spacer
//! paragraphs, an underlined signature line as the last body paragraph, and
//! `w:sdt` content controls. The docx-rs-built templates in `render.rs`
//! cannot exercise those shapes, which is how the v0.22 underline, font,
//! and spacing regressions shipped green.

use std::io::{Cursor, Read};

use claria_core::models::report::{ReportBlock, ReportContent, ReportDraft, ReportSection};
use claria_docx::render_report_with_template;
use quick_xml::{Reader, events::Event};
use uuid::Uuid;

const CLINICAL_TEMPLATE: &[u8] =
    include_bytes!("../../../fixtures/docx-templates/clinical-eval.docx");
const CONTENT_CONTROLS_TEMPLATE: &[u8] =
    include_bytes!("../../../fixtures/docx-templates/content-controls.docx");

/// One `<w:p>` of the exported document, flattened for assertions.
#[derive(Debug, Clone)]
struct Paragraph {
    style: Option<String>,
    text: String,
    in_table: bool,
    /// Run-level properties seen on any run that carries visible text.
    text_run_underlined: bool,
    text_run_bold: bool,
    text_run_fonts: Vec<String>,
    line_breaks: usize,
    numbered: bool,
}

impl Paragraph {
    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

fn document_xml(package: &[u8]) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(package)).expect("zip package");
    let mut file = archive.by_name("word/document.xml").expect("document.xml");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).expect("read document.xml");
    bytes
}

fn local(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

/// Flatten every paragraph of `word/document.xml` in document order.
fn paragraphs(package: &[u8]) -> Vec<Paragraph> {
    let xml = document_xml(package);
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut output = Vec::new();
    let mut buffer = Vec::new();
    let mut table_depth = 0_usize;
    let mut current: Option<Paragraph> = None;
    // Run-property state while inside <w:rPr>.
    let mut in_rpr = false;
    let mut run_underline = false;
    let mut run_bold = false;
    let mut run_font: Option<String> = None;
    let mut in_text = false;
    loop {
        let event = reader.read_event_into(&mut buffer).expect("fixture XML");
        match &event {
            Event::Start(start) | Event::Empty(start) => {
                let is_empty = matches!(event, Event::Empty(_));
                match local(start.name().as_ref()) {
                    b"tbl" if !is_empty => table_depth += 1,
                    b"p" => {
                        let paragraph = Paragraph {
                            style: None,
                            text: String::new(),
                            in_table: table_depth > 0,
                            text_run_underlined: false,
                            text_run_bold: false,
                            text_run_fonts: Vec::new(),
                            line_breaks: 0,
                            numbered: false,
                        };
                        if is_empty {
                            output.push(paragraph);
                        } else {
                            current = Some(paragraph);
                        }
                    }
                    b"pStyle" => {
                        if let (Some(paragraph), Some(value)) =
                            (&mut current, attribute(start, b"val"))
                        {
                            paragraph.style = Some(value);
                        }
                    }
                    b"rPr" => in_rpr = true,
                    b"u" if in_rpr => run_underline = true,
                    b"b" if in_rpr => run_bold = true,
                    b"rFonts" if in_rpr => run_font = attribute(start, b"ascii"),
                    b"r" if !is_empty => {
                        run_underline = false;
                        run_bold = false;
                        run_font = None;
                    }
                    b"br" => {
                        if let Some(paragraph) = &mut current {
                            paragraph.line_breaks += 1;
                        }
                    }
                    b"numPr" => {
                        if let Some(paragraph) = &mut current {
                            paragraph.numbered = true;
                        }
                    }
                    b"t" if !is_empty => in_text = true,
                    _ => {}
                }
            }
            Event::Text(text) if in_text => {
                if let Some(paragraph) = &mut current {
                    paragraph.text.push_str(&text.decode().expect("text"));
                    paragraph.text_run_underlined |= run_underline;
                    paragraph.text_run_bold |= run_bold;
                    if let Some(font) = &run_font
                        && !paragraph.text_run_fonts.contains(font)
                    {
                        paragraph.text_run_fonts.push(font.clone());
                    }
                }
            }
            Event::End(end) => match local(end.name().as_ref()) {
                b"tbl" => table_depth = table_depth.saturating_sub(1),
                b"p" => {
                    if let Some(paragraph) = current.take() {
                        output.push(paragraph);
                    }
                }
                b"rPr" => in_rpr = false,
                b"t" => in_text = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    output
}

fn attribute(start: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    start
        .attributes()
        .with_checks(false)
        .flatten()
        .find_map(|attribute| {
            (local(attribute.key.as_ref()) == name)
                .then(|| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
        })
}

fn body_paragraph<'a>(paragraphs: &'a [Paragraph], needle: &str) -> &'a Paragraph {
    paragraphs
        .iter()
        .find(|paragraph| paragraph.text.contains(needle))
        .unwrap_or_else(|| panic!("no paragraph contains {needle:?}"))
}

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph {
        text: text.to_string(),
    }
}

fn section(heading: &str, blocks: Vec<ReportBlock>) -> ReportSection {
    ReportSection {
        skipped: false,
        template_blocks: None,
        authorship: None,
        id: Uuid::new_v4(),
        heading: heading.to_string(),
        blocks,
    }
}

fn draft(title: &str, sections: Vec<ReportSection>) -> ReportDraft {
    let timestamp = "2026-08-12T00:00:00Z".parse().expect("timestamp");
    ReportDraft {
        revision: 1,
        content: ReportContent {
            title: title.to_string(),
            sections,
        },
        created_at: timestamp,
        updated_at: timestamp,
        last_applied_proposal_id: None,
    }
}

/// A generated report that outgrows the template: three sections against the
/// template's two, more body paragraphs than template exemplars, and a
/// filled results table.
fn clinical_growth_draft() -> ReportDraft {
    draft(
        "Psychoeducational Evaluation",
        vec![
            section(
                "Reason for Referral",
                vec![
                    paragraph("Guardian requested evaluation for attention concerns."),
                    paragraph("Teacher reports difficulty sustaining attention in class."),
                ],
            ),
            section(
                "Assessment Results",
                vec![
                    paragraph("The BASC-3 was completed by the parent and teacher."),
                    paragraph(
                        "Attention Problems T-score was 72, in the clinically significant range.",
                    ),
                    paragraph("Working memory performance fell below the 10th percentile."),
                    ReportBlock::Table {
                        rows: vec![
                            vec!["Domain".to_string(), "Score".to_string()],
                            vec!["Working Memory".to_string(), "82".to_string()],
                        ],
                        has_header: true,
                        column_widths: None,
                    },
                ],
            ),
            section(
                "Summary and Recommendations",
                vec![
                    paragraph("Findings are consistent with attention-related difficulties."),
                    paragraph("Classroom accommodations are recommended."),
                ],
            ),
        ],
    )
}

#[test]
fn clinical_fixture_reconstructs_and_keeps_package_parts() {
    let (output, _) = render_report_with_template(CLINICAL_TEMPLATE, &clinical_growth_draft())
        .expect("template render");

    // Every non-document part is copied byte-for-byte.
    let mut source = zip::ZipArchive::new(Cursor::new(CLINICAL_TEMPLATE)).expect("source zip");
    let mut rendered = zip::ZipArchive::new(Cursor::new(output.as_slice())).expect("output zip");
    assert_eq!(source.len(), rendered.len());
    let mut source_styles = String::new();
    source
        .by_name("word/styles.xml")
        .expect("source styles")
        .read_to_string(&mut source_styles)
        .expect("read source styles");
    let mut rendered_styles = String::new();
    rendered
        .by_name("word/styles.xml")
        .expect("rendered styles")
        .read_to_string(&mut rendered_styles)
        .expect("read rendered styles");
    assert_eq!(source_styles, rendered_styles);

    // Every sentinel paragraph and table cell of the draft is present.
    let flattened = paragraphs(&output);
    for sentinel in [
        "Psychoeducational Evaluation",
        "Teacher reports difficulty sustaining attention in class.",
        "Attention Problems T-score was 72, in the clinically significant range.",
        "Findings are consistent with attention-related difficulties.",
        "Classroom accommodations are recommended.",
    ] {
        body_paragraph(&flattened, sentinel);
    }

    // Table content lands inside the table, and the template's two blank
    // spacer paragraphs survive the rewrite.
    assert!(body_paragraph(&flattened, "Working Memory").in_table);
    assert!(!body_paragraph(&flattened, "Classroom accommodations are recommended.").in_table);
    let blanks = flattened
        .iter()
        .filter(|paragraph| !paragraph.in_table && paragraph.is_blank())
        .count();
    assert_eq!(blanks, 2);
}

#[test]
fn blank_spacers_follow_the_report_instead_of_piling_at_the_top() {
    let (output, _) = render_report_with_template(CLINICAL_TEMPLATE, &clinical_growth_draft())
        .expect("template render");
    let flattened = paragraphs(&output);
    let index_of = |needle: &str| {
        flattened
            .iter()
            .position(|paragraph| paragraph.text.contains(needle))
            .unwrap_or_else(|| panic!("no paragraph contains {needle:?}"))
    };
    let blank_indexes = flattened
        .iter()
        .enumerate()
        .filter(|(_, paragraph)| !paragraph.in_table && paragraph.is_blank())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(blank_indexes.len(), 2);

    // The template's two spacers spread with the report as it grows. The
    // v0.22 renderer emitted every gap while walking the first
    // template-length prefix of the generated report, bunching both
    // spacers into the opening paragraphs.
    assert!(blank_indexes[0] > index_of("The BASC-3 was completed by the parent and teacher."));
    assert!(
        blank_indexes[1] > index_of("Findings are consistent with attention-related difficulties.")
    );
}

#[test]
fn generated_body_text_never_inherits_label_or_signature_decoration() {
    let (output, _) = render_report_with_template(CLINICAL_TEMPLATE, &clinical_growth_draft())
        .expect("template render");
    let flattened = paragraphs(&output);

    // This paragraph's exemplar is the template's "Assessment: Pending
    // review." label paragraph — the inserted text must take the plain
    // Garamond body run, not the bold underlined label run.
    let body = body_paragraph(
        &flattened,
        "Working memory performance fell below the 10th percentile.",
    );
    assert!(
        !body.text_run_underlined,
        "label underline leaked: {body:?}"
    );
    assert!(!body.text_run_bold, "label bold leaked: {body:?}");
    assert_eq!(body.text_run_fonts, ["Garamond"]);

    // The final body paragraph's exemplar is the underlined signature
    // line; direct decoration must be stripped for unrelated text.
    let last = body_paragraph(&flattened, "Classroom accommodations are recommended.");
    assert!(
        !last.text_run_underlined,
        "signature underline leaked: {last:?}"
    );
}

#[test]
fn bullets_stay_numbered_after_a_multi_line_paragraph() {
    // The template has no bulleted exemplar, so list items must come from a
    // generated exemplar selected by KIND. The multi-line paragraph earlier
    // in the report used to shift the generated document's span indices, so
    // the positional lookup handed list items a plain body paragraph and
    // the bullets lost their numbering.
    let report = draft(
        "Psychoeducational Evaluation",
        vec![section(
            "Reason for Referral",
            vec![
                paragraph("First observation.\nSecond observation."),
                ReportBlock::BulletList {
                    items: vec![
                        "Difficulty sustaining attention".to_string(),
                        "Incomplete classwork".to_string(),
                    ],
                },
            ],
        )],
    );
    let (output, _) =
        render_report_with_template(CLINICAL_TEMPLATE, &report).expect("template render");
    let flattened = paragraphs(&output);
    assert!(body_paragraph(&flattened, "Difficulty sustaining attention").numbered);
    assert!(body_paragraph(&flattened, "Incomplete classwork").numbered);

    // A multi-line paragraph renders its newline as a real <w:br/>; a
    // literal newline inside <w:t> displays as nothing in Word.
    let multi_line = body_paragraph(&flattened, "First observation.");
    assert_eq!(multi_line.line_breaks, 1, "{multi_line:?}");
    assert!(multi_line.text.contains("Second observation."));
    assert!(!multi_line.text.contains('\n'));

    // The template has no numbering part, so the bullets' numbering
    // definition, its content-type override, and its document relationship
    // must all be merged into the package — a dangling numId drops the
    // bullet glyphs or makes Word prompt to repair.
    let mut archive = zip::ZipArchive::new(Cursor::new(output.as_slice())).expect("output zip");
    let mut numbering = String::new();
    archive
        .by_name("word/numbering.xml")
        .expect("merged numbering part")
        .read_to_string(&mut numbering)
        .expect("read numbering");
    assert!(numbering.contains("w:numId=\"42\""));
    let mut content_types = String::new();
    archive
        .by_name("[Content_Types].xml")
        .expect("content types")
        .read_to_string(&mut content_types)
        .expect("read content types");
    assert!(content_types.contains("/word/numbering.xml"));
    let mut relationships = String::new();
    archive
        .by_name("word/_rels/document.xml.rels")
        .expect("document relationships")
        .read_to_string(&mut relationships)
        .expect("read relationships");
    assert!(relationships.contains("relationships/numbering"));
}

#[test]
fn custom_named_heading_styles_classify_as_headings() {
    let (output, _) = render_report_with_template(CLINICAL_TEMPLATE, &clinical_growth_draft())
        .expect("template render");
    let flattened = paragraphs(&output);

    // The template's headings use a custom "SectionHeading" style whose
    // heading-ness lives in the style definition (outline level). Heading
    // targets — including a brand-new section — must clone those
    // exemplars, and body text must never land on one.
    for heading in [
        "Reason for Referral",
        "Assessment Results",
        "Summary and Recommendations",
    ] {
        assert_eq!(
            body_paragraph(&flattened, heading).style.as_deref(),
            Some("SectionHeading"),
            "heading {heading:?} did not clone the template heading style"
        );
    }
    for body in [
        "Teacher reports difficulty sustaining attention in class.",
        "The BASC-3 was completed by the parent and teacher.",
        "Findings are consistent with attention-related difficulties.",
    ] {
        assert_ne!(
            body_paragraph(&flattened, body).style.as_deref(),
            Some("SectionHeading"),
            "body text {body:?} landed on a heading exemplar"
        );
    }
    assert_eq!(
        body_paragraph(&flattened, "Psychoeducational Evaluation")
            .style
            .as_deref(),
        Some("Title")
    );
}

#[test]
fn empty_template_cells_do_not_swallow_generated_content() {
    // The template's results table has an empty Score cell. Filling it must
    // never silently drop the value — the renderer regenerates the table
    // when the template cell has nowhere to put the text.
    let (output, _) = render_report_with_template(CLINICAL_TEMPLATE, &clinical_growth_draft())
        .expect("template render");
    let flattened = paragraphs(&output);
    assert!(body_paragraph(&flattened, "82").in_table);
}

#[test]
fn content_controls_fixture_still_renders_a_complete_report() {
    let (output, fidelity) =
        render_report_with_template(CONTENT_CONTROLS_TEMPLATE, &clinical_growth_draft())
            .expect("template render");
    assert_eq!(
        fidelity,
        claria_docx::TemplateRenderFidelity::PlainBodyFallback
    );
    let flattened = paragraphs(&output);
    body_paragraph(
        &flattened,
        "Findings are consistent with attention-related difficulties.",
    );
}
