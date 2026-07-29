use claria_storage::error::StorageError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ReportAttemptMetadata;

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

#[derive(Debug, Error)]
pub enum ReportAuthoringError {
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

impl ReportAuthoringError {
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
