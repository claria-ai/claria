//! Which paragraph styles denote titles and headings, resolved from a
//! package's `word/styles.xml`.
//!
//! Real Word templates rarely use literal `Heading1` styleIds: clinicians
//! apply custom named styles ("Section Heading") whose heading-ness lives in
//! the style *definition* — its outline level, its display name, or a
//! `basedOn` chain reaching a built-in heading style. Both the importer and
//! the template renderer classify through this catalog so the two can never
//! disagree about the same package.

use std::collections::{HashMap, HashSet};

use quick_xml::{Reader, events::Event};

#[derive(Debug, Default)]
pub(crate) struct StyleCatalog {
    titles: HashSet<String>,
    headings: HashSet<String>,
}

#[derive(Debug, Default)]
struct StyleRecord {
    name: String,
    based_on: Option<String>,
    outline: bool,
}

impl StyleCatalog {
    /// Build the catalog from a full package. A package without a readable
    /// `word/styles.xml` yields an empty catalog; the literal styleId rules
    /// in [`is_title`]/[`is_heading`] still apply.
    pub(crate) fn from_package(bytes: &[u8]) -> Self {
        let Some(styles_xml) = styles_entry(bytes) else {
            return Self::default();
        };
        let Ok(records) = parse_styles(&styles_xml) else {
            return Self::default();
        };
        let mut catalog = Self::default();
        for style_id in records.keys() {
            match resolve(style_id, &records) {
                Some(Resolved::Title) => {
                    catalog.titles.insert(style_id.clone());
                }
                Some(Resolved::Heading) => {
                    catalog.headings.insert(style_id.clone());
                }
                None => {}
            }
        }
        catalog
    }

    /// Whether a normalized styleId denotes the document title.
    pub(crate) fn is_title(&self, normalized_style: &str) -> bool {
        normalized_style == "title" || self.titles.contains(normalized_style)
    }

    /// Whether a normalized styleId denotes a section heading.
    pub(crate) fn is_heading(&self, normalized_style: &str) -> bool {
        normalized_style.starts_with("heading") || self.headings.contains(normalized_style)
    }
}

enum Resolved {
    Title,
    Heading,
}

/// Classify one style through its `basedOn` chain (bounded, cycle-safe).
fn resolve(style_id: &str, records: &HashMap<String, StyleRecord>) -> Option<Resolved> {
    let mut current = style_id;
    for _ in 0..records.len().max(1) {
        let record = records.get(current)?;
        if current == "title" || record.name == "title" {
            return Some(Resolved::Title);
        }
        if current.starts_with("heading") || record.name.starts_with("heading") || record.outline {
            return Some(Resolved::Heading);
        }
        current = record.based_on.as_deref()?;
    }
    None
}

/// Reduce a styleId or style name to its comparable form: alphanumerics,
/// lowercased. Shared by the importer and the template renderer.
pub(crate) fn normalize_style(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn styles_entry(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name("word/styles.xml").ok()?;
    let mut value = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut value).ok()?;
    Some(value)
}

/// Parse `word/styles.xml` into normalized per-style records.
fn parse_styles(xml: &[u8]) -> Result<HashMap<String, StyleRecord>, quick_xml::Error> {
    let mut reader = Reader::from_reader(xml);
    let mut records = HashMap::new();
    let mut buffer = Vec::new();
    let mut current: Option<(String, StyleRecord)> = None;
    let mut style_depth = 0_usize;
    loop {
        let event = reader.read_event_into(&mut buffer)?;
        match &event {
            Event::Start(start) | Event::Empty(start) => {
                let is_empty = matches!(event, Event::Empty(_));
                let name = start.name();
                match local(name.as_ref()) {
                    b"style" => {
                        let paragraph_type = attribute(start, b"type")
                            .is_none_or(|value| value == "paragraph");
                        if let (true, Some(style_id)) =
                            (paragraph_type, attribute(start, b"styleId"))
                        {
                            current =
                                Some((normalize_style(&style_id), StyleRecord::default()));
                        }
                        if !is_empty {
                            style_depth += 1;
                        }
                    }
                    b"name" => {
                        if let (Some((_, record)), Some(value)) =
                            (&mut current, attribute(start, b"val"))
                        {
                            record.name = normalize_style(&value);
                        }
                    }
                    b"basedOn" => {
                        if let (Some((_, record)), Some(value)) =
                            (&mut current, attribute(start, b"val"))
                        {
                            record.based_on = Some(normalize_style(&value));
                        }
                    }
                    b"outlineLvl" => {
                        if let Some((_, record)) = &mut current {
                            record.outline = true;
                        }
                    }
                    _ => {}
                }
                if is_empty
                    && local(name.as_ref()) == b"style"
                    && let Some((style_id, record)) = current.take()
                {
                    records.insert(style_id, record);
                }
            }
            Event::End(end) => {
                if local(end.name().as_ref()) == b"style" {
                    style_depth = style_depth.saturating_sub(1);
                    if style_depth == 0
                        && let Some((style_id, record)) = current.take()
                    {
                        records.insert(style_id, record);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(records)
}

fn local(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
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
