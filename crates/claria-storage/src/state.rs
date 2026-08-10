use aws_sdk_s3::Client;
use serde::{de::DeserializeOwned, Serialize};

use crate::error::StorageError;
use crate::objects;

/// Load a JSON state file from S3. Returns the deserialized value and its ETag.
pub async fn load_state<T: DeserializeOwned>(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<(T, String), StorageError> {
    load_state_checked(client, bucket, key, None, |_| Ok(())).await
}

/// Load a JSON state file from S3, enforce an optional size bound, and run the
/// caller's domain validation over the decoded value before returning it with
/// its ETag.
///
/// This is the one implementation of the get → deserialize → validate pattern;
/// callers must not re-roll it. A validation failure maps to
/// [`StorageError::InvalidState`] carrying the closure's message, so callers
/// can surface it or translate it into their own error type.
pub async fn load_state_checked<T, V>(
    client: &Client,
    bucket: &str,
    key: &str,
    max_bytes: Option<u64>,
    validate: V,
) -> Result<(T, String), StorageError>
where
    T: DeserializeOwned,
    V: FnOnce(&T) -> Result<(), String>,
{
    let output = match max_bytes {
        Some(limit) => objects::get_object_bounded(client, bucket, key, limit).await?,
        None => objects::get_object(client, bucket, key).await?,
    };
    let value: T = serde_json::from_slice(&output.body).map_err(|e| {
        tracing::error!(bucket, key, error = %e, "failed to deserialize state from S3");
        StorageError::from(e)
    })?;
    validate(&value).map_err(|reason| StorageError::InvalidState {
        key: key.to_string(),
        reason,
    })?;
    Ok((value, output.etag.unwrap_or_default()))
}

/// Save a JSON state file to S3. Returns the new ETag.
pub async fn save_state<T: Serialize>(
    client: &Client,
    bucket: &str,
    key: &str,
    value: &T,
) -> Result<String, StorageError> {
    let body = serde_json::to_vec_pretty(value)?;
    objects::put_object(client, bucket, key, body, Some("application/json")).await
}

/// Save a JSON state file only if no current object exists at `key`.
pub async fn save_state_if_none_match<T: Serialize>(
    client: &Client,
    bucket: &str,
    key: &str,
    value: &T,
) -> Result<String, StorageError> {
    let body = serde_json::to_vec_pretty(value)?;
    objects::put_object_if_none_match(client, bucket, key, body, Some("application/json")).await
}

/// Save a JSON state file to S3 with ETag optimistic locking.
pub async fn save_state_if_match<T: Serialize>(
    client: &Client,
    bucket: &str,
    key: &str,
    value: &T,
    expected_etag: &str,
) -> Result<String, StorageError> {
    let body = serde_json::to_vec_pretty(value)?;
    objects::put_object_if_match(
        client,
        bucket,
        key,
        body,
        Some("application/json"),
        expected_etag,
    )
    .await
}
