//! Why a template carved into the sections it did.
//!
//! Section carving is style-driven: [`crate::import_template`] starts a new
//! section only at a paragraph whose style resolves to a heading, and drops
//! everything else into the section already open. A template whose author
//! made headings by bolding and enlarging body text therefore imports as one
//! section — correctly, by the rule, and bafflingly, to the person who wrote
//! it.
//!
//! This module answers that. It runs the real import and reports what each
//! paragraph was classified as and which style rule decided it, so the
//! explanation cannot drift from the behaviour it explains.

use std::collections::HashMap;

use claria_core::models::report::{ReportBlock, ReportTemplateWarning};

use crate::{
    DocxError, TemplateImportStats,
    import::{
        HeadingShape, Headings, MAX_INFERRED_HEADING_DENSITY, MIN_INFERRED_HEADINGS, ParagraphKind,
        ParagraphVerdict, import_with_trail,
    },
    style_catalog::{
        Resolved, ResolvedRule, StyleRecord, normalize_style, package_records, resolve,
    },
};

/// Everything the importer saw in one package.
#[derive(Debug, Clone)]
pub struct TemplateDiagnosis {
    pub source_sha256: String,
    pub title: String,
    pub stats: TemplateImportStats,
    pub warnings: Vec<ReportTemplateWarning>,
    /// The sections the import actually produced, in document order —
    /// after the appearance fallback, if it ran.
    pub sections: Vec<DiagnosedSection>,
    /// Whether the carve above came from the appearance fallback rather
    /// than from applied heading styles.
    pub sections_inferred: bool,
    /// Every paragraph style the package declares, most-used first.
    pub styles: Vec<DiagnosedStyle>,
    /// Every paragraph that carried visible text, in document order.
    pub paragraphs: Vec<DiagnosedParagraph>,
    pub verdict: SectioningVerdict,
}

/// What a fallback that infers headings from appearance would produce for
/// this package, and whether it should be trusted to run.
#[derive(Debug, Clone)]
pub struct InferredSectioning {
    /// Paragraphs the appearance rule would promote, in document order.
    pub headings: Vec<DiagnosedParagraph>,
    /// Share of body paragraphs the rule would promote. A template where
    /// most paragraphs look like headings is one where the rule has found a
    /// house style, not a structure.
    pub density: f32,
    pub trusted: bool,
    pub rejected_because: Option<&'static str>,
}

impl TemplateDiagnosis {
    /// Body paragraphs that look like headings to a reader but carry no
    /// heading style — the ones a clinician means by "it missed my
    /// headings". Ordered as they appear in the document.
    pub fn missed_headings(&self) -> impl Iterator<Item = &DiagnosedParagraph> {
        self.paragraphs
            .iter()
            .filter(|paragraph| paragraph.reads_as_heading())
    }

    /// What an appearance-driven fallback would do with this package.
    ///
    /// Reported for every template, including ones that already carve
    /// correctly, so the rule can be measured against documents it must
    /// never run on.
    pub fn inferred_sectioning(&self) -> InferredSectioning {
        let headings: Vec<DiagnosedParagraph> = self
            .paragraphs
            .iter()
            .filter(|paragraph| paragraph.shape.reads_as_heading())
            .cloned()
            .collect();
        let density = if self.paragraphs.is_empty() {
            0.0
        } else {
            headings.len() as f32 / self.paragraphs.len() as f32
        };
        let rejected_because = if headings.len() < MIN_INFERRED_HEADINGS {
            Some("fewer than two paragraphs read as headings")
        } else if density > MAX_INFERRED_HEADING_DENSITY {
            Some("too many paragraphs read as headings to be a structure")
        } else {
            None
        };
        InferredSectioning {
            headings,
            density,
            trusted: rejected_because.is_none(),
            rejected_because,
        }
    }

    /// Styles that decide a section, whether or not the document uses them.
    pub fn heading_styles(&self) -> impl Iterator<Item = &DiagnosedStyle> {
        self.styles
            .iter()
            .filter(|style| style.verdict == Some(StyleVerdict::Heading))
    }
}

/// What the carve came out as, and whether that is the interesting answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectioningVerdict {
    /// Paragraphs carried heading styles and the carve follows them.
    HeadingsFound,
    /// The package declares heading styles, but no paragraph applies one.
    /// The styles exist in the styles pane and were never used.
    HeadingStylesDeclaredButUnused,
    /// Nothing in the package resolves to a heading style at all.
    NoHeadingStyles,
}

impl SectioningVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HeadingsFound => "headings found",
            Self::HeadingStylesDeclaredButUnused => "heading styles declared but never applied",
            Self::NoHeadingStyles => "no heading styles",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiagnosedSection {
    pub heading: String,
    pub blocks: usize,
    pub characters: usize,
    /// True when the importer invented this section because content arrived
    /// before any heading did.
    pub synthetic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleVerdict {
    Title,
    Heading,
}

impl StyleVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Heading => "heading",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiagnosedStyle {
    /// Normalized styleId — what the rules actually compare.
    pub style_id: String,
    /// Display name as the package spells it, which is what Word's styles
    /// pane shows.
    pub name: String,
    pub based_on: Option<String>,
    pub outline_level: bool,
    pub verdict: Option<StyleVerdict>,
    /// Plain-language reason the verdict came out that way.
    pub because: Option<String>,
    /// Paragraphs in the body that name this style.
    pub used_by: usize,
}

#[derive(Debug, Clone)]
pub struct DiagnosedParagraph {
    pub index: usize,
    pub style_id: Option<String>,
    /// What the importer called it: title, heading, list item, or body.
    pub kind: &'static str,
    pub preview: String,
    pub characters: usize,
    pub shape: HeadingShape,
}

impl DiagnosedParagraph {
    /// Classified as body text while carrying at least two of the signals a
    /// reader uses to see a heading.
    pub fn reads_as_heading(&self) -> bool {
        self.kind == ParagraphKind::Body.as_str() && self.shape.signals() >= 2
    }

    /// Whether the paragraph itself claims an outline level, independently
    /// of its style. Word writes this when a paragraph is promoted in the
    /// navigation pane.
    pub fn claims_outline_level(&self) -> bool {
        self.shape.outline_level
    }
}

/// Import `bytes` and report how the importer classified every paragraph.
///
/// Fails exactly where [`crate::import_template`] fails: this runs the same
/// import rather than a parallel reader, so a package too large, malformed,
/// or empty of supported content is reported as such instead of being
/// half-analyzed.
pub fn analyze_template(bytes: &[u8]) -> Result<TemplateDiagnosis, DocxError> {
    // The styles-only pass: the diagnostic reports what the styles alone
    // decided, and separately what the appearance fallback would add, so a
    // reader can see both halves of the two-tier rule.
    let (styled, trail) = import_with_trail(bytes, Headings::StylesOnly)?;
    // What the importer really produces, which is the styles-only carve
    // unless the fallback took over. Reported alongside the per-paragraph
    // verdicts so both halves of the two-tier rule are visible at once.
    let template = crate::import_template(bytes)?;
    let sections_inferred = template.content.sections.len() > styled.content.sections.len();
    let records = package_records(bytes);
    let usage = style_usage(&trail);

    let paragraphs: Vec<DiagnosedParagraph> = trail.into_iter().map(diagnosed).collect();
    let any_heading_applied = paragraphs
        .iter()
        .any(|paragraph| paragraph.kind == ParagraphKind::Heading.as_str());
    let styles = diagnosed_styles(&records, &usage);
    let any_heading_style = styles
        .iter()
        .any(|style| style.verdict == Some(StyleVerdict::Heading));

    let verdict = match (any_heading_applied, any_heading_style) {
        (true, _) => SectioningVerdict::HeadingsFound,
        (false, true) => SectioningVerdict::HeadingStylesDeclaredButUnused,
        (false, false) => SectioningVerdict::NoHeadingStyles,
    };

    Ok(TemplateDiagnosis {
        source_sha256: template.source_sha256,
        title: template.content.title,
        stats: template.stats,
        warnings: template.warnings,
        sections: template
            .content
            .sections
            .iter()
            .map(|section| DiagnosedSection {
                heading: section.heading.clone(),
                blocks: section.blocks.len(),
                characters: section.blocks.iter().map(block_characters).sum(),
                synthetic: !any_heading_applied && !sections_inferred,
            })
            .collect(),
        sections_inferred,
        styles,
        paragraphs,
        verdict,
    })
}

fn diagnosed(verdict: ParagraphVerdict) -> DiagnosedParagraph {
    DiagnosedParagraph {
        index: verdict.index,
        style_id: verdict.style_id,
        kind: verdict.kind.as_str(),
        preview: verdict.preview,
        characters: verdict.characters,
        shape: verdict.shape,
    }
}

/// How many body paragraphs name each normalized styleId.
fn style_usage(trail: &[ParagraphVerdict]) -> HashMap<String, usize> {
    let mut usage = HashMap::new();
    for verdict in trail {
        let style = verdict
            .style_id
            .as_deref()
            .map(normalize_style)
            .unwrap_or_default();
        *usage.entry(style).or_insert(0) += 1;
    }
    usage
}

fn diagnosed_styles(
    records: &HashMap<String, StyleRecord>,
    usage: &HashMap<String, usize>,
) -> Vec<DiagnosedStyle> {
    let mut styles: Vec<DiagnosedStyle> = records
        .iter()
        .map(|(style_id, record)| {
            let resolved = resolve(style_id, records);
            DiagnosedStyle {
                style_id: style_id.clone(),
                name: record.raw_name.clone(),
                based_on: record.based_on.clone(),
                outline_level: record.outline,
                verdict: resolved.as_ref().map(|(resolved, _)| match resolved {
                    Resolved::Title => StyleVerdict::Title,
                    Resolved::Heading => StyleVerdict::Heading,
                }),
                because: resolved.as_ref().map(|(_, because)| {
                    let rule = match because.rule {
                        ResolvedRule::StyleId => "its styleId",
                        ResolvedRule::Name => "its name",
                        ResolvedRule::OutlineLevel => "its outline level",
                    };
                    if because.hops == 0 {
                        format!("{rule} says so")
                    } else {
                        format!(
                            "{rule} says so, {} basedOn {} away, on `{}`",
                            because.hops,
                            if because.hops == 1 { "hop" } else { "hops" },
                            because.style_id
                        )
                    }
                }),
                used_by: usage.get(style_id).copied().unwrap_or(0),
            }
        })
        .collect();
    // Most-used first: the style carrying the document is the one worth
    // reading about, and an unused declaration is a footnote.
    styles.sort_by(|left, right| {
        right
            .used_by
            .cmp(&left.used_by)
            .then_with(|| left.style_id.cmp(&right.style_id))
    });
    styles
}

fn block_characters(block: &ReportBlock) -> usize {
    match block {
        ReportBlock::Paragraph { text } => text.chars().count(),
        ReportBlock::BulletList { items } => items.iter().map(|item| item.chars().count()).sum(),
        ReportBlock::Table { rows, .. } => rows
            .iter()
            .flat_map(|row| row.iter())
            .map(|cell| cell.chars().count())
            .sum(),
    }
}
