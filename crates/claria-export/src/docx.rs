use std::io::Cursor;

use docx_rs::{AlignmentType, BreakType, Docx, Paragraph, Run, RunFonts, Style, StyleType};

use crate::error::ExportError;
use crate::styles::DocumentStyles;

/// Generate a DOCX document from rendered Markdown-ish template output.
///
/// The `rendered` content uses a simple subset:
/// - `# Heading` → DOCX Heading 1
/// - `## Heading` → DOCX Heading 2
/// - `### Heading` → DOCX Heading 3
/// - `- item` → bullet list item (prefixed with bullet character)
/// - `**bold**` → bold run
/// - `---` or `***` → page break
/// - Everything else → normal paragraph
pub fn generate_docx(rendered: &str, styles: &DocumentStyles) -> Result<Vec<u8>, ExportError> {
    let mut docx = Docx::new();

    // Define heading styles
    docx = docx
        .add_style(heading_style("Heading1", "heading 1", styles.heading1_size))
        .add_style(heading_style("Heading2", "heading 2", styles.heading2_size))
        .add_style(heading_style("Heading3", "heading 3", styles.heading3_size));

    for line in rendered.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            docx = docx.add_paragraph(Paragraph::new());
            continue;
        }

        if let Some(text) = trimmed.strip_prefix("### ") {
            docx = docx.add_paragraph(heading_paragraph(text, "Heading3"));
        } else if let Some(text) = trimmed.strip_prefix("## ") {
            docx = docx.add_paragraph(heading_paragraph(text, "Heading2"));
        } else if let Some(text) = trimmed.strip_prefix("# ") {
            docx = docx.add_paragraph(heading_paragraph(text, "Heading1"));
        } else if let Some(text) = trimmed.strip_prefix("- ") {
            docx = docx.add_paragraph(bullet_paragraph(text, styles));
        } else if trimmed == "---" || trimmed == "***" {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_break(BreakType::Page)),
            );
        } else {
            docx = docx.add_paragraph(body_paragraph(trimmed, styles));
        }
    }

    let mut buf = Cursor::new(Vec::new());
    docx.build()
        .pack(&mut buf)
        .map_err(|e| ExportError::Docx(e.to_string()))?;

    Ok(buf.into_inner())
}

fn heading_style(style_id: &str, name: &str, size_pt: usize) -> Style {
    Style::new(style_id, StyleType::Paragraph)
        .name(name)
        .size(size_pt * 2) // OOXML uses half-points
}

fn heading_paragraph(text: &str, style_id: &str) -> Paragraph {
    Paragraph::new()
        .style(style_id)
        .add_run(Run::new().add_text(text))
}

fn bullet_paragraph(text: &str, styles: &DocumentStyles) -> Paragraph {
    let bullet_run = Run::new()
        .add_text("\u{2022} ")
        .fonts(RunFonts::new().ascii(&styles.body_font));

    let mut para = Paragraph::new()
        .align(AlignmentType::Left)
        .add_run(bullet_run);

    for run in parse_inline(text, styles) {
        para = para.add_run(run);
    }

    para
}

fn body_paragraph(text: &str, styles: &DocumentStyles) -> Paragraph {
    let mut para = Paragraph::new().align(AlignmentType::Left);
    for run in parse_inline(text, styles) {
        para = para.add_run(run);
    }
    para
}

/// Parse simple inline formatting: **bold** segments.
fn parse_inline(text: &str, styles: &DocumentStyles) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("**") {
        let before = &remaining[..start];
        if !before.is_empty() {
            runs.push(
                Run::new()
                    .add_text(before)
                    .fonts(RunFonts::new().ascii(&styles.body_font)),
            );
        }

        let after_start = &remaining[start + 2..];
        if let Some(end) = after_start.find("**") {
            let bold_text = &after_start[..end];
            runs.push(
                Run::new()
                    .add_text(bold_text)
                    .bold()
                    .fonts(RunFonts::new().ascii(&styles.body_font)),
            );
            remaining = &after_start[end + 2..];
        } else {
            // No closing **, treat rest as normal text
            runs.push(
                Run::new()
                    .add_text(remaining)
                    .fonts(RunFonts::new().ascii(&styles.body_font)),
            );
            return runs;
        }
    }

    if !remaining.is_empty() {
        runs.push(
            Run::new()
                .add_text(remaining)
                .fonts(RunFonts::new().ascii(&styles.body_font)),
        );
    }

    runs
}
