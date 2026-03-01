use thiserror::Error;

#[derive(Debug, Error)]
pub enum WhisperError {
    #[error("failed to load whisper model: {0}")]
    ModelLoad(String),

    #[error("transcription failed: {0}")]
    Transcription(String),

    #[error("tokenizer error: {0}")]
    Tokenizer(String),
}
