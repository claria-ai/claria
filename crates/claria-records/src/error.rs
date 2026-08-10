use claria_storage::error::StorageError;
use thiserror::Error;

/// The one error type for client records and their lifecycle.
///
/// Messages are complete, user-facing sentences: `claria-desktop` stringifies
/// them once at the Tauri boundary.
#[derive(Debug, Error)]
pub enum RecordsError {
    #[error("Client name cannot be empty.")]
    EmptyClientName,

    #[error("Client name cannot exceed {0} characters.")]
    ClientNameTooLong(usize),

    /// A stored client record does not match the storage key it was read
    /// from (or the requested ID is the nil UUID).
    #[error("The client record does not match its storage key.")]
    InvalidClient,

    #[error("Client record is missing its S3 version identifier; reload and try again.")]
    MissingConcurrencyToken,

    #[error("This client record changed on another computer. Reload it before renaming.")]
    ConcurrentClientEdit,

    #[error("The client deletion recovery state is invalid.")]
    InvalidRecoveryState,

    #[error("Client storage is unavailable while {operation}: {source}")]
    Storage {
        operation: &'static str,
        #[source]
        source: StorageError,
    },

    #[error("The report workspace could not be restored safely.")]
    ReportRestore {
        #[source]
        source: claria_report_authoring::ReportAuthoringError,
    },

    #[error("Client deletion failed and was rolled back safely.")]
    DeletionRolledBack {
        #[source]
        source: Box<RecordsError>,
    },

    #[error(
        "Client deletion stopped and automatic recovery is incomplete. Retry the deletion before editing this client."
    )]
    RecoveryIncomplete {
        #[source]
        source: Box<RecordsError>,
    },

    #[error("A record could not be encoded or decoded: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("The record cache is unavailable.")]
    CacheUnavailable,
}

/// Attach an operation name to a storage failure.
pub(crate) fn storage(operation: &'static str, source: StorageError) -> RecordsError {
    RecordsError::Storage { operation, source }
}
