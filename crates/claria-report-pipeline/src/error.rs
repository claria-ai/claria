use claria_report_store::{ReportAttemptMetadata, ReportFailureCode, ReportStoreError};
use claria_storage::error::StorageError;
use thiserror::Error;

/// The one error type for report-writing orchestration.
///
/// Messages are complete, user-facing sentences; callers stringify them once
/// at their own presentation boundary.
#[derive(Debug, Error)]
pub enum ReportPipelineError {
    #[error("{0}")]
    InvalidInput(String),

    #[error("The client no longer exists. Return to the client list and reload.")]
    ClientNotFound,

    #[error("The report changed on another computer. Reload it before continuing.")]
    Conflict,

    #[error("The persisted report workspace is invalid: {0}")]
    InvalidWorkspace(String),

    #[error("Report storage is unavailable while {operation}. Try again.")]
    Storage {
        operation: &'static str,
        #[source]
        source: StorageError,
    },

    #[error("{message}")]
    TurnFailed {
        message: String,
        code: ReportFailureCode,
        attempt: Box<ReportAttemptMetadata>,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

/// Durable-store failures reach the turn loop's callers unchanged: the store
/// owns the same persistence variants under its own name, and every message
/// here is byte-identical to the one it replaces.
impl From<ReportStoreError> for ReportPipelineError {
    fn from(error: ReportStoreError) -> Self {
        match error {
            ReportStoreError::InvalidInput(message) => Self::InvalidInput(message),
            ReportStoreError::ClientNotFound => Self::ClientNotFound,
            ReportStoreError::Conflict => Self::Conflict,
            ReportStoreError::InvalidWorkspace(message) => Self::InvalidWorkspace(message),
            ReportStoreError::Storage { operation, source } => Self::Storage { operation, source },
        }
    }
}

impl ReportPipelineError {
    pub fn storage(operation: &'static str, source: StorageError) -> Self {
        Self::Storage { operation, source }
    }

    pub fn attempt(&self) -> Option<&ReportAttemptMetadata> {
        match self {
            Self::TurnFailed { attempt, .. } => Some(attempt),
            _ => None,
        }
    }

    pub fn failure_code(&self) -> Option<ReportFailureCode> {
        match self {
            Self::TurnFailed { code, .. } => Some(*code),
            _ => None,
        }
    }
}
