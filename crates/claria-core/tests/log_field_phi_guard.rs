//! Guard for the "Logging & audit" rule in CLAUDE.md: log fields are UUIDs,
//! hashes, counts, byte sizes, model IDs, and durations — never a filename.
//!
//! Client-chosen filenames are PHI, and the console export is a support
//! artifact in a HIPAA app, so this walks every `crates/*/src` tree and fails
//! if a `tracing` event, span, or `#[instrument]` attribute mentions a
//! filename anywhere in its fields. Log the extension and the byte size
//! instead (see `claria-bedrock`'s document extraction).

use std::{fs, path::Path};

/// Macro and attribute heads whose parenthesized arguments become log fields.
const FIELD_BEARING_CALLS: &[&str] = &[
    "trace!(",
    "debug!(",
    "info!(",
    "warn!(",
    "error!(",
    "event!(",
    "span!(",
    "trace_span!(",
    "debug_span!(",
    "info_span!(",
    "warn_span!(",
    "error_span!(",
    "instrument(",
    ".record(",
];

/// Spellings of a filename field. Matched case-insensitively as substrings,
/// so `sidecar_filename` and `%file_name` are caught too.
const FORBIDDEN_FRAGMENTS: &[&str] = &["filename", "file_name"];

/// Sites that name a filename in a log field on purpose, as
/// `crate-relative/path.rs:line`. An entry needs a comment explaining why the
/// value cannot identify a person.
const ALLOWLIST: &[&str] = &[];

#[test]
fn no_tracing_field_logs_a_filename() {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("claria-core sits in crates/")
        .to_path_buf();

    let mut sources = Vec::new();
    for entry in fs::read_dir(&crates_dir).expect("crates/ is readable") {
        let src = entry.expect("readable dir entry").path().join("src");
        if src.is_dir() {
            collect_rust_files(&src, &mut sources);
        }
    }
    assert!(
        sources.len() > 50,
        "the source walk found only {} files — the layout moved",
        sources.len()
    );

    let mut offenders = Vec::new();
    for path in &sources {
        let text = fs::read_to_string(path).expect("source file is UTF-8");
        let relative = path
            .strip_prefix(&crates_dir)
            .expect("under crates/")
            .display()
            .to_string();

        for (offset, args) in field_bearing_arguments(&text) {
            let lowered = args.to_lowercase();
            if !FORBIDDEN_FRAGMENTS
                .iter()
                .any(|fragment| lowered.contains(fragment))
            {
                continue;
            }
            let line = text[..offset].lines().count();
            let site = format!("{relative}:{line}");
            if ALLOWLIST.contains(&site.as_str()) {
                continue;
            }
            let snippet: String = args
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(160)
                .collect();
            offenders.push(format!("{site}: {snippet}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "log fields must never carry a filename (log the extension and byte size instead):\n  {}",
        offenders.join("\n  ")
    );
}

fn collect_rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).expect("readable directory") {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every `(...)` argument list belonging to a field-bearing macro or
/// attribute, paired with the byte offset of the call.
fn field_bearing_arguments(text: &str) -> Vec<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut cursor = 0;

    while cursor < bytes.len() {
        let Some((head_at, open_paren)) = next_call(text, cursor) else {
            break;
        };
        match closing_paren(bytes, open_paren) {
            Some(close) => {
                found.push((head_at, &text[open_paren + 1..close]));
                cursor = open_paren + 1;
            }
            None => break,
        }
    }

    found
}

/// The next field-bearing call at or after `from`, as
/// `(offset of the head, offset of its opening paren)`.
fn next_call(text: &str, from: usize) -> Option<(usize, usize)> {
    FIELD_BEARING_CALLS
        .iter()
        .filter_map(|needle| {
            text[from..]
                .find(needle)
                .map(|at| (from + at, needle.len()))
        })
        .min_by_key(|(at, _)| *at)
        .map(|(at, len)| (at, at + len - 1))
}

/// Offset of the paren closing the one at `open`, skipping string and char
/// literals so that a parenthesis inside a log message cannot unbalance the
/// scan.
fn closing_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;

    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
            }
            b'\'' if bytes.get(index + 1).is_some_and(|c| *c != b'\'') => {
                // A char literal, not a lifetime: skip to its closing quote
                // when one is within the next few bytes.
                if let Some(end) = bytes[index + 1..index + 5.min(bytes.len() - index)]
                    .iter()
                    .position(|c| *c == b'\'')
                {
                    index += end + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }

    None
}
