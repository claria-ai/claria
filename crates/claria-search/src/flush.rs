use std::path::Path;

use aws_sdk_s3::Client;
use tracing::info;

use claria_core::s3_keys;
use claria_storage::objects;

use crate::error::SearchError;

/// Compress the index directory to a tar.zst blob and upload to S3.
///
/// Uses `If-Match` with the provided ETag for optimistic locking.
/// Returns the new ETag on success.
pub async fn flush_index(
    client: &Client,
    bucket: &str,
    index_dir: &Path,
    expected_etag: &str,
) -> Result<String, SearchError> {
    info!("flushing Tantivy index to s3://{}/{}", bucket, s3_keys::INDEX);

    let blob = compress_index_dir(index_dir)?;

    let new_etag = objects::put_object_if_match(
        client,
        bucket,
        s3_keys::INDEX,
        blob,
        Some("application/zstd"),
        expected_etag,
    )
    .await
    .map_err(|e| match e {
        claria_storage::error::StorageError::PreconditionFailed { .. } => {
            SearchError::ETagMismatch
        }
        other => SearchError::Storage(other),
    })?;

    info!("index flushed, new etag={}", new_etag);
    Ok(new_etag)
}

/// Upload a fresh index (no ETag precondition). Used for initial index creation.
pub async fn flush_index_unconditional(
    client: &Client,
    bucket: &str,
    index_dir: &Path,
) -> Result<String, SearchError> {
    info!("uploading initial Tantivy index to s3://{}/{}", bucket, s3_keys::INDEX);

    let blob = compress_index_dir(index_dir)?;

    let etag = objects::put_object(
        client,
        bucket,
        s3_keys::INDEX,
        blob,
        Some("application/zstd"),
    )
    .await?;

    info!("initial index uploaded, etag={}", etag);
    Ok(etag)
}

/// Compress an index directory into a tar.zst byte vector.
fn compress_index_dir(index_dir: &Path) -> Result<Vec<u8>, SearchError> {
    let mut buf = Vec::new();
    {
        let encoder = zstd::Encoder::new(&mut buf, 3)?;
        let mut tar_builder = tar::Builder::new(encoder);
        tar_builder.append_dir_all(".", index_dir)?;
        let encoder = tar_builder.into_inner()?;
        encoder.finish()?;
    }
    Ok(buf)
}
