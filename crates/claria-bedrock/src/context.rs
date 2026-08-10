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
/// Embedded document text is escaped (`&` and `<`) so file content can
/// never forge a closing `</file>` or `</record_context>` delimiter and
/// break out of the untrusted-data block; `"` in filenames is escaped so a
/// name cannot terminate the `name` attribute.
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

/// Escape embedded document text: `&` first, then `<`, so no sequence in the
/// original text can produce a delimiter tag.
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

/// Escape a value used inside a double-quoted XML attribute.
fn escape_attribute(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}
