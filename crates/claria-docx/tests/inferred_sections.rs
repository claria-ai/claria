//! Sectioning a template whose author never applied a heading style.
//!
//! Word's heading styles are the contract, and a template that uses them is
//! carved exactly as it asks. Most real clinical templates do not: their
//! headings are body text someone bolded. Those used to import as a single
//! section holding the whole document, which is a report the writer has to
//! produce in one enormous response.
//!
//! The fallback reads appearance instead, and every test here is about the
//! line between a structure and a formatting habit.

use std::io::Cursor;

use claria_core::models::report::ReportTemplateWarningCode;
use claria_docx::import_template;
use docx_rs::{Docx, Paragraph, Run};

fn pack(document: Docx) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    document.build().pack(&mut output).expect("pack test DOCX");
    output.into_inner()
}

fn bold(text: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().bold().add_text(text))
}

fn plain(text: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(text))
}

fn inferred_from_formatting(imported: &claria_docx::ImportedTemplate) -> bool {
    imported
        .warnings
        .iter()
        .any(|warning| warning.code == ReportTemplateWarningCode::SectionsInferredFromFormatting)
}

fn headings(imported: &claria_docx::ImportedTemplate) -> Vec<String> {
    imported
        .content
        .sections
        .iter()
        .map(|section| section.heading.clone())
        .collect()
}

/// The shape of a real clinical template: nine Heading styles sitting
/// unused in the styles pane while every section header is bolded body
/// text.
#[test]
fn bold_headers_carve_sections_when_no_heading_style_was_applied() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Title")
                    .add_run(Run::new().add_text("Evaluation Template")),
            )
            .add_paragraph(bold("Reason for Referral"))
            .add_paragraph(plain("The child was referred for attention concerns."))
            .add_paragraph(bold("Background Information"))
            .add_paragraph(plain("A concise summary of presenting concerns."))
            .add_paragraph(bold("Recommendations"))
            .add_paragraph(plain("Consider classroom accommodations.")),
    );

    let imported = import_template(&bytes).expect("import template");

    assert_eq!(
        headings(&imported),
        vec![
            "Reason for Referral",
            "Background Information",
            "Recommendations"
        ]
    );
    assert!(
        inferred_from_formatting(&imported),
        "a carve this speculative has to say so"
    );
}

/// Capitals are the other convention a template author picks, and the rule
/// takes either.
#[test]
fn capitalized_headers_carve_sections_too() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(plain("REASON FOR REFERRAL"))
            .add_paragraph(plain("The child was referred for attention concerns."))
            .add_paragraph(plain("BACKGROUND INFORMATION"))
            .add_paragraph(plain("A concise summary of presenting concerns.")),
    );

    let imported = import_template(&bytes).expect("import template");

    assert_eq!(
        headings(&imported),
        vec!["REASON FOR REFERRAL", "BACKGROUND INFORMATION"]
    );
}

/// Styles win outright. A template that applied them gets exactly its own
/// carve, and the bold text inside it stays body text — otherwise every
/// emphasized lead-in in a working template would silently become a
/// section.
#[test]
fn applied_heading_styles_are_never_second_guessed_by_formatting() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("Findings")),
            )
            .add_paragraph(bold("Attention"))
            .add_paragraph(plain("Scores were low average."))
            .add_paragraph(bold("Memory"))
            .add_paragraph(plain("Scores were average.")),
    );

    let imported = import_template(&bytes).expect("import template");

    assert_eq!(headings(&imported), vec!["Findings"]);
    assert!(
        !inferred_from_formatting(&imported),
        "the fallback must not run when a heading style was applied"
    );
}

/// A document set entirely in bold has a house style, not a structure. A
/// section per paragraph is worse for the writer than one section, so the
/// density guard refuses the carve.
#[test]
fn a_document_that_is_entirely_bold_is_left_as_one_section() {
    let mut document = Docx::new();
    for ordinal in 0..10 {
        document = document.add_paragraph(bold(&format!("Emphasized line {ordinal}")));
    }
    let bytes = pack(document);

    let imported = import_template(&bytes).expect("import template");

    assert_eq!(headings(&imported), vec!["Imported content"]);
    assert!(!inferred_from_formatting(&imported));
}

/// One inferred heading produces the same single section the fallback
/// exists to avoid, so it is not worth the guess.
#[test]
fn a_single_bold_line_is_not_a_structure() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(bold("Reason for Referral"))
            .add_paragraph(plain("The child was referred for attention concerns."))
            .add_paragraph(plain("A concise summary of presenting concerns."))
            .add_paragraph(plain("Consider classroom accommodations."))
            .add_paragraph(plain("Scores were low average.")),
    );

    let imported = import_template(&bytes).expect("import template");

    assert_eq!(headings(&imported), vec!["Imported content"]);
}

/// Every shape condition is load-bearing. Each paragraph below is bold and
/// fails exactly one of them, and none may open a section.
#[test]
fn emphasis_alone_does_not_make_a_heading() {
    let long_line = "Bold narrative that runs well past the point where a label \
                     would have stopped and is plainly a sentence of prose";
    let bytes = pack(
        Docx::new()
            .add_paragraph(bold("Reason for Referral"))
            .add_paragraph(plain("Narrative."))
            .add_paragraph(bold("Background Information"))
            // Ends in a colon: a field label, not a heading.
            .add_paragraph(bold("Name of Patient:"))
            // Ends in a full stop: a sentence.
            .add_paragraph(bold("The child was referred for attention concerns."))
            // Too long to be a label.
            .add_paragraph(bold(long_line))
            // No letters: a typed rule, which is short and unpunctuated.
            .add_paragraph(bold("__________________________")),
    );

    let imported = import_template(&bytes).expect("import template");

    assert_eq!(
        headings(&imported),
        vec!["Reason for Referral", "Background Information"],
        "only the two label-shaped paragraphs may open a section"
    );
}

/// Content before the first inferred heading keeps the invented section it
/// already had, rather than being folded into the first real one.
#[test]
fn a_preamble_before_the_first_inferred_heading_keeps_its_own_section() {
    let bytes = pack(
        Docx::new()
            .add_paragraph(plain("Name of Patient [child's full name]"))
            .add_paragraph(plain("Age [child's age]"))
            .add_paragraph(bold("Reason for Referral"))
            .add_paragraph(plain("Narrative."))
            .add_paragraph(bold("Recommendations"))
            .add_paragraph(plain("Consider classroom accommodations.")),
    );

    let imported = import_template(&bytes).expect("import template");

    assert_eq!(
        headings(&imported),
        vec!["Imported content", "Reason for Referral", "Recommendations"]
    );
}
