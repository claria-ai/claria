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
    let output = objects::get_object(client, bucket, key).await?;
    let value: T = serde_json::from_slice(&output.body)?;
    let etag = output.etag.unwrap_or_default();
    Ok((value, etag))
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
