//! Error sidecar persistence: observe once, save text, then conditionally clear.
use claria_core::processing::{MAX_ERROR_BYTES, ProcessingError, ProcessingFailureCode};
use claria_storage::{error::StorageError, objects};

pub struct ObservedError {
    pub marker: Option<ProcessingError>,
    etag: Option<String>,
    occupied: bool,
}

pub async fn observe_error(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<ObservedError, StorageError> {
    match objects::get_object_bounded(s3, bucket, &format!("{key}.error"), MAX_ERROR_BYTES).await {
        Ok(output) => Ok(ObservedError {
            marker: ProcessingError::parse(&output.body),
            etag: output.etag,
            occupied: true,
        }),
        Err(StorageError::NotFound { .. }) => Ok(ObservedError {
            marker: None,
            etag: None,
            occupied: false,
        }),
        Err(StorageError::ObjectTooLarge { .. }) => Ok(ObservedError {
            marker: None,
            etag: None,
            occupied: true,
        }),
        Err(error) => Err(error),
    }
}

/// False means a user file or a concurrent revision was preserved.
pub async fn save_error(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    observed: &ObservedError,
    code: ProcessingFailureCode,
) -> Result<bool, StorageError> {
    let body = serde_json::to_vec(&ProcessingError::new(code))?;
    let key = format!("{key}.error");
    let result = if !observed.occupied {
        objects::put_object_if_none_match(s3, bucket, &key, body, Some("application/json")).await
    } else if observed.marker.is_some() {
        let Some(etag) = observed.etag.as_deref() else {
            return Ok(false);
        };
        objects::put_object_if_match(s3, bucket, &key, body, Some("application/json"), etag).await
    } else {
        return Ok(false);
    };
    match result {
        Ok(_) => Ok(true),
        Err(StorageError::PreconditionFailed { .. }) => Ok(false),
        Err(error) => Err(error),
    }
}

pub async fn clear_observed_error(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    observed: &ObservedError,
) -> Result<(), StorageError> {
    if observed.marker.is_none() {
        return Ok(());
    }
    let Some(etag) = observed.etag.as_deref() else {
        return Ok(());
    };
    match objects::delete_object_if_match(s3, bucket, &format!("{key}.error"), etag).await {
        Ok(()) | Err(StorageError::PreconditionFailed { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}
