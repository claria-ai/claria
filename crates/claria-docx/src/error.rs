use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocxError {
    #[error("failed to build Word document: {0}")]
    Render(String),
    #[error("could not import the Word template: {0}")]
    Import(String),
    #[error("the Word template was rejected: {0}")]
    UnsafeTemplate(String),
}
