//! Template-aware DOCX rendering.
//!
//! The original package remains the source of truth for styles, fonts, page
//! setup, headers, footers, media, paragraph spacing, and intentional blank
//! paragraphs. We update only visible body text/table cells in
//! `word/document.xml`; every other package part is copied unchanged.

use std::io::{Cursor, Read, Write};

use claria_core::models::report::{
    ReportBlock, ReportDraft, ReportTemplateWarningCode, validate_report_content,
};
use quick_xml::{
    Reader, Writer,
    events::{BytesText, Event},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    DocxError, ImportedTemplate,
    import::{SYNTHETIC_SECTION_HEADING, import_template},
    render::BULLET_NUMBERING_ID,
    render_report,
    style_catalog::{StyleCatalog, normalize_style},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowKind {
    Title,
    Heading,
    Body,
    List,
    Table,
}

impl FlowKind {
    const COUNT: usize = 5;

    fn index(self) -> usize {
        match self {
            Self::Title => 0,
            Self::Heading => 1,
            Self::Body => 2,
            Self::List => 3,
            Self::Table => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlowContent {
    Paragraph(String),
    Table(Vec<Vec<String>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetFlow {
    kind: FlowKind,
    content: FlowContent,
}

/// One section of the accepted draft, as blocks to place. `heading` is
/// `None` for the section the importer invented to hold content preceding
/// the template's first heading: it names no paragraph of the document and
/// must not become one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetSection {
    heading: Option<String>,
    blocks: Vec<TargetFlow>,
}

/// The accepted draft as the template renderer needs it: sectioned, and
/// carrying whether this template has a title paragraph at all.
#[derive(Debug, Clone)]
struct TargetDocument {
    title: String,
    /// False when the template import reported [`ReportTemplateWarningCode::MissingTitle`]
    /// — a template with no title paragraph must not gain one on export.
    emit_title: bool,
    sections: Vec<TargetSection>,
}

#[derive(Debug, Clone)]
struct FlowSpan {
    start: usize,
    end: usize,
    direct_body_child: bool,
    kind: FlowKind,
    content: FlowContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    /// A `w:t` element: its characters are ours to rewrite.
    Text,
    /// A real `<w:tab/>` element. The import flattens it to `'\t'`, so it
    /// has to take part in text alignment — but it is an element, not
    /// characters, and can only ever hold the tab it already is.
    Tab,
}

#[derive(Debug, Clone)]
struct TextSlot {
    event_index: usize,
    text: String,
    kind: SlotKind,
}

type ParagraphRange = (usize, usize);
type TableCells = Vec<Vec<Vec<ParagraphRange>>>;

/// How faithfully a template export could reuse the source package's
/// formatting. Anything but [`TemplateRenderFidelity::PlainBodyFallback`]
/// keeps the template's own body formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateRenderFidelity {
    /// Visible content already matches the template; source bytes returned.
    Exact,
    /// Text updated in place; every template element retained.
    PatchedInPlace,
    /// Structure changed; formatting cloned from template exemplars.
    Reconstructed,
    /// The template body could not be walked (e.g. content controls wrap
    /// it); the export used generated body formatting inside the template
    /// package.
    PlainBodyFallback,
}

/// Render an accepted report back through its original redacted Word package.
///
/// When visible content is unchanged, the source bytes are returned exactly.
/// For edits with the same structure, only text events are changed in place,
/// retaining run-level formatting and every non-text OOXML element. Structural
/// changes reuse the nearest source paragraph/table as a formatting exemplar.
pub fn render_report_with_template(
    template: &[u8],
    draft: &ReportDraft,
) -> Result<(Vec<u8>, TemplateRenderFidelity), DocxError> {
    validate_report_content(&draft.content)
        .map_err(|error| DocxError::Render(error.to_string()))?;
    let draft = &crate::render::exportable_draft(draft);
    // Reuse the bounded package preflight and parser before retaining any part
    // of an uploaded package in an exported report.
    let imported = import_template(template)?;
    let carve = TemplateCarve::of(&imported);
    let targets = TargetDocument::new(draft, &carve);
    if visible_content_matches(&imported.content, draft) {
        return Ok((template.to_vec(), TemplateRenderFidelity::Exact));
    }

    let document_xml = zip_entry(template, "word/document.xml")?;
    let events = parse_events(&document_xml)?;
    let catalog = StyleCatalog::from_package(template);
    let spans = discover_flow(&events, &catalog, &mut FlowClassifier::carve(&carve))?;

    let flat = targets.flatten();
    let (rewritten_events, fidelity) = if spans.len() == flat.len()
        && spans
            .iter()
            .zip(&flat)
            .all(|(source, target)| source.kind == target.kind)
    {
        let mut candidate = events.clone();
        if patch_in_place(&mut candidate, &spans, &flat)? {
            (candidate, TemplateRenderFidelity::PatchedInPlace)
        } else {
            reconstruct_flow(&events, &spans, &targets)?
        }
    } else {
        reconstruct_flow(&events, &spans, &targets)?
    };
    let rewritten_xml = write_events(&rewritten_events)?;
    let package = if references_generated_bullets(&rewritten_xml) {
        merge_bullet_numbering(template, &rewritten_xml)?
    } else {
        replace_zip_entry(template, "word/document.xml", &rewritten_xml)?
    };
    Ok((package, fidelity))
}

/// Whether the rewritten document uses the generated bullet definition —
/// list exemplars injected because the template had no bulleted paragraph
/// of its own reference [`BULLET_NUMBERING_ID`].
fn references_generated_bullets(document_xml: &[u8]) -> bool {
    let needle = format!("w:numId w:val=\"{BULLET_NUMBERING_ID}\"");
    document_xml
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Ensure the output package defines [`BULLET_NUMBERING_ID`] so injected
/// bullet paragraphs don't reference a numbering definition that does not
/// exist (Word drops the glyphs and indent, or prompts to repair).
///
/// Three cases: the template already defines the id (accept its definition —
/// the bullets adopt the template's own list format); the template has a
/// `word/numbering.xml` without the id (append the generated definition);
/// the template has no numbering part at all (add the generated part plus
/// its content-type override and document relationship).
fn merge_bullet_numbering(template: &[u8], document_xml: &[u8]) -> Result<Vec<u8>, DocxError> {
    let mut entries: Vec<(String, Vec<u8>)> =
        vec![("word/document.xml".to_string(), document_xml.to_vec())];
    let generated_numbering = generated_numbering_part()?;
    let num_marker = format!("w:numId=\"{BULLET_NUMBERING_ID}\"");

    match zip_entry(template, "word/numbering.xml") {
        Ok(existing) => {
            let text = String::from_utf8_lossy(&existing);
            if !text.contains(&num_marker) {
                let generated_text = String::from_utf8_lossy(&generated_numbering).into_owned();
                let definitions = extract_between(&generated_text, "<w:abstractNum", "</w:num>")
                    .ok_or_else(|| {
                        DocxError::Render(
                            "generated numbering definitions are malformed".to_string(),
                        )
                    })?;
                let merged =
                    insert_before(&text, "</w:numbering>", definitions).ok_or_else(|| {
                        DocxError::Render("template numbering.xml is malformed".to_string())
                    })?;
                entries.push(("word/numbering.xml".to_string(), merged.into_bytes()));
            }
        }
        Err(_) => {
            entries.push((
                "word/numbering.xml".to_string(),
                generated_numbering.clone(),
            ));
            let content_types = zip_entry(template, "[Content_Types].xml")?;
            let content_types = String::from_utf8_lossy(&content_types).into_owned();
            if !content_types.contains("/word/numbering.xml") {
                let updated = insert_before(
                    &content_types,
                    "</Types>",
                    "<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>",
                )
                .ok_or_else(|| {
                    DocxError::Render("template content types are malformed".to_string())
                })?;
                entries.push(("[Content_Types].xml".to_string(), updated.into_bytes()));
            }
            const NUMBERING_RELATIONSHIP: &str = "<Relationship Id=\"rIdClariaNumbering\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>";
            let relationships = match zip_entry(template, "word/_rels/document.xml.rels") {
                Ok(existing) => {
                    let text = String::from_utf8_lossy(&existing).into_owned();
                    if text.contains("relationships/numbering") {
                        text
                    } else {
                        insert_before(&text, "</Relationships>", NUMBERING_RELATIONSHIP)
                            .ok_or_else(|| {
                                DocxError::Render(
                                    "template document relationships are malformed".to_string(),
                                )
                            })?
                    }
                }
                Err(_) => format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{NUMBERING_RELATIONSHIP}</Relationships>",
                ),
            };
            entries.push((
                "word/_rels/document.xml.rels".to_string(),
                relationships.into_bytes(),
            ));
        }
    }
    rebuild_package(template, &entries)
}

/// The `word/numbering.xml` docx-rs emits for [`BULLET_NUMBERING_ID`],
/// extracted from a minimal generated package so the definition can never
/// drift from what the plain renderer produces.
fn generated_numbering_part() -> Result<Vec<u8>, DocxError> {
    let draft = single_target_draft(&TargetFlow {
        kind: FlowKind::List,
        content: FlowContent::Paragraph("bullet".to_string()),
    })?;
    let bytes = render_report(&draft)?;
    zip_entry(&bytes, "word/numbering.xml")
}

fn extract_between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)?;
    let to = text[from..].find(end)? + from + end.len();
    Some(&text[from..to])
}

fn insert_before(text: &str, marker: &str, insertion: &str) -> Option<String> {
    let position = text.rfind(marker)?;
    let mut output = String::with_capacity(text.len() + insertion.len());
    output.push_str(&text[..position]);
    output.push_str(insertion);
    output.push_str(&text[position..]);
    Some(output)
}

fn visible_content_matches(
    source: &claria_core::models::report::ReportContent,
    draft: &ReportDraft,
) -> bool {
    source.title == draft.content.title
        && source.sections.len() == draft.content.sections.len()
        && source
            .sections
            .iter()
            .zip(&draft.content.sections)
            .all(|(left, right)| left.heading == right.heading && left.blocks == right.blocks)
}

/// The import's own reading of this package: which paragraph it took as the
/// title, and which paragraphs opened a section.
///
/// Export classifies template paragraphs against this carve instead of
/// re-deriving heading-ness from styles and outline levels. Templates whose
/// sections were carved by appearance (bold pseudo-headings, no heading
/// styles) routinely also carry stray `w:outlineLvl` on ordinary prose, so a
/// second opinion at export time is not a second opinion — it is a different
/// document. One classifier, one owner.
#[derive(Debug, Clone)]
struct TemplateCarve {
    /// The title paragraph's text; `None` when the import found no title
    /// paragraph at all.
    title: Option<String>,
    /// Section headings in document order, excluding the invented lead.
    headings: Vec<String>,
    /// The import invented a section to hold content before the first
    /// heading.
    synthetic_lead: bool,
}

impl TemplateCarve {
    fn of(imported: &ImportedTemplate) -> Self {
        let synthetic_lead = imported
            .content
            .sections
            .first()
            .is_some_and(|section| section.heading == SYNTHETIC_SECTION_HEADING);
        let missing_title = imported
            .warnings
            .iter()
            .any(|warning| warning.code == ReportTemplateWarningCode::MissingTitle);
        Self {
            title: (!missing_title).then(|| normalize_flow_text(&imported.content.title)),
            headings: imported
                .content
                .sections
                .iter()
                .skip(usize::from(synthetic_lead))
                .map(|section| normalize_flow_text(&section.heading))
                .collect(),
            synthetic_lead,
        }
    }
}

/// Whitespace-insensitive form of a paragraph's text, for matching template
/// paragraphs against the carve. The import flattens `w:tab` and `w:br` into
/// characters; the walker here reads elements, so the two spell the same
/// heading with different whitespace.
fn normalize_flow_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How the flow walker decides what a paragraph is.
enum FlowClassifier<'a> {
    /// Reuse the import's carve of this very package.
    Carve {
        carve: &'a TemplateCarve,
        heading_cursor: usize,
        title_seen: bool,
    },
    /// A package this crate just generated, whose styles are our own.
    Styles { title_seen: bool },
}

impl<'a> FlowClassifier<'a> {
    fn carve(carve: &'a TemplateCarve) -> Self {
        Self::Carve {
            carve,
            heading_cursor: 0,
            title_seen: false,
        }
    }

    fn styles() -> Self {
        Self::Styles { title_seen: false }
    }

    fn classify(
        &mut self,
        text: &str,
        style: &str,
        numbered: bool,
        catalog: &StyleCatalog,
    ) -> FlowKind {
        match self {
            Self::Carve {
                carve,
                heading_cursor,
                title_seen,
            } => {
                let normalized = normalize_flow_text(text);
                if !*title_seen
                    && *heading_cursor == 0
                    && carve.title.as_deref() == Some(normalized.as_str())
                {
                    *title_seen = true;
                    return FlowKind::Title;
                }
                if carve
                    .headings
                    .get(*heading_cursor)
                    .is_some_and(|heading| *heading == normalized)
                {
                    *heading_cursor += 1;
                    return FlowKind::Heading;
                }
                if numbered {
                    FlowKind::List
                } else {
                    FlowKind::Body
                }
            }
            Self::Styles { title_seen } => {
                if catalog.is_title(style) && !*title_seen {
                    *title_seen = true;
                    FlowKind::Title
                } else if catalog.is_heading(style) {
                    FlowKind::Heading
                } else if numbered {
                    FlowKind::List
                } else {
                    FlowKind::Body
                }
            }
        }
    }
}

impl TargetDocument {
    fn new(draft: &ReportDraft, carve: &TemplateCarve) -> Self {
        let sections = draft
            .content
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| {
                let synthetic = index == 0
                    && carve.synthetic_lead
                    && section.heading == SYNTHETIC_SECTION_HEADING;
                let mut blocks = Vec::new();
                for block in &section.blocks {
                    match block {
                        ReportBlock::Paragraph { text } => blocks.push(TargetFlow {
                            kind: FlowKind::Body,
                            content: FlowContent::Paragraph(text.clone()),
                        }),
                        ReportBlock::BulletList { items } => {
                            blocks.extend(items.iter().cloned().map(|item| TargetFlow {
                                kind: FlowKind::List,
                                content: FlowContent::Paragraph(item),
                            }));
                        }
                        ReportBlock::Table { rows, .. } => blocks.push(TargetFlow {
                            kind: FlowKind::Table,
                            content: FlowContent::Table(rows.clone()),
                        }),
                    }
                }
                TargetSection {
                    heading: (!synthetic).then(|| section.heading.clone()),
                    blocks,
                }
            })
            .collect();
        Self {
            title: draft.content.title.clone(),
            emit_title: carve.title.is_some(),
            sections,
        }
    }

    fn title_target(&self) -> TargetFlow {
        TargetFlow {
            kind: FlowKind::Title,
            content: FlowContent::Paragraph(self.title.clone()),
        }
    }

    /// Every block to place, in document order — what the same-structure
    /// comparison against the template's flow spans needs.
    fn flatten(&self) -> Vec<TargetFlow> {
        let mut output = Vec::new();
        if self.emit_title {
            output.push(self.title_target());
        }
        for section in &self.sections {
            if let Some(heading) = &section.heading {
                output.push(TargetFlow {
                    kind: FlowKind::Heading,
                    content: FlowContent::Paragraph(heading.clone()),
                });
            }
            output.extend(section.blocks.iter().cloned());
        }
        output
    }
}

fn parse_events(xml: &[u8]) -> Result<Vec<Event<'static>>, DocxError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut events = Vec::new();
    let mut buffer = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|error| {
            DocxError::Render(format!("template document XML is invalid: {error}"))
        })?;
        let eof = matches!(event, Event::Eof);
        events.push(event.into_owned());
        buffer.clear();
        if eof {
            break;
        }
    }
    Ok(events)
}

fn write_events(events: &[Event<'static>]) -> Result<Vec<u8>, DocxError> {
    let mut writer = Writer::new(Vec::new());
    for event in events {
        writer.write_event(event.borrow()).map_err(|error| {
            DocxError::Render(format!("could not update template XML: {error}"))
        })?;
    }
    Ok(writer.into_inner())
}

fn discover_flow(
    events: &[Event<'static>],
    catalog: &StyleCatalog,
    classifier: &mut FlowClassifier<'_>,
) -> Result<Vec<FlowSpan>, DocxError> {
    let mut spans = Vec::new();
    let mut paragraph_start = None;
    let mut table_start = None;
    let mut table_depth = 0_usize;
    let mut depth = 0_usize;
    let mut body_child_depth = None;

    for (index, event) in events.iter().enumerate() {
        match event {
            Event::Start(start) => {
                let name = start.name();
                if local_name(name.as_ref()) == b"body" {
                    body_child_depth = Some(depth + 1);
                } else if local_name(name.as_ref()) == b"tbl" {
                    if table_depth == 0 {
                        table_start = Some((index, depth));
                    }
                    table_depth += 1;
                } else if local_name(name.as_ref()) == b"p"
                    && table_depth == 0
                    && paragraph_start.is_none()
                {
                    paragraph_start = Some((index, depth));
                }
                depth += 1;
            }
            Event::End(end) => {
                depth = depth.saturating_sub(1);
                let name = end.name();
                if local_name(name.as_ref()) == b"p" && table_depth == 0 {
                    if let Some((start, start_depth)) = paragraph_start.take()
                        && let Some((kind, text)) =
                            paragraph_flow(&events[start..=index], catalog, classifier)?
                    {
                        spans.push(FlowSpan {
                            start,
                            end: index,
                            direct_body_child: body_child_depth == Some(start_depth),
                            kind,
                            content: FlowContent::Paragraph(text),
                        });
                    }
                } else if local_name(name.as_ref()) == b"tbl" {
                    table_depth = table_depth.saturating_sub(1);
                    if table_depth == 0
                        && let Some((start, start_depth)) = table_start.take()
                        && let Some(rows) = table_flow(&events[start..=index])?
                    {
                        spans.push(FlowSpan {
                            start,
                            end: index,
                            direct_body_child: body_child_depth == Some(start_depth),
                            kind: FlowKind::Table,
                            content: FlowContent::Table(rows),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    spans.sort_by_key(|span| span.start);
    Ok(spans)
}

fn paragraph_flow(
    events: &[Event<'static>],
    catalog: &StyleCatalog,
    classifier: &mut FlowClassifier<'_>,
) -> Result<Option<(FlowKind, String)>, DocxError> {
    let text = visible_paragraph_text(events)?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let mut style = String::new();
    let mut numbered = false;
    for event in events {
        match event {
            Event::Start(start) | Event::Empty(start) => {
                let name = start.name();
                let local = local_name(name.as_ref());
                if local == b"pStyle" {
                    style = attribute_value(start, b"val").unwrap_or_default();
                } else if local == b"numPr" {
                    numbered = true;
                }
            }
            _ => {}
        }
    }
    let style = normalize_style(&style);
    let kind = classifier.classify(&text, &style, numbered, catalog);
    Ok(Some((kind, text)))
}

fn span_text(span: &FlowSpan) -> Option<&str> {
    match &span.content {
        FlowContent::Paragraph(text) => Some(text),
        FlowContent::Table(_) => None,
    }
}

fn visible_paragraph_text(events: &[Event<'static>]) -> Result<String, DocxError> {
    Ok(text_slots(events)?
        .into_iter()
        .map(|slot| slot.text)
        .collect::<String>())
}

/// Every place in a paragraph that holds visible characters, in order:
/// `w:t` elements, plus each real `<w:tab/>` as a fixed one-character slot.
/// The import flattens tabs into the text it hands the model, so a tab the
/// allocator cannot see is a character the two halves disagree about — which
/// is what merged the label and value runs of tabbed header blocks.
fn text_slots(events: &[Event<'static>]) -> Result<Vec<TextSlot>, DocxError> {
    let mut slots = Vec::new();
    let mut in_text = false;
    let mut excluded_depth = 0_usize;
    let mut run_depth = 0_usize;
    let mut property_depth = 0_usize;
    let mut run_hidden = false;
    let mut paragraph_hidden = false;
    for (index, event) in events.iter().enumerate() {
        match event {
            Event::Start(start) => {
                let name = start.name();
                let local = local_name(name.as_ref());
                match local {
                    b"del" | b"moveFrom" => excluded_depth += 1,
                    b"r" => {
                        run_depth += 1;
                        run_hidden = paragraph_hidden;
                    }
                    b"pPr" | b"rPr" => property_depth += 1,
                    b"t" if excluded_depth == 0 && !run_hidden => in_text = true,
                    b"vanish" | b"specVanish" => {
                        if run_depth > 0 {
                            run_hidden = true;
                        } else {
                            paragraph_hidden = true;
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(empty) => {
                let name = empty.name();
                let local = local_name(name.as_ref());
                if matches!(local, b"vanish" | b"specVanish") {
                    if run_depth > 0 {
                        run_hidden = true;
                    } else {
                        paragraph_hidden = true;
                    }
                } else if local == b"tab"
                    // A `w:tab` inside properties is a tab *stop* on the
                    // paragraph, not a tab in the text.
                    && run_depth > 0
                    && property_depth == 0
                    && excluded_depth == 0
                    && !run_hidden
                {
                    slots.push(TextSlot {
                        event_index: index,
                        text: "\t".to_string(),
                        kind: SlotKind::Tab,
                    });
                }
            }
            Event::End(end) => {
                let name = end.name();
                let local = local_name(name.as_ref());
                match local {
                    b"del" | b"moveFrom" => excluded_depth = excluded_depth.saturating_sub(1),
                    b"r" => {
                        run_depth = run_depth.saturating_sub(1);
                        run_hidden = paragraph_hidden;
                    }
                    b"pPr" | b"rPr" => property_depth = property_depth.saturating_sub(1),
                    b"t" => in_text = false,
                    _ => {}
                }
            }
            Event::Text(text) if in_text && excluded_depth == 0 && !run_hidden => {
                let decoded = text.decode().map_err(|error| {
                    DocxError::Render(format!("template text is invalid: {error}"))
                })?;
                let unescaped = quick_xml::escape::unescape(&decoded).map_err(|error| {
                    DocxError::Render(format!("template text is invalid: {error}"))
                })?;
                slots.push(TextSlot {
                    event_index: index,
                    text: unescaped.into_owned(),
                    kind: SlotKind::Text,
                });
            }
            _ => {}
        }
    }
    if paragraph_hidden {
        Ok(Vec::new())
    } else {
        Ok(slots)
    }
}

fn table_flow(events: &[Event<'static>]) -> Result<Option<Vec<Vec<String>>>, DocxError> {
    let cells = table_cells(events)?;
    if cells.is_empty() || cells.iter().any(Vec::is_empty) {
        return Ok(None);
    }
    let columns = cells[0].len();
    if cells.iter().any(|row| row.len() != columns) {
        return Ok(None);
    }
    let mut rows = Vec::with_capacity(cells.len());
    for row in cells {
        let mut values = Vec::with_capacity(row.len());
        for cell in row {
            let mut paragraphs = Vec::new();
            for (start, end) in cell {
                paragraphs.push(visible_paragraph_text(&events[start..=end])?);
            }
            values.push(paragraphs.join("\n").trim().to_string());
        }
        rows.push(values);
    }
    if rows.iter().flatten().all(|cell| cell.is_empty()) {
        Ok(None)
    } else {
        Ok(Some(rows))
    }
}

/// Rows → cells → paragraph event ranges, all relative to `events`.
fn table_cells(events: &[Event<'static>]) -> Result<TableCells, DocxError> {
    let mut rows = Vec::new();
    let mut current_row: Option<Vec<Vec<(usize, usize)>>> = None;
    let mut current_cell: Option<Vec<(usize, usize)>> = None;
    let mut paragraph_start = None;
    let mut nested_table_depth = 0_usize;
    for (index, event) in events.iter().enumerate() {
        match event {
            Event::Start(start) => match local_name(start.name().as_ref()) {
                b"tbl" => nested_table_depth += 1,
                b"tr" if nested_table_depth == 1 => current_row = Some(Vec::new()),
                b"tc" if nested_table_depth == 1 => current_cell = Some(Vec::new()),
                b"p" if nested_table_depth == 1 => paragraph_start = Some(index),
                b"gridSpan" | b"vMerge" if nested_table_depth == 1 => return Ok(Vec::new()),
                _ => {}
            },
            Event::Empty(empty)
                if nested_table_depth == 1
                    && matches!(local_name(empty.name().as_ref()), b"gridSpan" | b"vMerge") =>
            {
                return Ok(Vec::new());
            }
            Event::End(end) => match local_name(end.name().as_ref()) {
                b"p" if nested_table_depth == 1 => {
                    if let Some(start) = paragraph_start.take()
                        && let Some(cell) = &mut current_cell
                    {
                        cell.push((start, index));
                    }
                }
                b"tc" if nested_table_depth == 1 => {
                    if let (Some(row), Some(cell)) = (&mut current_row, current_cell.take()) {
                        row.push(cell);
                    }
                }
                b"tr" if nested_table_depth == 1 => {
                    if let Some(row) = current_row.take() {
                        rows.push(row);
                    }
                }
                b"tbl" => nested_table_depth = nested_table_depth.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
    }
    Ok(rows)
}

fn patch_in_place(
    events: &mut [Event<'static>],
    spans: &[FlowSpan],
    targets: &[TargetFlow],
) -> Result<bool, DocxError> {
    for (span, target) in spans.iter().zip(targets) {
        // Same-position edits keep every direct run property: the template
        // author formatted exactly this spot.
        if !patch_span(events, span, target, false)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn patch_span(
    events: &mut [Event<'static>],
    span: &FlowSpan,
    target: &TargetFlow,
    exemplar: bool,
) -> Result<bool, DocxError> {
    match (&span.content, &target.content) {
        (FlowContent::Paragraph(_), FlowContent::Paragraph(text)) => {
            patch_paragraph(&mut events[span.start..=span.end], text, exemplar)
        }
        (FlowContent::Table(_), FlowContent::Table(rows)) => {
            patch_table(&mut events[span.start..=span.end], rows, exemplar)
        }
        _ => Ok(false),
    }
}

/// Returns `Ok(false)` when the paragraph has no text slot to hold a
/// nonempty target — the caller must fall back to another exemplar instead
/// of silently dropping the content.
fn patch_paragraph(
    events: &mut [Event<'static>],
    target: &str,
    exemplar: bool,
) -> Result<bool, DocxError> {
    let slots = text_slots(events)?;
    if !slots.iter().any(|slot| slot.kind == SlotKind::Text) {
        return Ok(target.trim().is_empty());
    }
    let source = slots
        .iter()
        .map(|slot| slot.text.as_str())
        .collect::<String>();
    if source.trim() == target {
        return Ok(true);
    }
    if let Some(segments) = tab_segments(&slots, target) {
        // Same tab shape: each stretch between tabs is patched on its own,
        // so a bold label keeps its own words and bolding while the value
        // after the tab is replaced.
        for (range, text) in segments {
            apply_allocation(events, &slots[range], text, exemplar);
        }
        return Ok(true);
    }
    // Different tab shape: the paragraph's tabs belong to words that are
    // gone, so they go with them, and the target's own tabs are written as
    // real elements below.
    for slot in slots.iter().filter(|slot| slot.kind == SlotKind::Tab) {
        events[slot.event_index] = Event::Text(BytesText::new("").into_owned());
    }
    let writable = slots
        .into_iter()
        .filter(|slot| slot.kind == SlotKind::Text)
        .collect::<Vec<_>>();
    apply_allocation(events, &writable, target, exemplar);
    Ok(true)
}

/// Slot ranges between the paragraph's tabs, paired with the target text
/// that belongs in each — or `None` when the paragraph and the target do not
/// have the same tab shape, or a stretch with text to place has no `w:t` to
/// place it in.
fn tab_segments<'a>(
    slots: &[TextSlot],
    target: &'a str,
) -> Option<Vec<(std::ops::Range<usize>, &'a str)>> {
    if !slots.iter().any(|slot| slot.kind == SlotKind::Tab) {
        return None;
    }
    let mut ranges = Vec::new();
    let mut start = 0_usize;
    for (index, slot) in slots.iter().enumerate() {
        if slot.kind == SlotKind::Tab {
            ranges.push(start..index);
            start = index + 1;
        }
    }
    ranges.push(start..slots.len());
    let pieces = target.split('\t').collect::<Vec<_>>();
    if pieces.len() != ranges.len() {
        return None;
    }
    if ranges
        .iter()
        .zip(&pieces)
        .any(|(range, piece)| range.is_empty() && !piece.trim().is_empty())
    {
        return None;
    }
    Some(ranges.into_iter().zip(pieces).collect())
}

fn apply_allocation(
    events: &mut [Event<'static>],
    slots: &[TextSlot],
    target: &str,
    exemplar: bool,
) {
    if slots.is_empty() {
        return;
    }
    let allocation = allocate_text(slots, target);
    if exemplar && allocation.full_replacement {
        // The inserted text bears no relation to the exemplar's own words,
        // so text-bound decoration (a bold "Assessment: " label, an
        // underlined signature blank) must not carry over. Fonts, sizes,
        // and paragraph properties stay.
        strip_direct_decoration(events, slots[allocation.owner].event_index);
    }
    for (slot, replacement) in slots.iter().zip(&allocation.replacements) {
        events[slot.event_index] = if replacement.contains(['\n', '\t']) {
            structured_text_event(events, slot.event_index, replacement)
        } else {
            Event::Text(BytesText::new(replacement).into_owned())
        };
    }
}

/// Build a replacement for a text slot whose new content carries line breaks
/// or tabs. A literal newline inside `<w:t>` renders as nothing in Word and a
/// literal tab renders as a space, so each one closes the text element, emits
/// the real OOXML element, and reopens the text element (space-preserving),
/// using the same namespace prefix as the enclosing element.
fn structured_text_event(
    events: &[Event<'static>],
    text_index: usize,
    replacement: &str,
) -> Event<'static> {
    let text_name = events[..text_index]
        .iter()
        .rev()
        .find_map(|event| match event {
            Event::Start(start) if local_name(start.name().as_ref()) == b"t" => {
                Some(String::from_utf8_lossy(start.name().as_ref()).into_owned())
            }
            _ => None,
        })
        .unwrap_or_else(|| "w:t".to_string());
    let prefix = text_name
        .rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default();
    let mut output = String::with_capacity(replacement.len());
    let mut pending = String::new();
    for character in replacement.chars() {
        let element = match character {
            '\n' => "br",
            '\t' => "tab",
            _ => {
                pending.push(character);
                continue;
            }
        };
        output.push_str(&quick_xml::escape::escape(&pending));
        pending.clear();
        output.push_str(&format!(
            "</{text_name}><{prefix}{element}/><{text_name} xml:space=\"preserve\">"
        ));
    }
    output.push_str(&quick_xml::escape::escape(&pending));
    Event::Text(BytesText::from_escaped(output).into_owned())
}

struct TextAllocation {
    replacements: Vec<String>,
    /// Index of the slot that received the unshared middle of the target.
    owner: usize,
    /// True when the shared prefix/suffix is negligible — the inserted text
    /// is unrelated to the exemplar's original words (an incidental shared
    /// period or newline does not make texts related).
    full_replacement: bool,
}

fn allocate_text(slots: &[TextSlot], target: &str) -> TextAllocation {
    let source_chars = slots
        .iter()
        .flat_map(|slot| slot.text.chars())
        .collect::<Vec<_>>();
    let target_chars = target.chars().collect::<Vec<_>>();
    let prefix = source_chars
        .iter()
        .zip(&target_chars)
        .take_while(|(left, right)| left == right)
        .count();
    let max_suffix = source_chars
        .len()
        .saturating_sub(prefix)
        .min(target_chars.len().saturating_sub(prefix));
    let suffix = source_chars
        .iter()
        .rev()
        .zip(target_chars.iter().rev())
        .take(max_suffix)
        .take_while(|(left, right)| left == right)
        .count();

    let mut ranges = Vec::with_capacity(slots.len());
    let mut position = 0_usize;
    for slot in slots {
        let end = position + slot.text.chars().count();
        ranges.push((position, end));
        position = end;
    }
    let shared = prefix + suffix;
    let full_replacement = shared * 4 < source_chars.len().min(target_chars.len());
    let owner = if full_replacement {
        // Unrelated text: land it in the visually dominant (longest) run,
        // not run 0 — which in "Assessment: ____"-style label paragraphs is
        // a bold/underlined prefix whose formatting would swallow the whole
        // inserted paragraph. Earliest run wins length ties.
        ranges
            .iter()
            .enumerate()
            .max_by_key(|(index, (start, end))| (end - start, std::cmp::Reverse(*index)))
            .map(|(index, _)| index)
            .unwrap_or_default()
    } else {
        ranges
            .iter()
            .position(|(start, end)| *start <= prefix && prefix < *end)
            .unwrap_or_else(|| ranges.len().saturating_sub(1))
    };
    let source_suffix_start = source_chars.len().saturating_sub(suffix);
    let target_suffix_start = target_chars.len().saturating_sub(suffix);

    let replacements = ranges
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| {
            let mut output = String::new();
            if start < prefix {
                output.extend(target_chars[start..end.min(prefix)].iter());
            }
            if index == owner {
                output.extend(target_chars[prefix..target_suffix_start].iter());
            }
            let suffix_start = start.max(source_suffix_start);
            if suffix_start < end {
                let target_start = target_suffix_start + (suffix_start - source_suffix_start);
                let target_end = target_start + (end - suffix_start);
                output.extend(target_chars[target_start..target_end].iter());
            }
            output
        })
        .collect();
    TextAllocation {
        replacements,
        owner,
        full_replacement,
    }
}

/// Blank direct `w:u`/`w:b`/`w:i` (and complex-script variants) from the run
/// that owns the text event at `text_event_index`. Fonts, sizes, colors, and
/// paragraph properties are untouched; style-derived decoration (e.g. an
/// underlined heading style) is intentionally preserved.
fn strip_direct_decoration(events: &mut [Event<'static>], text_event_index: usize) {
    const DECORATION: [&[u8]; 5] = [b"u", b"b", b"i", b"bCs", b"iCs"];
    let Some(run_start) = events[..text_event_index].iter().rposition(
        |event| matches!(event, Event::Start(start) if local_name(start.name().as_ref()) == b"r"),
    ) else {
        return;
    };
    let mut in_run_properties = false;
    for event in events[run_start..text_event_index].iter_mut() {
        let blank = match &*event {
            Event::Start(start) => {
                let name = start.name();
                let local = local_name(name.as_ref());
                if local == b"rPr" {
                    in_run_properties = true;
                    false
                } else {
                    in_run_properties && DECORATION.contains(&local)
                }
            }
            Event::Empty(empty) => {
                let name = empty.name();
                in_run_properties && DECORATION.contains(&local_name(name.as_ref()))
            }
            Event::End(end) => {
                let name = end.name();
                let local = local_name(name.as_ref());
                if local == b"rPr" {
                    break;
                }
                in_run_properties && DECORATION.contains(&local)
            }
            _ => false,
        };
        if blank {
            *event = Event::Text(BytesText::new("").into_owned());
        }
    }
}

fn patch_table(
    events: &mut [Event<'static>],
    target_rows: &[Vec<String>],
    exemplar: bool,
) -> Result<bool, DocxError> {
    let cells = table_cells(events)?;
    if cells.len() != target_rows.len()
        || cells
            .iter()
            .zip(target_rows)
            .any(|(source, target)| source.len() != target.len())
    {
        return Ok(false);
    }
    for (source_row, target_row) in cells.into_iter().zip(target_rows) {
        for (paragraphs, target) in source_row.into_iter().zip(target_row) {
            if paragraphs.is_empty() {
                // A cell with no writable paragraph cannot hold a nonempty
                // target; report failure so the caller regenerates the
                // table instead of silently dropping the content.
                if target.trim().is_empty() {
                    continue;
                }
                return Ok(false);
            }
            let lines = target.split('\n').collect::<Vec<_>>();
            for (index, (start, end)) in paragraphs.iter().copied().enumerate() {
                let replacement = if index + 1 == paragraphs.len() && lines.len() > paragraphs.len()
                {
                    lines[index..].join("\n")
                } else {
                    lines.get(index).copied().unwrap_or_default().to_string()
                };
                if !patch_paragraph(&mut events[start..=end], &replacement, exemplar)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// One section of the template's flow: the span holding its heading, and the
/// span range holding its body. `heading` is `None` for the leading range
/// before the document's first heading.
#[derive(Debug, Clone, Copy)]
struct TemplateSection {
    heading: Option<usize>,
    start: usize,
    end: usize,
}

impl TemplateSection {
    /// Where in the template's index space this section sits, for choosing a
    /// document-wide exemplar when the section itself has none.
    fn anchor(&self) -> usize {
        self.heading.unwrap_or(self.start)
    }
}

/// The template's flow spans segmented into sections.
#[derive(Debug, Clone)]
struct TemplateLayout {
    title: Option<usize>,
    /// Always nonempty: element 0 is the leading range, which carries no
    /// heading of its own.
    sections: Vec<TemplateSection>,
}

impl TemplateLayout {
    fn of(spans: &[FlowSpan]) -> Self {
        let mut title = None;
        let mut sections = Vec::new();
        let mut current = TemplateSection {
            heading: None,
            start: 0,
            end: 0,
        };
        for (index, span) in spans.iter().enumerate() {
            match span.kind {
                FlowKind::Title if title.is_none() && current.start == index => {
                    title = Some(index);
                    current.start = index + 1;
                }
                FlowKind::Heading => {
                    current.end = index;
                    sections.push(current);
                    current = TemplateSection {
                        heading: Some(index),
                        start: index + 1,
                        end: index + 1,
                    };
                }
                _ => {}
            }
        }
        current.end = spans.len();
        sections.push(current);
        Self { title, sections }
    }
}

/// Which template section each draft section belongs to.
///
/// Draft sections descend from an import of this same package, so a heading
/// that still reads the same is the same section. A run of draft sections
/// whose headings were rewritten takes the template sections lying between
/// its matched neighbours, in order — a renamed section still owns the
/// paragraphs the author wrote for it, which is what keeps a rewritten
/// heading on its own bold template paragraph. Anything left over is a
/// section the report added, and anything unclaimed is a section it dropped.
fn align_sections(
    targets: &TargetDocument,
    layout: &TemplateLayout,
    spans: &[FlowSpan],
) -> Vec<Option<usize>> {
    let mut alignment = vec![None; targets.sections.len()];
    let mut cursor = 0_usize;
    for (position, section) in targets.sections.iter().enumerate() {
        let Some(heading) = &section.heading else {
            // The import's invented lead section maps to the template's
            // content before its first heading, and to nothing else.
            if cursor == 0
                && layout
                    .sections
                    .first()
                    .is_some_and(|section| section.heading.is_none())
            {
                alignment[position] = Some(0);
                cursor = 1;
            }
            continue;
        };
        let normalized = normalize_flow_text(heading);
        if let Some(index) = (cursor..layout.sections.len()).find(|index| {
            layout.sections[*index].heading.is_some_and(|span| {
                span_text(&spans[span]).is_some_and(|text| normalize_flow_text(text) == normalized)
            })
        }) {
            alignment[position] = Some(index);
            cursor = index + 1;
        }
    }

    let mut position = 0_usize;
    while position < targets.sections.len() {
        if alignment[position].is_some() || targets.sections[position].heading.is_none() {
            position += 1;
            continue;
        }
        let start = position;
        let mut end = position;
        while end < targets.sections.len()
            && alignment[end].is_none()
            && targets.sections[end].heading.is_some()
        {
            end += 1;
        }
        let lower = (0..start)
            .rev()
            .find_map(|index| alignment[index])
            .map_or(0, |index| index + 1);
        let upper = (end..targets.sections.len())
            .find_map(|index| alignment[index])
            .unwrap_or(layout.sections.len());
        let mut available =
            (lower..upper).filter(|index| layout.sections[*index].heading.is_some());
        for slot in alignment.iter_mut().take(end).skip(start) {
            match available.next() {
                Some(index) => *slot = Some(index),
                None => break,
            }
        }
        position = end;
    }
    alignment
}

/// Assembles the rewritten body, span by span.
struct FlowWriter<'a> {
    events: &'a [Event<'static>],
    spans: &'a [FlowSpan],
    output: Vec<Event<'static>>,
    /// Whether the gap preceding each span has already been emitted, so a
    /// span reused as an exemplar cannot duplicate a spacer.
    gap_emitted: Vec<bool>,
}

impl<'a> FlowWriter<'a> {
    fn new(
        events: &'a [Event<'static>],
        spans: &'a [FlowSpan],
        output: Vec<Event<'static>>,
    ) -> Self {
        Self {
            events,
            spans,
            output,
            gap_emitted: vec![false; spans.len()],
        }
    }

    /// Emit the template's own paragraph for this slot. Its formatting is
    /// not borrowed, so nothing is stripped: identical text short-circuits
    /// in [`patch_paragraph`] and the author's bold heading survives.
    fn aligned(&mut self, span_index: usize, target: &TargetFlow) -> Result<(), DocxError> {
        self.gap_before(span_index)?;
        self.span(span_index, target, false)
    }

    /// Clone a paragraph from elsewhere purely for its formatting.
    fn exemplar(&mut self, span_index: usize, target: &TargetFlow) -> Result<(), DocxError> {
        self.span(span_index, target, true)
    }

    fn generated(&mut self, target: &TargetFlow) -> Result<(), DocxError> {
        self.output.extend(generated_span(target)?);
        Ok(())
    }

    fn span(
        &mut self,
        span_index: usize,
        target: &TargetFlow,
        exemplar: bool,
    ) -> Result<(), DocxError> {
        // A heading exemplar's direct decoration is not decoration: in a
        // template whose sections were carved by appearance, bold is the
        // only thing that makes the paragraph a heading. Stripping it would
        // demote the heading to body text on the way back in.
        let exemplar = exemplar && !matches!(target.kind, FlowKind::Heading | FlowKind::Title);
        let source = &self.spans[span_index];
        let mut replacement = source.clone_events(self.events);
        let synthetic_span = FlowSpan {
            start: 0,
            end: replacement.len().saturating_sub(1),
            direct_body_child: true,
            kind: source.kind,
            content: source.content.clone(),
        };
        if !patch_span(&mut replacement, &synthetic_span, target, exemplar)? {
            replacement = generated_span(target)?;
        }
        self.output.extend(replacement);
        Ok(())
    }

    /// Emit the template's own material between the previous span and this
    /// one — blank spacer paragraphs above all. Gaps ride with the span they
    /// precede, so a section's spacers stay inside that section instead of
    /// landing wherever a proportional walk crossed a boundary.
    fn gap_before(&mut self, span_index: usize) -> Result<(), DocxError> {
        if span_index == 0 || self.gap_emitted[span_index] {
            return Ok(());
        }
        self.gap_emitted[span_index] = true;
        let start = self.spans[span_index - 1].end + 1;
        let end = self.spans[span_index].start;
        if start < end {
            let gap = gap_events(&self.events[start..end])?;
            self.output.extend(gap);
        }
        Ok(())
    }
}

fn reconstruct_flow(
    events: &[Event<'static>],
    spans: &[FlowSpan],
    targets: &TargetDocument,
) -> Result<(Vec<Event<'static>>, TemplateRenderFidelity), DocxError> {
    if spans.is_empty() || spans.iter().any(|span| !span.direct_body_child) {
        return Ok((
            generated_document_events(targets)?,
            TemplateRenderFidelity::PlainBodyFallback,
        ));
    }
    let layout = TemplateLayout::of(spans);
    let alignment = align_sections(targets, &layout, spans);
    let first = spans.first().expect("checked nonempty");
    let last = spans.last().expect("checked nonempty");
    let mut writer = FlowWriter::new(events, spans, gap_events(&events[..first.start])?);

    if targets.emit_title {
        let target = targets.title_target();
        match layout.title {
            Some(index) => writer.aligned(index, &target)?,
            None => match nearest_of_kind(spans, 0, FlowKind::Title) {
                Some(index) => writer.exemplar(index, &target)?,
                None => writer.generated(&target)?,
            },
        }
    }

    let mut anchor = layout.sections[0].anchor();
    for (section, aligned) in targets.sections.iter().zip(&alignment) {
        let template = aligned.map(|index| layout.sections[index]);
        if let Some(template) = template {
            anchor = template.anchor();
        }
        if let Some(heading) = &section.heading {
            let target = TargetFlow {
                kind: FlowKind::Heading,
                content: FlowContent::Paragraph(heading.clone()),
            };
            match template.and_then(|template| template.heading) {
                Some(index) => writer.aligned(index, &target)?,
                None => match nearest_of_kind(spans, anchor, FlowKind::Heading) {
                    Some(index) => writer.exemplar(index, &target)?,
                    None => writer.generated(&target)?,
                },
            }
        }
        let mut placed = [0_usize; FlowKind::COUNT];
        for block in &section.blocks {
            let candidates = template
                .map(|template| {
                    (template.start..template.end)
                        .filter(|index| spans[*index].kind == block.kind)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let ordinal = placed[block.kind.index()];
            placed[block.kind.index()] += 1;
            if let Some(index) = candidates.get(ordinal) {
                writer.aligned(*index, block)?;
            } else if let Some(index) = candidates.last() {
                writer.exemplar(*index, block)?;
            } else if let Some(index) = nearest_of_kind(spans, anchor, block.kind) {
                writer.exemplar(index, block)?;
            } else {
                writer.generated(block)?;
            }
        }
        if let Some(template) = template {
            anchor = template.end;
        }
    }

    let mut output = writer.output;
    output.extend(gap_events(&events[last.end + 1..])?);
    Ok((output, TemplateRenderFidelity::Reconstructed))
}

/// The events between two flow spans, stripped of anything carrying content.
///
/// Blank spacer paragraphs, `sectPr`, and bookmarks are the template's own
/// layout and must survive. A `w:tbl` or a text-bearing paragraph that failed
/// span recognition is content the accepted draft does not contain — a
/// merged-cell table the model never saw, most often — and re-emitting it
/// smuggles template text into a clinical report.
fn gap_events(events: &[Event<'static>]) -> Result<Vec<Event<'static>>, DocxError> {
    let mut output = Vec::new();
    let mut index = 0_usize;
    while index < events.len() {
        match &events[index] {
            Event::Start(start) if local_name(start.name().as_ref()) == b"tbl" => {
                index = element_end(events, index) + 1;
            }
            Event::Start(start) if local_name(start.name().as_ref()) == b"p" => {
                let end = element_end(events, index);
                if visible_paragraph_text(&events[index..=end])?
                    .trim()
                    .is_empty()
                {
                    output.extend_from_slice(&events[index..=end]);
                }
                index = end + 1;
            }
            event => {
                output.push(event.clone());
                index += 1;
            }
        }
    }
    Ok(output)
}

/// Index of the event closing the element that opens at `start`.
fn element_end(events: &[Event<'static>], start: usize) -> usize {
    let Event::Start(element) = &events[start] else {
        return start;
    };
    let name = local_name(element.name().as_ref()).to_vec();
    let mut depth = 0_usize;
    for (index, event) in events.iter().enumerate().skip(start) {
        match event {
            Event::Start(open) if local_name(open.name().as_ref()) == name => depth += 1,
            Event::End(close) if local_name(close.name().as_ref()) == name => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    events.len().saturating_sub(1)
}

impl FlowSpan {
    fn clone_events(&self, events: &[Event<'static>]) -> Vec<Event<'static>> {
        events[self.start..=self.end].to_vec()
    }
}

/// The span of this kind nearest to `anchor` in the template's own index
/// space — the last resort before generating formatting from nothing.
fn nearest_of_kind(spans: &[FlowSpan], anchor: usize, kind: FlowKind) -> Option<usize> {
    spans
        .iter()
        .enumerate()
        .filter(|(_, span)| span.kind == kind)
        .min_by_key(|(index, _)| index.abs_diff(anchor))
        .map(|(index, _)| index)
}

fn generated_span(target: &TargetFlow) -> Result<Vec<Event<'static>>, DocxError> {
    // Render a minimal draft holding only this target and select the span
    // of the matching kind. Rendering the whole report and indexing by
    // position was wrong whenever an earlier multi-line paragraph split
    // into several <w:p> elements — the shifted index handed back a block
    // of the wrong kind (e.g. a heading-styled paragraph for body text).
    let draft = single_target_draft(target)?;
    let bytes = render_report(&draft)?;
    let xml = zip_entry(&bytes, "word/document.xml")?;
    let events = parse_events(&xml)?;
    let spans = discover_flow(
        &events,
        &StyleCatalog::from_package(&bytes),
        &mut FlowClassifier::styles(),
    )?;
    spans
        .iter()
        .find(|span| span.kind == target.kind)
        .map(|span| span.clone_events(&events))
        .ok_or_else(|| {
            DocxError::Render("could not construct a formatted template block".to_string())
        })
}

fn single_target_draft(target: &TargetFlow) -> Result<ReportDraft, DocxError> {
    let section =
        |heading: &str, blocks: Vec<ReportBlock>| claria_core::models::report::ReportSection {
            id: uuid::Uuid::new_v4(),
            heading: heading.to_string(),
            blocks,
            skipped: false,
            template_blocks: None,
            authorship: None,
        };
    let (title, sections) = match (&target.kind, &target.content) {
        (FlowKind::Title, FlowContent::Paragraph(text)) => (text.clone(), Vec::new()),
        (FlowKind::Heading, FlowContent::Paragraph(text)) => {
            ("Generated".to_string(), vec![section(text, Vec::new())])
        }
        (FlowKind::Body, FlowContent::Paragraph(text)) => (
            "Generated".to_string(),
            vec![section(
                "Generated",
                vec![ReportBlock::Paragraph { text: text.clone() }],
            )],
        ),
        (FlowKind::List, FlowContent::Paragraph(text)) => (
            "Generated".to_string(),
            vec![section(
                "Generated",
                vec![ReportBlock::BulletList {
                    items: vec![text.clone()],
                }],
            )],
        ),
        (FlowKind::Table, FlowContent::Table(rows)) => (
            "Generated".to_string(),
            vec![section(
                "Generated",
                vec![ReportBlock::Table {
                    rows: rows.clone(),
                    has_header: false,
                    column_widths: None,
                }],
            )],
        ),
        _ => {
            return Err(DocxError::Render(
                "could not construct a formatted template block".to_string(),
            ));
        }
    };
    let timestamp = "1970-01-01T00:00:00Z"
        .parse()
        .map_err(|error| DocxError::Render(format!("invalid fallback timestamp: {error}")))?;
    Ok(ReportDraft {
        revision: 0,
        content: claria_core::models::report::ReportContent { title, sections },
        created_at: timestamp,
        updated_at: timestamp,
        last_applied_proposal_id: None,
    })
}

fn generated_document_events(targets: &TargetDocument) -> Result<Vec<Event<'static>>, DocxError> {
    let draft = targets.to_draft()?;
    let bytes = render_report(&draft)?;
    let xml = zip_entry(&bytes, "word/document.xml")?;
    parse_events(&xml)
}

impl TargetDocument {
    /// This path is used only when an uploaded document has an unsupported
    /// body wrapper. Reusing the already validated draft representation would
    /// require carrying it through every helper, so reconstruct a minimal
    /// equivalent. Generated formatting cannot express a document without a
    /// title or a section without a heading, so both are named here.
    fn to_draft(&self) -> Result<ReportDraft, DocxError> {
        let sections = self
            .sections
            .iter()
            .map(|section| claria_core::models::report::ReportSection {
                id: uuid::Uuid::new_v4(),
                heading: section
                    .heading
                    .clone()
                    .unwrap_or_else(|| SYNTHETIC_SECTION_HEADING.to_string()),
                blocks: blocks_from_targets(&section.blocks),
                skipped: false,
                template_blocks: None,
                authorship: None,
            })
            .collect();
        let timestamp = "1970-01-01T00:00:00Z"
            .parse()
            .map_err(|error| DocxError::Render(format!("invalid fallback timestamp: {error}")))?;
        Ok(ReportDraft {
            revision: 0,
            content: claria_core::models::report::ReportContent {
                title: self.title.clone(),
                sections,
            },
            created_at: timestamp,
            updated_at: timestamp,
            last_applied_proposal_id: None,
        })
    }
}

fn blocks_from_targets(targets: &[TargetFlow]) -> Vec<ReportBlock> {
    let mut blocks: Vec<ReportBlock> = Vec::new();
    for target in targets {
        match (&target.kind, &target.content) {
            (FlowKind::List, FlowContent::Paragraph(text)) => match blocks.last_mut() {
                Some(ReportBlock::BulletList { items }) => items.push(text.clone()),
                _ => blocks.push(ReportBlock::BulletList {
                    items: vec![text.clone()],
                }),
            },
            (_, FlowContent::Paragraph(text)) => {
                blocks.push(ReportBlock::Paragraph { text: text.clone() });
            }
            (_, FlowContent::Table(rows)) => blocks.push(ReportBlock::Table {
                rows: rows.clone(),
                has_header: false,
                column_widths: None,
            }),
        }
    }
    blocks
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn attribute_value(start: &quick_xml::events::BytesStart<'_>, local: &[u8]) -> Option<String> {
    start
        .attributes()
        .with_checks(false)
        .flatten()
        .find_map(|attribute| {
            (local_name(attribute.key.as_ref()) == local)
                .then(|| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
        })
}

fn zip_entry(bytes: &[u8], name: &str) -> Result<Vec<u8>, DocxError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| DocxError::Render(format!("template package is invalid: {error}")))?;
    let mut file = archive.by_name(name).map_err(|error| {
        DocxError::Render(format!("template package is missing {name}: {error}"))
    })?;
    let mut value = Vec::new();
    file.read_to_end(&mut value)
        .map_err(|error| DocxError::Render(format!("could not read {name}: {error}")))?;
    Ok(value)
}

fn replace_zip_entry(bytes: &[u8], name: &str, replacement: &[u8]) -> Result<Vec<u8>, DocxError> {
    rebuild_package(bytes, &[(name.to_string(), replacement.to_vec())])
}

/// Re-emit the package with the named entries replaced; entries the source
/// archive does not contain are appended as new package parts.
fn rebuild_package(bytes: &[u8], entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, DocxError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| DocxError::Render(format!("template package is invalid: {error}")))?;
    let mut written = std::collections::HashSet::new();
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut output);
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).map_err(|error| {
                DocxError::Render(format!("could not read template package entry: {error}"))
            })?;
            let entry_name = file.name().to_string();
            let options = SimpleFileOptions::default()
                .compression_method(file.compression())
                .last_modified_time(file.last_modified().unwrap_or_default());
            if file.is_dir() {
                writer
                    .add_directory(&entry_name, options)
                    .map_err(|error| {
                        DocxError::Render(format!("could not copy template directory: {error}"))
                    })?;
                continue;
            }
            writer.start_file(&entry_name, options).map_err(|error| {
                DocxError::Render(format!("could not copy template entry: {error}"))
            })?;
            if let Some((_, replacement)) = entries.iter().find(|(name, _)| *name == entry_name) {
                written.insert(entry_name);
                writer.write_all(replacement).map_err(|error| {
                    DocxError::Render(format!("could not write updated template XML: {error}"))
                })?;
            } else {
                std::io::copy(&mut file, &mut writer).map_err(|error| {
                    DocxError::Render(format!("could not copy template entry: {error}"))
                })?;
            }
        }
        for (name, replacement) in entries {
            if written.contains(name) {
                continue;
            }
            writer
                .start_file(name, SimpleFileOptions::default())
                .map_err(|error| {
                    DocxError::Render(format!("could not add template entry: {error}"))
                })?;
            writer.write_all(replacement).map_err(|error| {
                DocxError::Render(format!("could not write added template entry: {error}"))
            })?;
        }
        writer.finish().map_err(|error| {
            DocxError::Render(format!("could not finish template DOCX: {error}"))
        })?;
    }
    Ok(output.into_inner())
}
