use claria_storage::error::StorageError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a report-writing attempt ended. Serialized inside persisted attempt
/// metadata, so the taxonomy belongs to the durable store rather than to
/// whichever crate happens to run the turn loop.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportFailureCode {
    Bedrock,
    ContextBudget,
    InvalidProtocol,
    RecordStorage,
    ToolRoundLimit,
    UnexpectedStop,
    WorkspaceConflict,
    UsagePersistence,
}

/// The one error type for durable writer state.
///
/// Messages are complete, user-facing sentences; callers stringify them once
/// at their own presentation boundary.
#[derive(Debug, Error)]
pub enum ReportStoreError {
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
}

impl ReportStoreError {
    pub fn storage(operation: &'static str, source: StorageError) -> Self {
        Self::Storage { operation, source }
    }
}
