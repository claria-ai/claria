//! Record context builder for chat conversations.
//!
//! Assembles text content from a client's record files into a structured
//! context block that can be prepended to the system prompt. This gives
//! the chat model awareness of all documents in the client's record.

use serde::{Deserialize, Serialize};

/// A record file with its extracted text content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFile {
    pub filename: String,
    pub text: String,
}

/// Build a structured context block from record files.
///
/// Returns an XML-style block appended after the system prompt. If `files`
/// is empty, returns an empty string (no context to inject).
///
/// Only `<` characters that begin one of this block's own delimiter tags are
/// escaped, so file content can never forge a `<file>`/`</file>` or
/// `<record_context>`/`</record_context>` tag and break out of the
/// untrusted-data block — while ordinary clinical text (`T-score >70`,
/// `<3rd percentile`, `Parent & Teacher`) passes through verbatim. `"` in
/// filenames is escaped so a name cannot terminate the `name` attribute.
pub fn build_context_block(files: &[ContextFile]) -> String {
    if files.is_empty() {
        return String::new();
    }

    let mut block = String::from("<record_context>\n");

    for file in files {
        let text = escape_text(&file.text);
        block.push_str(&format!(
            "<file name=\"{}\">\n",
            escape_attribute(&file.filename)
        ));
        block.push_str(&text);
        if !text.ends_with('\n') {
            block.push('\n');
        }
        block.push_str("</file>\n");
    }

    block.push_str("</record_context>");
    block
}

/// Tag-name stems whose opening/closing forms chat record context must never
/// contain literally.
const CHAT_DELIMITER_STEMS: &[&str] = &["record_context", "file"];

/// Escape embedded document text so it cannot contain a literal form of this
/// block's delimiter tags. Everything else — including bare `<`, `>`, and
/// `&` in clinical prose — is left untouched.
fn escape_text(text: &str) -> String {
    escape_delimiter_forgeries(text, CHAT_DELIMITER_STEMS, "&lt;")
}

/// Escape a value used inside a double-quoted XML attribute.
fn escape_attribute(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}

/// Rewrite every `<` in `text` that begins a delimiter-forging sequence —
/// `<` or `</` followed case-insensitively by one of `stems` — with
/// `replacement`, leaving every other character untouched.
///
/// A stem ending in `_` is an open-ended prefix (`untrusted_` catches every
/// `<untrusted_...>` tag). Any other stem must end at a tag-name boundary
/// (the next character must not be an ASCII alphanumeric, `-`, or `_`), so
/// prose like `<filed under intake>` is untouched while `<file name="x">`,
/// `</file>`, and `<record_context>` are neutralized. The escaped output
/// cannot itself contain a literal opening or closing form of the named
/// delimiters — the property the callers' system prompts rely on.
pub fn escape_delimiter_forgeries(text: &str, stems: &[&str], replacement: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find('<') {
        let (head, tail) = rest.split_at(position);
        output.push_str(head);
        if forges_delimiter(&tail[1..], stems) {
            output.push_str(replacement);
        } else {
            output.push('<');
        }
        rest = &tail[1..];
    }
    output.push_str(rest);
    output
}

/// Whether the text immediately after a `<` spells one of `stems` (with an
/// optional leading `/`), honoring the boundary rules described on
/// [`escape_delimiter_forgeries`].
fn forges_delimiter(after_angle: &str, stems: &[&str]) -> bool {
    let name = after_angle.strip_prefix('/').unwrap_or(after_angle);
    stems.iter().any(|stem| {
        let Some(candidate) = name.get(..stem.len()) else {
            return false;
        };
        if !candidate.eq_ignore_ascii_case(stem) {
            return false;
        }
        if stem.ends_with('_') {
            return true;
        }
        !name[stem.len()..]
            .chars()
            .next()
            .is_some_and(|next| next.is_ascii_alphanumeric() || next == '_' || next == '-')
    })
}
