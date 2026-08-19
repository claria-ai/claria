//! `claria-docx` — what Claria sees when it imports a template.
//!
//! A support tool for the question "why did my long document import as one
//! section?". It runs the real importer and prints the classification it
//! produced, so the answer is the code's, not a second opinion.

use std::path::PathBuf;

use clap::Parser;
use claria_docx::{
    DiagnosedParagraph, MAX_TEMPLATE_DOCX_BYTES, SectioningVerdict, TemplateDiagnosis,
    analyze_template,
};
use eyre::{Context, bail};

#[derive(Parser)]
#[command(
    name = "claria-docx",
    about = "Report how Claria's importer classifies a .docx template"
)]
struct Args {
    /// The .docx to analyze.
    path: PathBuf,
    /// List every paragraph and what it was classified as, not just the
    /// headings and the ones that were missed.
    #[arg(long)]
    paragraphs: bool,
    /// List every declared paragraph style, including ones the document
    /// never applies.
    #[arg(long)]
    styles: bool,
    /// Show what an appearance-driven fallback would carve, whether or not
    /// the styles already carve it.
    #[arg(long)]
    infer: bool,
}

fn main() -> eyre::Result<()> {
    let args = Args::parse();

    let bytes =
        std::fs::read(&args.path).wrap_err_with(|| format!("reading {}", args.path.display()))?;
    if bytes.len() as u64 > MAX_TEMPLATE_DOCX_BYTES {
        bail!(
            "{} is {} bytes; Claria refuses templates over {MAX_TEMPLATE_DOCX_BYTES} bytes",
            args.path.display(),
            bytes.len()
        );
    }

    let diagnosis = analyze_template(&bytes).wrap_err("importing the template")?;
    report(&args, &diagnosis);
    Ok(())
}

fn report(args: &Args, diagnosis: &TemplateDiagnosis) {
    let stats = &diagnosis.stats;
    println!("{}", args.path.display());
    println!("  sha256      {}", diagnosis.source_sha256);
    println!("  title       {}", diagnosis.title);
    println!(
        "  content     {} paragraphs, {} bullet lists, {} tables, {} placeholders",
        stats.paragraphs, stats.bullet_lists, stats.tables, stats.placeholder_count
    );
    println!(
        "  sections    {} ({}{})",
        diagnosis.sections.len(),
        diagnosis.verdict.as_str(),
        if diagnosis.sections_inferred {
            "; split by formatting"
        } else {
            ""
        }
    );

    println!("\nSections");
    for (ordinal, section) in diagnosis.sections.iter().enumerate() {
        let synthetic = if section.synthetic {
            "  <- invented; no paragraph carried a heading style"
        } else {
            ""
        };
        println!(
            "  {:>3}. {}  [{} blocks, {} chars]{synthetic}",
            ordinal + 1,
            section.heading,
            section.blocks,
            section.characters
        );
    }

    let heading_styles: Vec<_> = diagnosis.heading_styles().collect();
    println!("\nStyles that would start a section");
    if heading_styles.is_empty() {
        println!("  (none — nothing in this package resolves to a heading style)");
    } else {
        for style in heading_styles {
            let because = style.because.as_deref().unwrap_or("");
            println!(
                "  {:<28} {:<24} used by {:>4} paragraphs   {because}",
                style.name, style.style_id, style.used_by
            );
        }
    }

    if args.styles {
        println!("\nEvery declared paragraph style");
        for style in &diagnosis.styles {
            let verdict = style
                .verdict
                .map(|verdict| verdict.as_str())
                .unwrap_or("body");
            let based_on = style
                .based_on
                .as_deref()
                .map(|base| format!(" basedOn={base}"))
                .unwrap_or_default();
            let outline = if style.outline_level {
                " outlineLvl"
            } else {
                ""
            };
            println!(
                "  {:<28} {:<24} {:<8} used by {:>4}{based_on}{outline}",
                style.name, style.style_id, verdict, style.used_by
            );
        }
    }

    if args.infer {
        let inferred = diagnosis.inferred_sectioning();
        println!(
            "\nAppearance fallback: {} headings, {:.0}% of paragraphs, {}",
            inferred.headings.len(),
            inferred.density * 100.0,
            inferred.rejected_because.unwrap_or("would run")
        );
        for heading in &inferred.headings {
            println!("  {:>4}  {}", heading.index, heading.preview);
        }
    }

    let outlined = diagnosis
        .paragraphs
        .iter()
        .filter(|paragraph| paragraph.claims_outline_level())
        .count();
    if outlined > 0 {
        println!("\nParagraphs carrying their own outline level: {outlined}");
    }

    let missed: Vec<&DiagnosedParagraph> = diagnosis.missed_headings().collect();
    if !missed.is_empty() {
        println!(
            "\nParagraphs that read as headings but carry no heading style ({})",
            missed.len()
        );
        for paragraph in &missed {
            println!(
                "  {:>4}  {:<20} {}",
                paragraph.index,
                signal_summary(paragraph),
                paragraph.preview
            );
        }
    }

    if args.paragraphs {
        println!("\nEvery paragraph");
        for paragraph in &diagnosis.paragraphs {
            println!(
                "  {:>4}  {:<10} {:<24} {}",
                paragraph.index,
                paragraph.kind,
                paragraph.style_id.as_deref().unwrap_or("(no style)"),
                paragraph.preview
            );
        }
    }

    if !diagnosis.warnings.is_empty() {
        println!("\nImport warnings");
        for warning in &diagnosis.warnings {
            println!("  {:?} x{}", warning.code, warning.count);
        }
    }

    println!("\n{}", explanation(diagnosis));
}

/// The signals that made this paragraph look like a heading, abbreviated.
fn signal_summary(paragraph: &DiagnosedParagraph) -> String {
    let shape = paragraph.shape;
    let mut signals = Vec::new();
    if shape.all_bold {
        signals.push("bold");
    }
    if shape.all_caps {
        signals.push("caps");
    }
    if shape.short {
        signals.push("short");
    }
    if shape.unpunctuated {
        signals.push("no-stop");
    }
    signals.join("+")
}

/// The takeaway, in the terms the reader came with.
fn explanation(diagnosis: &TemplateDiagnosis) -> String {
    let missed = diagnosis.missed_headings().count();
    match diagnosis.verdict {
        SectioningVerdict::HeadingsFound => format!(
            "Carved into {} sections from applied heading styles.",
            diagnosis.stats.sections
        ),
        SectioningVerdict::HeadingStylesDeclaredButUnused => format!(
            "This package declares heading styles but no paragraph applies one, so the whole \
             document imported as a single section. {missed} paragraphs read as headings by \
             shape. Fix it in Word by selecting each heading and applying a Heading style from \
             the styles pane."
        ),
        SectioningVerdict::NoHeadingStyles => format!(
            "Nothing in this package resolves to a heading style — no styleId starting with \
             `heading`, no style named for one, no outline level — so the whole document \
             imported as a single section. {missed} paragraphs read as headings by shape. Fix \
             it in Word by applying a Heading style to each one."
        ),
    }
}
