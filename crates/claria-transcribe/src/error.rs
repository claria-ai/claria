use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscribeError {
    #[error("transcription job failed: {0}")]
    JobFailed(String),

    #[error("transcription API error: {0}")]
    Api(String),

    #[error("failed to parse transcript: {0}")]
    Parse(String),

    #[error(
        "Transcribe Medical only supports English (en-US); requested language {0} is unsupported"
    )]
    MedicalUnsupportedLanguage(String),

    #[error(
        "Transcribe Medical is not available in region {0}; provision a supported region or use Standard"
    )]
    MedicalUnsupportedRegion(String),

    #[error("incompatible options: {0}")]
    IncompatibleOptions(String),

    #[error(
        "transcription job {job_name} did not finish within {minutes} minutes; it may still be running in AWS Transcribe"
    )]
    Timeout { job_name: String, minutes: u64 },
}
