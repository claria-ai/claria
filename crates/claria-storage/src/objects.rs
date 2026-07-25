use aws_sdk_s3::{presigning::PresigningConfig, Client};
use aws_smithy_types::byte_stream::ByteStream;
use std::time::Duration;

use crate::error::StorageError;

/// Result of a GET operation, including the body and ETag.
pub struct GetObjectOutput {
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub content_type: Option<String>,
}

// Timing spans below log S3 keys/prefixes (already visible in-app) plus byte
// counts and elapsed_ms — never object contents, which may hold PHI.

/// Get an object from S3.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(bucket = %bucket, key = %key, bytes = tracing::field::Empty)
)]
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
                let msg = err.to_string();
                tracing::error!(bucket, key, error = %msg, "S3 GetObject failed");
                StorageError::GetObject(msg)
            }
        })?;

    let etag = resp.e_tag().map(|s| s.to_string());
    let content_type = resp.content_type().map(|s| s.to_string());
    let body = resp
        .body
        .collect()
        .await
        .map_err(|e| {
            tracing::error!(bucket, key, error = %e, "S3 GetObject body stream failed");
            StorageError::GetObject(e.to_string())
        })?
        .into_bytes()
        .to_vec();

    tracing::Span::current().record("bytes", body.len() as u64);

    Ok(GetObjectOutput {
        body,
        etag,
        content_type,
    })
}

/// Put an object to S3. Returns the new ETag.
#[tracing::instrument(level = "trace", skip_all, fields(bucket = %bucket, key = %key, bytes = body.len()))]
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
        .map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::error!(bucket, key, error = %msg, "S3 PutObject failed");
            StorageError::PutObject(msg)
        })?;

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
        // S3 answers an If-Match miss with 412 Precondition Failed. Read the
        // structured status and error code rather than sniffing the Display
        // string, which is free to change with any SDK release.
        let status_412 = e
            .raw_response()
            .is_some_and(|r| r.status().as_u16() == 412);
        let err = e.into_service_error();

        if status_412 || err.meta().code() == Some("PreconditionFailed") {
            StorageError::PreconditionFailed {
                key: key.to_string(),
            }
        } else {
            let msg = err.to_string();
            tracing::error!(bucket, key, error = %msg, "S3 PutObject (if-match) failed");
            StorageError::PutObject(msg)
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
        .map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::error!(bucket, key, error = %msg, "S3 DeleteObject failed");
            StorageError::DeleteObject(msg)
        })?;

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
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(bucket = %bucket, prefix = %prefix, count = tracing::field::Empty)
)]
pub async fn list_objects_with_metadata(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<ObjectMeta>, StorageError> {
    let mut objects = Vec::new();
    let mut pages = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(prefix)
        .into_paginator()
        .send();

    while let Some(page) = pages.next().await {
        let page = page.map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::error!(bucket, prefix, error = %msg, "S3 ListObjectsV2 failed");
            StorageError::ListObjects(msg)
        })?;

        for obj in page.contents() {
            if let Some(key) = obj.key() {
                objects.push(ObjectMeta {
                    key: key.to_string(),
                    size: obj.size().unwrap_or(0),
                    last_modified: obj.last_modified().map(|t| t.to_string()),
                });
            }
        }
    }

    tracing::Span::current().record("count", objects.len() as u64);

    Ok(objects)
}

/// List objects under a prefix. Returns keys.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(bucket = %bucket, prefix = %prefix, count = tracing::field::Empty)
)]
pub async fn list_objects(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<String>, StorageError> {
    let mut keys = Vec::new();
    let mut pages = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(prefix)
        .into_paginator()
        .send();

    while let Some(page) = pages.next().await {
        let page = page.map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::error!(bucket, prefix, error = %msg, "S3 ListObjectsV2 failed");
            StorageError::ListObjects(msg)
        })?;

        for obj in page.contents() {
            if let Some(key) = obj.key() {
                keys.push(key.to_string());
            }
        }
    }

    tracing::Span::current().record("count", keys.len() as u64);

    Ok(keys)
}

/// List objects under a prefix, returning `(key, etag)` pairs in listing order.
///
/// The ETag is the freshness probe for the record cache: it comes back on the
/// `ListObjectsV2` response the listing already makes, so a cached body can be
/// revalidated without a GET. When `e_tag()` is absent the etag is
/// `String::new()`, which never matches a cache entry — a safe miss.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(bucket = %bucket, prefix = %prefix, count = tracing::field::Empty)
)]
pub async fn list_object_etags(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<(String, String)>, StorageError> {
    let mut pairs = Vec::new();
    let mut pages = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(prefix)
        .into_paginator()
        .send();

    while let Some(page) = pages.next().await {
        let page = page.map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::error!(bucket, prefix, error = %msg, "S3 ListObjectsV2 failed");
            StorageError::ListObjects(msg)
        })?;

        for obj in page.contents() {
            if let Some(key) = obj.key() {
                pairs.push((
                    key.to_string(),
                    obj.e_tag().unwrap_or_default().to_string(),
                ));
            }
        }
    }

    tracing::Span::current().record("count", pairs.len() as u64);

    Ok(pairs)
}

// ---------------------------------------------------------------------------
// Versioning operations
// ---------------------------------------------------------------------------

/// Metadata for a single version of an S3 object.
pub struct ObjectVersion {
    pub version_id: String,
    pub size: i64,
    pub last_modified: Option<String>,
    pub is_latest: bool,
    pub is_delete_marker: bool,
}

/// A deleted object identified by its key and the delete-marker version ID.
pub struct DeletedObject {
    pub key: String,
    pub version_id: String,
    pub last_modified: Option<String>,
}

/// Hand each page of `ListObjectVersions` under `prefix` to `on_page`.
///
/// The AWS SDK ships no paginator for this operation — unlike ListObjectsV2,
/// ListObjectVersions isn't marked paginated in the S3 model — so the
/// key/version marker walk stays hand-rolled, but only in one place.
async fn for_each_version_page<F>(
    client: &Client,
    bucket: &str,
    prefix: &str,
    mut on_page: F,
) -> Result<(), StorageError>
where
    F: FnMut(&aws_sdk_s3::operation::list_object_versions::ListObjectVersionsOutput),
{
    let mut key_marker: Option<String> = None;
    let mut version_id_marker: Option<String> = None;

    loop {
        let mut req = client.list_object_versions().bucket(bucket).prefix(prefix);

        if let Some(km) = &key_marker {
            req = req.key_marker(km);
        }
        if let Some(vm) = &version_id_marker {
            req = req.version_id_marker(vm);
        }

        let resp = req.send().await.map_err(|e| {
            let msg = e.into_service_error().to_string();
            tracing::error!(bucket, prefix, error = %msg, "S3 ListObjectVersions failed");
            StorageError::ListObjectVersions(msg)
        })?;

        on_page(&resp);

        if resp.is_truncated() == Some(true) {
            key_marker = resp.next_key_marker().map(|s| s.to_string());
            version_id_marker = resp.next_version_id_marker().map(|s| s.to_string());
        } else {
            return Ok(());
        }
    }
}

/// List all versions of a specific object (identified by exact key).
pub async fn list_object_versions(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<Vec<ObjectVersion>, StorageError> {
    let mut versions = Vec::new();

    for_each_version_page(client, bucket, key, |page| {
        // Only include entries for the exact key (prefix match may return more).
        versions.extend(page.versions().iter().filter(|v| v.key() == Some(key)).map(
            |v| ObjectVersion {
                version_id: v.version_id().unwrap_or_default().to_string(),
                size: v.size().unwrap_or(0),
                last_modified: v.last_modified().map(|t| t.to_string()),
                is_latest: v.is_latest().unwrap_or(false),
                is_delete_marker: false,
            },
        ));

        versions.extend(
            page.delete_markers()
                .iter()
                .filter(|dm| dm.key() == Some(key))
                .map(|dm| ObjectVersion {
                    version_id: dm.version_id().unwrap_or_default().to_string(),
                    size: 0,
                    last_modified: dm.last_modified().map(|t| t.to_string()),
                    is_latest: dm.is_latest().unwrap_or(false),
                    is_delete_marker: true,
                }),
        );
    })
    .await?;

    Ok(versions)
}

/// List objects under a prefix that have been deleted (have a delete marker as the latest version).
pub async fn list_deleted_objects(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<DeletedObject>, StorageError> {
    let mut deleted = Vec::new();

    for_each_version_page(client, bucket, prefix, |page| {
        deleted.extend(
            page.delete_markers()
                .iter()
                .filter(|dm| dm.is_latest().unwrap_or(false))
                .filter_map(|dm| {
                    Some(DeletedObject {
                        key: dm.key()?.to_string(),
                        version_id: dm.version_id().unwrap_or_default().to_string(),
                        last_modified: dm.last_modified().map(|t| t.to_string()),
                    })
                }),
        );
    })
    .await?;

    Ok(deleted)
}

/// Get the first (oldest, non-delete-marker) version of an object.
///
/// Useful when the caller wants the "original" of a versioned object — e.g.
/// restoring a Transcribe-generated transcript after a user has edited it.
pub async fn get_first_version(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<GetObjectOutput, StorageError> {
    let mut versions = list_object_versions(client, bucket, key).await?;
    versions.retain(|v| !v.is_delete_marker);
    versions.sort_by(|a, b| a.last_modified.cmp(&b.last_modified));

    let first = versions.first().ok_or_else(|| StorageError::NotFound {
        key: key.to_string(),
    })?;

    get_object_version(client, bucket, key, &first.version_id).await
}

/// Get a specific version of an object.
pub async fn get_object_version(
    client: &Client,
    bucket: &str,
    key: &str,
    version_id: &str,
) -> Result<GetObjectOutput, StorageError> {
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .version_id(version_id)
        .send()
        .await
        .map_err(|e| {
            let err = e.into_service_error();
            if err.is_no_such_key() {
                StorageError::NotFound {
                    key: key.to_string(),
                }
            } else {
                let msg = err.to_string();
                tracing::error!(bucket, key, version_id, error = %msg, "S3 GetObject (versioned) failed");
                StorageError::GetObject(msg)
            }
        })?;

    let etag = resp.e_tag().map(|s| s.to_string());
    let content_type = resp.content_type().map(|s| s.to_string());
    let body = resp
        .body
        .collect()
        .await
        .map_err(|e| {
            tracing::error!(bucket, key, version_id, error = %e, "S3 GetObject (versioned) body stream failed");
            StorageError::GetObject(e.to_string())
        })?
        .into_bytes()
        .to_vec();

    Ok(GetObjectOutput {
        body,
        etag,
        content_type,
    })
}

// ---------------------------------------------------------------------------
// Presigning
// ---------------------------------------------------------------------------

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
        .map_err(|e| {
            tracing::warn!(bucket, key, error = %e, "S3 presign GET failed");
            StorageError::Presign(e.to_string())
        })?;

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
        .map_err(|e| {
            tracing::warn!(bucket, key, error = %e, "S3 presign PUT failed");
            StorageError::Presign(e.to_string())
        })?;

    Ok(presigned.uri().to_string())
}
