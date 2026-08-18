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
use claria_docx::{import_template, render_report_with_template};
use quick_xml::{Reader, events::Event};
use uuid::Uuid;

const CLINICAL_TEMPLATE: &[u8] =
    include_bytes!("../../../fixtures/docx-templates/clinical-eval.docx");
const CONTENT_CONTROLS_TEMPLATE: &[u8] =
    include_bytes!("../../../fixtures/docx-templates/content-controls.docx");
const TEMPLATE_C_LIKE: &[u8] =
    include_bytes!("../../../fixtures/docx-templates/template-c-like.docx");

/// One `<w:r>` of the exported document that carries visible text.
#[derive(Debug, Clone)]
struct TextRun {
    text: String,
    bold: bool,
    underlined: bool,
}

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
    /// Real `<w:tab/>` elements inside runs — tab *stops* in `w:pPr` are
    /// paragraph properties and are not counted.
    tabs: usize,
    runs: Vec<TextRun>,
}

impl Paragraph {
    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    fn run_with(&self, needle: &str) -> &TextRun {
        self.runs
            .iter()
            .find(|run| run.text.contains(needle))
            .unwrap_or_else(|| panic!("no run of {:?} contains {needle:?}", self.text))
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
    let mut run_text = String::new();
    let mut in_run = false;
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
                            tabs: 0,
                            runs: Vec::new(),
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
                        in_run = true;
                        run_underline = false;
                        run_bold = false;
                        run_font = None;
                        run_text.clear();
                    }
                    b"tab" if in_run && !in_rpr => {
                        if let Some(paragraph) = &mut current {
                            paragraph.tabs += 1;
                        }
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
                    let decoded = text.decode().expect("text");
                    paragraph.text.push_str(&decoded);
                    run_text.push_str(&decoded);
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
                b"r" => {
                    in_run = false;
                    if let Some(paragraph) = &mut current
                        && !run_text.is_empty()
                    {
                        paragraph.runs.push(TextRun {
                            text: std::mem::take(&mut run_text),
                            bold: run_bold,
                            underlined: run_underline,
                        });
                    }
                    run_text.clear();
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
fn blank_spacers_stay_inside_the_section_they_were_authored_in() {
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

    // Each spacer rides with the template paragraph it precedes, so it
    // stays in the section its author put it in however far the report
    // outgrows the template. The v0.22 renderer scattered spacers at
    // proportional positions instead.
    assert!(
        index_of("Teacher reports difficulty sustaining attention in class.") < blank_indexes[0]
    );
    assert!(blank_indexes[0] < index_of("Assessment Results"));
    assert!(index_of("Assessment Results") < blank_indexes[1]);
    assert!(blank_indexes[1] < index_of("Summary and Recommendations"));
}

#[test]
fn generated_body_text_never_inherits_label_or_signature_decoration() {
    let (output, _) = render_report_with_template(CLINICAL_TEMPLATE, &clinical_growth_draft())
        .expect("template render");
    let flattened = paragraphs(&output);

    // The first body paragraph of "Assessment Results" is written back into
    // the template's own paragraph for that slot — the "Assessment: Pending
    // review." label line — so it keeps the Garamond body run and lands in
    // it rather than in the bold underlined label run.
    let aligned = body_paragraph(
        &flattened,
        "The BASC-3 was completed by the parent and teacher.",
    );
    assert_eq!(aligned.text_run_fonts, ["Garamond"]);
    assert!(!aligned.text_run_bold, "label bold leaked: {aligned:?}");
    assert!(
        !aligned.text_run_underlined,
        "label underline leaked: {aligned:?}"
    );

    // This paragraph outgrew the section's template paragraphs, so it is a
    // clone of the underlined signature line — borrowed formatting, whose
    // direct decoration must be stripped.
    let cloned = body_paragraph(
        &flattened,
        "Working memory performance fell below the 10th percentile.",
    );
    assert!(
        !cloned.text_run_underlined,
        "signature underline leaked: {cloned:?}"
    );
    assert!(!cloned.text_run_bold, "signature bold leaked: {cloned:?}");

    // A section the template never had clones its exemplars document-wide,
    // and strips them the same way.
    let added = body_paragraph(&flattened, "Classroom accommodations are recommended.");
    assert!(
        !added.text_run_underlined,
        "signature underline leaked: {added:?}"
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

/// A drafting run against the appearance-carved template: the tabbed header
/// block filled in and grown by a row the template never had, one section
/// renamed, bodies rewritten, the results table filled, the two underlined
/// test names left alone, and one section added at the end.
fn template_c_draft() -> ReportDraft {
    let mut content = import_template(TEMPLATE_C_LIKE)
        .expect("import template-c-like")
        .content;
    let find = |content: &ReportContent, heading: &str| {
        content
            .sections
            .iter()
            .position(|section| section.heading == heading)
            .unwrap_or_else(|| panic!("no imported section {heading:?}"))
    };

    let header = find(&content, "Imported content");
    content.sections[header].blocks = vec![
        paragraph("Name of Patient\tJordan Thomas Rivera"),
        paragraph("Date of Birth\tNovember 4, 2002"),
        paragraph("Age\t\t23 years, 9 months"),
        paragraph("Examiner\tAlice Chen, PsyD"),
    ];

    let referral = find(&content, "Reason for Referral");
    content.sections[referral].heading = "Referral Question".to_string();
    content.sections[referral].blocks = vec![paragraph(
        "Aiden was referred by his classroom teacher for concerns about attention and impulsivity.",
    )];

    let background = find(&content, "Background Information");
    content.sections[background].blocks = vec![
        paragraph("Aiden lives with his mother and sister and attends the neighborhood school."),
        paragraph("Prenatal and birth history were unremarkable per maternal report."),
    ];

    let results = find(&content, "Results");
    content.sections[results].blocks = vec![
        paragraph("The Conners CPT3 was administered under standard conditions."),
        ReportBlock::Table {
            rows: vec![
                vec!["Conners CPT3 Scale".to_string(), "T-Score".to_string()],
                vec!["Detectability".to_string(), "62".to_string()],
            ],
            has_header: true,
            column_widths: None,
        },
    ];

    let summary = find(&content, "Summary and Clinical Interpretation");
    content.sections[summary].blocks = vec![paragraph(
        "Findings are consistent with an attention-related presentation.",
    )];

    let recommendations = find(&content, "Recommendations");
    content.sections[recommendations].blocks = vec![paragraph(
        "Classroom accommodations and a follow-up review in six months are advised.",
    )];

    content.sections.push(section(
        "Diagnostic Impression",
        vec![paragraph(
            "Attention-Deficit/Hyperactivity Disorder, predominantly inattentive presentation.",
        )],
    ));

    let mut report = draft("placeholder", Vec::new());
    report.content = content;
    report
}

#[test]
fn appearance_carved_headings_keep_their_bold_when_rewritten() {
    let (output, fidelity) =
        render_report_with_template(TEMPLATE_C_LIKE, &template_c_draft()).expect("template render");
    assert_eq!(fidelity, claria_docx::TemplateRenderFidelity::Reconstructed);
    let flattened = paragraphs(&output);

    // The template applies no heading style at all: bold is the only thing
    // that makes these paragraphs headings, including the renamed one and
    // the section the report added.
    for heading in [
        "Referral Question",
        "Background Information",
        "Assessment Procedures",
        "Results",
        "Summary and Clinical Interpretation",
        "Recommendations",
        "Diagnostic Impression",
    ] {
        let paragraph = body_paragraph(&flattened, heading);
        assert!(
            paragraph.text_run_bold,
            "heading {heading:?} lost its bold: {paragraph:?}"
        );
    }

    // Prose and instruction paragraphs of this template class carry stray
    // w:outlineLvl. Classifying by it promoted them to headings and pushed
    // the real headings into body text.
    for body in [
        "Aiden was referred by his classroom teacher",
        "Prenatal and birth history were unremarkable",
        "The Conners CPT3 was administered",
    ] {
        let paragraph = body_paragraph(&flattened, body);
        assert!(
            !paragraph.text_run_bold,
            "body text {body:?} landed on a heading exemplar: {paragraph:?}"
        );
    }

    // Body text the report did not rewrite keeps the template's own direct
    // formatting.
    assert!(
        body_paragraph(&flattened, "Conners Continuous Performance Test")
            .run_with("Conners Continuous Performance Test")
            .underlined,
        "an unrewritten test name lost its underline"
    );
}

#[test]
fn the_export_re_imports_with_the_sections_the_report_had() {
    let report = template_c_draft();
    let (output, _) =
        render_report_with_template(TEMPLATE_C_LIKE, &report).expect("template render");
    let round_trip = import_template(&output).expect("re-import the export");
    assert_eq!(
        round_trip
            .content
            .sections
            .iter()
            .map(|section| section.heading.as_str())
            .collect::<Vec<_>>(),
        report
            .content
            .sections
            .iter()
            .map(|section| section.heading.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn tabbed_header_rows_export_real_tabs_and_keep_their_label_bolding() {
    let (output, _) =
        render_report_with_template(TEMPLATE_C_LIKE, &template_c_draft()).expect("template render");
    let flattened = paragraphs(&output);

    // A literal tab inside <w:t> is not a tab in Word.
    for paragraph in &flattened {
        assert!(
            !paragraph.text.contains('\t'),
            "literal tab inside <w:t>: {paragraph:?}"
        );
    }

    let patient = body_paragraph(&flattened, "Jordan Thomas Rivera");
    assert_eq!(patient.tabs, 1, "{patient:?}");
    assert!(patient.run_with("Name of Patient").bold);
    assert!(
        !patient.run_with("Jordan Thomas Rivera").bold,
        "the label's bold bled onto the value: {patient:?}"
    );

    let age = body_paragraph(&flattened, "23 years, 9 months");
    assert_eq!(age.tabs, 2, "{age:?}");
    assert!(age.run_with("Age").bold);

    // A row the template never had brings its own tab and leaves the
    // exemplar's tab stops behind, instead of opening with tab soup.
    let examiner = body_paragraph(&flattened, "Alice Chen");
    assert_eq!(examiner.tabs, 1, "{examiner:?}");
    assert!(!examiner.run_with("Examiner").bold);
}

#[test]
fn the_export_invents_neither_a_title_nor_an_imported_content_heading() {
    let (output, _) =
        render_report_with_template(TEMPLATE_C_LIKE, &template_c_draft()).expect("template render");
    let flattened = paragraphs(&output);
    for invented in ["Imported content", "Imported report template"] {
        assert!(
            flattened
                .iter()
                .all(|paragraph| !paragraph.text.contains(invented)),
            "the importer's invented {invented:?} was written into the document"
        );
    }
    assert!(
        flattened
            .iter()
            .all(|paragraph| paragraph.style.as_deref() != Some("Title")),
        "a template with no title paragraph gained one"
    );

    // The header block still leads the document, it just has no heading.
    let index_of = |needle: &str| {
        flattened
            .iter()
            .position(|paragraph| paragraph.text.contains(needle))
            .unwrap_or_else(|| panic!("no paragraph contains {needle:?}"))
    };
    assert!(index_of("Jordan Thomas Rivera") < index_of("Referral Question"));
}

#[test]
fn merged_template_tables_never_reappear_in_the_export() {
    let (output, _) =
        render_report_with_template(TEMPLATE_C_LIKE, &template_c_draft()).expect("template render");
    let xml = String::from_utf8(document_xml(&output)).expect("document XML is UTF-8");
    assert!(!xml.contains("gridSpan"), "a merged table survived");
    assert!(!xml.contains("vMerge"), "a merged table survived");

    let flattened = paragraphs(&output);
    for absent in [
        "BASC-3 Parent Rating Scales",
        "Externalizing Problems",
        "BRIEF2 Composite Summary",
        "Behavior Regulation Index",
    ] {
        assert!(
            flattened
                .iter()
                .all(|paragraph| !paragraph.text.contains(absent)),
            "{absent:?} came back from a table the draft never held"
        );
    }

    // The table the draft does hold is written back into the template's own.
    assert!(body_paragraph(&flattened, "Detectability").in_table);
    assert!(body_paragraph(&flattened, "62").in_table);
}

#[test]
fn spacers_stay_in_their_sections_when_merged_tables_are_dropped() {
    let (output, _) =
        render_report_with_template(TEMPLATE_C_LIKE, &template_c_draft()).expect("template render");
    let flattened = paragraphs(&output);
    let index_of = |needle: &str| {
        flattened
            .iter()
            .position(|paragraph| paragraph.text.contains(needle))
            .unwrap_or_else(|| panic!("no paragraph contains {needle:?}"))
    };
    let blanks = flattened
        .iter()
        .enumerate()
        .filter(|(_, paragraph)| !paragraph.in_table && paragraph.is_blank())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(blanks.len(), 5, "{blanks:?}");

    assert!(index_of("Alice Chen") < blanks[0] && blanks[0] < index_of("Referral Question"));
    assert!(
        index_of("Referral Question") < blanks[1] && blanks[1] < index_of("Background Information")
    );
    // The two spacers that framed the merged tables stay where the tables
    // were, above the next heading.
    assert!(index_of("Detectability") < blanks[2]);
    assert!(blanks[3] < index_of("Summary and Clinical Interpretation"));
    assert!(
        index_of("Summary and Clinical Interpretation") < blanks[4]
            && blanks[4] < index_of("Recommendations")
    );
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
