use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("template rendering failed: {0}")]
    TemplateRender(String),

    #[error("template not found: {0}")]
    TemplateNotFound(String),

    #[error("template parse error: {0}")]
    TemplateParse(String),

    #[error("DOCX generation failed: {0}")]
    Docx(String),

    #[error("PDF generation failed: {0}")]
    Pdf(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl From<tera::Error> for ExportError {
    fn from(e: tera::Error) -> Self {
        ExportError::TemplateRender(e.to_string())
    }
}
