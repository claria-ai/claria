use aws_sdk_s3::Client;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_smithy_types::byte_stream::ByteStream;
use std::time::Duration;

use crate::error::StorageError;

/// Result of a GET operation, including the body and ETag.
pub struct GetObjectOutput {
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub content_type: Option<String>,
}

/// Get an object from S3.
pub async fn get_object(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<GetObjectOutput, StorageError> {
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| {
            let err = e.into_service_error();
            if err.is_no_such_key() {
                StorageError::NotFound {
                    key: key.to_string(),
                }
            } else {
                StorageError::GetObject(err.to_string())
            }
        })?;

    let etag = resp.e_tag().map(|s| s.to_string());
    let content_type = resp.content_type().map(|s| s.to_string());
    let body = resp
        .body
        .collect()
        .await
        .map_err(|e| StorageError::GetObject(e.to_string()))?
        .into_bytes()
        .to_vec();

    Ok(GetObjectOutput {
        body,
        etag,
        content_type,
    })
}

/// Put an object to S3. Returns the new ETag.
pub async fn put_object(
    client: &Client,
    bucket: &str,
    key: &str,
    body: Vec<u8>,
    content_type: Option<&str>,
) -> Result<String, StorageError> {
    let mut req = client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from(body));

    if let Some(ct) = content_type {
        req = req.content_type(ct);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| StorageError::PutObject(e.into_service_error().to_string()))?;

    Ok(resp.e_tag().unwrap_or_default().to_string())
}

/// Put an object to S3 with an If-Match precondition (ETag optimistic locking).
/// Returns the new ETag on success, or `StorageError::PreconditionFailed` if the
/// ETag doesn't match.
pub async fn put_object_if_match(
    client: &Client,
    bucket: &str,
    key: &str,
    body: Vec<u8>,
    content_type: Option<&str>,
    expected_etag: &str,
) -> Result<String, StorageError> {
    let mut req = client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from(body))
        .if_match(expected_etag);

    if let Some(ct) = content_type {
        req = req.content_type(ct);
    }

    let resp = req.send().await.map_err(|e| {
        let err = e.into_service_error();
        // S3 returns 412 Precondition Failed when If-Match doesn't match
        if err
            .to_string()
            .contains("PreconditionFailed")
        {
            StorageError::PreconditionFailed {
                key: key.to_string(),
            }
        } else {
            StorageError::PutObject(err.to_string())
        }
    })?;

    Ok(resp.e_tag().unwrap_or_default().to_string())
}

/// Delete an object from S3.
pub async fn delete_object(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<(), StorageError> {
    client
        .delete_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| StorageError::DeleteObject(e.into_service_error().to_string()))?;

    Ok(())
}

/// Delete all objects under a prefix.
///
/// Lists all keys with the given prefix and deletes each one.
/// Returns the number of objects deleted.
pub async fn delete_objects_by_prefix(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> Result<usize, StorageError> {
    let keys = list_objects(client, bucket, prefix).await?;
    let count = keys.len();
    for key in &keys {
        delete_object(client, bucket, key).await?;
    }
    Ok(count)
}

/// Metadata for a single S3 object, returned by [`list_objects_with_metadata`].
pub struct ObjectMeta {
    pub key: String,
    pub size: i64,
    pub last_modified: Option<String>,
}

/// List objects under a prefix with size and last-modified metadata.
pub async fn list_objects_with_metadata(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<ObjectMeta>, StorageError> {
    let mut objects = Vec::new();
    let mut continuation_token: Option<String> = None;

    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix);

        if let Some(token) = &continuation_token {
            req = req.continuation_token(token);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| StorageError::ListObjects(e.into_service_error().to_string()))?;

        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                objects.push(ObjectMeta {
                    key: key.to_string(),
                    size: obj.size().unwrap_or(0),
                    last_modified: obj.last_modified().map(|t| t.to_string()),
                });
            }
        }

        if resp.is_truncated() == Some(true) {
            continuation_token = resp.next_continuation_token().map(|s| s.to_string());
        } else {
            break;
        }
    }

    Ok(objects)
}

/// List objects under a prefix. Returns keys.
pub async fn list_objects(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<String>, StorageError> {
    let mut keys = Vec::new();
    let mut continuation_token: Option<String> = None;

    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix);

        if let Some(token) = &continuation_token {
            req = req.continuation_token(token);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| StorageError::ListObjects(e.into_service_error().to_string()))?;

        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                keys.push(key.to_string());
            }
        }

        if resp.is_truncated() == Some(true) {
            continuation_token = resp.next_continuation_token().map(|s| s.to_string());
        } else {
            break;
        }
    }

    Ok(keys)
}

/// Generate a presigned GET URL for an object.
pub async fn presign_get(
    client: &Client,
    bucket: &str,
    key: &str,
    expires_in: Duration,
) -> Result<String, StorageError> {
    let presign_config = PresigningConfig::builder()
        .expires_in(expires_in)
        .build()
        .map_err(|e| StorageError::Presign(e.to_string()))?;

    let presigned = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .presigned(presign_config)
        .await
        .map_err(|e| StorageError::Presign(e.to_string()))?;

    Ok(presigned.uri().to_string())
}

/// Generate a presigned PUT URL for uploading an object.
pub async fn presign_put(
    client: &Client,
    bucket: &str,
    key: &str,
    content_type: Option<&str>,
    expires_in: Duration,
) -> Result<String, StorageError> {
    let presign_config = PresigningConfig::builder()
        .expires_in(expires_in)
        .build()
        .map_err(|e| StorageError::Presign(e.to_string()))?;

    let mut req = client.put_object().bucket(bucket).key(key);

    if let Some(ct) = content_type {
        req = req.content_type(ct);
    }

    let presigned = req
        .presigned(presign_config)
        .await
        .map_err(|e| StorageError::Presign(e.to_string()))?;

    Ok(presigned.uri().to_string())
}
