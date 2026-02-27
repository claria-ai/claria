use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Unified API error type for all route handlers.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Unauthorized(String),
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            ApiError::Internal(msg) => {
                tracing::error!("internal error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };

        (status, Json(ErrorBody { error: message })).into_response()
    }
}

impl From<claria_storage::error::StorageError> for ApiError {
    fn from(e: claria_storage::error::StorageError) -> Self {
        match e {
            claria_storage::error::StorageError::NotFound { key } => {
                ApiError::NotFound(format!("object not found: {key}"))
            }
            other => ApiError::Internal(other.to_string()),
        }
    }
}

impl From<claria_search::error::SearchError> for ApiError {
    fn from(e: claria_search::error::SearchError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl From<claria_bedrock::error::BedrockError> for ApiError {
    fn from(e: claria_bedrock::error::BedrockError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl From<claria_export::error::ExportError> for ApiError {
    fn from(e: claria_export::error::ExportError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::BadRequest(e.to_string())
    }
}
