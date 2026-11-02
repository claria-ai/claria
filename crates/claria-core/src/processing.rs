//! Stateless extraction outcomes. Only this versioned, validated envelope is
//! an internal error artifact; an ordinary user `.error` file remains a record.
use serde::{Deserialize, Serialize};

pub const MAX_ERROR_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingFailureCode {
    ModelNotConfigured,
    ProviderUnavailable,
    AccessDenied,
    ModelUnavailable,
    Throttled,
    Truncated,
    InvalidText,
    StorageUnavailable,
}
impl ProcessingFailureCode {
    pub fn reason(self) -> &'static str {
        match self {
            Self::ModelNotConfigured => {
                "Choose a document extraction model in Preferences, then retry."
            }
            Self::AccessDenied => {
                "AWS denied access to text processing. Check permissions and model access in the active account and region, then retry."
            }
            Self::ModelUnavailable => {
                "The extraction model is unavailable. Choose an available model in Preferences, then retry."
            }
            Self::Throttled => "AWS limited this processing request. Wait briefly, then retry.",
            Self::Truncated => {
                "The model stopped before extracting the whole document. Split the document or choose another model, then retry."
            }
            Self::ProviderUnavailable => {
                "Text processing could not complete. Check model access and your connection, then retry."
            }
            Self::InvalidText => {
                "Processing returned no usable text. Choose another extraction model, then retry."
            }
            Self::StorageUnavailable => {
                "Processed text could not be saved. Check storage access and your connection, then retry."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingError {
    pub kind: String,
    pub version: u32,
    pub code: ProcessingFailureCode,
    pub reason: String,
    pub revision: uuid::Uuid,
}
impl ProcessingError {
    pub fn new(code: ProcessingFailureCode) -> Self {
        Self {
            kind: "claria_processing_error".into(),
            version: 1,
            code,
            reason: code.reason().into(),
            revision: uuid::Uuid::new_v4(),
        }
    }
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() as u64 > MAX_ERROR_BYTES {
            return None;
        }
        let value: Self = serde_json::from_slice(bytes).ok()?;
        (value.kind == "claria_processing_error"
            && value.version == 1
            && value.reason == value.code.reason()
            && !value.revision.is_nil())
        .then_some(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProcessingStatus {
    Ready,
    NotSupported,
    Unknown,
    Failed {
        code: ProcessingFailureCode,
        reason: String,
    },
}

pub fn usable_text(bytes: &[u8]) -> Option<&str> {
    crate::record_text::decode_record_text(bytes).filter(|text| !text.trim().is_empty())
}
