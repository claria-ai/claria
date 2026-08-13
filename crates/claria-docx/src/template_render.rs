//! Template-aware DOCX rendering.
//!
//! The original package remains the source of truth for styles, fonts, page
//! setup, headers, footers, media, paragraph spacing, and intentional blank
//! paragraphs. We update only visible body text/table cells in
//! `word/document.xml`; every other package part is copied unchanged.

use std::io::{Cursor, Read, Write};

use claria_core::models::report::{ReportBlock, ReportDraft, validate_report_content};
use quick_xml::{
    Reader, Writer,
    events::{BytesText, Event},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{DocxError, import_template, render_report};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowKind {
    Title,
    Heading,
    Body,
    List,
    Table,
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

#[derive(Debug, Clone)]
struct FlowSpan {
    start: usize,
    end: usize,
    direct_body_child: bool,
    kind: FlowKind,
    content: FlowContent,
}

#[derive(Debug, Clone)]
struct TextSlot {
    event_index: usize,
    text: String,
}

type ParagraphRange = (usize, usize);
type TableCells = Vec<Vec<Vec<ParagraphRange>>>;

/// Render an accepted report back through its original redacted Word package.
///
/// When visible content is unchanged, the source bytes are returned exactly.
/// For edits with the same structure, only text events are changed in place,
/// retaining run-level formatting and every non-text OOXML element. Structural
/// changes reuse the nearest source paragraph/table as a formatting exemplar.
pub fn render_report_with_template(
    template: &[u8],
    draft: &ReportDraft,
) -> Result<Vec<u8>, DocxError> {
    validate_report_content(&draft.content)
        .map_err(|error| DocxError::Render(error.to_string()))?;
    // Reuse the bounded package preflight and parser before retaining any part
    // of an uploaded package in an exported report.
    let imported = import_template(template)?;
    let targets = target_flow(draft);
    if visible_content_matches(&imported.content, draft) {
        return Ok(template.to_vec());
    }

    let document_xml = zip_entry(template, "word/document.xml")?;
    let events = parse_events(&document_xml)?;
    let spans = discover_flow(&events)?;

    let rewritten_events = if spans.len() == targets.len()
        && spans
            .iter()
            .zip(&targets)
            .all(|(source, target)| source.kind == target.kind)
    {
        let mut candidate = events.clone();
        if patch_in_place(&mut candidate, &spans, &targets)? {
            candidate
        } else {
            reconstruct_flow(&events, &spans, &targets)?
        }
    } else {
        reconstruct_flow(&events, &spans, &targets)?
    };
    let rewritten_xml = write_events(&rewritten_events)?;
    replace_zip_entry(template, "word/document.xml", &rewritten_xml)
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

fn target_flow(draft: &ReportDraft) -> Vec<TargetFlow> {
    let mut output = vec![TargetFlow {
        kind: FlowKind::Title,
        content: FlowContent::Paragraph(draft.content.title.clone()),
    }];
    for section in &draft.content.sections {
        output.push(TargetFlow {
            kind: FlowKind::Heading,
            content: FlowContent::Paragraph(section.heading.clone()),
        });
        for block in &section.blocks {
            match block {
                ReportBlock::Paragraph { text } => output.push(TargetFlow {
                    kind: FlowKind::Body,
                    content: FlowContent::Paragraph(text.clone()),
                }),
                ReportBlock::BulletList { items } => {
                    output.extend(items.iter().cloned().map(|item| TargetFlow {
                        kind: FlowKind::List,
                        content: FlowContent::Paragraph(item),
                    }));
                }
                ReportBlock::Table { rows, .. } => output.push(TargetFlow {
                    kind: FlowKind::Table,
                    content: FlowContent::Table(rows.clone()),
                }),
            }
        }
    }
    output
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

fn discover_flow(events: &[Event<'static>]) -> Result<Vec<FlowSpan>, DocxError> {
    let mut spans = Vec::new();
    let mut paragraph_start = None;
    let mut table_start = None;
    let mut table_depth = 0_usize;
    let mut depth = 0_usize;
    let mut body_child_depth = None;
    let mut title_seen = false;

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
                            paragraph_flow(&events[start..=index], title_seen)?
                    {
                        title_seen |= kind == FlowKind::Title;
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
    title_seen: bool,
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
    let kind = if style == "title" && !title_seen {
        FlowKind::Title
    } else if style.starts_with("heading") {
        FlowKind::Heading
    } else if numbered {
        FlowKind::List
    } else {
        FlowKind::Body
    };
    Ok(Some((kind, text)))
}

fn visible_paragraph_text(events: &[Event<'static>]) -> Result<String, DocxError> {
    Ok(text_slots(events)?
        .into_iter()
        .map(|slot| slot.text)
        .collect::<String>())
}

fn text_slots(events: &[Event<'static>]) -> Result<Vec<TextSlot>, DocxError> {
    let mut slots = Vec::new();
    let mut in_text = false;
    let mut excluded_depth = 0_usize;
    let mut run_depth = 0_usize;
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
            patch_paragraph(&mut events[span.start..=span.end], text, exemplar)?;
            Ok(true)
        }
        (FlowContent::Table(_), FlowContent::Table(rows)) => {
            patch_table(&mut events[span.start..=span.end], rows, exemplar)
        }
        _ => Ok(false),
    }
}

fn patch_paragraph(
    events: &mut [Event<'static>],
    target: &str,
    exemplar: bool,
) -> Result<(), DocxError> {
    let slots = text_slots(events)?;
    if slots.is_empty() {
        return Ok(());
    }
    let source = slots
        .iter()
        .map(|slot| slot.text.as_str())
        .collect::<String>();
    if source.trim() == target {
        return Ok(());
    }
    let allocation = allocate_text(&slots, target);
    if exemplar && allocation.full_replacement {
        // The inserted text bears no relation to the exemplar's own words,
        // so text-bound decoration (a bold "Assessment: " label, an
        // underlined signature blank) must not carry over. Fonts, sizes,
        // and paragraph properties stay.
        strip_direct_decoration(events, slots[allocation.owner].event_index);
    }
    for (slot, replacement) in slots.iter().zip(&allocation.replacements) {
        events[slot.event_index] = Event::Text(BytesText::new(replacement).into_owned());
    }
    Ok(())
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
    let Some(run_start) = events[..text_event_index].iter().rposition(|event| {
        matches!(event, Event::Start(start) if local_name(start.name().as_ref()) == b"r")
    }) else {
        return;
    };
    let mut in_run_properties = false;
    for index in run_start..text_event_index {
        let blank = match &events[index] {
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
            events[index] = Event::Text(BytesText::new("").into_owned());
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
                continue;
            }
            let lines = target.split('\n').collect::<Vec<_>>();
            for (index, (start, end)) in paragraphs.iter().copied().enumerate() {
                let replacement = if index + 1 == paragraphs.len() && lines.len() > paragraphs.len()
                {
                    lines[index..].join("\n")
                } else {
                    lines.get(index).copied().unwrap_or_default().to_string()
                };
                patch_paragraph(&mut events[start..=end], &replacement, exemplar)?;
            }
        }
    }
    Ok(true)
}

fn reconstruct_flow(
    events: &[Event<'static>],
    spans: &[FlowSpan],
    targets: &[TargetFlow],
) -> Result<Vec<Event<'static>>, DocxError> {
    if spans.is_empty() || spans.iter().any(|span| !span.direct_body_child) {
        return generated_document_events(targets);
    }
    let first = spans.first().expect("checked nonempty");
    let last = spans.last().expect("checked nonempty");
    let mut output = events[..first.start].to_vec();

    for (target_index, target) in targets.iter().enumerate() {
        let source_index = nearest_span(spans, targets.len(), target_index, target.kind);
        let mut replacement = if let Some(source_index) = source_index {
            spans[source_index].clone_events(events)
        } else {
            generated_span(targets, target_index)?
        };
        let replacement_len = replacement.len();
        let synthetic_span = FlowSpan {
            start: 0,
            end: replacement_len.saturating_sub(1),
            direct_body_child: true,
            kind: target.kind,
            content: target.content.clone(),
        };
        if !patch_span(&mut replacement, &synthetic_span, target, true)? {
            replacement = generated_span(targets, target_index)?;
        }
        output.extend(replacement);

        // Preserve intentional blank paragraphs and other non-flow body
        // nodes: emit each inter-span gap exactly once, at the point where
        // the proportional walk crosses from one source span to the next.
        // Indexing the gaps with the target cursor piled every spacer after
        // the first spans.len() targets of a report that outgrew its
        // template — the v0.22 "line spacing is wrong in some sections" bug.
        let here = scaled_source_position(spans.len(), targets.len(), target_index);
        let next = if target_index + 1 < targets.len() {
            scaled_source_position(spans.len(), targets.len(), target_index + 1)
        } else {
            spans.len().saturating_sub(1)
        };
        for boundary in here..next {
            let gap_start = spans[boundary].end + 1;
            let gap_end = spans[boundary + 1].start;
            output.extend_from_slice(&events[gap_start..gap_end]);
        }
    }
    output.extend_from_slice(&events[last.end + 1..]);
    Ok(output)
}

/// Map a generated-report position onto the template's span list, so a
/// report that outgrew its template walks the whole source proportionally
/// instead of pinning everything past the template's length to its final
/// span.
fn scaled_source_position(spans_len: usize, targets_len: usize, target_index: usize) -> usize {
    if targets_len == 0 || spans_len == 0 {
        return 0;
    }
    (target_index * spans_len / targets_len).min(spans_len - 1)
}

impl FlowSpan {
    fn clone_events(&self, events: &[Event<'static>]) -> Vec<Event<'static>> {
        events[self.start..=self.end].to_vec()
    }
}

fn nearest_span(
    spans: &[FlowSpan],
    targets_len: usize,
    target_index: usize,
    kind: FlowKind,
) -> Option<usize> {
    // Anchor the search in the template's own index space. Comparing raw
    // report indices against template indices meant every paragraph past
    // the template's length "nearest-matched" the final same-kind span —
    // in clinical templates, an underlined signature line whose formatting
    // then bled over most of the generated report.
    let anchor = scaled_source_position(spans.len(), targets_len, target_index);
    spans
        .iter()
        .enumerate()
        .filter(|(_, span)| span.kind == kind)
        .min_by_key(|(index, _)| index.abs_diff(anchor))
        .map(|(index, _)| index)
}

fn generated_span(
    targets: &[TargetFlow],
    target_index: usize,
) -> Result<Vec<Event<'static>>, DocxError> {
    let events = generated_document_events(targets)?;
    let spans = discover_flow(&events)?;
    let span = spans.get(target_index).ok_or_else(|| {
        DocxError::Render("could not construct a formatted template block".to_string())
    })?;
    Ok(span.clone_events(&events))
}

fn generated_document_events(targets: &[TargetFlow]) -> Result<Vec<Event<'static>>, DocxError> {
    let draft = draft_from_targets(targets)?;
    let bytes = render_report(&draft)?;
    let xml = zip_entry(&bytes, "word/document.xml")?;
    parse_events(&xml)
}

fn draft_from_targets(targets: &[TargetFlow]) -> Result<ReportDraft, DocxError> {
    // This path is used only when an uploaded document has an unsupported body
    // wrapper. Reusing the already validated draft representation would require
    // carrying it through every helper, so reconstruct a minimal equivalent.
    let title = targets
        .first()
        .and_then(|target| match &target.content {
            FlowContent::Paragraph(text) => Some(text.clone()),
            FlowContent::Table(_) => None,
        })
        .ok_or_else(|| DocxError::Render("the report has no title".to_string()))?;
    let mut sections = Vec::new();
    let mut current: Option<claria_core::models::report::ReportSection> = None;
    for target in targets.iter().skip(1) {
        match (&target.kind, &target.content) {
            (FlowKind::Heading, FlowContent::Paragraph(heading)) => {
                if let Some(section) = current.take() {
                    sections.push(section);
                }
                current = Some(claria_core::models::report::ReportSection {
                    id: uuid::Uuid::new_v4(),
                    heading: heading.clone(),
                    blocks: Vec::new(),
                });
            }
            (FlowKind::Body, FlowContent::Paragraph(text)) => {
                if let Some(section) = &mut current {
                    section
                        .blocks
                        .push(ReportBlock::Paragraph { text: text.clone() });
                }
            }
            (FlowKind::List, FlowContent::Paragraph(text)) => {
                if let Some(section) = &mut current {
                    match section.blocks.last_mut() {
                        Some(ReportBlock::BulletList { items }) => items.push(text.clone()),
                        _ => section.blocks.push(ReportBlock::BulletList {
                            items: vec![text.clone()],
                        }),
                    }
                }
            }
            (FlowKind::Table, FlowContent::Table(rows)) => {
                if let Some(section) = &mut current {
                    section.blocks.push(ReportBlock::Table {
                        rows: rows.clone(),
                        has_header: false,
                        column_widths: None,
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
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

fn normalize_style(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
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
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| DocxError::Render(format!("template package is invalid: {error}")))?;
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
            if entry_name == name {
                writer.write_all(replacement).map_err(|error| {
                    DocxError::Render(format!("could not write updated template XML: {error}"))
                })?;
            } else {
                std::io::copy(&mut file, &mut writer).map_err(|error| {
                    DocxError::Render(format!("could not copy template entry: {error}"))
                })?;
            }
        }
        writer.finish().map_err(|error| {
            DocxError::Render(format!("could not finish template DOCX: {error}"))
        })?;
    }
    Ok(output.into_inner())
}
