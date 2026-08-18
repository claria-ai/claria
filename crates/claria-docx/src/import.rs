//! Bounded DOCX content import for the Writing workspace.
//!
//! Imported packages are untrusted input. This module preflights ZIP limits,
//! rejects active/embedded content, disables image decoding, and converts only
//! supported visible body content into Claria's structured report model. It
//! never retains the source package, filename, path, headers, or relationships.

use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
};

use claria_core::models::report::{
    ReportBlock, ReportContent, ReportSection, ReportTemplateWarning, ReportTemplateWarningCode,
    report_template_placeholder_count, validate_report_content,
};
use docx_rs::{
    DocumentChild, InsertChild, MoveToChild, Paragraph, ParagraphChild, ReadDocxOptions, Run,
    RunChild, SectionChild, StructuredDataTag, StructuredDataTagChild, Table, TableCell,
    TableCellContent, TableChild, TableRowChild, read_docx_with_options,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::ZipArchive;

use crate::{
    error::DocxError,
    style_catalog::{StyleCatalog, normalize_style},
};

pub const MAX_TEMPLATE_DOCX_BYTES: u64 = 10 * 1024 * 1024;
const MAX_TEMPLATE_UNCOMPRESSED_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TEMPLATE_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TEMPLATE_ZIP_ENTRIES: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportedTemplate {
    pub content: ReportContent,
    pub source_sha256: String,
    pub warnings: Vec<ReportTemplateWarning>,
    pub stats: TemplateImportStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateImportStats {
    pub sections: u32,
    pub paragraphs: u32,
    pub bullet_lists: u32,
    pub tables: u32,
    pub table_cells: u32,
    pub placeholder_count: u32,
}

/// Fewest inferred headings worth re-carving for: one produces the same
/// single section the fallback exists to avoid.
pub(crate) const MIN_INFERRED_HEADINGS: usize = 2;

/// Largest share of paragraphs the appearance rule may promote before its
/// result is refused.
///
/// A template that strictly alternates heading and paragraph is already
/// half headings and is a perfectly good structure, so the bar has to sit
/// above one half. Past it there is more heading than content, which is
/// what a document set entirely in bold looks like — and a section per
/// paragraph is worse for a writer than one section.
pub(crate) const MAX_INFERRED_HEADING_DENSITY: f32 = 0.6;

pub fn import_template(bytes: &[u8]) -> Result<ImportedTemplate, DocxError> {
    // Styles first, always. A template whose author applied Word's heading
    // styles gets exactly the carve it asks for, and nothing below can
    // change that.
    let (template, trail) = import_package(bytes, Headings::StylesOnly, Detail::Skip)?;
    if trail
        .iter()
        .any(|verdict| verdict.kind == ParagraphKind::Heading)
    {
        return Ok(template);
    }

    // No paragraph carried one, so the whole document is sitting in one
    // invented section. Appearance is all that is left to read.
    if !inferred_headings_are_trustworthy(&trail) {
        return Ok(template);
    }
    let (inferred, _) = import_package(bytes, Headings::StylesOrAppearance, Detail::Skip)?;
    Ok(inferred)
}

/// Whether the appearance rule found something that looks like a structure
/// rather than a formatting habit.
fn inferred_headings_are_trustworthy(trail: &[ParagraphVerdict]) -> bool {
    if trail.is_empty() {
        return false;
    }
    let inferred = trail
        .iter()
        .filter(|verdict| verdict.shape.reads_as_heading())
        .count();
    let density = inferred as f32 / trail.len() as f32;
    inferred >= MIN_INFERRED_HEADINGS && density <= MAX_INFERRED_HEADING_DENSITY
}

/// Which paragraphs may open a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Headings {
    /// Only paragraphs whose style resolves to a heading.
    StylesOnly,
    /// Those, plus paragraphs that read as headings by appearance. Used
    /// only after a styles-only pass found none.
    StylesOrAppearance,
}

/// Whether a pass keeps the text and style of each paragraph, which only
/// the diagnostic needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detail {
    Keep,
    Skip,
}

/// The same import, keeping a per-paragraph record of how each one was
/// classified.
///
/// The diagnostic reports what the importer actually did rather than a second
/// implementation of the rules, which is the only way its answer to "why is
/// this one section" can be trusted.
pub(crate) fn import_with_trail(
    bytes: &[u8],
    headings: Headings,
) -> Result<(ImportedTemplate, Vec<ParagraphVerdict>), DocxError> {
    import_package(bytes, headings, Detail::Keep)
}

fn import_package(
    bytes: &[u8],
    headings: Headings,
    detail: Detail,
) -> Result<(ImportedTemplate, Vec<ParagraphVerdict>), DocxError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TEMPLATE_DOCX_BYTES {
        return Err(DocxError::UnsafeTemplate(
            "the file exceeds the 10 MiB import limit".to_string(),
        ));
    }

    let preflight = preflight_package(bytes)?;
    // A malformed package must not be able to terminate the desktop process
    // through an unchecked assumption in the third-party reader.
    let parsed = std::panic::catch_unwind(|| {
        read_docx_with_options(bytes, ReadDocxOptions::default().with_image_previews(false))
    })
    .map_err(|_| DocxError::Import("the DOCX package could not be parsed safely".to_string()))?
    .map_err(|_| DocxError::Import("the DOCX package is malformed or unsupported".to_string()))?;

    let catalog = StyleCatalog::from_package(bytes);
    let mut builder = ImportBuilder::new(preflight.warnings, catalog, headings, detail);
    for child in &parsed.document.children {
        builder.document_child(child);
    }
    let (content, mut stats, warnings, trail) = builder.finish()?;
    validate_report_content(&content)
        .map_err(|error| DocxError::Import(format!("imported content is not valid: {error}")))?;
    stats.placeholder_count = report_template_placeholder_count(&content);

    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((
        ImportedTemplate {
            content,
            source_sha256: digest,
            warnings,
            stats,
        },
        trail,
    ))
}

/// How one paragraph with visible text was classified, and the shape signals
/// a reader would have used to call it a heading by eye.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParagraphVerdict {
    /// Ordinal among paragraphs that carried visible text.
    pub(crate) index: usize,
    /// The styleId as the package spells it; `None` when the paragraph names
    /// no style at all, which is what Word writes for body text.
    pub(crate) style_id: Option<String>,
    pub(crate) kind: ParagraphKind,
    /// Leading characters of the text, for identifying the paragraph.
    pub(crate) preview: String,
    pub(crate) characters: usize,
    pub(crate) shape: HeadingShape,
}

/// Signals that make a paragraph *look* like a heading. Never consulted by
/// the importer — carving is style-driven and stays that way — but a
/// paragraph carrying several of them and classified as body text is the
/// exact thing a reader means by "this heading wasn't picked up".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeadingShape {
    /// The paragraph itself carries `<w:outlineLvl>`, independently of its
    /// style. Word writes this when a paragraph is promoted in the
    /// navigation pane, so it is an authored claim rather than an
    /// appearance.
    pub outline_level: bool,
    /// Every run in the paragraph is bold, counting paragraph-level direct
    /// formatting as well as run-level.
    pub all_bold: bool,
    /// The text has no lowercase letters.
    pub all_caps: bool,
    /// Short enough to be a label rather than a sentence.
    pub short: bool,
    /// Does not end in sentence punctuation.
    pub unpunctuated: bool,
    /// Contains at least one letter. A rule made of underscores is short,
    /// bold and unpunctuated, and is not a heading.
    pub lettered: bool,
}

impl HeadingShape {
    /// Longest a paragraph can be and still read as a label.
    const SHORT_CHARACTERS: usize = 80;

    /// How many signals fired. Two or more is what the report treats as a
    /// heading a human would have seen.
    pub fn signals(self) -> u8 {
        u8::from(self.all_bold)
            + u8::from(self.all_caps)
            + u8::from(self.short)
            + u8::from(self.unpunctuated)
    }

    /// Whether this paragraph would be treated as a heading by the
    /// appearance fallback: emphasized, label-shaped, and made of words.
    ///
    /// Bold or all-caps is the emphasis — a template author picks one and
    /// keeps to it. Both length and the absent full stop are required
    /// because either alone matches ordinary sentences: a short field label
    /// ends in a colon, and a fragment that runs to two lines is prose.
    pub fn reads_as_heading(self) -> bool {
        self.lettered && self.short && self.unpunctuated && (self.all_bold || self.all_caps)
    }

    fn of(paragraph: &Paragraph, text: &str) -> Self {
        Self {
            outline_level: paragraph.property.outline_lvl.is_some(),
            all_bold: paragraph_is_all_bold(paragraph),
            all_caps: text.chars().any(char::is_alphabetic)
                && !text.chars().any(char::is_lowercase),
            short: text.chars().count() <= Self::SHORT_CHARACTERS,
            unpunctuated: !text.trim_end().ends_with(['.', '?', '!', ':', ';', ',']),
            lettered: text.chars().any(char::is_alphabetic),
        }
    }
}

/// Whether every run carrying text in this paragraph is bold. An empty
/// paragraph is not bold, and one unbolded run is enough to say no.
fn paragraph_is_all_bold(paragraph: &Paragraph) -> bool {
    // Word writes bold either on every run or once on the paragraph mark,
    // depending on how the text was typed and edited. Both mean the same
    // thing to a reader, so both count.
    let paragraph_bold = paragraph.property.run_property.bold.is_some();
    let mut saw_text = false;
    for child in &paragraph.children {
        if let ParagraphChild::Run(run) = child {
            if run_text(run).trim().is_empty() {
                continue;
            }
            saw_text = true;
            if run.run_property.bold.is_none() && !paragraph_bold {
                return false;
            }
        }
    }
    saw_text
}

struct Preflight {
    warnings: WarningCounts,
}

fn preflight_package(bytes: &[u8]) -> Result<Preflight, DocxError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        DocxError::Import("the selected file is not a valid DOCX package".to_string())
    })?;
    if archive.len() > MAX_TEMPLATE_ZIP_ENTRIES {
        return Err(DocxError::UnsafeTemplate(format!(
            "the package contains more than {MAX_TEMPLATE_ZIP_ENTRIES} entries"
        )));
    }

    let mut total_size = 0_u64;
    let mut relationship_entries = Vec::new();
    let mut entry_names = HashSet::new();
    let mut has_document = false;
    let mut warnings = WarningCounts::default();
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|_| {
            DocxError::Import("the DOCX package directory is malformed".to_string())
        })?;
        if file.encrypted() {
            return Err(DocxError::UnsafeTemplate(
                "encrypted DOCX packages are not supported".to_string(),
            ));
        }
        if file.enclosed_name().is_none() {
            return Err(DocxError::UnsafeTemplate(
                "the package contains an unsafe entry path".to_string(),
            ));
        }
        if file.size() > MAX_TEMPLATE_ENTRY_BYTES {
            return Err(DocxError::UnsafeTemplate(
                "a package entry exceeds the 16 MiB limit".to_string(),
            ));
        }
        total_size = total_size.saturating_add(file.size());
        if total_size > MAX_TEMPLATE_UNCOMPRESSED_BYTES {
            return Err(DocxError::UnsafeTemplate(
                "the expanded package exceeds the 32 MiB limit".to_string(),
            ));
        }

        let name = file.name().replace('\\', "/");
        if !entry_names.insert(name.clone()) {
            return Err(DocxError::UnsafeTemplate(
                "the package contains duplicate entry names".to_string(),
            ));
        }
        has_document |= name == "word/document.xml";
        if name.ends_with(".rels") {
            relationship_entries.push(name.clone());
        }
        if name.starts_with("word/embeddings/")
            || name.starts_with("word/activeX/")
            || name.contains("vbaProject")
        {
            return Err(DocxError::UnsafeTemplate(
                "embedded objects and macros are not accepted".to_string(),
            ));
        }
        if name.starts_with("word/media/") {
            warnings.add(ReportTemplateWarningCode::ImagesOmitted, 1);
        }
        if name.starts_with("word/header") || name.starts_with("word/footer") {
            warnings.add(ReportTemplateWarningCode::HeadersFootersOmitted, 1);
        }
        if name == "word/footnotes.xml" || name == "word/endnotes.xml" {
            warnings.add(ReportTemplateWarningCode::FootnotesEndnotesOmitted, 1);
        }
        if name.starts_with("word/comments") {
            warnings.add(ReportTemplateWarningCode::CommentsOmitted, 1);
        }
    }
    if !has_document {
        return Err(DocxError::Import(
            "the package has no main Word document".to_string(),
        ));
    }

    let content_types = read_zip_text(&mut archive, "[Content_Types].xml")?;
    if content_types.contains("macroEnabled") || content_types.contains("vbaProject") {
        return Err(DocxError::UnsafeTemplate(
            "macro-enabled Word packages are not accepted".to_string(),
        ));
    }

    for name in relationship_entries {
        let xml = read_zip_text(&mut archive, &name)?;
        warnings.add(
            ReportTemplateWarningCode::ExternalLinksRemoved,
            count_occurrences(&xml, "TargetMode=\"External\"")
                .saturating_add(count_occurrences(&xml, "TargetMode='External'")),
        );
    }

    let document = read_zip_text(&mut archive, "word/document.xml")?;
    warnings.add(
        ReportTemplateWarningCode::TextBoxesOmitted,
        count_occurrences(&document, "txbxContent"),
    );
    warnings.add(
        ReportTemplateWarningCode::UnsupportedElementsOmitted,
        count_occurrences(&document, "<w:vanish")
            .saturating_add(count_occurrences(&document, "<w:specVanish")),
    );
    let tracked = ["<w:ins", "<w:del", "<w:moveFrom", "<w:moveTo"]
        .iter()
        .map(|tag| count_occurrences(&document, tag))
        .fold(0_u32, u32::saturating_add);
    warnings.add(ReportTemplateWarningCode::TrackedChangesResolved, tracked);

    Ok(Preflight { warnings })
}

fn read_zip_text(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, DocxError> {
    let mut file = archive
        .by_name(name)
        .map_err(|_| DocxError::Import("a required DOCX package part is missing".to_string()))?;
    if file.size() > MAX_TEMPLATE_ENTRY_BYTES {
        return Err(DocxError::UnsafeTemplate(
            "a required package part exceeds the safe limit".to_string(),
        ));
    }
    let mut value = String::new();
    file.read_to_string(&mut value)
        .map_err(|_| DocxError::Import("a required DOCX XML part is invalid".to_string()))?;
    Ok(value)
}

fn count_occurrences(value: &str, pattern: &str) -> u32 {
    u32::try_from(value.matches(pattern).count()).unwrap_or(u32::MAX)
}

#[derive(Default)]
struct WarningCounts(HashMap<ReportTemplateWarningCode, u32>);

impl WarningCounts {
    fn add(&mut self, code: ReportTemplateWarningCode, count: u32) {
        if count > 0 {
            let current = self.0.entry(code).or_default();
            *current = current.saturating_add(count);
        }
    }

    fn into_sorted(self) -> Vec<ReportTemplateWarning> {
        WARNING_ORDER
            .iter()
            .filter_map(|code| {
                self.0
                    .get(code)
                    .copied()
                    .map(|count| ReportTemplateWarning { code: *code, count })
            })
            .collect()
    }
}

/// The order warnings are shown in, most consequential first.
///
/// This list is also the filter: [`WarningCounts::into_sorted`] drops any
/// code missing from it, so a new variant that is not added here is counted
/// and then silently thrown away.
const WARNING_ORDER: &[ReportTemplateWarningCode] = &[
    ReportTemplateWarningCode::SectionsInferredFromFormatting,
    ReportTemplateWarningCode::MissingTitle,
    ReportTemplateWarningCode::HeadersFootersOmitted,
    ReportTemplateWarningCode::HeadingLevelsFlattened,
    ReportTemplateWarningCode::ImagesOmitted,
    ReportTemplateWarningCode::TextBoxesOmitted,
    ReportTemplateWarningCode::FootnotesEndnotesOmitted,
    ReportTemplateWarningCode::CommentsOmitted,
    ReportTemplateWarningCode::TrackedChangesResolved,
    ReportTemplateWarningCode::ExternalLinksRemoved,
    ReportTemplateWarningCode::NestedTablesOmitted,
    ReportTemplateWarningCode::MergedTablesOmitted,
    ReportTemplateWarningCode::IrregularTablesOmitted,
    ReportTemplateWarningCode::NumberedListsImportedAsBullets,
    ReportTemplateWarningCode::UnsupportedElementsOmitted,
];

struct ImportBuilder {
    title: Option<String>,
    sections: Vec<ReportSection>,
    current_heading: Option<String>,
    current_blocks: Vec<ReportBlock>,
    pending_bullets: Vec<String>,
    warnings: WarningCounts,
    stats: TemplateImportStats,
    catalog: StyleCatalog,
    headings: Headings,
    /// Kept on every pass: the styles-only pass has to report whether it
    /// found a heading and what the appearance rule would have found, which
    /// is what decides whether a second pass runs.
    trail: Vec<ParagraphVerdict>,
    /// Text and style are carried only for the diagnostic, so an ordinary
    /// import does not hold a preview of every paragraph in the document.
    detail: Detail,
}

impl ImportBuilder {
    fn new(
        warnings: WarningCounts,
        catalog: StyleCatalog,
        headings: Headings,
        detail: Detail,
    ) -> Self {
        Self {
            title: None,
            sections: Vec::new(),
            current_heading: None,
            current_blocks: Vec::new(),
            pending_bullets: Vec::new(),
            warnings,
            stats: TemplateImportStats::default(),
            catalog,
            headings,
            trail: Vec::new(),
            detail,
        }
    }

    fn document_child(&mut self, child: &DocumentChild) {
        match child {
            DocumentChild::Paragraph(paragraph) => self.paragraph(paragraph),
            DocumentChild::Table(table) => self.table(table),
            DocumentChild::StructuredDataTag(tag) => self.structured_data_tag(tag),
            DocumentChild::Section(section) => {
                for child in section.children() {
                    self.section_child(child);
                }
            }
            DocumentChild::TableOfContents(_) => self
                .warnings
                .add(ReportTemplateWarningCode::UnsupportedElementsOmitted, 1),
            DocumentChild::BookmarkStart(_)
            | DocumentChild::BookmarkEnd(_)
            | DocumentChild::CommentStart(_)
            | DocumentChild::CommentEnd(_) => {}
        }
    }

    fn section_child(&mut self, child: &SectionChild) {
        match child {
            SectionChild::Paragraph(paragraph) => self.paragraph(paragraph),
            SectionChild::Table(table) => self.table(table),
            SectionChild::StructuredDataTag(tag) => self.structured_data_tag(tag),
            SectionChild::TableOfContents(_) => self
                .warnings
                .add(ReportTemplateWarningCode::UnsupportedElementsOmitted, 1),
            SectionChild::BookmarkStart(_)
            | SectionChild::BookmarkEnd(_)
            | SectionChild::CommentStart(_)
            | SectionChild::CommentEnd(_) => {}
        }
    }

    fn structured_data_tag(&mut self, tag: &StructuredDataTag) {
        for child in &tag.children {
            match child {
                StructuredDataTagChild::Run(run) => {
                    let text = run_text(run).trim().to_string();
                    if !text.is_empty() {
                        self.regular_paragraph(text, false);
                    }
                }
                StructuredDataTagChild::Paragraph(paragraph) => self.paragraph(paragraph),
                StructuredDataTagChild::Table(table) => self.table(table),
                StructuredDataTagChild::StructuredDataTag(nested) => {
                    self.structured_data_tag(nested)
                }
                StructuredDataTagChild::BookmarkStart(_)
                | StructuredDataTagChild::BookmarkEnd(_)
                | StructuredDataTagChild::CommentStart(_)
                | StructuredDataTagChild::CommentEnd(_) => {}
            }
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph) {
        let text = paragraph_text(paragraph).trim().to_string();
        if text.is_empty() {
            return;
        }
        let styled = paragraph_kind(paragraph, &self.catalog);
        let shape = HeadingShape::of(paragraph, &text);
        // Appearance is consulted only on a pass that was told to, and only
        // for paragraphs the styles left as body text. It can promote a
        // paragraph, never demote one.
        let kind = if self.headings == Headings::StylesOrAppearance
            && styled == ParagraphKind::Body
            && shape.reads_as_heading()
        {
            self.warnings
                .add(ReportTemplateWarningCode::SectionsInferredFromFormatting, 1);
            ParagraphKind::Heading
        } else {
            styled
        };
        self.trail.push(ParagraphVerdict {
            index: self.trail.len(),
            style_id: match self.detail {
                Detail::Keep => paragraph
                    .property
                    .style
                    .as_ref()
                    .map(|style| style.val.clone()),
                Detail::Skip => None,
            },
            kind: styled,
            preview: match self.detail {
                Detail::Keep => preview_of(&text),
                Detail::Skip => String::new(),
            },
            characters: text.chars().count(),
            shape,
        });
        match kind {
            ParagraphKind::Title if self.title.is_none() => {
                self.flush_bullets();
                self.title = Some(text);
            }
            ParagraphKind::Heading => {
                if nested_heading_was_flattened(paragraph) {
                    self.warnings
                        .add(ReportTemplateWarningCode::HeadingLevelsFlattened, 1);
                }
                self.flush_section();
                self.current_heading = Some(text);
            }
            ParagraphKind::List => self.regular_paragraph(text, true),
            ParagraphKind::Title | ParagraphKind::Body => self.regular_paragraph(text, false),
        }
    }

    fn regular_paragraph(&mut self, text: String, list: bool) {
        self.ensure_section();
        if list {
            self.pending_bullets.push(text);
            self.warnings
                .add(ReportTemplateWarningCode::NumberedListsImportedAsBullets, 1);
        } else {
            self.flush_bullets();
            self.current_blocks.push(ReportBlock::Paragraph { text });
            self.stats.paragraphs = self.stats.paragraphs.saturating_add(1);
        }
    }

    fn table(&mut self, table: &Table) {
        self.ensure_section();
        self.flush_bullets();
        match import_table(table) {
            Ok(Some(block)) => {
                if let ReportBlock::Table { rows, .. } = &block {
                    self.stats.tables = self.stats.tables.saturating_add(1);
                    self.stats.table_cells = self.stats.table_cells.saturating_add(
                        u32::try_from(rows.iter().map(Vec::len).sum::<usize>()).unwrap_or(u32::MAX),
                    );
                }
                self.current_blocks.push(block);
            }
            Ok(None) => self
                .warnings
                .add(ReportTemplateWarningCode::IrregularTablesOmitted, 1),
            Err(TableImportIssue::Nested) => self
                .warnings
                .add(ReportTemplateWarningCode::NestedTablesOmitted, 1),
            Err(TableImportIssue::Merged) => self
                .warnings
                .add(ReportTemplateWarningCode::MergedTablesOmitted, 1),
        }
    }

    fn ensure_section(&mut self) {
        if self.current_heading.is_none() {
            self.current_heading = Some("Imported content".to_string());
        }
    }

    fn flush_bullets(&mut self) {
        if self.pending_bullets.is_empty() {
            return;
        }
        self.current_blocks.push(ReportBlock::BulletList {
            items: std::mem::take(&mut self.pending_bullets),
        });
        self.stats.bullet_lists = self.stats.bullet_lists.saturating_add(1);
    }

    fn flush_section(&mut self) {
        self.flush_bullets();
        if let Some(heading) = self.current_heading.take() {
            self.sections.push(ReportSection {
                id: Uuid::new_v4(),
                heading,
                blocks: std::mem::take(&mut self.current_blocks),
                skipped: false,
                template_blocks: None,
                authorship: None,
            });
        }
    }

    fn finish(
        mut self,
    ) -> Result<
        (
            ReportContent,
            TemplateImportStats,
            Vec<ReportTemplateWarning>,
            Vec<ParagraphVerdict>,
        ),
        DocxError,
    > {
        self.flush_section();
        if self.sections.is_empty() {
            return Err(DocxError::Import(
                "the document contains no supported body content".to_string(),
            ));
        }
        let title = self.title.unwrap_or_else(|| {
            self.warnings
                .add(ReportTemplateWarningCode::MissingTitle, 1);
            "Imported report template".to_string()
        });
        self.stats.sections = u32::try_from(self.sections.len()).unwrap_or(u32::MAX);
        Ok((
            ReportContent {
                title,
                sections: self.sections,
            },
            self.stats,
            self.warnings.into_sorted(),
            self.trail,
        ))
    }
}

/// Leading characters of a paragraph, for naming it in a report a human
/// reads. Bounded so a diagnostic over a clinical template stays a summary
/// rather than a second copy of the document.
fn preview_of(text: &str) -> String {
    const PREVIEW_CHARACTERS: usize = 72;
    let mut preview: String = text.chars().take(PREVIEW_CHARACTERS).collect();
    if text.chars().count() > PREVIEW_CHARACTERS {
        preview.push('\u{2026}');
    }
    preview
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParagraphKind {
    Title,
    Heading,
    List,
    Body,
}

impl ParagraphKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Heading => "heading",
            Self::List => "list item",
            Self::Body => "body",
        }
    }
}

fn paragraph_kind(paragraph: &Paragraph, catalog: &StyleCatalog) -> ParagraphKind {
    let style = paragraph
        .property
        .style
        .as_ref()
        .map(|style| normalize_style(&style.val))
        .unwrap_or_default();
    if catalog.is_title(&style) {
        ParagraphKind::Title
    } else if catalog.is_heading(&style) {
        ParagraphKind::Heading
    } else if paragraph.property.numbering_property.is_some() {
        ParagraphKind::List
    } else {
        ParagraphKind::Body
    }
}

fn nested_heading_was_flattened(paragraph: &Paragraph) -> bool {
    paragraph
        .property
        .style
        .as_ref()
        .map(|style| normalize_style(&style.val))
        .and_then(|style| style.strip_prefix("heading")?.parse::<u8>().ok())
        .is_some_and(|level| level > 1)
}

fn paragraph_text(paragraph: &Paragraph) -> String {
    if paragraph.property.run_property.vanish.is_some()
        || paragraph.property.run_property.spec_vanish.is_some()
    {
        return String::new();
    }
    let mut output = String::new();
    append_paragraph_children(&paragraph.children, &mut output);
    output
}

fn append_paragraph_children(children: &[ParagraphChild], output: &mut String) {
    for child in children {
        match child {
            ParagraphChild::Run(run) => output.push_str(&run_text(run)),
            ParagraphChild::Insert(insert) => {
                for child in &insert.children {
                    if let InsertChild::Run(run) = child {
                        output.push_str(&run_text(run));
                    }
                }
            }
            ParagraphChild::MoveTo(moved) => {
                for child in &moved.children {
                    if let MoveToChild::Run(run) = child {
                        output.push_str(&run_text(run));
                    }
                }
            }
            ParagraphChild::Hyperlink(link) => {
                append_paragraph_children(&link.children, output);
            }
            ParagraphChild::StructuredDataTag(tag) => append_inline_tag(tag, output),
            ParagraphChild::Delete(_)
            | ParagraphChild::MoveFrom(_)
            | ParagraphChild::BookmarkStart(_)
            | ParagraphChild::BookmarkEnd(_)
            | ParagraphChild::CommentStart(_)
            | ParagraphChild::CommentEnd(_)
            | ParagraphChild::PageNum(_)
            | ParagraphChild::NumPages(_) => {}
        }
    }
}

fn append_inline_tag(tag: &StructuredDataTag, output: &mut String) {
    for child in &tag.children {
        match child {
            StructuredDataTagChild::Run(run) => output.push_str(&run_text(run)),
            StructuredDataTagChild::Paragraph(paragraph) => {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(&paragraph_text(paragraph));
            }
            StructuredDataTagChild::StructuredDataTag(nested) => append_inline_tag(nested, output),
            StructuredDataTagChild::Table(_)
            | StructuredDataTagChild::BookmarkStart(_)
            | StructuredDataTagChild::BookmarkEnd(_)
            | StructuredDataTagChild::CommentStart(_)
            | StructuredDataTagChild::CommentEnd(_) => {}
        }
    }
}

fn run_text(run: &Run) -> String {
    if run.run_property.vanish.is_some() || run.run_property.spec_vanish.is_some() {
        return String::new();
    }
    let mut output = String::new();
    for child in &run.children {
        match child {
            RunChild::Text(text) => output.push_str(&text.text),
            RunChild::Tab(_) | RunChild::PTab(_) => output.push('\t'),
            RunChild::Break(_) | RunChild::CarriageReturn(_) => output.push('\n'),
            RunChild::Sym(_)
            | RunChild::DeleteText(_)
            | RunChild::Drawing(_)
            | RunChild::Shape(_)
            | RunChild::CommentStart(_)
            | RunChild::CommentEnd(_)
            | RunChild::FieldChar(_)
            | RunChild::InstrText(_)
            | RunChild::DeleteInstrText(_)
            | RunChild::InstrTextString(_)
            | RunChild::FootnoteReference(_)
            | RunChild::Shading(_) => {}
        }
    }
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableImportIssue {
    Nested,
    Merged,
}

fn import_table(table: &Table) -> Result<Option<ReportBlock>, TableImportIssue> {
    let mut rows = Vec::new();
    let mut header_cells = Vec::new();
    for (row_index, child) in table.rows.iter().enumerate() {
        let TableChild::TableRow(row) = child;
        let mut cells = Vec::new();
        let mut row_header_cells = Vec::new();
        for child in &row.cells {
            let TableRowChild::TableCell(cell) = child;
            if cell_is_merged(cell) {
                return Err(TableImportIssue::Merged);
            }
            cells.push(table_cell_text(cell)?);
            if row_index == 0 {
                row_header_cells.push(cell_looks_like_header(cell));
            }
        }
        if row_index == 0 {
            header_cells = row_header_cells;
        }
        rows.push(cells);
    }
    let Some(columns) = rows.first().map(Vec::len) else {
        return Ok(None);
    };
    if columns == 0
        || columns > claria_core::models::report::MAX_TABLE_COLUMNS
        || rows.len() > claria_core::models::report::MAX_TABLE_ROWS
        || rows.iter().any(|row| row.len() != columns)
        || rows.iter().flatten().all(|cell| cell.trim().is_empty())
    {
        return Ok(None);
    }

    let has_header = rows.len() > 1
        && header_cells.len() == columns
        && header_cells.iter().all(|header| *header)
        && rows[0].iter().all(|cell| !cell.trim().is_empty());
    Ok(Some(ReportBlock::Table {
        column_widths: normalized_widths(&table.grid, columns),
        rows,
        has_header,
    }))
}

fn table_cell_text(cell: &TableCell) -> Result<String, TableImportIssue> {
    let mut paragraphs = Vec::new();
    for child in &cell.children {
        match child {
            TableCellContent::Paragraph(paragraph) => paragraphs.push(paragraph_text(paragraph)),
            TableCellContent::Table(_) => return Err(TableImportIssue::Nested),
            TableCellContent::StructuredDataTag(tag) => {
                if tag_contains_table(tag) {
                    return Err(TableImportIssue::Nested);
                }
                let mut text = String::new();
                append_inline_tag(tag, &mut text);
                paragraphs.push(text);
            }
            TableCellContent::TableOfContents(_) => {}
        }
    }
    Ok(paragraphs.join("\n").trim().to_string())
}

fn tag_contains_table(tag: &StructuredDataTag) -> bool {
    tag.children.iter().any(|child| match child {
        StructuredDataTagChild::Table(_) => true,
        StructuredDataTagChild::StructuredDataTag(nested) => tag_contains_table(nested),
        _ => false,
    })
}

fn cell_is_merged(cell: &TableCell) -> bool {
    serde_json::to_value(&cell.property).is_ok_and(|property| {
        property
            .get("gridSpan")
            .is_some_and(|value| !value.is_null())
            || property
                .get("verticalMerge")
                .is_some_and(|value| !value.is_null())
    })
}

fn cell_looks_like_header(cell: &TableCell) -> bool {
    let shaded = serde_json::to_value(&cell.property).is_ok_and(|property| {
        property
            .get("shading")
            .is_some_and(|value| !value.is_null())
    });
    shaded
        || cell.children.iter().any(|child| match child {
            TableCellContent::Paragraph(paragraph) => paragraph_has_bold_text(paragraph),
            _ => false,
        })
}

fn paragraph_has_bold_text(paragraph: &Paragraph) -> bool {
    let visible_runs: Vec<&Run> = paragraph
        .children
        .iter()
        .filter_map(|child| match child {
            ParagraphChild::Run(run) if !run_text(run).trim().is_empty() => Some(run.as_ref()),
            _ => None,
        })
        .collect();
    !visible_runs.is_empty()
        && visible_runs
            .iter()
            .all(|run| run.run_property.bold.is_some())
}

fn normalized_widths(grid: &[usize], columns: usize) -> Option<Vec<u16>> {
    if grid.len() != columns || grid.contains(&0) {
        return None;
    }
    let total = grid.iter().try_fold(0_u64, |sum, width| {
        sum.checked_add(u64::try_from(*width).ok()?)
    })?;
    if total == 0 {
        return None;
    }
    let mut widths = grid
        .iter()
        .map(|width| {
            let width = u64::try_from(*width).ok()?;
            u16::try_from(width.saturating_mul(10_000) / total).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    if widths.contains(&0) {
        return None;
    }
    let assigned: u32 = widths.iter().map(|width| u32::from(*width)).sum();
    let remainder = 10_000_u32.checked_sub(assigned)?;
    let last = widths.last_mut()?;
    *last = last.checked_add(u16::try_from(remainder).ok()?)?;
    Some(widths)
}
