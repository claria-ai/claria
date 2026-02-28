//! Document text extraction via the Bedrock Converse API.
//!
//! Sends PDF or DOCX files to a Claude model using the `DocumentBlock`
//! content type and asks for pure text extraction. The Converse API handles
//! parsing the document format natively.

use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, DocumentBlock, DocumentFormat, DocumentSource, Message,
    SystemContentBlock,
};
use tracing::info;

use crate::error::BedrockError;

const EXTRACTION_SYSTEM_PROMPT: &str = "\
Extract the complete text content from this document. \
Return only the plain text, preserving paragraph structure. \
Do not add commentary, headers, or formatting.";

/// Extract plain text from a PDF or DOCX document via Bedrock.
///
/// Sends the document bytes to the given model using the Converse API's
/// `DocumentBlock`, which handles PDF and DOCX parsing natively.
///
/// The caller chooses the model (e.g. a Claude Opus inference profile).
pub async fn extract_document_text(
    config: &aws_config::SdkConfig,
    model_id: &str,
    bytes: &[u8],
    filename: &str,
    format: DocumentFormat,
) -> Result<String, BedrockError> {
    let client = aws_sdk_bedrockruntime::Client::new(config);

    let doc_name = sanitize_document_name(filename);

    let doc_block = DocumentBlock::builder()
        .format(format)
        .name(doc_name)
        .source(DocumentSource::Bytes(aws_smithy_types::Blob::new(bytes)))
        .build()
        .map_err(|e| BedrockError::Invocation(e.to_string()))?;

    let message = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Document(doc_block))
        .content(ContentBlock::Text(
            "Extract the full text from this document.".to_string(),
        ))
        .build()
        .map_err(|e| BedrockError::Invocation(e.to_string()))?;

    info!(model_id, filename, "extracting text from document");

    let response = client
        .converse()
        .model_id(model_id)
        .system(SystemContentBlock::Text(
            EXTRACTION_SYSTEM_PROMPT.to_string(),
        ))
        .messages(message)
        .send()
        .await
        .map_err(|e| BedrockError::Invocation(e.into_service_error().to_string()))?;

    let output_message = response
        .output()
        .and_then(|o| o.as_message().ok())
        .ok_or_else(|| BedrockError::ResponseParse("no message in response".to_string()))?;

    let text = output_message
        .content()
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text(t) = block {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    info!(
        model_id,
        filename,
        text_len = text.len(),
        "document text extraction complete"
    );

    Ok(text)
}

/// Sanitize a filename for use as a Bedrock `DocumentBlock` name.
///
/// The name field only allows alphanumeric characters, single whitespace,
/// hyphens, parentheses, and square brackets.
fn sanitize_document_name(filename: &str) -> String {
    let sanitized: String = filename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '(' || c == ')' || c == '[' || c == ']' {
                c
            } else {
                ' '
            }
        })
        .collect();

    // Collapse consecutive whitespace.
    let mut result = String::with_capacity(sanitized.len());
    let mut prev_space = false;
    for c in sanitized.chars() {
        if c == ' ' {
            if !prev_space {
                result.push(c);
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }

    result.trim().to_string()
}

/// Map a file extension to a Bedrock `DocumentFormat`.
///
/// Returns `None` for extensions that don't support text extraction.
pub fn document_format_for_extension(ext: &str) -> Option<DocumentFormat> {
    match ext.to_lowercase().as_str() {
        "pdf" => Some(DocumentFormat::Pdf),
        "docx" => Some(DocumentFormat::Docx),
        "doc" => Some(DocumentFormat::Doc),
        _ => None,
    }
}
