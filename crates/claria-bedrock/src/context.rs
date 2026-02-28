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
/// Returns an XML-style block that can be prepended to the system prompt.
/// If `files` is empty, returns an empty string (no context to inject).
pub fn build_context_block(files: &[ContextFile]) -> String {
    if files.is_empty() {
        return String::new();
    }

    let mut block = String::from("<record_context>\n");

    for file in files {
        block.push_str(&format!("<file name=\"{}\">\n", file.filename));
        block.push_str(&file.text);
        if !file.text.ends_with('\n') {
            block.push('\n');
        }
        block.push_str("</file>\n");
    }

    block.push_str("</record_context>");
    block
}
