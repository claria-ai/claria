//! Shared policy for treating uploaded record objects as readable text.
//!
//! Filenames are deliberately irrelevant. JSON, Markdown, CSV, source files,
//! and extensionless notes are all readable when their bytes are safe UTF-8
//! text. Binary containers remain dependent on their generated `.text`
//! sidecars.

/// Maximum size of one original record object or generated text sidecar that
/// Claria will load into memory for preview, Chat, or Writing.
pub const MAX_RECORD_TEXT_BYTES: u64 = 2 * 1024 * 1024;

/// Return an uploaded record body as text when it is valid, printable UTF-8.
///
/// Newlines, carriage returns, and tabs are accepted. Other control characters
/// and common binary-container signatures are rejected. The check is based on
/// content rather than filename, so structured formats such as JSON and
/// Markdown remain unchanged and readable under their original names.
pub fn decode_record_text(bytes: &[u8]) -> Option<&str> {
    if has_binary_container_signature(bytes) {
        return None;
    }

    let text = std::str::from_utf8(bytes).ok()?;
    text.chars()
        .all(|character| matches!(character, '\n' | '\r' | '\t') || !character.is_control())
        .then_some(text)
}

fn has_binary_container_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
        || bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"RIFF")
        || bytes.starts_with(b"fLaC")
        || bytes.starts_with(b"OggS")
        || bytes.starts_with(b"ID3")
        || bytes.starts_with(b"\x1f\x8b")
        || bytes
            .get(4..8)
            .is_some_and(|signature| signature == b"ftyp")
}
