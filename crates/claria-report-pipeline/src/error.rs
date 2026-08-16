use claria_report_store::{ReportAttemptMetadata, ReportFailureCode, ReportStoreError};
use claria_storage::error::StorageError;
use thiserror::Error;
use uuid::Uuid;

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

    #[error("The persisted drafting run is invalid: {0}")]
    InvalidRun(String),

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

    /// The reader pressed Stop. Shaped as an error because the turn produced
    /// no outcome, but it is not a failure: a stopped drafting run keeps
    /// every section it landed and stays resumable, and a stopped targeted
    /// turn changed nothing at all.
    ///
    /// `run_id` names the drafting run left parked and resumable; `None` is
    /// a targeted turn, which owns no run.
    #[error("{}", stopped_message(run_id))]
    Stopped {
        run_id: Option<Uuid>,
        attempt: Box<ReportAttemptMetadata>,
    },
}

/// What the reader stopped, in their words. A drafting run is picked back up
/// from the writing surface; a targeted turn simply did not happen.
fn stopped_message(run_id: &Option<Uuid>) -> &'static str {
    if run_id.is_some() {
        "The draft run was stopped."
    } else {
        "The writer turn was stopped."
    }
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
            ReportStoreError::InvalidRun(message) => Self::InvalidRun(message),
            ReportStoreError::Storage { operation, source } => Self::Storage { operation, source },
        }
    }
}

impl ReportPipelineError {
    pub fn storage(operation: &'static str, source: StorageError) -> Self {
        Self::Storage { operation, source }
    }

    /// The attempt receipt this turn recorded, for the caller's usage audit.
    /// A stopped turn has one too — it spent tokens before the Stop.
    pub fn attempt(&self) -> Option<&ReportAttemptMetadata> {
        match self {
            Self::TurnFailed { attempt, .. } | Self::Stopped { attempt, .. } => Some(attempt),
            _ => None,
        }
    }

    /// The drafting run a stop left parked and resumable.
    pub fn stopped_run_id(&self) -> Option<Uuid> {
        match self {
            Self::Stopped { run_id, .. } => *run_id,
            _ => None,
        }
    }

    /// Whether the reader stopped this turn, as opposed to it failing.
    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped { .. })
    }

    pub fn failure_code(&self) -> Option<ReportFailureCode> {
        match self {
            Self::TurnFailed { code, .. } => Some(*code),
            _ => None,
        }
    }
}
