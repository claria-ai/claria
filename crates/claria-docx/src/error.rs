use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocxError {
    #[error("failed to build Word document: {0}")]
    Render(String),
}
