use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscribeError {
    #[error("transcription job failed: {0}")]
    JobFailed(String),

    #[error("transcription API error: {0}")]
    Api(String),

    #[error("failed to parse transcript: {0}")]
    Parse(String),
}
