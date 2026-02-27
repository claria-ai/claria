use crate::error::ExportError;

/// Generate a PDF from rendered template output.
///
/// This is a placeholder — PDF generation requires a rendering library
/// (e.g. `typst`, `printpdf`, or shelling out to `weasyprint`).
/// For now, this returns an error indicating the feature is not yet implemented.
///
/// The intended flow is:
/// 1. Template + SchematizedAnswer → rendered Markdown (via Tera)
/// 2. Rendered Markdown → PDF bytes (this function)
pub fn generate_pdf(_rendered: &str) -> Result<Vec<u8>, ExportError> {
    Err(ExportError::Pdf(
        "PDF generation not yet implemented — library selection pending".to_string(),
    ))
}
