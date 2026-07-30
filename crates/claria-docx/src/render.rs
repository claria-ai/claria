use std::io::Cursor;

use claria_core::models::report::{ReportBlock, ReportDraft, validate_report_content};
use docx_rs::{
    AbstractNumbering, AlignmentType, BreakType, Docx, IndentLevel, Level, LevelJc, LevelText,
    NumberFormat, Numbering, NumberingId, PageMargin, Paragraph, Run, RunFonts, SpecialIndentType,
    Start, Style, StyleType,
};

use crate::error::DocxError;

const LETTER_WIDTH_TWIPS: u32 = 12_240;
const LETTER_HEIGHT_TWIPS: u32 = 15_840;
const ONE_INCH_TWIPS: i32 = 1_440;
const BULLET_NUMBERING_ID: usize = 42;
const BODY_FONT: &str = "Times New Roman";

pub fn render_report(draft: &ReportDraft) -> Result<Vec<u8>, DocxError> {
    validate_report_content(&draft.content)
        .map_err(|error| DocxError::Render(error.to_string()))?;

    let fonts = RunFonts::new()
        .ascii(BODY_FONT)
        .hi_ansi(BODY_FONT)
        .east_asia(BODY_FONT);
    let mut document = Docx::new()
        .default_fonts(fonts.clone())
        .default_size(24)
        .add_style(
            Style::new("Title", StyleType::Paragraph)
                .name("Title")
                .based_on("Normal")
                .next("Normal")
                .fonts(fonts.clone())
                .size(32)
                .bold()
                .align(AlignmentType::Center),
        )
        .add_style(
            Style::new("Heading1", StyleType::Paragraph)
                .name("Heading 1")
                .based_on("Normal")
                .next("Normal")
                .fonts(fonts)
                .size(28)
                .bold(),
        )
        .add_abstract_numbering(
            AbstractNumbering::new(BULLET_NUMBERING_ID).add_level(
                Level::new(
                    0,
                    Start::new(1),
                    NumberFormat::new("bullet"),
                    LevelText::new("•"),
                    LevelJc::new("left"),
                )
                .indent(Some(720), Some(SpecialIndentType::Hanging(360)), None, None)
                .fonts(RunFonts::new().ascii("Symbol").hi_ansi("Symbol")),
            ),
        )
        .add_numbering(Numbering::new(BULLET_NUMBERING_ID, BULLET_NUMBERING_ID))
        .page_size(LETTER_WIDTH_TWIPS, LETTER_HEIGHT_TWIPS)
        .page_margin(
            PageMargin::new()
                .top(ONE_INCH_TWIPS)
                .right(ONE_INCH_TWIPS)
                .bottom(ONE_INCH_TWIPS)
                .left(ONE_INCH_TWIPS)
                .header(720)
                .footer(720)
                .gutter(0),
        )
        .created_at(&draft.created_at.to_string())
        .updated_at(&draft.updated_at.to_string());

    let mut paragraph_id = 1_u32;
    document = document.add_paragraph(
        Paragraph::new()
            .id(next_paragraph_id(&mut paragraph_id))
            .style("Title")
            .add_run(Run::new().add_text(&draft.content.title)),
    );

    for section in &draft.content.sections {
        document = document.add_paragraph(
            Paragraph::new()
                .id(next_paragraph_id(&mut paragraph_id))
                .style("Heading1")
                .keep_next(true)
                .add_run(Run::new().add_text(&section.heading)),
        );

        for block in &section.blocks {
            match block {
                ReportBlock::Paragraph { text } => {
                    // Each source line is a real Word paragraph, including
                    // intentional blank lines between nonempty lines.
                    for line in text.split('\n') {
                        let mut paragraph = Paragraph::new()
                            .id(next_paragraph_id(&mut paragraph_id))
                            .style("Normal");
                        if !line.is_empty() {
                            paragraph = paragraph.add_run(Run::new().add_text(line));
                        }
                        document = document.add_paragraph(paragraph);
                    }
                }
                ReportBlock::BulletList { items } => {
                    for item in items {
                        let mut lines = item.split('\n');
                        let first = lines.next().unwrap_or_default();
                        let mut run = Run::new().add_text(first);
                        for line in lines {
                            run = run.add_break(BreakType::TextWrapping).add_text(line);
                        }
                        document = document.add_paragraph(
                            Paragraph::new()
                                .id(next_paragraph_id(&mut paragraph_id))
                                .style("Normal")
                                .numbering(
                                    NumberingId::new(BULLET_NUMBERING_ID),
                                    IndentLevel::new(0),
                                )
                                .add_run(run),
                        );
                    }
                }
            }
        }
    }

    let mut output = Cursor::new(Vec::new());
    document
        .build()
        .pack(&mut output)
        .map_err(|error| DocxError::Render(error.to_string()))?;
    Ok(output.into_inner())
}

fn next_paragraph_id(next: &mut u32) -> String {
    let current = *next;
    *next = next.saturating_add(1);
    format!("{current:08x}")
}
